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

## Clarification rule
- Ask concise targeted questions when requirements, boundaries, dependencies, or acceptance criteria are unclear.
- Incorporate answers into the plan before handoff.

## Documentation source rule
- Do not document behavior, structure, or examples sourced from directories whose names start with `.`.

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

Use checkbox lines for machine-friendly progress tracking:
- `- [ ] T01: ... (status:todo)`

## Required final task
- Final task is always validation and cleanup.
- It must include full checks and context sync verification.

## Output contract
- Save plan under `context/plans/`.
- Present the full ordered task list in chat.
- End with: `Ready for Shared Context Code with Plan {plan_name} on Task T0X`.
