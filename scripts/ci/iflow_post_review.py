#!/usr/bin/env python3
import json
import os
import re
import subprocess
import shlex
import sys
import tempfile
from datetime import datetime, timezone, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

BLOCKED_PREFIXES = (
    ".github/workflows/",
    "target/",
    "dist/",
)


def run(cmd, check=True, capture=False, text=True, **kwargs):
    if capture:
        return subprocess.check_output(cmd, text=text, **kwargs).strip()
    subprocess.run(cmd, check=check, **kwargs)
    return ""


def git(*args, capture=False, **kwargs):
    return run(["git", *args], capture=capture, **kwargs)


def gh(*args, capture=False, **kwargs):
    return run(["gh", *args], capture=capture, **kwargs)


def status_lines():
    raw = git("status", "--porcelain", capture=True)
    return [line for line in raw.splitlines() if line.strip()]


def status_paths(lines):
    paths = []
    for line in lines:
        if not line:
            continue
        # format: XY <path>
        path = line[3:] if len(line) > 3 else ""
        if path:
            paths.append(path)
    return paths


def filter_disallowed_files():
    lines = status_lines()
    if not lines:
        return
    blocked = []
    untracked = []
    for line in lines:
        path = line[3:] if len(line) > 3 else ""
        if not path:
            continue
        if any(path.startswith(prefix) for prefix in BLOCKED_PREFIXES):
            blocked.append(path)
        if line.startswith("??"):
            untracked.append(path)

    if blocked:
        subprocess.run(["git", "restore", "--staged", "--worktree", "--", *blocked], check=False)
    for path in untracked:
        if any(path.startswith(prefix) for prefix in BLOCKED_PREFIXES):
            try:
                (ROOT / path).unlink()
            except FileNotFoundError:
                pass


def ensure_clean_tree():
    if status_lines():
        raise RuntimeError("Working tree is dirty before review run.")


def parse_iso(ts):
    return datetime.fromisoformat(ts.replace("Z", "+00:00"))


def list_prs():
    raw = gh(
        "pr",
        "list",
        "--state",
        "open",
        "--limit",
        os.environ.get("IFLOW_REVIEW_LIMIT", "30"),
        "--json",
        "number,headRefName,updatedAt,labels,author,title,url",
        capture=True,
    )
    data = json.loads(raw)
    hours = int(os.environ.get("IFLOW_REVIEW_HOURS", "24"))
    cutoff = datetime.now(timezone.utc) - timedelta(hours=hours)
    wanted = []
    for pr in data:
        head = pr.get("headRefName", "")
        if not head.startswith("iflow/"):
            continue
        labels = {l["name"] for l in pr.get("labels", [])}
        if "iflow-reviewed" in labels:
            continue
        updated = parse_iso(pr["updatedAt"])
        if updated < cutoff:
            continue
        wanted.append(pr)
    max_prs = int(os.environ.get("IFLOW_REVIEW_MAX_PRS", "5"))
    return wanted[:max_prs]


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


def build_prompt(pr):
    base = os.environ.get("IFLOW_REVIEW_CONTEXT", "").strip()
    if not base:
        base = """
You are reviewing an existing PR branch. Improve quality and fix issues.
Rules:
- Do NOT create new branches or PRs.
- Only modify files in the current branch.
- Do NOT touch .github/workflows/.
- Run relevant tests for any changes you make.
- If tests fail, revert your changes and stop.
- If no changes are needed, exit without modifications.
""".strip()
    return f"{base}\n\nCurrent PR: #{pr['number']} {pr['title']} ({pr['url']})\nBranch: {pr['headRefName']}\n"


def commit_and_push(branch, pr_number):
    filter_disallowed_files()
    if not status_lines():
        gh("pr", "edit", str(pr_number), "--add-label", "iflow-reviewed")
        gh("pr", "comment", str(pr_number), "--body", "Review pass: no changes needed.")
        return
    git("add", "-A")
    git("commit", "-m", f"review: refine {branch}")
    git("push", "-u", "origin", branch)
    gh("pr", "edit", str(pr_number), "--add-label", "iflow-reviewed")
    gh("pr", "comment", str(pr_number), "--body", "Review pass: applied improvements to this branch.")


def main():
    ensure_clean_tree()
    model = os.environ.get("IFLOW_MODEL", "deepseek-v3.2-chat")
    max_turns = int(os.environ.get("IFLOW_MAX_TURNS", "60"))
    timeout = int(os.environ.get("IFLOW_TIMEOUT", "1200"))
    prs = list_prs()
    if not prs:
        print("No PRs eligible for review.")
        return 0

    # Ensure label exists (best effort)
    try:
        gh("label", "create", "iflow-reviewed", "--description", "Reviewed by iflow post-review", "--color", "0E8A16")
    except subprocess.CalledProcessError:
        pass

    for pr in prs:
        branch = pr["headRefName"]
        print(f"Reviewing PR #{pr['number']} on {branch}")
        git("fetch", "origin", branch)
        git("checkout", "-B", branch, f"origin/{branch}")
        ensure_clean_tree()
        prompt = build_prompt(pr)
        out_path = ROOT / f".iflow_review_{pr['number']}.json"
        run_iflow(prompt, model, max_turns, timeout, out_path)
        commit_and_push(branch, pr["number"])

    return 0


if __name__ == "__main__":
    sys.exit(main())
