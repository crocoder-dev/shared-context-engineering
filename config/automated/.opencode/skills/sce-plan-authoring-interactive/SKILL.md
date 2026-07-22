---
name: sce-plan-authoring-interactive
description: |
  Use when a user wants to create or update a Shared Context Engineering (SCE) implementation plan with interactive clarification. Triggers on requests like "write a plan", "create an implementation roadmap", "draft a rollout plan", "plan this change", or "break this into tasks" for a software change. Interactively resolves ambiguity by asking targeted clarifying questions, then produces a structured markdown plan under `context/plans/` with a change summary, success criteria, constraints, atomic task stack, and verification steps — ready to hand off to the Shared Context Code agent for execution.
compatibility: opencode
---

## Purpose
- Convert a change request into an atomic SCE plan using an explicit human clarification loop.

## Inputs
- Change request, optional plan target, repository/context state, and human answers.

## Preconditions
1. Require an existing baseline `context/` tree.
2. Run the full ambiguity check before writing.

## Workflow
1. Resolve new-versus-existing plan and inspect relevant context/code.
2. Ask 1-3 specific blocking questions when critical details are unclear.
3. Stop until the user answers every critical question.
4. Record assumptions only when explicitly authorized.
5. Write the standard plan sections and atomic task stack.
6. Make the final task validation and cleanup.
7. Save and return path, task order, and `/next-task {plan_name} T01`.

## Guardrails
- Do not write a partial plan while critical questions remain.
- Do not invent requirements or implement the plan.
- Keep one task aligned to one coherent commit by default.

## Outputs
- Focused questions while blocked, then a complete plan and handoff.

## Completion criteria
- All critical ambiguity is resolved or explicitly authorized as an assumption.
- The saved plan satisfies the standard shape and atomicity contract.

## Failure handling
- Keep planning blocked and state exactly which answer is still required.

## Related units
- `/change-to-plan-interactive` — command entrypoint.
- `sce-plan-authoring` — deterministic non-interactive variant.
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
