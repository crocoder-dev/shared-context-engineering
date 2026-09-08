from pathlib import Path
import subprocess

PR_HEAD = "9559ff97a3b664787fda04331f44dc85c73a07aa"
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
run("git", "config", "user.email", "davidabram@users.noreply.github.com")
run("git", "checkout", "--detach", PR_HEAD)

selected_rel = [
    "skills/sce-validate/references/validation.md",
    "skills/sce-validate/references/validation-report.md",
]
before_lines = {}
for target in TARGETS:
    before_lines[target] = sum(
        len((Path(target) / rel).read_text().splitlines()) for rel in selected_rel
    )

old_source_report_intro = """    This is plan-file content. The result returned to the workflow is defined
    separately in \\(validationResultFileRef.render.apply(mode)).
    
    Do not author this section while planning. Only `/validate` through \\(validation.render.apply(mode))
    writes it.
"""
new_source_report_intro = """    This is plan-file content. The result returned to the workflow is defined
    separately in \\(validationResultFileRef.render.apply(mode)).

    This reference owns only the persisted report's structure and presentation.
    Validation execution owns command selection and execution, evidence
    interpretation, acceptance-criterion state, outcome classification, and the
    non-repairing boundary. Consume those results here; do not redefine or rerun
    validation policy in this report reference.
    
    Do not author this section while planning. Only `/validate` through \\(validation.render.apply(mode))
    writes it.
"""

old_rendered_report_intro = """This is plan-file content. The result returned to the workflow is defined
separately in `references/validation.md`.

Do not author this section while planning. Only `/validate` through the **Validation phase**
writes it.
"""
new_rendered_report_intro = """This is plan-file content. The result returned to the workflow is defined
separately in `references/validation.md`.

This reference owns only the persisted report's structure and presentation.
Validation execution owns command selection and execution, evidence
interpretation, acceptance-criterion state, outcome classification, and the
non-repairing boundary. Consume those results here; do not redefine or rerun
validation policy in this report reference.

Do not author this section while planning. Only `/validate` through the **Validation phase**
writes it.
"""

old_source_report_rules = """    - Use **Status:** `validated` only when every acceptance criterion is met and
      every required full-validation command passed.
    - Use **Status:** `failed` when evidence was captured but required checks or
      criteria remain unsatisfied.
    - List every command that ran under **Commands run**, including ones that
      failed. Do not invent exit codes or outcomes.
    - Prefer the plan's `Full validation` commands and each criterion's `Validate:`
      line over rediscovering project defaults. Fall back to repository conventions
      only when the plan omits them.
    - Mark each acceptance criterion checkbox in the plan's `## Acceptance criteria`
      section to match the evidence. Do not mark a criterion met unless the check
      ran successfully or the inspection named by `Validate:` confirms it.
    - Under **Failed checks and follow-ups**, record every failing check and its
      evidence, including leftover debug-only flags, temporary artifacts, or local
      scaffolding. Do not describe code or test edits made during validation;
      validation does not modify tests or product code to clear failures. Write
      `None.` when status is `validated`.
    - When status is `failed`, always include **Retry** with the exact
      `/validate {plan path}` command. Omit **Retry** when status is `validated`.
    - Keep evidence concise and factual. Do not narrate the whole implementation
      history.
    - Do not claim durable context synchronization as part of validation.
    - Do not rewrite task evidence or reopen completed tasks.
    - When a previous `## Validation Report` already exists, replace it with the new
      one rather than stacking duplicates.
"""
new_source_report_rules = """    - Use the `validated` or `failed` status produced by validation execution; this
      report does not redefine status-selection criteria.
    - List every command result supplied by validation execution under **Commands
      run**. Preserve its exit code and concise outcome; do not invent either.
    - Under **Success-criteria verification**, render the acceptance-criterion
      checkbox state and evidence already established by validation execution. Do
      not independently re-evaluate or reclassify criteria here.
    - Under **Failed checks and follow-ups**, render every failure and follow-up
      supplied by validation execution. Write `None.` when status is `validated`.
    - When status is `failed`, always include **Retry** with the exact
      `/validate {plan path}` command. Omit **Retry** when status is `validated`.
    - Keep evidence concise and factual. Do not narrate the whole implementation
      history or add execution-policy claims absent from the validation result.
    - When a previous `## Validation Report` already exists, replace it with the new
      one rather than stacking duplicates.
"""

