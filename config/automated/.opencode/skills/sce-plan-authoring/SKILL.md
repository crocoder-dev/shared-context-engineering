---
name: sce-plan-authoring
description: |
  Transforms a change request into a structured implementation plan saved under context/plans/, breaking work down into atomic, commit-sized tasks with clear goals, scope boundaries, acceptance criteria, and verification steps. Use when a user wants to create or update a project plan, task breakdown, implementation roadmap, or work plan — including requests like "plan this feature", "break this into tasks", "write an implementation plan", or "scope out this change". Handles ambiguity resolution through a blocking clarification gate before writing any plan, and produces a machine-friendly task list ready for handoff to the Shared Context Engineering (SCE) implementation agent.
compatibility: opencode
---

## Purpose
- Convert a complete change request into an atomic SCE plan without interactive clarification.

## Inputs
- Complete change description, success criteria, constraints, non-goals, dependencies, architecture decisions, sequencing, and plan target.
- Relevant code and context state.

## Preconditions
1. Require an existing baseline `context/` tree.
2. Run the full ambiguity check before writing.
3. Require every critical detail to be explicit or already authoritative.

## Workflow
1. Resolve new-versus-existing plan and stable `plan_name`.
2. Inspect relevant context and code.
3. Collect every unresolved critical item and categorize it.
4. When any item remains, emit one structured error and stop without writing.
5. Otherwise write the standard plan sections and atomic task stack.
6. Make the final task validation and cleanup.
7. Save the plan and return path, task order, and `/next-task {plan_name} T01`.

## Guardrails
- Do not ask interactive questions.
- Do not invent assumptions silently.
- Do not implement the plan.
- Keep one task aligned to one coherent commit by default.

## Outputs
- A complete plan or one structured error containing all unresolved items.

## Completion criteria
- The plan satisfies the same shape, atomicity, and final-validation contract as the manual profile.

## Failure handling
- Use `PLANNING BLOCKED` and category labels such as `scope`, `dependency`, `criteria`, `domain`, `architecture`, and `sequencing`.
- Do not create a partial plan.

## Related units
- `/change-to-plan` — deterministic entrypoint.
- `sce-plan-authoring-interactive` — human clarification alternative.
- `sce-plan-review` — downstream consumer.

## Reference
Use this plan shape:

```markdown
# Plan: {plan_name}

## Change summary
...

## Success criteria
- ...

## Constraints and non-goals
- ...

## Assumptions
- ...  <!-- include only when explicitly allowed -->

## Task stack
- [ ] T01: `{single intent title}` (status:todo)
  - Task ID: T01
  - Goal: `{one outcome}`
  - Boundaries (in/out of scope): `{tight scope}`
  - Done when: `{observable acceptance checks}`
  - Verification notes (commands or checks): `{targeted evidence}`

## Open questions
- ...
```

Accept each executable task only when it has one primary intent, a narrow related touch area, and one coherent verification surface. Make the final task validation and cleanup, including full checks and context-sync verification.
