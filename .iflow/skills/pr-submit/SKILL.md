---
name: pr-submit
description: Create a PR from local changes using git + gh (or API) safely, without pushing to main/master.
---

# PR Submission (git + gh)

Use this skill when you need to turn local changes into a GitHub PR safely in CI or local shells.

## Preconditions
- Working tree has the intended changes.
- You have `GH_TOKEN` available (recommended in CI).

## Steps
1) Sync base branch:
   - `git fetch origin`
   - `git checkout main`
   - `git reset --hard origin/main`

2) Create a unique branch:
   - `git checkout -b iflow/<category>-<id>`

3) Commit changes:
   - `git add -A`
   - `git commit -m "<type>: <short summary>"`

4) Push branch:
   - `git push -u origin iflow/<category>-<id>`

5) Create PR with gh:
   - `gh pr create --title "<type>: <summary>" --body "Summary:\n- ...\n\nRisks:\n- ...\n\nTests:\n- ..." --base main --head iflow/<category>-<id>`

## Failure Handling
- If `GH_TOKEN` is missing or `gh` fails: stop and report.
- Never push to `main`/`master`.
