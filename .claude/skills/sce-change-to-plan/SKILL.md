---
name: sce-change-to-plan
description: >
  Turn one change request into a scoped SCE plan in one self-contained workflow
compatibility: claude
---

# SCE Change to Plan

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-skill handoffs. Do not invoke another SCE skill, sibling package, or
workflow command. Follow the canonical workflow's steps, gates, and stops exactly
as written: never invent, skip, reorder, or merge a step.

## User-visible output

Use `references/output.md` for every gate and terminal response. Render no raw
internal state. The reference contains only human-visible Markdown layouts.
User-visible output is limited to those layouts: never invent a layout, and never
wrap one in an added preamble, commentary, summary, or extra section.

## Composite control flow

Keep phase results as internal state and continue immediately whenever the
canonical workflow says to continue. Stop only at a user wait or terminal branch.
Approval, clarification, revision, failed-validation repair, and bootstrap waits
resume this same skill in the same session. Never expose an internal phase result
as the workflow's final response.

## Input

`$ARGUMENTS` is the change request, in free-form prose.

- The change request is required.
- It may describe a new plan or a change to an existing plan. Do not resolve which one applies; step 2 owns that decision.

When `$ARGUMENTS` is empty, report that a change request is required, state the expected argument, and stop. Do not infer a change request from the repository state or the conversation.

Pass the change request to step 2 unmodified. Do not restate, summarize, or pre-scope it.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is the path resolved in step 2 (`plan.path`, or an entry of `candidates`), so every emitted command is directly runnable.

## Workflow

### 1. Load durable context

Run the **Context load phase** with the change request as the focus.

`context/` is durable AI-first memory describing current state. Load it before planning so the plan starts from recorded truth. Where context and code disagree, the code is the source of truth.

#### 1.1 Confirm the context root

When `context/` does not exist, set internal status `bootstrap_required` immediately. Read
nothing further.

Bootstrapping is the workflow's decision, not this phase's.

#### 1.2 Read the entry points

Read, when present:

- `context/context-map.md`
- `context/overview.md`
- `context/glossary.md`

Read `context/architecture.md` when the focus touches structure, boundaries, or
data flow. Read `context/patterns.md` when it touches conventions the change
must follow.

A missing entry point is a gap, not a failure. Record it and continue.

#### 1.3 Select the relevant domain context

Consult `context/context-map.md` before any broad exploration. The map's
annotations name what each domain file owns; use them to select files, rather
than globbing or searching `context/`.

Select only files whose subject overlaps the focus. Follow at most one level of
links out of a selected file, and only when the link is needed to understand the
focus.

Do not read every domain file. A brief that includes everything has selected
nothing.

Record focus areas with no matching context file under `gaps`.

#### 1.4 Check recorded context against the code

For each selected file, spot-check its central claims against the code it
describes.

When context and code diverge, the code is the source of truth. Record the
divergence under `drift` with what context says, what the code shows, and the
repair the context needs.

Do not repair it here. Later phases decide whether repair belongs in the current
work.

Keep this proportional: check the claims the focus depends on, not every
sentence.

#### 1.5 Return the brief

Set exactly one internal state:

- `loaded`
- `bootstrap_required`

Report facts the workflow can act on. A brief that only lists file
paths has moved no knowledge.

Record only the internal state. Do not add explanatory prose before or after
it.

### Context load boundaries

Do not:

- Create, update, move, or delete any file under `context/`.
- Bootstrap `context/`.
- Repair drift or stale context.
- Modify application code or tests.
- Read the entire `context/` tree by default.
- Explore the repository beyond what the focus and the selected context require.
- Ask the user questions. Report gaps and drift, and let the workflow decide.
- Author a plan, select a task, or implement anything.


Branch on `status`:

`bootstrap_required` -> `context/` does not exist. Do not create it, and do not plan without it. Render the **Missing context bootstrap gate** layout from `references/output.md`.