old_rendered_report_rules = """- Use **Status:** `validated` only when every acceptance criterion is met and
  every required full-validation command passed.
- Use **Status:** `failed` when evidence was captured but required checks or
  criteria remain unsatisfied.
- List every command that ran under **Commands run**, including ones that
  failed. Do not invent exit codes or outcomes.
- Prefer the plan's `Full validation` commands and each criterion's `Validate:`
  line over rediscovering project defaults. Fall back to repository conventions
  only when the plan omits them.
- Mark each acceptance criterion checkbox in the plan's `## Acceptance criteria`
  section to match the evidence. Do not mark a criterion met unless the check
  ran successfully or the inspection named by `Validate:` confirms it.
- Under **Failed checks and follow-ups**, record every failing check and its
  evidence, including leftover debug-only flags, temporary artifacts, or local
  scaffolding. Do not describe code or test edits made during validation;
  validation does not modify tests or product code to clear failures. Write
  `None.` when status is `validated`.
- When status is `failed`, always include **Retry** with the exact
  `/validate {plan path}` command. Omit **Retry** when status is `validated`.
- Keep evidence concise and factual. Do not narrate the whole implementation
  history.
- Do not claim durable context synchronization as part of validation.
- Do not rewrite task evidence or reopen completed tasks.
- When a previous `## Validation Report` already exists, replace it with the new
  one rather than stacking duplicates.
"""
new_rendered_report_rules = """- Use the `validated` or `failed` status produced by validation execution; this
  report does not redefine status-selection criteria.
- List every command result supplied by validation execution under **Commands
  run**. Preserve its exit code and concise outcome; do not invent either.
- Under **Success-criteria verification**, render the acceptance-criterion
  checkbox state and evidence already established by validation execution. Do
  not independently re-evaluate or reclassify criteria here.
- Under **Failed checks and follow-ups**, render every failure and follow-up
  supplied by validation execution. Write `None.` when status is `validated`.
- When status is `failed`, always include **Retry** with the exact
  `/validate {plan path}` command. Omit **Retry** when status is `validated`.
- Keep evidence concise and factual. Do not narrate the whole implementation
  history or add execution-policy claims absent from the validation result.
- When a previous `## Validation Report` already exists, replace it with the new
  one rather than stacking duplicates.
"""

old_result_rules = """    - Never claim a check passed unless it ran successfully or the authorized
      inspection confirmed it.
    - Do not modify tests or product code to clear a failure; record it under
      **What failed**.
"""
new_result_rules = """    - Use the status and evidence already produced by validation execution; do not
      upgrade, reinterpret, or invent results while formatting this report.
"""

old_rendered_result_rules = """- Never claim a check passed unless it ran successfully or the authorized
  inspection confirmed it.
- Do not modify tests or product code to clear a failure; record it under
  **What failed**.
"""
new_rendered_result_rules = """- Use the status and evidence already produced by validation execution; do not
  upgrade, reinterpret, or invent results while formatting this report.
"""

source_path = Path("config/pkl/base/workflow-validate.pkl")
text = source_path.read_text()
text = replace_exact(text, old_source_report_intro, new_source_report_intro, 1, "canonical validation-report ownership intro")
text = replace_exact(text, old_source_report_rules, new_source_report_rules, 1, "canonical validation-report rules")
text = replace_exact(text, old_result_rules, new_result_rules, 1, "canonical returned-report rules")
source_path.write_text(text)

for target in TARGETS:
    report_path = Path(target) / "skills/sce-validate/references/validation-report.md"
    text = report_path.read_text()
    text = replace_exact(text, old_rendered_report_intro, new_rendered_report_intro, 1, f"{target} validation-report ownership intro")
    text = replace_exact(text, old_rendered_report_rules, new_rendered_report_rules, 1, f"{target} validation-report rules")
    report_path.write_text(text)

    validation_path = Path(target) / "skills/sce-validate/references/validation.md"
    text = validation_path.read_text()
    text = replace_exact(text, old_rendered_result_rules, new_rendered_result_rules, 1, f"{target} returned-report rules")
    validation_path.write_text(text)

