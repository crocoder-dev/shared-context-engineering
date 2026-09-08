from pathlib import Path
import subprocess

PR_HEAD = "9ceef9074ac861190c4da11057a93eb08ab90e29"
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

# Capture the rendered Markdown baseline before editing so the reduction is
# measured on the actual tracked skill references, not inferred from source.
before_lines = {}
for target in TARGETS:
    paths = [
        Path(target) / "skills/sce-change-to-plan/references/plan-authoring.md",
        Path(target) / "skills/sce-change-to-plan/references/plan-template.md",
    ]
    before_lines[target] = sum(len(path.read_text().splitlines()) for path in paths)

old_intro = """This phase exclusively owns:

- Resolving whether the request targets a new or an existing plan.
- The clarification gate.
- Normalizing the change summary, acceptance criteria, constraints, and non-goals.
- Slicing the task stack into one-task/one-atomic-commit units.
- Writing `context/plans/{plan_name}.md`.

Do not duplicate any of it elsewhere in the workflow.

Use the document format in `references/plan-template.md`. Read it before writing
the plan file.
"""
new_intro = """This phase owns the planning process:

- Resolve whether the request targets a new or an existing plan.
- Challenge the change and run the clarification gate.
- Derive plan-specific content from the request and loaded context.
- Decide task boundaries, dependencies, and ordering.
- Write or revise exactly one `context/plans/{plan_name}.md`.

`references/plan-template.md` is the sole owner of the persisted plan schema and
generic document-authoring rules: acceptance-criteria format and validation
semantics, task fields and atomic slicing, the no-validation-task rule, completion
records, and existing-plan update preservation. Read it before authoring or
revising and apply those rules rather than restating them here.
"""

old_existing = """When it targets an existing plan, read that plan before authoring. Preserve its
completed tasks, their recorded evidence, its structure, and its terminology.
"""
new_existing = """When it targets an existing plan, read that plan before authoring. Apply the
`Updating an existing plan` rules in `references/plan-template.md` when writing;
this step only resolves which plan is being revised.
"""

old_criteria = """## 2.5 Author the acceptance criteria

State how the finished plan is proven, before slicing tasks.

Each criterion describes observable behavior of the finished system and names the
check that proves it. Record repository-wide checks once under `Full validation`,
and the durable context the change must be reflected in under `Context sync`.

`/validate` runs this section after the last task completes. It is the only place
a plan says how it is validated.
"""
new_criteria = """## 2.5 Author the acceptance criteria

Derive the plan-specific success outcomes and checks before slicing tasks, then
apply the `Acceptance criteria rules` and exact section shape in
`references/plan-template.md`. The template owns their generic validation
semantics and placement.
"""

old_tasks = """## 2.6 Author the task stack

Slice the work into sequential tasks `T01..T0N` using the task format and the
atomic slicing contract in `references/plan-template.md`.

Every executable task must be completable and landable as one coherent commit.
Split any task that would require multiple independent commits. Convert broad
wrappers such as `polish` or `finalize` into specific outcomes with concrete
acceptance checks.

Order tasks so each one's declared dependencies precede it.

The last task is an ordinary implementation task. Do not author a trailing
validation-and-cleanup task, or any task whose only purpose is running the full
check suite, verifying durable context, or removing scaffolding.

Confirm every acceptance criterion is satisfied by at least one task. When one is
not, the task stack is incomplete.

A finished stack always leaves at least one incomplete task, so the workflow can
always hand off to `/next-task`. When the request resolves to a plan but produces
no incomplete task, because the change is already implemented or already covered
by completed tasks, set internal status `blocked` with category
`no_actionable_work` instead of writing the plan.
"""
new_tasks = """## 2.6 Author the task stack

Slice and order the plan-specific work after the acceptance criteria, applying the
`Task rules` and `No validation task` rules in `references/plan-template.md`.
The template owns the task field shape, atomic-commit constraint, dependency-order
rule, acceptance-coverage rule, and generic task exclusions.

A finished stack always leaves at least one incomplete task, so the workflow can
always hand off to `/next-task`. When the request resolves to a plan but produces
no incomplete task, because the change is already implemented or already covered
by completed tasks, set internal status `blocked` with category
`no_actionable_work` instead of writing the plan.
"""

old_write = """## 2.7 Write the plan

Write `context/plans/{plan_name}.md` using `references/plan-template.md`.

When updating an existing plan, keep completed tasks and their evidence intact,
and append or renumber new tasks without disturbing recorded history.
"""
new_write = """## 2.7 Write the plan

Write `context/plans/{plan_name}.md` by applying `references/plan-template.md`
exactly. For revisions, apply its `Updating an existing plan` rules.
"""

old_boundary = """- Author a validation, cleanup, or context-verification task. `/validate` owns
  that phase.
"""
new_boundary = """- Write a plan that violates `references/plan-template.md`.
"""

sequential_anchor = """## Task rules

- Every task is a checkbox line so progress stays machine-readable:
"""
sequential_replacement = """## Task rules

- Number tasks sequentially as `T01..T0N`.
- Every task is a checkbox line so progress stays machine-readable:
"""

