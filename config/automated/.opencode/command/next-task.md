---
description: "Run `sce-plan-review` -> `sce-task-execution` -> `sce-context-sync` for one approved SCE task"
agent: "Shared Context Code"
subtask: false
entry-skill: "sce-plan-review"
skills:
  - "sce-plan-review"
  - "sce-task-execution"
  - "sce-context-sync"
  - "sce-validation"
permission:
  default: block
  read: allow
  edit: allow
  glob: allow
  grep: allow
  list: allow
  bash: allow
  question: allow
  codesearch: allow
  lsp: allow
  skill:
    "*": block
    "sce-plan-review": allow
    "sce-task-execution": allow
    "sce-context-sync": allow
    "sce-validation": allow
---

## Purpose
- Review and execute exactly one SCE task non-interactively, then synchronize context and run final validation when applicable.

## Inputs
- `$ARGUMENTS`: plan name/path (required) and task ID (strongly preferred).

## Preconditions
1. Require an existing plan and context tree.
2. Auto-pass readiness only with explicit plan and task ID plus a clean review.
3. Stop with structured errors for every unresolved issue.

## Workflow
1. Load `sce-plan-review`.
2. When ready, load `sce-task-execution`; log intent and proceed.
3. Load `sce-context-sync` after implementation.
4. Apply only in-scope feedback fixes, rerun light checks, and sync again.
5. Run `sce-validation` for the final task; otherwise return the next `/next-task` command.

## Guardrails
- Keep orchestration thin and deterministic.
- Execute one task only.
- Do not wait for interactive implementation confirmation.
- Stop immediately on scope expansion or unresolved readiness.

## Outputs
- Readiness, implementation evidence, plan status, context-sync result, and final validation or next-task handoff.

## Completion criteria
- The task is complete with evidence and synchronized context.

## Failure handling
- Return a structured error with category, evidence, and required human action.

## Related units
- `sce-plan-review`, `sce-task-execution`, `sce-context-sync`, and `sce-validation`.
