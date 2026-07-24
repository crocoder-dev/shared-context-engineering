---
description: "Run `sce-handover-writer` to capture the current task for handoff"
agent: "Shared Context Code"
subtask: false
entry-skill: "sce-handover-writer"
skills:
  - "sce-handover-writer"
permission:
  default: ask
  read: allow
  edit: allow
  glob: allow
  grep: allow
  list: allow
  bash: deny
  question: allow
  codesearch: allow
  lsp: allow
  skill:
    "*": ask
    "sce-handover-writer": allow
---

## Purpose
- Create a durable handover for the current task by delegating to `sce-handover-writer`.

## Inputs
- `$ARGUMENTS`: optional plan name, task ID, scope note, or handover context.
- Current repository, plan, and task state available to the agent.

## Preconditions
1. The current plan and task can be identified when available.

## Workflow
1. Load `sce-handover-writer`.
2. Pass `$ARGUMENTS` and the current task state.
3. Let the skill choose task-aligned naming and write the handover under `context/handovers/`.
4. Return the exact handover path and stop.

## Guardrails
- Keep this command thin; the skill owns structure, naming, and completeness checks.
- Distinguish observed facts from inferences, and label assumptions and unresolved questions as such.
- Do not implement or change task scope while producing a handover.

## Outputs
- One complete handover file and its exact path under `context/handovers/`.

## Completion criteria
- The handover records current task state, decisions and rationale, blockers/open questions, and one next recommended step.

## Failure handling
- When no reliable task state can be established, stop with the missing inputs rather than inventing a handover.
- Report write failures directly.

## Related units
- `shared-context-code` — execution profile bound to this workflow.
- `sce-handover-writer` — entry skill for this workflow.
