---
description: "Use `sce-plan-authoring` to turn a change request into a scoped SCE plan"
agent: "Shared Context Plan"
subtask: false
entry-skill: "sce-plan-authoring"
skills:
  - "sce-plan-authoring"
permission:
  default: block
  read: allow
  edit: allow
  glob: allow
  grep: allow
  list: allow
  bash: block
  question: allow
  codesearch: allow
  lsp: allow
  skill:
    "*": block
    "sce-plan-authoring": allow
---

## Purpose
- Convert `$ARGUMENTS` into a deterministic SCE plan through `sce-plan-authoring`.

## Inputs
- `$ARGUMENTS`: complete change request, criteria, constraints, dependency decisions, and optional existing plan target.

## Preconditions
1. Require an existing `context/` tree.
2. Require all critical details to be explicit; this command does not conduct an interactive clarification loop.

## Workflow
1. Load `sce-plan-authoring`.
2. Pass `$ARGUMENTS` and let the skill validate ambiguity and task atomicity.
3. On success, write or update `context/plans/{plan_name}.md`.
4. Return path, full task order, and `/next-task {plan_name} T01`.
5. Stop after the planning handoff.

## Guardrails
- Keep this command thin.
- Do not ask interactive questions, invent assumptions, modify application code, or begin implementation.

## Outputs
- A complete plan handoff or one structured error listing all unresolved planning items.

## Completion criteria
- The skill reports a valid atomic plan saved at the returned path.

## Failure handling
- Stop with categorized unresolved items; do not write a partial plan.

## Related units
- `sce-plan-authoring` — deterministic planning owner.
- `/change-to-plan-interactive` — explicit interactive alternative.
