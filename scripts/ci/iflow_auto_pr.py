#!/usr/bin/env python3
import json
import os
import re
import subprocess
import shlex
import sys
import textwrap
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

ALLOWED_TYPES = {
    "refactor",
    "perf",
    "tests",
    "docs",
    "ci",
    "android-ui",
    "packaging",
    "bugfix",
}

MAX_PRS = 10
MAX_FILES = 0
MAX_REPAIR_ATTEMPTS = int(os.environ.get("IFLOW_REPAIR_ATTEMPTS", "1"))
TOOL_FALLBACK = os.environ.get("IFLOW_TOOL_FALLBACK", "1") == "1"
FORCE_EDIT = os.environ.get("IFLOW_FORCE_EDIT", "1") == "1"
RUN_ID = os.environ.get("GITHUB_RUN_ID") or str(os.getpid())
DIRECT_GIT = os.environ.get("IFLOW_DIRECT_GIT") == "1"


def run(cmd, check=True, capture=False, text=True, **kwargs):
    if capture:
        return subprocess.check_output(cmd, text=text, **kwargs).strip()
    subprocess.run(cmd, check=check, **kwargs)
    return ""


def git(*args, capture=False, **kwargs):
    return run(["git", *args], capture=capture, **kwargs)


def status_lines():
    raw = git("status", "--porcelain", capture=True)
    return [line for line in raw.splitlines() if line.strip()]


def status_paths(lines):
    paths = []
    for line in lines:
        parts = line.split()
        if not parts:
            continue
        paths.append(parts[-1])
    return paths


def build_prompt():
    top_level = sorted([p.name for p in ROOT.iterdir() if p.name != ".git"])
    files_preview = "\n".join(top_level)

    allowed = ", ".join(sorted(ALLOWED_TYPES))
    prompt = """
You are an automated refactoring bot for the repo at {root}. You may propose up to {max_prs} independent pull requests.
Each PR must be ONE category only from: {allowed}.
You can modify any text source file except secrets or generated artifacts.
Do NOT touch: .git/, target/, dist/, build outputs, or any secrets/keys.
You may modify any number of files.
If a change would exceed limits, split it into a separate PR or skip it.
Use tools to inspect files when necessary; do not assume file contents.

Do NOT output JSON or patches to stdout. Make concrete file edits only.

Rules:
- Tools are allowed, but only modify files within the repo workspace.
- Never push directly to main/master; only create PR branches.
- Keep changes organized by category: docs, tests, ci, android-ui, packaging, bugfix, refactor, perf.
- Write a plan file at .iflow_pr_plan.json with a list of PRs and their file lists. This is required.
  The automation will use your plan to split PRs.

Top-level entries:
{files_preview}
""".format(
        root=ROOT,
        max_prs=MAX_PRS,
        allowed=allowed,
        files_preview=files_preview,
    )
    prompt = textwrap.dedent(prompt).strip()
    if DIRECT_GIT:
        prompt += (
            "\n\nDirect Git Mode:\n"
            "- You may run git/gh commands to create PRs yourself.\n"
            "- Never push to main/master.\n"
            "- Use branches prefixed with iflow/.\n"
        )
    else:
        prompt += (
            "\n\nManaged PR Mode:\n"
            "- Do NOT run git commands or create PRs yourself.\n"
            "- The automation will create PRs from .iflow_pr_plan.json.\n"
        )
    return prompt


def extract_json(text):
    marker_match = re.search(r"BEGIN_JSON([\s\S]*?)END_JSON", text)
    if marker_match:
        payload = marker_match.group(1).strip()
        try:
            return json.loads(payload)
        except json.JSONDecodeError:
            return None
    match = re.search(r"\{[\s\S]*\}", text)
    if not match:
        return None
    try:
        return json.loads(match.group(0))
    except json.JSONDecodeError:
        return None


def validate_pr(pr):
    required = {"title", "branch_name", "type", "rationale", "self_proof", "self_review", "tests", "patch"}
    if not required.issubset(pr):
        return False, "missing required fields"
    if pr["type"] not in ALLOWED_TYPES:
        return False, f"invalid type {pr['type']}"
    if not pr["patch"].strip():
        return False, "empty patch"
    return True, ""


def normalize_patch(text):
    # Remove invalid git metadata lines and normalize line endings.
    cleaned = []
    for line in text.replace("\r\n", "\n").replace("\r", "\n").splitlines():
        if line.startswith("index "):
            continue
        cleaned.append(line)
    return "\n".join(cleaned).rstrip() + "\n"


