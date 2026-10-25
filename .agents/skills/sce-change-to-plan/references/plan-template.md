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

## Design decisions

{Optional. Include only real architectural choices this plan commits to. Remove
the section when there are none.}

- D1: {decision}
  - Boundary: {architectural boundary the decision governs}
  - Rationale: {why}
  - Rejected: {alternative} — {why not}
  - Trade-offs: {known limitation or cost}
  - Review: required
  - Review question: {optional; one question for senior engineering judgment; remove both review lines when none is needed}

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: {observable outcome, stated as behavior rather than as work done}
  - Validate: `{command, assertion, or inspection that proves AC1}`
  - Decisions: {decision IDs, e.g. D1; omit the field when no decision applies}
  - Scenarios: {scenario IDs, e.g. S1, S2}
- [ ] AC2: {observable outcome}
  - Validate: `{command, assertion, or inspection that proves AC2}`
  - Scenarios: none (inspection)

### Verification scenarios

- S1: {short title}
  - Given: {preconditions}
  - When: {action}
  - Then: {exact observable result or invariant}
  - Check: {planned test, command, or named inspection}

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `{full check suite command}`
- `{generated-output or parity check command, when applicable}`

### Context sync

- {Durable context files that must describe the change once implemented.}

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

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
  - Covers: {acceptance criterion IDs, e.g. AC1}
  - Scope: In — {tight scope}. Out — {excluded work}.
  - Dependencies: {task IDs, or none}
  - Done when: {clear acceptance for one coherent change}
  - Verify: {targeted checks for this change}
  - Context synchronization: pending

- [ ] T02: `{single intent title}` (status:todo)
  - Task ID: T02
  - Covers: {acceptance criterion IDs, e.g. AC2}
  - Scope: In — {tight scope}. Out — {excluded work}.
  - Dependencies: T01
  - Done when: {clear acceptance for one coherent change}
  - Verify: {targeted checks for this change}
  - Context synchronization: pending

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
  - Covers: AC1
  - Scope: In — route handler, token validation logic, response schema. Out — refresh token rotation policy (covered in T03), client-side storage changes.
  - Dependencies: T01
  - Done when: `POST /auth/refresh` returns a signed JWT on valid input and 401 on expired or invalid token; targeted tests pass; OpenAPI spec updated.
  - Verify: `pnpm test src/auth/refresh.test.ts`; `curl -X POST localhost:3000/auth/refresh -d '{"token":"..."}' -w "%{http_code}"`.
  - Context synchronization: pending
```

## Filled-in structured plan example

```markdown
## Design decisions

- D1: The existing session store validates refresh tokens; no new token service is added.
  - Boundary: `src/auth/` owns token validation; route handlers call it and never read the session store directly.
  - Rationale: The session store already tracks revocation, so one owner keeps revocation consistent.
  - Rejected: Stateless refresh tokens — they cannot be revoked before they expire.
  - Trade-offs: Every refresh costs one session-store read.

## Acceptance criteria

- [ ] AC1: `POST /auth/refresh` returns a signed JWT for a valid refresh token and 401 for an expired or revoked one.
  - Validate: `pnpm test src/auth/refresh.test.ts`
  - Decisions: D1
  - Scenarios: S1
- [ ] AC2: The OpenAPI spec documents `POST /auth/refresh` with its 200 and 401 responses.
  - Validate: inspection of `openapi.yaml` under `paths./auth/refresh`.
  - Scenarios: none (inspection)

### Verification scenarios

- S1: Revoked refresh token is rejected
  - Given: A refresh token that has not expired but is revoked in the session store.
  - When: The client calls `POST /auth/refresh` with that token.
  - Then: The response is 401 and no JWT is issued.
  - Check: `pnpm test src/auth/refresh.test.ts -t "rejects a revoked token"`, created by T01.

## Task stack

- [ ] T01: `Add /auth/refresh endpoint` (status:todo)
  - Task ID: T01
  - Covers: AC1, AC2
  - Scope: In — route handler, session-store validation call, OpenAPI entry. Out — refresh token rotation policy.
  - Dependencies: none
  - Done when: The endpoint returns a JWT or 401 as AC1 states, the revoked-token test exists and passes, and the OpenAPI entry is present.
  - Verify: `pnpm test src/auth/refresh.test.ts`.
  - Context synchronization: pending
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
- Task-level `Verify` proves one task. Acceptance criteria prove the
  plan. Keep them distinct: a task's checks are narrow and local, a criterion's
  check is end-to-end.
- The union of the acceptance criteria must cover every success signal in the
  change request. If a criterion has no task that could satisfy it, the task
  stack is incomplete.

## Design decision rules

- Record a design decision only for a real architectural choice the plan
  commits to, such as an ownership boundary, a data shape, an interface, or an
  alternative a reviewer would otherwise reopen.
- Every decision carries `Boundary:`, `Rationale:`, `Rejected:`, and
  `Trade-offs:`.
- A decision may also carry the optional review pair `Review: required` and
  `Review question: {one question}`. Mark only a decision that needs senior
  engineering judgment before the change merges, such as a trade-off a reviewer
  must accept. Leave every other decision unmarked.
