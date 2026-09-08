from pathlib import Path
import subprocess

PR_HEAD = "5bcf5888a2d3eae10e10ce410d9c3cc53b078b94"
PR_BRANCH = "codex/compress-workflow-preambles"
TARGETS = [".pi", ".claude", ".agents"]


def run(*args):
    subprocess.run(list(args), check=True)


def replace_exact(text, old, new, expected=1, label="replacement"):
    count = text.count(old)
    if count != expected:
        raise SystemExit(f"{label}: expected {expected} occurrence(s), found {count}")
    return text.replace(old, new)


run("git", "config", "user.name", "David Abram")
run("git", "config", "user.email", "david@crocoder.dev")
run("git", "checkout", "--detach", PR_HEAD)

old_sync_branch = """`sync_debt` -> Read `references/context-sync.md`, then run the **Task context synchronization phase** using the debt task's persisted `Context synchronization handoff` — and, when present, its persisted `Context synchronization blocker` — named by the **Plan review phase**. Do not reconstruct a missing handoff from conversation history."""
new_sync_branch = """`sync_debt` -> Read `references/context-sync.md`, then run the **Task context synchronization phase** with the resolved plan path, debt task ID/title, and completed task record returned by the **Plan review phase**, plus its persisted `Context synchronization blocker` when present. Pass that completed task record verbatim. Do not reconstruct missing task data from conversation history."""

old_recovery_summary = """The review inspects every completed task's `Context synchronization` field in
the plan, in plan order, regardless of its position relative to the task being
selected or resumed, before allowing a new implementation task to start. A
missing field, or any value other than `synced`, is unresolved synchronization
debt. Never infer `synced` from conversation history. When the debt-carrying
task has no durable `Context synchronization handoff` subsection, the **Plan
review phase** returns `blocked` directly with a legacy-migration required
action; otherwise it returns `sync_debt`, resolved by the branch above."""
new_recovery_summary = """The review inspects every completed task's `Context synchronization` field in
the plan, in plan order, regardless of its position relative to the task being
selected or resumed, before allowing a new implementation task to start. A
missing field, or any value other than `synced`, is unresolved synchronization
debt. Never infer `synced` from conversation history. When the debt-carrying
task has no durable completed-task record, the **Plan review phase** returns
`blocked` directly with a legacy-migration required action; otherwise it returns
`sync_debt` with the resolved plan path, debt task ID/title, completed task
record, and persisted blocker when present, for the branch above to route."""

old_plan_review_debt = """- Otherwise, set internal status `sync_debt`, naming the debt task (its ID and
  title) and its own completed record — read directly from the plan by plan
  path and task ID — including, when its field is `blocked`, its persisted
  `Context synchronization blocker`. Do not run or cite the Task context
  synchronization phase. Stop. Do not select or start a new task."""
new_plan_review_debt = """- Otherwise, set internal status `sync_debt`, naming the resolved plan path,
  the debt task (its ID and title), and its own completed record — read
  directly from the plan by plan path and task ID — including, when its field
  is `blocked`, its persisted `Context synchronization blocker`. Do not run
  or cite the Task context synchronization phase. Stop. Do not select or start
  a new task."""

old_sync_debt_result = """A `sync_debt` result must identify:

- The debt-carrying task's ID and title.
- Its own completed record, read directly from the plan by plan path and task ID.
- Its persisted `Context synchronization blocker`, when present."""
new_sync_debt_result = """A `sync_debt` result must identify:

- The resolved plan path.
- The debt-carrying task's ID and title.
- Its own completed record, read directly from the plan by plan path and task ID.
- Its persisted `Context synchronization blocker`, when present."""

# Canonical composite entrypoint.
content_path = Path("config/pkl/base/workflow-content.pkl")
text = content_path.read_text()
text = replace_exact(text, old_sync_branch, new_sync_branch, 1, "workflow-content sync-debt branch")
text = replace_exact(text, old_recovery_summary, new_recovery_summary, 1, "workflow-content recovery summary")
content_path.write_text(text)