Wait for the user. When they report the command ran, run the **Context load phase** again and continue in this session. Do not restart planning, and do not ask for the change request again.

`loaded` -> Continue to the next step.

Do not read `context/` yourself. Do not repair drift or stale context; the brief reports it and the plan may schedule the repair.

### 2. Author the plan

Run the **Plan authoring phase** with the change request and the complete `loaded` brief from the **Context load phase**.

Pass the brief verbatim. Do not restate, summarize, or reinterpret it.

The **Plan authoring phase** exclusively owns:

- Resolving whether the request targets a new or an existing plan.
- The clarification gate.
- Normalizing the change summary, acceptance criteria, constraints, and non-goals.
- Slicing the task stack into one-task/one-atomic-commit units.
- Writing `context/plans/{plan_name}.md`.

Do not duplicate any of it. Do not write or edit the plan file yourself.

Use the document format defined in the **Plan template** section embedded in this file.

The workflow renders that result as the summary defined in:

`references/output.md`

The change request may name a plan, describe a change to an existing plan, or
describe entirely new work. Resolving which applies is this phase's
responsibility.

The context brief is the durable memory this plan starts from. Treat its
`key_facts` as recorded current state, its `gaps` as areas with no durable
context, and its `drift` as context the code has already outrun.

When no brief is supplied, load the context named by the change request before
authoring, and follow the selection discipline in *Inspect relevant context*.

Answers the user gave to earlier clarification questions arrive as part of the
change request. Incorporate them into the plan.

A revision of a plan authored earlier in the session also arrives as the change
request, and it is usually terse: a task boundary the user disagrees with, an
ordering they want changed, work they want added or dropped. Read it against the
existing plan, which supplies the scope, criteria, and terminology it omits.
Terseness is not ambiguity. Do not set internal status `needs_clarification` for detail the
plan already carries; ask only when the revision itself is genuinely undecidable.

#### 2.1 Resolve the plan target

Determine whether the request targets a new plan or an existing plan in
`context/plans/`.

When it targets an existing plan, read that plan before authoring. Preserve its
completed tasks, their recorded evidence, its structure, and its terminology.

When multiple existing plans match and none can be selected safely, return
`blocked` with the matching candidates.

When the request targets a new plan, derive `plan_name` as a short kebab-case
slug of the change, and confirm it does not collide with an existing plan.

Resolve exactly one plan target per invocation.

#### 2.2 Challenge the change

Before planning how to build the change, work out whether it is worth building.
A plan is a commitment of someone's time; authoring one for work that should not
happen is worse than authoring none.

Interrogate the request:

- What breaks, or stays broken, if this is never built? If the answer is
  nothing concrete, say so.
- What problem is it actually solving, as opposed to what it proposes to do? A
  request that names only a solution has not stated a problem.
- Does the repository already do this, or most of it? The brief's `key_facts`
  are the first place to check.
- Is there a materially smaller version that gets most of the value? Name it.
- What does this cost beyond the tasks: new dependency, new concept in the
  glossary, a boundary crossed, a surface that now needs maintaining forever?
- Does the stated justification survive contact with the code, or does the code
  show the premise is already false?

Doubt that survives this is not an implementation detail to be tidied away. It
belongs in the plan's `Open questions` and in `open_questions`, in the plain
words you would use to a colleague. "Is this worth doing at all, given X?" is a
legitimate open question. So is "this looks like it duplicates Y".

Weigh honestly in both directions. A request that is obviously worth building
gets no manufactured doubt: inventing questions to look rigorous is its own
failure, and it teaches the user to ignore the section. Most changes are fine.
Say nothing when there is nothing to say.

Keep going regardless. Skepticism shapes the plan and the open questions; it
does not withhold the plan. The only value judgment that stops authoring is
`no_actionable_work`, when the change is already implemented.

#### 2.3 Run the clarification gate

Before writing or updating any plan file, check the request for critical
unresolved detail:

