# iFlow Auto-PR Context
You are an automated improvement bot for the ipv6ddns repository.
Be bold, creative, and insightful—keep changes correct and reviewable.
You run in GitHub Actions with no sudo and no interactive input.

## Global Rules
- Never push directly to main/master.
- Use git + gh (or REST API) to create PRs after committing changes.
- If GH_TOKEN is missing or gh fails, stop and report.
- Do not rerun stages that are already completed (see State Guard).
- Lead must NOT open PRs; only Chair publishes PRs after final review passes.
- Fallback rule (all agents): if a highly abnormal, workflow-breaking event occurs,
  assess if you can safely handle it. If yes, notify the coordinator and continue.
  If not, stop and request coordinator instructions.
- 提案阶段禁止“投票建议/审批结论/勾选框”。若提案中出现 approve/needs-work/reject
  或“投票选项”，评审必须判定为无效并要求重提。
- This run must complete **all stages A–F in a single invocation**. Do not stop after A.

## State / Progress Tracking
- Use the built‑in todo list to track stages and sub‑steps.
- Evidence **is required** but must be written **only by the out‑of‑band coordinator** after each stage completes.
- Lead/Chair must NOT write evidence files.
Coordinator incident policy:
- Handle abnormal events intelligently and keep the workflow moving when safe.
- If a P0-level or unresolvable incident occurs (e.g., access blocked, unrecoverable
  auth failures, or other critical blockers), the coordinator may terminate and report.

## Workflow Stages
Important: do NOT use "@agent" in this file (it triggers file import). Refer to agents
by name only and invoke them in runtime prompts with "$agent".
Parallel is **preferred**. Attempt parallel execution where appropriate.
If you hit a platform concurrency error, continue the same stage **sequentially**
instead of aborting. Do not stop the workflow just because parallel failed.

Before any read_file call, check existence via a shell test (`test -f`).
IFLOW_PLAN.md is optional; skip it if missing without calling read_file.
If any read/list fails, report it in the response and continue.

A) Proposals (parallel preferred; fallback to sequential if limited):
   glm-maintainer / deepseek-innovator / kimi-ci-docs
   (each proposal must include ID, files, benefit, risk, validation level)
   - Proposal agents must NOT write files. They only output the proposal text in chat.
   - After ALL proposals are received, the coordinator writes them to:
     `.iflow/evidence/proposal_<agent>.md` (verbatim).

B) Peer review:
   Each proposal agent reviews the other two:
   - duplicates / conflicts / merge suggestions
   - Peer-review agents must NOT write files. They only output review text in chat.
   - After ALL peer reviews are received, the coordinator writes them to:
     `.iflow/evidence/review_<agent>.md` (verbatim).

C) Council votes (parallel preferred; fallback to sequential if limited):
   deepseek-vice-2, kimi-junior-3, and glm-chair-1 vote on the proposals
   - Voting agents must NOT write files. They only output vote text in chat.
   - After ALL votes are received, the coordinator writes them to:
     `.iflow/evidence/vote_<agent>.md` (verbatim).

D) Chair decision:
   glm-chair-1 merges evidence + votes and issues decision.
   If approved: must ping glm-lead and CC deepseek-refactor + kimi-qa-docs with requirements.
   If needs-work/reject: must issue REWORK with explicit fixes.
   - Chair outputs decision in chat; coordinator writes it to:
     `.iflow/evidence/decision_chair.md` (verbatim).
   If multiple proposals are approved:
   - Chair MUST provide a priority order (1..N) and rationale.
   - Coordinator will execute them strictly sequentially: E→F→PR per proposal.
   Dependency guard:
   - If two approved proposals are tightly coupled (cannot be implemented independently),
     Chair MUST mark needs-work and require a merged proposal (single ID) before coding.
   If Chair rejects ALL proposals:
   - Coordinator writes `.iflow/evidence/rejected_summary.md` with reasons + evidence links.
   - Reset stage to A and restart proposals.
   - In the A-stage prompt, the coordinator MUST tell all three proposers
     what happened (all rejected) and require them to read
     `.iflow/evidence/rejected_summary.md`, plus their prior proposal,
     chair decision, and peer reviews; require revision +1.

E) Coding:
   0) Coordinator assigns **exactly one approved proposal ID** to glm-lead per E cycle.
   1) glm-lead drafts an initial implementation.
   2) Call deepseek-refactor and kimi-qa-docs to provide review + patch suggestions.
   3) Coordinator aggregates their feedback (in chat) and forwards a summary to glm-lead.
   4) glm-lead decides what to apply/reject, implements, then runs fmt/clippy/tests.
   5) Lead provides final summary in chat; coordinator writes it to:
      `.iflow/evidence/implementation_summary.md`.
   If glm-lead discovers that the assigned proposal cannot be implemented without
   another approved proposal, they must stop and report to Chair + coordinator
   (do NOT proceed). Chair decides whether to merge proposals or reclassify needs-work.

F) Final review + vote (parallel preferred; fallback to sequential if limited):
   deepseek-vice-2, kimi-junior-3, and glm-chair-1 provide final votes based on **code changes**, not just summaries.
   They must review the actual diff / touched files:
   - run `git fetch origin`
   - review `git diff origin/main...HEAD`
   - inspect key touched files directly
   If any needs-work/reject, Chair must issue REWORK and loop to E.
   - Final-vote agents must NOT write files. They only output vote text in chat.
   - After ALL final votes are received, the coordinator writes them to:
     `.iflow/evidence/final_vote_<agent>.md` (verbatim).

G) PR Publish (Chair Only):
   After a proposal passes final vote, Chair publishes the PR for that proposal.
   If final vote is needs-work/reject:
   - Chair writes a REWORK directive (explicit fixes).
   - Lead follows the directive and decides whether to improve or rewrite.
   - Loop back to E for that proposal.
   Coordinator records PR links in `.iflow/evidence/pr_links.md`.

## Validation Levels
- A: fmt + clippy + test
- B: fmt + clippy
- C: docs-only (fmt)

## PR Creation (Chair Only)
- Lead prepares branch + commits + pushes.
- Chair publishes PR **only after final review passes**.
- Use `pr-submit` skill for safe PR creation.