def extract_patch(text):
    match = re.search(r"^diff --git[\\s\\S]*", text, flags=re.M)
    if not match:
        return None
    patch = match.group(0)
    return normalize_patch(patch)


def run_iflow(prompt, model, max_turns, timeout, out_path):
    iflow_cmd = [
        "iflow",
        "-m",
        model,
        "--thinking",
        "--yolo",
        "--max-turns",
        str(max_turns),
        "--timeout",
        str(timeout),
        "--checkpointing",
        "false",
        "-o",
        str(out_path),
        "-p",
        prompt,
    ]
    if os.environ.get("IFLOW_DEBUG") == "1":
        iflow_cmd.insert(1, "--debug")
    cmd_str = shlex.join(iflow_cmd)
    outer_timeout = int(os.environ.get("IFLOW_OUTER_TIMEOUT", str(timeout + 120)))
    env = os.environ.copy()
    env.setdefault("GIT_TERMINAL_PROMPT", "0")
    env.setdefault("SUDO_ASKPASS", "/bin/false")
    env.setdefault("SUDO_ASKPASS_REQUIRE", "force")
    env.setdefault("SUDO_PROMPT", "[sudo blocked] ")
    if env.get("IFLOW_DISABLE_SUDO") == "1":
        stub_dir = tempfile.mkdtemp(prefix="nosudo-")
        stub_path = Path(stub_dir) / "sudo"
        stub_path.write_text("#!/bin/sh\n" "echo 'sudo disabled in iflow automation' >&2\n" "exit 1\n")
        stub_path.chmod(0o755)
        env["PATH"] = f"{stub_dir}:{env.get('PATH','')}"
    cmd = ["script", "-q", "-c", cmd_str, "/dev/null"]
    if outer_timeout > 0:
        cmd = ["timeout", str(outer_timeout)] + cmd
    output = subprocess.check_output(cmd, text=True, env=env)
    return output