contract_path = Path("config/pkl/renderers/generation-contract-check.pkl")
text = contract_path.read_text()
anchor = """hidden assertValidationIsObservational = (documents: Mapping) ->
"""
assertion = """local validationExecutionPolicyTokens = new Listing {
  "Prefer the plan's authored checks."
  "Treat leftover debug-only flags, temporary files"
  "Never report a check as passed unless it ran successfully"
  "Do not modify tests, application code, or configuration to make a check pass."
  "Do not reopen completed tasks, rewrite task evidence, or change the task stack."
}

local validationReportForbiddenExecutionTokens = new Listing {
  "Use **Status:** `validated` only when every acceptance criterion is met"
  "Use **Status:** `failed` when evidence was captured"
  "Prefer the plan's `Full validation` commands"
  "Fall back to repository conventions"
  "Do not mark a criterion met unless"
  "leftover debug-only flags"
  "validation does not modify tests or product code"
  "Do not claim durable context synchronization"
  "Do not rewrite task evidence or reopen completed tasks"
}

local requiredValidationReportPresentationTokens = new Listing {
  "This reference owns only the persisted report's structure and presentation."
  "Validation execution owns command selection and execution"
  "Use the `validated` or `failed` status produced by validation execution"
  "render the acceptance-criterion"
  "already established by validation execution"
  "render every failure and follow-up"
  "When a previous `## Validation Report` already exists, replace it"
}

hidden assertValidationReportPolicyOwnership = (documents: Mapping) ->
  if (
    documents.every((path, text) ->
      if (path.endsWith("/skills/sce-validate/references/validation.md"))
        validationExecutionPolicyTokens.every((token) -> text.contains(token))
      else if (path.endsWith("/skills/sce-validate/references/validation-report.md"))
        requiredValidationReportPresentationTokens.every((token) -> text.contains(token))
        && validationReportForbiddenExecutionTokens.every((token) -> !text.contains(token))
      else true
    )
  ) "sce-validate: validation owns execution policy and validation-report owns persisted presentation"
  else throw("sce-validate validation.md must own command/evidence/classification/non-repair policy; validation-report.md may render established results but must not restate execution policy")

"""
if assertion not in text:
    text = replace_exact(text, anchor, assertion + anchor, 1, "validation report ownership assertion insertion")
check_anchor = """  [\"validation-observational\"] = assertValidationIsObservational.apply(workflowDocuments)
"""
check_line = """  [\"validation-report-policy-ownership\"] = assertValidationReportPolicyOwnership.apply(workflowDocuments)
"""
if check_line not in text:
    text = replace_exact(text, check_anchor, check_line + check_anchor, 1, "validation report ownership check registration")
contract_path.write_text(text)

ownership_path = Path("context/sce/dedup-ownership-table.md")
ownership = ownership_path.read_text()
old_row = "| Final validation and validation report | `sce-validation` in `workflow-validate.pkl` | `/validate`; composed into `sce-validate`; thin OpenCode Code agent | intentional/keep |"
new_rows = "| Final validation execution, evidence interpretation, and outcome classification | `references/validation.md` generated from `renderValidationSkillBody` in `workflow-validate.pkl` | `/validate` routes the phase; the persisted report and returned report formats consume established status/evidence without redefining command selection, pass/fail interpretation, or non-repairing boundaries | intentional/keep |\n| Persisted plan-file Validation Report schema and presentation | `references/validation-report.md` generated from `renderValidationReport` in `workflow-validate.pkl` | Validation writes or replaces this section on `validated`/`failed`; it renders recorded command results, criterion states, failures, risks, and retry without re-running or redefining validation policy | intentional/keep |"
ownership = replace_exact(ownership, old_row, new_rows, 1, "validation ownership table row")
ownership_path.write_text(ownership)

after_lines = {}
reductions = {}
for target in TARGETS:
    after_lines[target] = sum(
        len((Path(target) / rel).read_text().splitlines()) for rel in selected_rel
    )
    reductions[target] = before_lines[target] - after_lines[target]

if len(set(reductions.values())) != 1:
    raise SystemExit(f"target reductions differ: {reductions}")
reduction_per_target = next(iter(reductions.values()))
if reduction_per_target <= 0:
    raise SystemExit(f"expected positive Markdown reduction, got {reduction_per_target}")

