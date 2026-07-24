---
name: sce-task-execution
description: |
  Executes a single planned implementation task with a mandatory approval gate, scope guardrails, and evidence capture. Use when a user wants to implement, run, or execute a specific task from a project plan — such as coding a feature, applying a patch, or making targeted file changes — while enforcing explicit scope boundaries, a pre-implementation confirmation prompt, test/lint verification, and status tracking in context/plans/{plan_id}.md.
compatibility: claude
---

## Purpose
- Implement one approved SCE plan task with an explicit pre-implementation gate, strict scope control, evidence capture, and plan status tracking.

## Inputs
- A reviewed task with goal, boundaries, done checks, verification notes, and `ready_for_implementation: yes`.
- User authorization to continue with implementation.
- Relevant repository and context state.

## Preconditions
1. Default to exactly one task for the session.
2. Before modifying code, present task goal, in/out boundaries, done checks, expected files/components, approach, trade-offs, and risks.
3. Ask `Continue with implementation now? (yes/no)` and wait for confirmation.

## Workflow
1. Restate the approved task and expected touch scope.
2. Present the implementation approach, trade-offs, and risks.
3. Stop for explicit confirmation.
4. Implement the smallest in-scope change after confirmation.
5. Run targeted task-level tests/checks and lints; run a build when it is light and fast.
6. Capture commands, exit codes, and key evidence.
7. Classify context impact as root-edit required or verify-only.
8. Keep session-only scraps under `context/tmp/`.
9. Update the task status and evidence in `context/plans/{plan_id}.md`.

## Guardrails
- Do not edit code before explicit confirmation.
- Do not execute multiple tasks without explicit approval.
- Stop before out-of-scope edits, dependency changes, plan reordering, or unrelated refactors.
- Prefer targeted checks over a full suite during task execution unless the task requires full validation.

## Outputs
- Minimal task implementation.
- Task-level verification evidence.
- Context-impact classification.
- Updated plan task status.

## Completion criteria
- The task's done checks pass with evidence.
- The implementation stays within approved boundaries.
- The plan records status, files changed, evidence, and relevant notes.

## Failure handling
- Stop when confirmation is denied or absent.
- Stop with the exact out-of-scope requirement when scope expansion is needed.
- Report failed checks and preserve the task as incomplete unless the failure is resolved and reverified.

## Related units
- `sce-plan-review` — supplies the ready task.
- `sce-context-sync` — mandatory post-implementation reconciliation.
- `sce-validation` — final-plan full validation.

## Reference
Pre-implementation gate:

```text
Task goal: ...
In scope: ...
Out of scope: ...
Done checks: ...
Expected changes: ...
Approach: ...
Trade-offs: ...
Risks: ...

Continue with implementation now? (yes/no)
```

Record completion in the plan with status, completion date, files changed, evidence, and notes.
