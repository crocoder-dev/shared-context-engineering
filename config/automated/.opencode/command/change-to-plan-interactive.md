---
description: "Create or update an SCE plan from a change request with interactive clarification"
agent: "Shared Context Plan"
---

## Purpose
- Convert `$ARGUMENTS` into an SCE plan while explicitly allowing a human clarification loop.

## Inputs
- `$ARGUMENTS`: change request and optional existing plan target.
- Human answers to targeted planning questions.

## Preconditions
1. Require an existing `context/` tree.
2. Permit interactive clarification for unresolved critical details.

## Workflow
1. Load `sce-plan-authoring-interactive`.
2. Pass `$ARGUMENTS` and preserve the skill's blocking clarification gate.
3. After all questions are resolved, write or update `context/plans/{plan_name}.md`.
4. Return path, full task order, and `/next-task {plan_name} T01`.
5. Stop after the planning handoff.

## Guardrails
- Keep this command thin.
- Do not invent assumptions, modify application code, or begin implementation.

## Outputs
- Focused clarification questions followed by a complete plan handoff when resolved.

## Completion criteria
- The interactive skill reports a valid atomic plan saved at the returned path.

## Failure handling
- Keep planning blocked while any critical question remains unanswered.

## Related units
- `sce-plan-authoring-interactive` — interactive planning owner.
- `/change-to-plan` — deterministic non-interactive alternative.