# Canonical composite plan-review reference.
next_task_path = Path("config/pkl/base/workflow-next-task.pkl")
text = next_task_path.read_text()
text = replace_exact(text, old_plan_review_debt, new_plan_review_debt, 1, "workflow-next-task plan-review debt result")
text = replace_exact(text, old_sync_debt_result, new_sync_debt_result, 1, "workflow-next-task sync-debt result contract")
next_task_path.write_text(text)

# Strengthen the generated contract: a sync-debt branch must use the completed
# task record and may not mention the removed synchronization-handoff object.
contract_path = Path("config/pkl/renderers/generation-contract-check.pkl")
text = contract_path.read_text()
old_required = """local requiredPlanReviewCompletedRecordTokens = new Listing {
  \"no durable completed-task record (no `Files changed`,\"
  \"read directly from the plan by plan path and task ID\"
}"""
new_required = """local requiredPlanReviewCompletedRecordTokens = new Listing {
  \"no durable completed-task record (no `Files changed`,\"
  \"read directly from the plan by plan path and task ID\"
  \"The resolved plan path.\"
}"""
text = replace_exact(text, old_required, new_required, 1, "plan-review completed-record contract")

old_assertion = """hidden assertSyncDebtRecoveryBranch = (documents: Mapping) ->
  if (
    documents.every((path, text) ->
      !path.endsWith(\"/skills/sce-next-task/SKILL.md\")
      || (
        text.contains(\"`sync_debt` ->\")
        && let (branch = text.drop(text.indexOf(\"`sync_debt` ->\")))
          let (paragraph = if (branch.contains(\"\\n\\n\")) branch.take(branch.indexOf(\"\\n\\n\")) else branch)
            paragraph.contains(\"references/context-sync.md\")
            && paragraph.contains(\"Task context synchronization phase\")
            && paragraph.indexOf(\"references/context-sync.md\") < paragraph.indexOf(\"Task context synchronization phase\")
      )
    )
  ) \"generated sce-next-task SKILL.md: sync-debt recovery cites context-sync.md before invoking the phase\"
  else throw(\"sce-next-task SKILL.md sync-debt recovery branch must cite references/context-sync.md before invoking the Task context synchronization phase\")"""
new_assertion = """hidden assertSyncDebtRecoveryBranch = (documents: Mapping) ->
  if (
    documents.every((path, text) ->
      !path.endsWith(\"/skills/sce-next-task/SKILL.md\")
      || (
        text.contains(\"`sync_debt` ->\")
        && !text.contains(\"Context synchronization handoff\")
        && let (branch = text.drop(text.indexOf(\"`sync_debt` ->\")))
          let (paragraph = if (branch.contains(\"\\n\\n\")) branch.take(branch.indexOf(\"\\n\\n\")) else branch)
            paragraph.contains(\"references/context-sync.md\")
            && paragraph.contains(\"Task context synchronization phase\")
            && paragraph.contains(\"resolved plan path\")
            && paragraph.contains(\"debt task ID/title\")
            && paragraph.contains(\"completed task record\")
            && paragraph.indexOf(\"references/context-sync.md\") < paragraph.indexOf(\"Task context synchronization phase\")
      )
    )
  ) \"generated sce-next-task SKILL.md: sync-debt recovery routes the resolved completed task record\"
  else throw(\"sce-next-task SKILL.md sync-debt recovery must cite context-sync.md before running the phase and route the resolved plan path, debt task identity, and completed task record without a persisted Context synchronization handoff\")"""
text = replace_exact(text, old_assertion, new_assertion, 1, "sync-debt recovery generation contract")
contract_path.write_text(text)

# Keep the three tracked composite mirrors aligned with the canonical source.
for target in TARGETS:
    skill_path = Path(target) / "skills/sce-next-task/SKILL.md"
    text = skill_path.read_text()
    text = replace_exact(text, old_sync_branch, new_sync_branch, 1, f"{target} sync-debt branch")
    text = replace_exact(text, old_recovery_summary, new_recovery_summary, 1, f"{target} recovery summary")
    skill_path.write_text(text)

    review_path = Path(target) / "skills/sce-next-task/references/plan-review.md"
    text = review_path.read_text()
    text = replace_exact(text, old_plan_review_debt, new_plan_review_debt, 1, f"{target} plan-review debt result")
    text = replace_exact(text, old_sync_debt_result, new_sync_debt_result, 1, f"{target} sync-debt result contract")
    review_path.write_text(text)

