---
name: sce-plan-authoring
description: Use when user wants to Create or update an SCE implementation plan with scoped atomic tasks.
compatibility: opencode
---

## Goal
Turn a human change request into `context/plans/{plan_name}.md`.

## Intake trigger
- If a request includes both a change description and success criteria, planning is mandatory before implementation.
- Planning does not imply execution approval.

## Clarification gate (blocking)
- Before writing or updating any plan, run an ambiguity check.
- If any critical detail is unclear, ask 1-3 targeted questions and stop.
- Do not write or update `context/plans/{plan_name}.md` until the user answers.
- Critical details that must be resolved before planning include:
  - scope boundaries and out-of-scope items
  - success criteria and acceptance signals
  - constraints and non-goals
  - dependency choices (new libs/services, versions, and integration approach)
  - domain ambiguity (unclear business rules, terminology, or ownership)
  - architecture concerns (patterns, interfaces, data flow, migration strategy, and risk tradeoffs)
  - task ordering assumptions and prerequisite sequencing
- Do not silently invent missing requirements.
- If the user explicitly allows assumptions, record them in an `Assumptions` section.
- Incorporate user answers into the plan before handoff.

## Documentation source rule

## Plan format
1) Change summary
2) Success criteria
3) Constraints and non-goals
4) Task stack (`T01..T0N`)
5) Open questions (if any)

## Task format (required)
For each task include:
- Task ID
- Goal
- Boundaries (in/out of scope)
- Done when
- Verification notes (commands or checks)

## Atomic task slicing contract (required)
- Author each executable task as one atomic commit unit by default.
- Every task must be scoped so one contributor can complete it and land it as one coherent commit without bundling unrelated changes.
- If a candidate task would require multiple independent commits (for example: refactor + behavior change + docs), split it into separate sequential tasks before finalizing the plan.
- Keep broad wrappers (`polish`, `finalize`, `misc updates`) out of executable tasks; convert them into specific outcomes with concrete acceptance checks.

Use this quick atomicity check before accepting each task:
- `single_intent`: task delivers one primary outcome
- `single_area`: task touch scope is narrow and related
- `single_verification`: done checks validate one coherent change set

Example compliant skeleton:
- [ ] T0X: <single intent title> (status:todo)
  - Task ID: T0X
  - Goal: <one outcome>
  - Boundaries (in/out of scope): <tight scope>
  - Done when: <clear acceptance for one coherent change>
  - Verification notes (commands or checks): <targeted checks for this change>

Use checkbox lines for machine-friendly progress tracking:
- `- [ ] T01: ... (status:todo)`

## Required final task
- Final task is always validation and cleanup.
- It must include full checks and context sync verification.

## Output contract
- Save plan under `context/plans/`.
- Confirm plan creation with `plan_name` and exact file path.
- Present the full ordered task list in chat.
- Prompt the user to start a new session with Shared Context Code agent to implement `T01`.
- Provide one canonical next command: `/next-task {plan_name} T01`.