# Canonical source owns both references.
source_path = Path("config/pkl/base/workflow-change-to-plan.pkl")
text = source_path.read_text()
text = replace_exact(text, old_intro, new_intro, 1, "canonical plan-authoring ownership intro")
text = replace_exact(text, old_existing, new_existing, 1, "canonical existing-plan resolution")
text = replace_exact(text, old_criteria, new_criteria, 1, "canonical acceptance criteria process")
text = replace_exact(text, old_tasks, new_tasks, 1, "canonical task slicing process")
text = replace_exact(text, old_write, new_write, 1, "canonical plan write process")
text = replace_exact(text, old_boundary, new_boundary, 1, "canonical plan-authoring boundary")
text = replace_exact(text, sequential_anchor, sequential_replacement, 1, "canonical sequential task rule")
source_path.write_text(text)

# Tracked composite mirrors.
for target in TARGETS:
    authoring_path = Path(target) / "skills/sce-change-to-plan/references/plan-authoring.md"
    text = authoring_path.read_text()
    text = replace_exact(text, old_intro, new_intro, 1, f"{target} plan-authoring ownership intro")
    text = replace_exact(text, old_existing, new_existing, 1, f"{target} existing-plan resolution")
    text = replace_exact(text, old_criteria, new_criteria, 1, f"{target} acceptance criteria process")
    text = replace_exact(text, old_tasks, new_tasks, 1, f"{target} task slicing process")
    text = replace_exact(text, old_write, new_write, 1, f"{target} plan write process")
    text = replace_exact(text, old_boundary, new_boundary, 1, f"{target} plan-authoring boundary")
    authoring_path.write_text(text)

    template_path = Path(target) / "skills/sce-change-to-plan/references/plan-template.md"
    text = template_path.read_text()
    text = replace_exact(text, sequential_anchor, sequential_replacement, 1, f"{target} sequential task rule")
    template_path.write_text(text)

# Generated semantic contract: plan-template owns persisted schema + generic
# authoring policy, while plan-authoring owns only process and plan-specific decisions.
contract_path = Path("config/pkl/renderers/generation-contract-check.pkl")
text = contract_path.read_text()
anchor = """local requiredNextTaskCompletionWritingTokens = new Listing {
"""
assertion = """local planTemplateOwnedPolicyTokens = new Listing {
  "Acceptance criteria describe the finished system, not the work."
  "Every criterion carries a `Validate:` line."
  "List repository-wide checks once under `Full validation`"
  "Number tasks sequentially as `T01..T0N`."
  "Author each executable task as one atomic commit unit by default."
  "Split any candidate task that would require multiple independent commits"
  "The last task in the stack is an ordinary implementation task."
  "Preserve completed tasks, their `(status:done)` markers"
}

local requiredPlanAuthoringTemplateReferences = new Listing {
  "`references/plan-template.md` is the sole owner of the persisted plan schema"
  "`Acceptance criteria rules`"
  "`Task rules` and `No validation task` rules"
  "`Updating an existing plan` rules"
}

hidden assertPlanTemplatePolicyOwnership = (documents: Mapping) ->
  if (
    documents.every((path, text) ->
      if (path.endsWith("/skills/sce-change-to-plan/references/plan-template.md"))
        planTemplateOwnedPolicyTokens.every((token) -> text.contains(token))
      else if (path.endsWith("/skills/sce-change-to-plan/references/plan-authoring.md"))
        requiredPlanAuthoringTemplateReferences.every((token) -> text.contains(token))
        && planTemplateOwnedPolicyTokens.every((token) -> !text.contains(token))
      else true
    )
  ) "sce-change-to-plan: plan-template owns persisted schema and generic authoring policy"
  else throw("sce-change-to-plan plan-template.md must own the persisted plan schema and generic acceptance/task/update rules; plan-authoring.md may reference those rules but must not restate them")

"""
if assertion not in text:
    text = replace_exact(text, anchor, assertion + anchor, 1, "plan-template ownership assertion insertion")
check_anchor = """  ["compact-plan-template-schema"] = assertCompactPlanTemplateSchema.apply(workflowDocuments)
"""
check_line = """  ["plan-template-policy-ownership"] = assertPlanTemplatePolicyOwnership.apply(workflowDocuments)
"""
if check_line not in text:
    text = replace_exact(text, check_anchor, check_anchor + check_line, 1, "plan-template ownership check registration")
contract_path.write_text(text)

# Durable ownership documentation.
ownership_path = Path("context/sce/dedup-ownership-table.md")
ownership = ownership_path.read_text()
ownership = replace_exact(
    ownership,
    "| Plan authoring, clarification, and task slicing | `sce-plan-authoring` in `workflow-change-to-plan.pkl` | `/change-to-plan`; composed into `sce-change-to-plan`; thin OpenCode Plan agent | intentional/keep |",
    "| Plan authoring process, clarification, and plan-specific task slicing | `sce-plan-authoring` in `workflow-change-to-plan.pkl` | `/change-to-plan`; reads the template before every write/revision; composed into `sce-change-to-plan`; thin OpenCode Plan agent | intentional/keep |\n| Persisted plan schema and generic authoring rules | `references/plan-template.md` generated from `changeToPlanPlanTemplate` in `workflow-change-to-plan.pkl` | Plan authoring derives plan-specific content and applies the template's acceptance, task, no-validation-task, completion-record, and existing-plan-update rules without restating them | intentional/keep |",
    1,
    "ownership table plan authoring row",
)
ownership_path.write_text(ownership)