# Record the ownership boundary in durable context.
ownership_path = Path("context/sce/dedup-ownership-table.md")
ownership = ownership_path.read_text()
if "| Synchronization-debt recovery identity |" not in ownership:
    anchor = "| Live post-task execution handoff schema |"
    lines = ownership.splitlines()
    for i, line in enumerate(lines):
        if line.startswith(anchor):
            lines.insert(i + 1, "| Synchronization-debt recovery identity | `sce-plan-review` resolves the plan path, debt task identity, completed task record, and persisted blocker | `/next-task` only routes that resolved record to task context sync; task context sync validates and consumes it directly from the plan; no separate persisted handoff exists | intentional/keep |")
            break
    else:
        raise SystemExit("ownership table live-handoff row not found")
    ownership_path.write_text("\n".join(lines) + "\n")

# Extend the active PR plan with this correctness follow-up.
plan_path = Path("context/plans/compress-workflow-execution-preamble.md")
plan = plan_path.read_text()
if "T04: `Repair next-task synchronization-debt recovery`" not in plan:
    plan += """

## Follow-up: synchronization-debt recovery contract

- [x] T04: `Repair next-task synchronization-debt recovery` (status:done)
  - Scope: the composite `/next-task` sync-debt branch, plan-review's `sync_debt`
    result identity, the generated semantic contract, the Pi/Claude/Codex tracked
    mirrors, and the ownership table.
  - Ownership after change: plan review resolves the plan path, debt task identity,
    completed task record, and persisted blocker; `/next-task` routes that record
    verbatim; task context sync validates and consumes it. No separate persisted
    `Context synchronization handoff` object exists.
  - Behavior preserved: all-completed-task debt scanning, legacy incomplete-record
    blocking, lifecycle writes (`synced` / refreshed `blocked`), sync-specific blocked
    output, and post-recovery re-review are unchanged.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted Pi/Claude/Codex ownership assertions,
    and `git diff --check`.
  - Context synchronization: synced.
"""
    plan_path.write_text(plan)

# Targeted invariants on tracked mirrors.
for target in TARGETS:
    skill = (Path(target) / "skills/sce-next-task/SKILL.md").read_text()
    review = (Path(target) / "skills/sce-next-task/references/plan-review.md").read_text()
    sync = (Path(target) / "skills/sce-next-task/references/context-sync.md").read_text()

    if "Context synchronization handoff" in skill:
        raise SystemExit(f"{target}: ghost synchronization handoff remains in entrypoint")
    if "resolved plan path, debt task ID/title, and completed task record" not in skill:
        raise SystemExit(f"{target}: entrypoint does not route resolved completed record")
    if "The resolved plan path." not in review:
        raise SystemExit(f"{target}: sync_debt result omits resolved plan path")
    if "naming the resolved plan path" not in review:
        raise SystemExit(f"{target}: plan review does not explicitly own debt identity")
    if "Context synchronization handoff" in review or "Context synchronization handoff" in sync:
        raise SystemExit(f"{target}: removed synchronization handoff reintroduced in a phase reference")
    if "completed task record" not in sync or "read directly from the plan" not in sync:
        raise SystemExit(f"{target}: context sync does not consume the completed task record")

run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")
run("nix", "run", ".#pkl-check-generated")
run("git", "diff", "--check")

planned = {
    "config/pkl/base/workflow-content.pkl",
    "config/pkl/base/workflow-next-task.pkl",
    "config/pkl/renderers/generation-contract-check.pkl",
    "context/plans/compress-workflow-execution-preamble.md",
    "context/sce/dedup-ownership-table.md",
}
for target in TARGETS:
    planned.add(f"{target}/skills/sce-next-task/SKILL.md")
    planned.add(f"{target}/skills/sce-next-task/references/plan-review.md")

changed = set(subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines())
if changed != planned:
    raise SystemExit(f"unexpected changed paths: extra={sorted(changed-planned)} missing={sorted(planned-changed)}")

run("git", "add", *sorted(planned))
run("git", "commit", "-m", "fix: Repair next-task sync debt recovery")
run("git", "push", "origin", f"HEAD:{PR_BRANCH}")
print(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip())
