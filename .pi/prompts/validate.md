---
description: "Run `sce-validation` to finish an SCE plan with validation and cleanup"
argument-hint: "<plan-name>"
---

## Purpose
- Run the final SCE validation phase by delegating to `sce-validation`.

## Inputs
- `$ARGUMENTS`: target plan name/path or change identifier.
- The plan's success criteria and current repository state.

## Preconditions
- Before acting, read `.pi/skills/sce-validation/SKILL.md` completely and follow it as the entry procedure.
1. Establish the active workflow's authority, boundaries, and observable completion criteria before writes.
2. Resolve blockers or ambiguity required by that workflow before irreversible or scope-expanding action.
3. Inspect existing worktree state and preserve unrelated changes.
1. Resolve the target plan or completed change.
2. Confirm implementation is ready for final validation.

## Workflow
1. Load `sce-validation`.
2. Pass the target and let the skill discover project checks, capture evidence, clean temporary scaffolding, and verify context.
3. Return the pass/fail result and validation-report location.

## Guardrails
- Do not expand scope, change dependencies, or overwrite unrelated work without explicit approval.
- Respect capability approvals before process execution, repository writes, or version-control actions when required.
- Keep stdout/stderr, generated-source ownership, and repository conventions intact.
- Treat the human as owner of architecture, risk, and final decisions.
- Treat code as source of truth when code and `context/` disagree; repair context instead of rationalizing drift.
- Keep temporary session material under `context/tmp/` and durable context current-state oriented.
- Delete a context file only when it exists and has no uncommitted changes.
- Keep this command thin; validation scope, command discovery, repairs, evidence, and report shape remain skill-owned.

## Outputs
- Validation status, commands and evidence summary, residual risks, and report location.

## Completion criteria
- `sce-validation` records a conclusive result against every success criterion.

## Failure handling
- Stop for a human decision before scope expansion, destructive action, or unresolved architecture and risk choices.
- Report failed checks with their command and relevant evidence; never claim success without proof.
- Preserve partial in-scope evidence and identify the workflow phase that failed.
- Report unresolved failures and their evidence; do not close the plan or convert a failed result into success while required checks remain failed or unevaluated.

## Related units
- `shared-context-code` — execution profile bound to this workflow.
- `sce-validation` — entry skill for this workflow.
