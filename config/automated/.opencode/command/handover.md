---
description: "Run `sce-handover-writer` to capture the current task for handoff"
agent: "Shared Context Code"
---

## Purpose
- Create a durable handover for the current task by delegating to `sce-handover-writer`.

## Inputs
- `$ARGUMENTS`: optional plan name, task ID, scope note, or handover context.
- Current repository, plan, and task state available to the agent.

## Preconditions
1. Identify the current plan/task when possible.
2. Distinguish observed facts from inferred details.

## Workflow
1. Load `sce-handover-writer`.
2. Pass `$ARGUMENTS` and the current task state.
3. Let the skill choose task-aligned naming and write the handover under `context/handovers/`.
4. Return the exact handover path and stop.

## Guardrails
- Keep this command thin; the skill owns structure, naming, and completeness checks.
- Label unsupported inferences as assumptions.
- Do not implement or change task scope while producing a handover.

## Outputs
- One complete handover file and its exact path.

## Completion criteria
- The handover records current task state, decisions and rationale, blockers/open questions, and one next recommended step.

## Failure handling
- When no reliable task state can be established, stop with the missing inputs rather than inventing a handover.
- Report write failures directly.

## Related units
- `sce-handover-writer` — sole owner of handover content and file shape.
- `Shared Context Code` — default agent for this command.
