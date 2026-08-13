# Internal persisted-document format: Plan template

The document format for `context/plans/{plan_name}.md`. This is the plan file
written to disk, not the result returned to the workflow.

Copy the template below and fill every `{placeholder}`. Omit optional sections
entirely rather than writing them empty.

---

## Template

```markdown
# Plan: {plan-name}

## Change summary

{One or two paragraphs: what changes, where, and why. State whether this
extends existing behavior, replaces it, or preserves work already in progress.}

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: {observable outcome, stated as behavior rather than as work done}
  - Validate: `{command, assertion, or inspection that proves AC1}`
- [ ] AC2: {observable outcome}
  - Validate: `{command, assertion, or inspection that proves AC2}`

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `{full check suite command}`
- `{generated-output or parity check command, when applicable}`

### Context sync

- {Durable context files that must describe the change once implemented.}

## Constraints and non-goals

- **In scope:** {files, modules, and surfaces this plan may touch}
- **Out of scope:** {adjacent work explicitly excluded}
- **Constraints:** {dependencies, conventions, compatibility, or policy limits}
- **Non-goal:** {tempting generalization this plan deliberately avoids}

## Assumptions

{Include only when the user allowed assumptions, or ordinary local choices were
recorded. Remove the section otherwise.}

- {Assumption, and the convention or decision record it rests on.}

## Task stack

- [ ] T01: `{single intent title}` (status:todo)
  - Task ID: T01
  - Goal: {one outcome}
  - Boundaries (in/out of scope): In — {tight scope}. Out — {excluded work}.
  - Dependencies: {task IDs, or none}
  - Done when: {clear acceptance for one coherent change}
  - Verification notes (commands or checks): {targeted checks for this change}

- [ ] T02: `{single intent title}` (status:todo)
  - Task ID: T02
  - Goal: {one outcome}
  - Boundaries (in/out of scope): In — {tight scope}. Out — {excluded work}.
  - Dependencies: T01
  - Done when: {clear acceptance for one coherent change}
  - Verification notes (commands or checks): {targeted checks for this change}

## Open questions

{Non-blocking questions only. A question that would change scope, success
criteria, or task ordering blocks authoring instead. Write `None.` with a short
justification when nothing remains.}

{Unresolved doubt about the change's value belongs here — whether it is worth
building, whether it duplicates behavior the repository already has, whether a
smaller version would do. State it plainly and name the alternative. Do not
invent one: `None.` is the expected answer for a well-specified change.}
```

---

## Filled-in task example

```markdown
- [ ] T02: `Add /auth/refresh endpoint` (status:todo)
  - Task ID: T02
  - Goal: Implement a POST `/auth/refresh` endpoint that exchanges a valid refresh token for a new access token.
  - Boundaries (in/out of scope): In — route handler, token validation logic, response schema. Out — refresh token rotation policy (covered in T03), client-side storage changes.
  - Dependencies: T01
  - Done when: `POST /auth/refresh` returns a signed JWT on valid input and 401 on expired or invalid token; targeted tests pass; OpenAPI spec updated.
  - Verification notes (commands or checks): `pnpm test src/auth/refresh.test.ts`; `curl -X POST localhost:3000/auth/refresh -d '{"token":"..."}' -w "%{http_code}"`.
```

## Acceptance criteria rules

- Acceptance criteria describe the finished system, not the work. Prefer "the
  endpoint returns 401 on an expired token" over "add expiry handling".
- Every criterion carries a `Validate:` line. A criterion nobody can check is
  not an acceptance criterion.
- Prefer a runnable command. Fall back to a named inspection only when no
  automated check exists, and say exactly what to look at.
- List repository-wide checks once under `Full validation` instead of repeating
  them per criterion.
- Task-level `Verification notes` prove one task. Acceptance criteria prove the
  plan. Keep them distinct: a task's checks are narrow and local, a criterion's
  check is end-to-end.
- The union of the acceptance criteria must cover every success signal in the
  change request. If a criterion has no task that could satisfy it, the task
  stack is incomplete.

## Task rules

- Every task is a checkbox line so progress stays machine-readable:
  `- [ ] T01: {title} (status:todo)`.
- Author each executable task as one atomic commit unit by default.
- Scope every task so one contributor can complete it and land it as one
  coherent commit without bundling unrelated changes.
- Split any candidate task that would require multiple independent commits, for
  example a refactor plus a behavior change plus documentation.
- Keep broad wrappers such as `polish`, `finalize`, or `misc updates` out of
  executable tasks. Convert them into specific outcomes with concrete
  acceptance checks.
- Order tasks so each one's declared dependencies precede it.

## No validation task

- The last task in the stack is an ordinary implementation task. Do not author a
  trailing "validation and cleanup" task.
- Final validation, cleanup, and success-criteria verification are run by
  `/validate` from the `Acceptance criteria` section after the last task
  completes.
- Do not author a task whose only purpose is running the full check suite,
  verifying durable context, or removing scaffolding.
- A task may still create or update durable context when that context is part of
  the change itself.

## Completion records

When a task completes, the **Task execution phase** appends its evidence and flips the
checkbox and status:

```markdown
- [x] T01: `{title}` (status:done)
  - {authored fields, unchanged}
  - Completed: {YYYY-MM-DD}
  - Files changed: {paths}
  - Evidence: {commands run and their outcomes}
  - Notes: {material deviations or approved assumptions}
```

`/validate` appends a `## Validation Report` section at the end of the plan.
Do not author either while planning.

## Updating an existing plan

- Preserve completed tasks, their `(status:done)` markers, and their recorded
  evidence verbatim.
- Preserve the plan's existing structure and terminology.
- Append new tasks after the existing stack. Renumber only when added work must
  run earlier, and never renumber a completed task.
- Add acceptance criteria for newly planned outcomes rather than rewriting
  criteria already satisfied.