# Measure the rendered Markdown reduction before recording task evidence.
after_lines = {}
reductions = {}
for target in TARGETS:
    paths = [
        Path(target) / "skills/sce-change-to-plan/references/plan-authoring.md",
        Path(target) / "skills/sce-change-to-plan/references/plan-template.md",
    ]
    after_lines[target] = sum(len(path.read_text().splitlines()) for path in paths)
    reductions[target] = before_lines[target] - after_lines[target]

if len(set(reductions.values())) != 1:
    raise SystemExit(f"target reductions differ: {reductions}")
reduction_per_target = next(iter(reductions.values()))
if reduction_per_target <= 0:
    raise SystemExit(f"expected positive Markdown reduction, got {reduction_per_target}")

# Record T06 on the active plan.
plan_path = Path("context/plans/compress-workflow-execution-preamble.md")
plan = plan_path.read_text()
if "T06: `Deduplicate plan authoring from the persisted plan template`" not in plan:
    plan += f"""

## Follow-up: plan-template authoring-policy ownership

- [x] T06: `Deduplicate plan authoring from the persisted plan template` (status:done)
  - Scope: change-to-plan plan-authoring and plan-template references, canonical Pkl,
    generated semantic checks, tracked Pi/Claude/Codex mirrors, and the ownership table.
  - Ownership after change: `references/plan-template.md` solely owns the persisted
    plan schema plus generic acceptance-criteria, task-format/atomic-slicing,
    no-validation-task, completion-record, and existing-plan-update rules. The
    Plan authoring phase owns process order, plan-specific decisions, clarification,
    and the `no_actionable_work` outcome, and references the template at write boundaries.
  - Behavior preserved: criteria are still authored before tasks; tasks remain
    sequential atomic-commit units with ordered dependencies and no trailing validation
    task; existing completed-task history stays protected; every written plan still uses
    the same persisted schema. The sequential `T01..T0N` rule moved into the template
    rather than being dropped.
  - Result: plan-authoring + plan-template shrink by {reduction_per_target} Markdown
    lines per tracked target, {reduction_per_target * len(TARGETS)} lines across
    Pi/Claude/Codex.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted ownership assertions across all three
    tracked targets, and `git diff --check`.
  - Context synchronization: synced.
"""
    plan_path.write_text(plan)

# Focused mirror assertions independent of the Pkl checker.
policy_tokens = [
    "Acceptance criteria describe the finished system, not the work.",
    "Every criterion carries a `Validate:` line.",
    "List repository-wide checks once under `Full validation`",
    "Number tasks sequentially as `T01..T0N`.",
    "Author each executable task as one atomic commit unit by default.",
    "Split any candidate task that would require multiple independent commits",
    "The last task in the stack is an ordinary implementation task.",
    "Preserve completed tasks, their `(status:done)` markers",
]
for target in TARGETS:
    authoring = (Path(target) / "skills/sce-change-to-plan/references/plan-authoring.md").read_text()
    template = (Path(target) / "skills/sce-change-to-plan/references/plan-template.md").read_text()
    if "`references/plan-template.md` is the sole owner of the persisted plan schema" not in authoring:
        raise SystemExit(f"{target}: missing template ownership statement")
    for token in policy_tokens:
        if token in authoring:
            raise SystemExit(f"{target}: plan-authoring still restates template policy: {token}")
        if token not in template:
            raise SystemExit(f"{target}: plan-template lost owned policy: {token}")

run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")
run("nix", "run", ".#pkl-check-generated")
run("git", "diff", "--check")

planned = {
    "config/pkl/base/workflow-change-to-plan.pkl",
    "config/pkl/renderers/generation-contract-check.pkl",
    "context/plans/compress-workflow-execution-preamble.md",
    "context/sce/dedup-ownership-table.md",
}
for target in TARGETS:
    planned.add(f"{target}/skills/sce-change-to-plan/references/plan-authoring.md")
    planned.add(f"{target}/skills/sce-change-to-plan/references/plan-template.md")

changed = set(subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines())
if changed != planned:
    raise SystemExit(f"unexpected changed paths: extra={sorted(changed-planned)} missing={sorted(planned-changed)}")

print(f"plan-authoring + plan-template Markdown reduction per tracked target: {reduction_per_target}")
print(f"total tracked Pi/Claude/Codex reduction: {reduction_per_target * len(TARGETS)}")
run("git", "add", *sorted(planned))
run("git", "commit", "-m", "config: Deduplicate change-to-plan template policy")
run("git", "push", "origin", f"HEAD:{PR_BRANCH}")
print(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip())