- Write the review question as one specific question that a reviewer can
  accept or reject. Name the architectural choice and its consequence, so the
  reviewer can answer from the decision alone. Prefer "Is reading plans from
  the working tree instead of the `--head` commit acceptable for stable
  links?" over "Is this design OK?" or "Any concerns about plan reads?".
- Ask about each concern once. Do not mark a second decision that raises the
  same concern as another marked decision, and do not combine two concerns in
  one question.
- Mark every decision that meets this bar. There is no maximum number of
  marked decisions. Do not drop an important review question or merge
  unrelated questions to lower the count.
- Three or more marked decisions are a signal that the change may be too large
  for one focused review. Consider moving independent changes into a separate
  pull request. This is advisory: keep tightly coupled changes together, and
  complete the plan either way.
- The review pair is all or nothing: a decision carries both fields or
  neither. `Review:` is exactly `required`, and `Review question:` is one
  nonempty line that states one question.
- An unmarked decision carries no review field. Do not add a placeholder value
  such as `Review: none` or an empty `Review question:`.
- A design decision belongs to this plan only. It does not replace an
  architecture decision record.

## Verification scenario rules

- A scenario states one behavior that an acceptance criterion depends on, as
  `Given:`, `When:`, `Then:`, and `Check:`.
- `Then:` states an exact observable result or invariant, not an intention.
- `Check:` names the planned test, command, or named inspection that would
  prove `Then:`. Name a test only when it already exists or a task in the stack
  creates it.

## Task rules

- Number tasks sequentially as `T01..T0N`.
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

## Reference rules

- Read plan structure only from Markdown outside fenced code blocks. A fenced
  code block runs from a line that opens a fence of three or more backticks or
  tildes to its closing fence. Headings, definitions, and fields inside it are
  example text, not plan structure. Keep fenced examples in place; never delete
  one to make a plan read correctly.
- A section runs from its heading to the next heading of any level outside
  fenced code blocks. A definition counts only in its own section: `D` entries
  under `## Design decisions`, `AC` entries under `## Acceptance criteria`, `S`
  entries under `### Verification scenarios`, and `T` entries under
  `## Task stack`. A line shaped like a definition anywhere else, such as in a
  fenced example or a validation report, defines nothing.
- A definition's fields are the `- Field:` lines indented one level under its
  entry line, up to the next entry. `Boundary:`, `Rationale:`, `Rejected:`,
  `Trade-offs:`, and the optional `Review:` and `Review question:` belong to a
  design decision; `Validate:`, `Decisions:`, and
  `Scenarios:` to an acceptance criterion; `Given:`, `When:`, `Then:`, and
  `Check:` to a verification scenario; and `Covers:` to a task.
- To check a fenced example itself, extract the block's content and apply these
  rules to it as a separate sample document.
- IDs are `D1..Dn` for design decisions, `AC1..ACn` for acceptance criteria,
  `S1..Sn` for verification scenarios, and `T01..T0N` for tasks. Each entry
  starts its own line with its ID: `- D1:`, `- [ ] AC1:`, `- S1:`, and
  `- [ ] T01:`.
- Define each ID exactly once.
- Number decisions, criteria, and scenarios sequentially in order of first
  assignment. Never renumber or reuse a decision, criterion, or scenario ID; a
  withdrawn entry keeps its ID and says that it is withdrawn.
- Each reference field fits on one line: `Decisions:` and `Scenarios:` on an
  acceptance criterion, and `Covers:` on a task. Its value is a list of IDs
  separated by a comma and a space, for example `S1, S2`.
- Every referenced ID resolves to a definition in the same plan.
- Every acceptance criterion carries `Scenarios:`, with scenario IDs or the
  literal `none (inspection)` when only an inspection can prove it. Omit
  `Decisions:` when no decision applies.
- Every task carries `Covers:` naming at least one
  acceptance criterion defined in the plan. A task without `Covers:` or with an
  empty `Covers:` is a defect even when other tasks cover every criterion.
- Every acceptance criterion is named by at least one task's `Covers:`.
- Every scenario is referenced by at least one acceptance criterion's
  `Scenarios:`.
- A planned `Check:` is not execution evidence. Only `/validate` running or
  performing the check verifies a scenario.

## No validation task

- The last task in the stack is an ordinary implementation task. Do not author a
  trailing "validation and cleanup" task.
- Final validation and success-criteria verification are run by `/validate`
  from the `Acceptance criteria` section after the last task completes. Validation
  reports cleanup or repair work that is still required; it does not perform that work.
- Do not author a task whose only purpose is running the full check suite,
  verifying durable context, or removing scaffolding.
- A task may still create or update durable context when that context is part of
  the change itself.

## Completion records

When a task completes, the **Task execution phase** appends its evidence and flips the
checkbox and status:

```markdown
- [x] T01: `{title}` (status:done)
  - {authored fields, unchanged: Task ID, Scope, Dependencies, Done when}
  - Verify: {each planned check, updated with its actual outcome}
  - Completed: {YYYY-MM-DD}
  - Files changed: {paths}
  - Result: {concise factual outcome, not a prose diff}
  - Context impact: {durable context this change affects, or none}
  - Context synchronization: pending | synced | blocked
  - Context synchronization blocker: {present only when status is blocked} Blocker: {problem}; Required action: {action}; Retry condition: {condition}
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
- Give each new decision, criterion, or scenario the next unused ID of its kind.