- Scope boundaries and out-of-scope items.
- Acceptance criteria and the checks that prove them.
- Constraints and non-goals.
- Dependency choices, including new libraries or services, versions, and the
  integration approach.
- Domain ambiguity, including unclear business rules, terminology, or ownership.
- Architecture concerns, including patterns, interfaces, data flow, migration
  strategy, and risk tradeoffs.
- Task ordering assumptions and prerequisite sequencing.

Set internal status `needs_clarification` with one to three targeted questions when any of
these would materially change the plan. Write no plan file in that case.

Use repository conventions for ordinary local choices. Do not block on:

- Naming inferable from surrounding code.
- Established formatting or style.
- Reversible local implementation details.
- Details that do not change scope, acceptance criteria, or task ordering.

Record those choices under `assumptions`.

Do not silently invent missing requirements. When the user has explicitly
allowed assumptions, record them in the plan's `Assumptions` section instead of
asking.

A justification that does not survive inspection is itself a critical unresolved
detail. "For consistency", "to make it cleaner", "we will need it later" name no
outcome and prove nothing; ask what the change is actually for before planning
around it. Do not treat confident phrasing as evidence.

#### 2.4 Inspect relevant context

Start from the context brief. Read code only where the brief leaves the change
underspecified:

- Existing behavior the change affects.
- Applicable repository conventions.
- Architectural boundaries.
- Relevant tests and available verification commands.
- Decisions or specifications connected to the change.

Where the brief reports `drift`, the code is the source of truth. Plan against
the code, and schedule the context repair as part of the change when it falls
inside scope.

Where the brief reports `gaps`, the plan may need to establish durable context
the repository does not yet have.

Do not explore the entire repository by default.

#### 2.5 Author the acceptance criteria

State how the finished plan is proven, before slicing tasks.

Each criterion describes observable behavior of the finished system and names
the check that proves it. Record repository-wide checks once under
`Full validation`, and the durable context the change must be reflected in
under `Context sync`.

`/validate` runs this section after the last task completes. It is the only
place a plan says how it is validated.

#### 2.6 Author the task stack

Slice the work into sequential tasks `T01..T0N` using the task format and the
atomic slicing contract in the **Plan template** section embedded in this file.

Every executable task must be completable and landable as one coherent commit.
Split any task that would require multiple independent commits. Convert broad
wrappers such as `polish` or `finalize` into specific outcomes with concrete
acceptance checks.

Order tasks so each one's declared dependencies precede it.

The last task is an ordinary implementation task. Do not author a trailing
validation-and-cleanup task, or any task whose only purpose is running the full
check suite, verifying durable context, or removing scaffolding.

Confirm every acceptance criterion is satisfied by at least one task. When one
is not, the task stack is incomplete.

A finished stack always leaves at least one incomplete task, so the workflow
can always hand off to `/next-task`. When the request resolves to a
plan but produces no incomplete task, because the change is already implemented
or already covered by completed tasks, set internal status `blocked` with category
`no_actionable_work` instead of writing the plan.

#### 2.7 Write the plan

Write `context/plans/{plan_name}.md` using the **Plan template** section embedded in this file.

When updating an existing plan, keep completed tasks and their evidence intact,
and append or renumber new tasks without disturbing recorded history.

#### 2.8 Return the result

Set exactly one internal state:

- `plan_ready`
- `needs_clarification`
- `blocked`

Record only the internal state. Do not add explanatory prose before or after
it.

### Plan authoring tone

Every question and open question this phase writes is read by the user. Write
them the way a senior engineer talks in review: direct, specific, and unbothered
by the possibility of being unwelcome.

- Ask about the thing that actually worries you, not a safer neighbouring thing.
  A question you would not bother asking a colleague is not worth the user's
  attention either.
- State a doubt as a doubt. "I do not think this is worth the two tasks it
  costs, because X" is useful. "It may be worth considering whether this aligns
  with broader goals" is noise.