def apply_check(patch_path):
    result = subprocess.run(
        ["git", "apply", "--check", patch_path],
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        return True, ""
    err = (result.stderr or result.stdout or "").strip()
    return False, err


def assert_not_main_branch():
    branch = git("rev-parse", "--abbrev-ref", "HEAD", capture=True)
    if branch in {"main", "master"}:
        raise RuntimeError(f"Refusing to push from protected branch: {branch}")


def write_plan_md(text):
    path = ROOT / "IFLOW_PLAN.md"
    path.write_text(text)
    return path


def classify_file(path):
    if path.startswith("android/"):
        return "android-ui"
    if path.startswith(".github/"):
        return "ci"
    if path.startswith("packaging/") or path in {"deploy.sh"} or path.startswith("etc/"):
        return "packaging"
    if path.startswith("tests/") or path.endswith("_test.rs") or path.endswith("/tests.rs"):
        return "tests"
    if path.endswith(".md") or path.startswith("docs/"):
        return "docs"
    return "refactor"


def build_prs_from_categories(changed_files):
    groups = {}
    for path in changed_files:
        category = classify_file(path)
        groups.setdefault(category, []).append(path)
    prs = []
    for idx, (category, files) in enumerate(groups.items(), 1):
        title = f"{category}: update {len(files)} file(s)"
        prs.append({
            "title": title,
            "branch_name": f"{category}-{idx}",
            "type": category,
            "files": files,
            "rationale": [f"Grouped {category} changes for focused review."],
            "self_proof": [f"Only {category} files were modified: {len(files)} file(s)."],
            "self_review": ["Reviewed diffs for scope; no functional review beyond category grouping."],
            "tests": ["Not run (automation)."],
        })
    return prs


def load_plan_prs():
    plan_path = ROOT / ".iflow_pr_plan.json"
    if not plan_path.exists():
        return None
    try:
        data = json.loads(plan_path.read_text())
    except json.JSONDecodeError:
        return None
    if isinstance(data, dict):
        prs = data.get("prs")
    else:
        prs = data
    if not isinstance(prs, list) or not prs:
        return None
    return prs


def create_fallback_pr():
    changed_files = [f for f in git("diff", "--name-only", capture=True).splitlines() if f.strip()]
    if not changed_files:
        return False
    if MAX_FILES and len(changed_files) > MAX_FILES:
        print(f"Skipping fallback PR: too many files changed ({len(changed_files)}).")
        return False
    forbidden_prefixes = (".git/", "target/", "dist/", "build/")
    for f in changed_files:
        if f.startswith(forbidden_prefixes):
            print(f"Skipping fallback PR: forbidden path changed ({f}).")
            return False
    branch = f"iflow/workspace-{os.getpid()}"
    title = "auto: apply iflow workspace changes"
    stat = git("diff", "--stat", capture=True)
    body = "\n".join([
        "Type: auto",
        "",
        "Summary:",
        "```\n" + (stat.strip() or "(no diff)") + "\n```",
        "",
        "Generated by iFlow nightly automation (workspace fallback).",
    ])

    git("checkout", "-b", branch)
    assert_not_main_branch()
    git("add", "-A")
    git("commit", "-m", title)
    git("push", "-u", "origin", branch)
    run([
        "gh",
        "pr",
        "create",
        "--title",
        title,
        "--body",
        body,
        "--base",
        "main",
        "--head",
        branch,
    ])
    print("Created 1 fallback PR.")
    return True


def create_prs_from_file_lists(prs, temp_branch, changed_files):
    changed_set = set(changed_files)
    used = set()
    created = 0
    for idx, pr in enumerate(prs, 1):
        files = pr.get("files") or []
        if not isinstance(files, list) or not files:
            print(f"Skipping PR {idx}: missing files list")
            continue
        files = [f for f in files if f in changed_set]
        if not files:
            print(f"Skipping PR {idx}: no files to include")
            continue
        overlap = used.intersection(files)
        if overlap:
            print(f"Skipping PR {idx}: files overlap with earlier PRs: {sorted(overlap)[:5]}")
            continue
        used.update(files)

        branch = f"iflow/{sanitize_branch(pr.get('branch_name', f'auto-{idx}'), idx)}-{RUN_ID}"
        title = pr.get("title", f"auto: change set {idx}").strip()[:72]
        body = "\n".join([
            f"Type: {pr.get('type','auto')}",
            "",
            "Rationale:",
            *[f"- {r}" for r in pr.get("rationale", [])],
            "",
            "Self-proof:",
            *[f"- {r}" for r in pr.get("self_proof", [])],
            "",
            "Self-review:",
            *[f"- {r}" for r in pr.get("self_review", [])],
            "",
            "Tests:",
            *[f"- {t}" for t in pr.get("tests", [])],
            "",
            "Generated by iFlow nightly automation (tool-mode split).",
        ])

        git("checkout", "-b", branch, "origin/main")
        assert_not_main_branch()
        git("checkout", temp_branch, "--", *files)
        git("add", *files)
        git("commit", "-m", f"{pr.get('type','auto')}: {title}")
        git("push", "-u", "origin", branch)
        run([
            "gh",
            "pr",
            "create",
            "--title",
            title,
            "--body",
            body,
            "--base",
            "main",
            "--head",
            branch,
        ])
        created += 1
        git("checkout", "main")
        git("reset", "--hard", "origin/main")
        git("clean", "-fd")

    return created


def maybe_write_iflow_context():
    if os.environ.get("IFLOW_WRITE_CONTEXT") != "1":
        return None, None, None
    path = ROOT / "IFLOW.md"
    original = path.read_text() if path.exists() else None
    content = os.environ.get("IFLOW_CONTEXT")
    if not content:
        content = textwrap.dedent(
            f"""            # iFlow Auto-PR Context

            You are an automated refactoring bot running in GitHub Actions for {ROOT}.
            Goal: propose safe PRs; size is less important than correctness.
            Consult IFLOW_PLAN.md if present, then write .iflow_pr_plan.json describing PR splits.
            Each PR must be a single category and include a clean unified diff.
            Do not use sudo or interactive prompts.
            Tools are allowed, but only modify files within the repo.
            Avoid writing patch files; prefer direct file edits or JSON diffs.
            """
        ).strip()
    path.write_text(content)
    return path, original, content



def sanitize_branch(name, idx):
    cleaned = re.sub(r"[^A-Za-z0-9._-]+", "-", name).strip("-")
    return cleaned if cleaned else f"auto-pr-{idx}"


def count_patch_stats(patch_path):
    try:
        stats = run(["git", "apply", "--numstat", patch_path], capture=True)
    except subprocess.CalledProcessError:
        return None, None
    files = 0
    lines = 0
    for line in stats.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) >= 3:
            add, delete = parts[0], parts[1]
            try:
                lines += int(add) + int(delete)
            except ValueError:
                lines += 0
            files += 1
    return files, lines