plan_path = Path("context/plans/compress-workflow-execution-preamble.md")
plan = plan_path.read_text()
if "T08: `Deduplicate validation execution policy from report formatting`" not in plan:
    plan += f"""

## Follow-up: validation report policy ownership

- [x] T08: `Deduplicate validation execution policy from report formatting` (status:done)
  - Scope: `sce-validate` validation phase, persisted `validation-report.md`, returned
    validation-result formatting rules, canonical Pkl, generated semantic checks,
    tracked Pi/Claude/Codex mirrors, and the ownership table.
  - Ownership after change: `references/validation.md` owns command selection and
    execution, evidence interpretation, acceptance-criterion state, outcome
    classification, and non-repairing boundaries. `references/validation-report.md`
    owns only persisted report structure/presentation and renders the phase's
    established results without re-evaluating them.
  - Behavior preserved: plan-authored checks remain preferred, repository fallback
    still applies only when needed, debug/scaffolding evidence can still fail
    validation, failures remain observational, acceptance checkboxes still follow
    evidence, failed plan reports still carry `/validate {{plan path}}`, and prior
    Validation Report sections are replaced rather than stacked.
  - Result: `references/validation.md` plus `references/validation-report.md` shrink
    by {reduction_per_target} Markdown lines per tracked target, {reduction_per_target * len(TARGETS)}
    lines across Pi/Claude/Codex.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted ownership assertions across all three
    tracked targets, and `git diff --check`.
  - Context synchronization: synced.
"""
    plan_path.write_text(plan)

forbidden_report = [
    "Use **Status:** `validated` only when every acceptance criterion is met",
    "Use **Status:** `failed` when evidence was captured",
    "Prefer the plan's `Full validation` commands",
    "Fall back to repository conventions",
    "Do not mark a criterion met unless",
    "leftover debug-only flags",
    "validation does not modify tests or product code",
    "Do not claim durable context synchronization",
    "Do not rewrite task evidence or reopen completed tasks",
]
required_report = [
    "This reference owns only the persisted report's structure and presentation.",
    "Validation execution owns command selection and execution",
    "Use the `validated` or `failed` status produced by validation execution",
    "already established by validation execution",
    "render every failure and follow-up",
]
required_validation = [
    "Prefer the plan's authored checks.",
    "Treat leftover debug-only flags, temporary files",
    "Never report a check as passed unless it ran successfully",
    "Do not modify tests, application code, or configuration to make a check pass.",
    "Do not reopen completed tasks, rewrite task evidence, or change the task stack.",
]
for target in TARGETS:
    report = (Path(target) / "skills/sce-validate/references/validation-report.md").read_text()
    validation = (Path(target) / "skills/sce-validate/references/validation.md").read_text()
    for token in forbidden_report:
        if token in report:
            raise SystemExit(f"{target}: validation-report still duplicates execution policy: {token}")
    for token in required_report:
        if token not in report:
            raise SystemExit(f"{target}: validation-report missing presentation ownership token: {token}")
    for token in required_validation:
        if token not in validation:
            raise SystemExit(f"{target}: validation phase lost execution policy token: {token}")
    if "Never claim a check passed unless it ran successfully or the authorized" in validation.split("## Report rules", 1)[1]:
        raise SystemExit(f"{target}: returned Report rules still restate pass-evidence execution policy")
    if "Do not modify tests or product code to clear a failure" in validation.split("## Report rules", 1)[1]:
        raise SystemExit(f"{target}: returned Report rules still restate non-repair execution policy")

run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")
run("nix", "run", ".#pkl-check-generated")
run("git", "diff", "--check")

planned = {
    "config/pkl/base/workflow-validate.pkl",
    "config/pkl/renderers/generation-contract-check.pkl",
    "context/plans/compress-workflow-execution-preamble.md",
    "context/sce/dedup-ownership-table.md",
}
for target in TARGETS:
    planned.add(f"{target}/skills/sce-validate/references/validation.md")
    planned.add(f"{target}/skills/sce-validate/references/validation-report.md")

changed = set(subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines())
if changed != planned:
    raise SystemExit(f"unexpected changed paths: extra={sorted(changed-planned)} missing={sorted(planned-changed)}")

print(f"validation reference + plan-report Markdown reduction per tracked target: {reduction_per_target}")
print(f"total tracked Pi/Claude/Codex reduction: {reduction_per_target * len(TARGETS)}")
run("git", "add", *sorted(planned))
run("git", "commit", "-m", "config: Deduplicate validation report policy")
run("git", "push", "origin", f"HEAD:{PR_BRANCH}")
print(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip())
