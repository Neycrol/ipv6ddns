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
MAX_FILES = 8
MAX_LINES = 200
MAX_REPAIR_ATTEMPTS = int(os.environ.get("IFLOW_REPAIR_ATTEMPTS", "1"))


def run(cmd, check=True, capture=False, text=True, **kwargs):
    if capture:
        return subprocess.check_output(cmd, text=text, **kwargs).strip()
    subprocess.run(cmd, check=check, **kwargs)
    return ""


def git(*args, capture=False, **kwargs):
    return run(["git", *args], capture=capture, **kwargs)


def build_prompt():
    top_level = sorted([p.name for p in ROOT.iterdir() if p.name != ".git"])
    files_preview = "\n".join(top_level)

    allowed = ", ".join(sorted(ALLOWED_TYPES))
    prompt = f"""
You are an automated refactoring bot for the repo at {ROOT}. You may propose up to {MAX_PRS} independent pull requests.
Each PR must be ONE category only from: {allowed}.
You can modify any text source file except secrets or generated artifacts.
Do NOT touch: .git/, target/, dist/, build outputs, or any secrets/keys.
Do NOT modify files in .github/workflows that handle credentials. You may modify other CI files.
Each PR must be small: <= {MAX_FILES} files, <= {MAX_LINES} total changed lines.
If a change would exceed limits, split it into a separate PR or skip it.
Use tools to inspect files when necessary; do not assume file contents.

Output JSON only (no extra text) wrapped between lines BEGIN_JSON and END_JSON, with this schema:
{{
  "prs": [
    {{
      "title": "...",
      "branch_name": "...",
      "type": "refactor|perf|tests|docs|ci|android-ui|packaging|bugfix",
      "rationale": ["...", "..."],
      "self_proof": ["...", "..."],
      "self_review": ["...", "..."],
      "tests": ["..."],
      "patch": "<unified diff from repo root>"
    }}
  ]
}}

Rules:
- The patch must apply cleanly with `git apply --check`.
- Use a standard unified diff with `diff --git`, `---`, `+++`, and `@@` hunks.
- Do NOT include `index ...` lines or fake hashes.
- Do not include explanations outside JSON.
- If you are unsure, output {{"prs": []}}.
Format strictly as:
BEGIN_JSON
{{...}}
END_JSON

Top-level entries:
{files_preview}
"""
    return textwrap.dedent(prompt).strip()


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


def maybe_write_iflow_context():
    if os.environ.get("IFLOW_WRITE_CONTEXT") != "1":
        return None
    path = ROOT / "IFLOW.md"
    if path.exists():
        return None
    content = os.environ.get("IFLOW_CONTEXT")
    if not content:
        content = textwrap.dedent(
            f"""\
            # iFlow Auto-PR Context

            You are an automated refactoring bot running in GitHub Actions for {ROOT}.
            Goal: propose small, safe PRs (<= {MAX_FILES} files, <= {MAX_LINES} lines).
            Each PR must be a single category and include a clean unified diff.
            Do not use sudo or interactive prompts.
            """
        ).strip()
    path.write_text(content)
    return path


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

    ctx_path = maybe_write_iflow_context()

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
    data = extract_json(output)
    if not data:
        print("No JSON payload detected; aborting.")
        return 0

    prs = data.get("prs", [])[:MAX_PRS]
    if not prs:
        print("No PRs proposed.")
        return 0
    if dry_run:
        print("DRY RUN: Parsed PRs")
        for idx, pr in enumerate(prs, 1):
            print(f"- {idx}. {pr.get('type')}: {pr.get('title')}")
        return 0

    status = git("status", "--porcelain", capture=True)
    if status.strip():
        print("Working tree not clean; aborting.")
        return 1

    created = 0
    for idx, pr in enumerate(prs, 1):
        ok, reason = validate_pr(pr)
        if not ok:
            print(f"Skipping PR {idx}: {reason}")
            continue

        patch_text = normalize_patch(pr["patch"])
        patch_path = Path(f"/tmp/iflow_pr_{idx}.patch")
        patch_path.write_text(patch_text)

        ok, err = apply_check(str(patch_path))
        if not ok:
            print(f"Patch failed --check for PR {idx}")
            if err:
                print(err[:4000])

            repaired = False
            if MAX_REPAIR_ATTEMPTS > 0:
                for attempt in range(1, MAX_REPAIR_ATTEMPTS + 1):
                    print(f"Attempting patch repair {attempt}/{MAX_REPAIR_ATTEMPTS} for PR {idx}...")
                    repair_prompt = f"""
You generated a git patch for repo {ROOT}, but it failed to apply.
Error:
{err}

Original patch:
{patch_text}

Please output ONLY a corrected unified diff patch (starting with 'diff --git').
No explanations, no JSON, no index lines. The patch must apply cleanly with git apply --check.
"""
                    repair_output = run_iflow(
                        textwrap.dedent(repair_prompt).strip(),
                        model,
                        min(20, max_turns),
                        min(1200, timeout),
                        f"/tmp/iflow_repair_{idx}.json",
                    )
                    repaired_patch = extract_patch(repair_output)
                    if not repaired_patch:
                        print("Repair failed: no patch detected in output.")
                        continue
                    patch_path.write_text(repaired_patch)
                    ok, err = apply_check(str(patch_path))
                    if ok:
                        repaired = True
                        break
                    print("Repair patch still failed:")
                    if err:
                        print(err[:4000])
            if not repaired:
                print(f"Skipping PR {idx}: patch failed --check")
                continue

        files_changed, lines_changed = count_patch_stats(str(patch_path))
        if files_changed is None:
            print(f"Skipping PR {idx}: cannot compute stats")
            continue
        if files_changed > MAX_FILES or lines_changed > MAX_LINES:
            print(f"Skipping PR {idx}: too large ({files_changed} files, {lines_changed} lines)")
            continue

        branch = f"iflow/{sanitize_branch(pr['branch_name'], idx)}"
        title = pr["title"].strip()[:72]
        body = "\n".join([
            f"Type: {pr['type']}",
            "",
            "Rationale:",
            *[f"- {r}" for r in pr["rationale"]],
            "",
            "Self-proof:",
            *[f"- {r}" for r in pr["self_proof"]],
            "",
            "Self-review:",
            *[f"- {r}" for r in pr["self_review"]],
            "",
            "Tests:",
            *[f"- {t}" for t in pr["tests"]],
            "",
            "Generated by iFlow nightly automation.",
        ])

        git("checkout", "-b", branch)
        git("apply", str(patch_path))
        git("add", "-A")
        git("commit", "-m", f"{pr['type']}: {title}")
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

    print(f"Created {created} PR(s).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
