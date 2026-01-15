#!/usr/bin/env python3
import json
import os
import re
import subprocess
import sys
import textwrap
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


def run(cmd, check=True, capture=False, text=True, **kwargs):
    if capture:
        return subprocess.check_output(cmd, text=text, **kwargs).strip()
    subprocess.run(cmd, check=check, **kwargs)
    return ""


def git(*args, capture=False, **kwargs):
    return run(["git", *args], capture=capture, **kwargs)


def build_prompt():
    files = git("ls-files", capture=True).splitlines()
    files_preview = "\n".join(files[:400])
    todo_hits = ""
    try:
        todo_hits = run(["rg", "-n", "TODO|FIXME", str(ROOT)], capture=True)
        todo_hits = "\n".join(todo_hits.splitlines()[:200])
    except Exception:
        todo_hits = "(rg not available or no matches)"

    allowed = ", ".join(sorted(ALLOWED_TYPES))
    prompt = f"""
You are an automated refactoring bot for the repo at {ROOT}. You may propose up to {MAX_PRS} independent pull requests.
Each PR must be ONE category only from: {allowed}.
You can modify any text source file except secrets or generated artifacts.
Do NOT touch: .git/, target/, dist/, build outputs, or any secrets/keys.
Do NOT modify files in .github/workflows that handle credentials. You may modify other CI files.
Each PR must be small: <= {MAX_FILES} files, <= {MAX_LINES} total changed lines.
If a change would exceed limits, split it into a separate PR or skip it.

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
- Do not include explanations outside JSON.
- If you are unsure, output {{"prs": []}}.
Format strictly as:
BEGIN_JSON
{{...}}
END_JSON

Repo file list (first 400):
{files_preview}

TODO/FIXME hits (first 200 lines):
{todo_hits}
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
    iflow_cmd = [
        "iflow",
        "-m",
        "glm-4.7",
        "--thinking",
        "--max-turns",
        "2",
        "--timeout",
        "1800",
        "--checkpointing",
        "false",
        "--all-files",
        "-o",
        "/tmp/iflow_output.json",
        "-p",
        prompt,
    ]

    print("Running iFlow...")
    output = subprocess.check_output(iflow_cmd, text=True)
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

        patch_text = pr["patch"]
        patch_path = Path(f"/tmp/iflow_pr_{idx}.patch")
        patch_path.write_text(patch_text)

        if subprocess.run(["git", "apply", "--check", str(patch_path)]).returncode != 0:
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