def main():
    os.chdir(ROOT)
    if not os.environ.get("IFLOW_API_KEY"):
        print("IFLOW_API_KEY not set; aborting.")
        return 1
    dry_run = os.environ.get("IFLOW_DRY_RUN") == "1"

    prompt = build_prompt()
    max_turns = int(os.environ.get("IFLOW_MAX_TURNS", "20"))
    timeout = int(os.environ.get("IFLOW_TIMEOUT", "1800"))
    model = os.environ.get("IFLOW_MODEL", "glm-4.7")
    plan_mode = os.environ.get("IFLOW_PLAN_MODE") == "1"

    ctx_path, ctx_original, ctx_default = maybe_write_iflow_context()

    def cleanup_context():
        if ctx_path and ctx_path.exists():
            ctx_path.unlink()

    # Optional ping to validate auth/model before heavy work.
    if os.environ.get("IFLOW_PING") == "1":
        print("Running iFlow ping...", flush=True)
        try:
            ping_out = run_iflow(
                "Respond with a single word: pong.",
                model,
                1,
                min(60, timeout),
                "/tmp/iflow_ping.json",
            )
            print("=== iFlow ping output (truncated) ===")
            print(ping_out[:2000])
            print("=== end ===")
        except subprocess.CalledProcessError as exc:
            print(f"iFlow ping failed (exit {exc.returncode})", flush=True)
            if exc.output:
                print(exc.output[:4000], flush=True)
            if ctx_path and (ctx_original is not None or ctx_default is not None):
                ctx_path.write_text(ctx_original if ctx_original is not None else (ctx_default or ""))
            cleanup_context()
            return 1

    if plan_mode and ctx_path:
        plan_context = textwrap.dedent("""            # iFlow Plan Mode

            You are in PLAN-ONLY mode. Ignore any instructions about editing files.
            Do not modify files or run tools. Produce a concise plan only.
            """).strip()
        ctx_path.write_text(plan_context)
    if plan_mode:
        print("Running iFlow plan mode...", flush=True)
        plan_prompt = (
            "Analyze the repo and propose improvements. Output a concise plan "
            "for potential PRs and risks. Do not modify files."
        )
        try:
            plan_output = run_iflow(
                plan_prompt,
                model,
                min(10, max_turns),
                min(600, timeout),
                "/tmp/iflow_plan.json",
            )
            plan_path = write_plan_md(plan_output)
            print(f"Wrote plan to {plan_path}")
            # restore normal context for execution stage
            if ctx_path:
                ctx_path.write_text(ctx_original if ctx_original is not None else (ctx_default or ""))
        except subprocess.CalledProcessError as exc:
            print(f"iFlow plan failed (exit {exc.returncode})", flush=True)
            if exc.output:
                print(exc.output[:4000], flush=True)
            if ctx_path and (ctx_original is not None or ctx_default is not None):
                ctx_path.write_text(ctx_original if ctx_original is not None else (ctx_default or ""))
            cleanup_context()
            return 1
    print(f"Using model: {model}", flush=True)
    print("Running iFlow...", flush=True)
    try:
        output = run_iflow(prompt, model, max_turns, timeout, "/tmp/iflow_output.json")
    except subprocess.CalledProcessError as exc:
        print(f"iFlow failed (exit {exc.returncode})", flush=True)
        if exc.output:
            print(exc.output[:8000], flush=True)
        cleanup_context()
        return 1
    cleanup_context()
    plan_md = ROOT / "IFLOW_PLAN.md"
    if plan_md.exists() and os.environ.get("IFLOW_KEEP_PLAN") != "1":
        plan_md.unlink()
    print("=== iFlow raw output (truncated) ===")
    print(output[:8000])
    print("=== end ===")
    try:
        out_path = Path("/tmp/iflow_output.json")
        if out_path.exists():
            print("=== iFlow output file (truncated) ===")
            print(out_path.read_text()[:8000])
            print("=== end ===")
    except Exception:
        pass

    status = git("status", "--porcelain", capture=True)
    dirty = bool(status.strip())
    if not dirty:
        print("No changes detected.")
        return 0

    if DIRECT_GIT:
        print("Direct git mode enabled; leaving PR creation to iFlow.")
        return 0

    if dry_run:
        print("DRY RUN: Working tree has changes.")
        return 0

    changed_files = [f for f in git("diff", "--name-only", capture=True).splitlines() if f.strip()]
    prs = load_plan_prs() or build_prs_from_categories(changed_files)
    if prs:
        temp_branch = f"iflow/workspace-temp-{os.getpid()}"
        git("checkout", "-b", temp_branch)
        assert_not_main_branch()
        git("add", "-A")
        git("commit", "-m", "auto: workspace changes")
        git("checkout", "main")
        git("reset", "--hard", "origin/main")
        git("clean", "-fd")
        created = create_prs_from_file_lists(prs[:MAX_PRS], temp_branch, changed_files)
        print(f"Created {created} PR(s).")
        return 0

    create_fallback_pr()
    return 0



if __name__ == "__main__":
    sys.exit(main())
