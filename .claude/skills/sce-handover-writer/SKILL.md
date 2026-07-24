---
name: sce-handover-writer
description: |
  Creates a structured handover document summarizing task context, decisions made, open questions, and recommended next steps — saved to `context/handovers/`. Use when a user wants to hand off, transition, or pass a task to someone else, create handover notes, write a task transition document, or capture current progress for a future session. Trigger phrases include "create a handover", "hand this off", "write handover notes", "pass this task on", or "document where I'm up to".
compatibility: claude
---

## Purpose
- Preserve enough current-state task information for another human or AI session to continue safely.

## Inputs
- Current plan name/path and task ID when available.
- Repository status, recent changes, verification evidence, decisions, blockers, and next-step context.

## Preconditions
1. Inspect the current plan, task, relevant changes, and repository state.
2. Separate observed facts from assumptions.

## Workflow
1. Resolve task-aligned naming: `context/handovers/{plan_name}-{task_id}-{timestamp}.md` when identifiers are available; otherwise use a descriptive fallback.
2. Record current task state and degree of completion.
3. Record decisions and the rationale for each material choice.
4. Record open questions, blockers, dependencies, and failed checks.
5. Record one concrete next recommended step.
6. Label inferred details as assumptions.
7. Verify all required sections are populated and return the exact path.

## Guardrails
- Describe current state, not a narrative changelog.
- Do not invent decisions, evidence, owners, or completion status.
- Do not make implementation changes while writing the handover.

## Outputs
- One handover file under `context/handovers/` and its exact path.

## Completion criteria
- The file contains `Current Task State`, `Decisions Made`, `Open Questions / Blockers`, and `Next Recommended Step`.
- Every assumption is explicitly labelled.

## Failure handling
- When the current task cannot be identified reliably, request or report the missing plan/task information instead of fabricating context.
- Report write failures directly.

## Related units
- `/handover` — thin command entrypoint.
- `sce-plan-review` — source of plan/task readiness information.
- `sce-task-execution` — source of implementation and evidence state.

## Reference
```markdown
# Handover: {plan_name} - {task_id}

## Current Task State
...

## Decisions Made
...

## Open Questions / Blockers
...

## Next Recommended Step
...
```