- Name the alternative you have in mind. A challenge with no proposal behind it
  is just friction.
- Do not open with praise, do not close with reassurance, and do not apologize
  for asking. Do not pad a doubt with hedges to make it land more gently.
- Be persistent, not repetitive. Ask once, plainly, and let it stand; do not
  restate the same doubt in three shapes to give it more weight.
- Being disagreeable is not the goal. Being easy to agree with is the failure
  mode. A plan the user waves through without reading has cost them nothing and
  bought them nothing.

When the user overrules a doubt, record it and move on. Do not relitigate a
decision the user has made, and do not smuggle the objection back in as a
constraint, a non-goal, or a task.

### Plan authoring boundaries

Do not:

- Ask the user questions directly. Set internal status `needs_clarification` and let the
  workflow present the questions.
- Answer your own clarification questions.
- Write a plan file when returning `needs_clarification` or `blocked`.
- Implement any task in the plan.
- Modify application code or tests.
- Modify any file under `context/` outside `context/plans/`. Plan the context
  repair instead of performing it.
- Mark any task complete.
- Request implementation confirmation.
- Run task execution.
- Synchronize context.
- Run final validation.
- Author a validation, cleanup, or context-verification task. `/validate` owns
  that phase.
- Set internal status `plan_ready` for a plan with no incomplete task.
- Create a Git commit.
- Author more than one plan.


Branch on `status`:

`needs_clarification` -> No plan was written. Present the result as prose. Do not print the raw result. Render the **Clarification gate** layout from `references/output.md`.

Render one `##` block per entry in `questions`, in result order. Use the question's `id`, `category`, `question`, and `why_blocking` fields exactly as returned.

Do not answer the questions. Do not assume answers. Do not write a plan. Stop and wait.

`blocked` -> No plan was written. Render the **Blocked** layout from `references/output.md`, drawing its issues from `issues` and, when `candidates` is present, its candidate paths from `candidates`. Do not print the raw result. Stop.

`plan_ready` -> Continue to the next step.

### 3. Determine the continuation

Render the `plan_ready` result as the summary defined by the **Plan authoring phase** in `references/output.md`. Follow that layout exactly. Do not print the raw result.

Take the next task from `next_task`. A `plan_ready` result always names one. Do not evaluate its dependencies; the **Plan review phase** checks them when the emitted command runs and returns `blocked` if they are unmet.

The continuation invites revision. The plan was written from one prose request, so its assumptions are guesses about what the user meant, its scope is one reading of the request, and its task boundaries are the author's judgement. The user has seen none of it until now, and every one of those is cheaper to correct here than after a task has been built on it. A user who does not know revision is on the table will implement a plan they would have changed.

Write `task` rather than `tasks` when `total_tasks` is 1.

Offer revision, but do not gate the handoff on it, do not manufacture concerns, and do not ask the user to confirm the plan. When the summary lists open questions, leave them in the summary only — do not restate them in the continuation, do not answer them, and do not block the handoff on them. Blocking questions belong in `needs_clarification` (step 2), not here.

Render the **Ready continuation** layout from `references/output.md`.

Then stop and wait. Do not implement, and do not run the handoff yourself.

### 4. Revise the plan on request

When the user answers clarification questions from step 2, answers open questions listed in the summary, or answers with changes to the plan, revise it in this session. Do not ask them to rerun `/change-to-plan`, and do not ask for the original change request again.

Run the **Plan authoring phase** with their answer or correction and the same `loaded` brief from step 1. The brief still holds; durable context did not change because the user disagreed with a task boundary. Do not reload it.

An answer that resolves a doubt removes that open question. An answer that does not resolve it leaves the question standing; do not drop it because the user replied to it. If the reply raises a new doubt, the revised plan carries a new open question.

Pass the correction as written. Do not restate, soften, or pre-scope it. The **Plan authoring phase** owns resolving it against the existing plan, and owns preserving completed tasks and their evidence.

