---
name: sce-plan-authoring
description: Use when user wants to Create or update an SCE implementation plan with scoped atomic tasks.
---

## Purpose
- Convert a change request into a reviewable implementation plan at `context/plans/{plan_name}.md`.
- Slice executable work into atomic, commit-sized tasks with explicit acceptance and verification.

## Inputs
- Change description, success criteria, constraints, non-goals, dependencies, risks, and requested plan target.
- Relevant code and context needed to establish current truth.
- User answers to blocking clarification questions.

## Preconditions
1. Treat planning as mandatory when a request contains both a change description and success criteria.
2. Run an ambiguity check before writing or updating the plan.
3. Resolve scope boundaries, acceptance signals, constraints, dependency choices, domain rules, architecture concerns, migration strategy, and sequencing assumptions.
4. Ask 1-3 targeted questions and stop when any critical detail remains unresolved.

## Workflow
1. Resolve whether to create a new plan or update an existing plan and choose a stable kebab-case `plan_name`.
2. Inspect relevant context first, then only the code needed to ground the plan.
3. Run the clarification gate and incorporate user answers.
4. Record assumptions only when the user explicitly authorizes assumptions.
5. Write `Change summary`, `Success criteria`, `Constraints and non-goals`, optional `Assumptions`, `Task stack`, and `Open questions`.
6. Give each task a stable ID, one goal, explicit in/out boundaries, observable done checks, and targeted verification notes.
7. Split any task that would require multiple independent commits or unrelated outcomes.
8. Make the final task validation and cleanup with full checks and context-sync verification.
9. Save the plan, return the exact path and full ordered task list, and provide `/next-task {plan_name} T01`.

## Guardrails
- Do not implement the plan.
- Do not silently invent requirements or dependency choices.
- Do not use vague executable tasks such as `polish`, `misc updates`, or `finalize` without concrete outcomes.
- Treat planning as a proposal, not execution approval.
- Keep one task aligned to one coherent atomic commit by default.

## Outputs
- A complete plan file under `context/plans/`.
- Exact path, ordered task list, and canonical first-task command.
- Focused questions instead of a partial plan when blocked.

## Completion criteria
- All critical ambiguity is resolved or explicitly recorded as an approved assumption.
- Every task is executable, bounded, verifiable, and atomic by default.
- The final validation/cleanup task is present.

## Failure handling
- Stop before writing when critical information is unresolved.
- Ask specific questions that name the decision category and why it blocks safe planning.
- Report a write failure without claiming the plan exists.

## Related units
- `/change-to-plan` — thin command entrypoint.
- `Shared Context Plan` — orchestrates this skill.
- `sce-plan-review` — consumes the completed plan before implementation.

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