Branch on `status` exactly as in step 2. A revision may legitimately return `needs_clarification` or `blocked`.

On `plan_ready`, render the summary again and the continuation exactly as in step 3, replacing `is ready` with `revised` in the heading.

Revise as many times as the user asks. Each revision is one invocation of the **Plan authoring phase** against the same plan.

When the user signals the plan is good, or asks to begin, return the handoff without re-authoring the plan. Say so plainly if questions are still open: the user may proceed over an unresolved doubt, and that is their call, but do not record it as resolved.

Stop.

## Rules

- Plan at most one change request per invocation. Revisions to the plan that request produced are part of the same invocation, not a second request.
- Always tell the user the plan can be revised, and always name its assumptions as the first thing worth checking.
- Do not gate the handoff on open questions listed in the plan summary. Blocking questions return `needs_clarification` before any plan is written. Offering revision is not the same as demanding it, and inventing doubts to justify a review gate is not allowed.
- Do not suppress, soften, or answer an open question or clarification question on the user's behalf.
- Do not defer the user's revision to a rerun of `/change-to-plan`, and do not defer it to the implementation phase. Revise the plan here.
- Do not narrow, expand, or reinterpret a revision the user asked for. Pass it to the **Plan authoring phase** as written.
- Do not duplicate the internal instructions of embedded phases.
- Do not plan before durable context is loaded.
- Do not bootstrap `context/` yourself. `sce setup --bootstrap-context` owns that.
- Do not modify any file under `context/` outside `context/plans/`.
- Do not implement any part of the plan.
- Do not ask for implementation confirmation.
- Do not run task execution, context synchronization, or full-plan validation.
- Do not emit a `/validate` command. This workflow always hands off to `/next-task`.
- Do not answer the skill's clarification questions on the user's behalf.
- Do not execute the continuation returned at the end.
- Do not infer success when the **Plan authoring phase** returns a non-`plan_ready` status.

## Internal persisted-document format: Plan template

The document format for `context/plans/{plan_name}.md`. This is the plan file
written to disk, not the result returned to the workflow.

Copy the template below and fill every `{placeholder}`. Omit optional sections
entirely rather than writing them empty.

---

### Template

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

### Filled-in task example

```markdown
- [ ] T02: `Add /auth/refresh endpoint` (status:todo)
  - Task ID: T02
  - Goal: Implement a POST `/auth/refresh` endpoint that exchanges a valid refresh token for a new access token.
  - Boundaries (in/out of scope): In — route handler, token validation logic, response schema. Out — refresh token rotation policy (covered in T03), client-side storage changes.
  - Dependencies: T01
  - Done when: `POST /auth/refresh` returns a signed JWT on valid input and 401 on expired or invalid token; targeted tests pass; OpenAPI spec updated.
  - Verification notes (commands or checks): `pnpm test src/auth/refresh.test.ts`; `curl -X POST localhost:3000/auth/refresh -d '{"token":"..."}' -w "%{http_code}"`.
```

### Acceptance criteria rules

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

### Task rules

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

### No validation task

- The last task in the stack is an ordinary implementation task. Do not author a
  trailing "validation and cleanup" task.
- Final validation, cleanup, and success-criteria verification are run by
  `/validate` from the `Acceptance criteria` section after the last task
  completes.
- Do not author a task whose only purpose is running the full check suite,
  verifying durable context, or removing scaffolding.
- A task may still create or update durable context when that context is part of
  the change itself.

### Completion records

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

### Updating an existing plan

- Preserve completed tasks, their `(status:done)` markers, and their recorded
  evidence verbatim.
- Preserve the plan's existing structure and terminology.
- Append new tasks after the existing stack. Renumber only when added work must
  run earlier, and never renumber a completed task.
- Add acceptance criteria for newly planned outcomes rather than rewriting
  criteria already satisfied.
