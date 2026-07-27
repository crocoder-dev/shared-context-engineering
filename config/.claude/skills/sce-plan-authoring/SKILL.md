---
name: sce-plan-authoring
description: >
  Internal SCE workflow skill that turns one change request into a scoped plan
  in `context/plans/`, sliced into atomic implementation tasks, and returns one
  YAML result: plan_ready, needs_clarification, or blocked. Use from
  /change-to-plan. Do not implement plan tasks, request implementation approval,
  synchronize context, or run final validation.
compatibility: claude
---

# SCE Plan Authoring

## Purpose

Turn exactly one change request into `context/plans/{plan_name}.md` without
inventing material requirements.

This skill owns:

- Resolving whether the request targets a new or an existing plan.
- Judging whether the change is worth making, and recording the doubt when it
  is not clear that it is.
- Deciding whether the request can be planned safely.
- Normalizing the change summary, acceptance criteria, constraints, and
  non-goals.
- Slicing the work into atomic implementation tasks.
- Writing the plan file.
- Returning one structured authoring result.

Use the document format defined in:

`references/plan-template.md`

Return a result matching:

`references/authoring-contract.yaml`

The invoking workflow renders that result as the summary defined in:

`references/plan-summary.md`

## Input

The invoking workflow provides:

- One change request, in free-form prose.
- The `loaded` context brief from `sce-context-load`.

The change request may name a plan, describe a change to an existing plan, or
describe entirely new work. Resolving which applies is this skill's
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
Terseness is not ambiguity. Do not return `needs_clarification` for detail the
plan already carries; ask only when the revision itself is genuinely undecidable.

## Workflow

### 1. Resolve the plan target

Determine whether the request targets a new plan or an existing plan in
`context/plans/`.

When it targets an existing plan, read that plan before authoring. Preserve its
completed tasks, their recorded evidence, its structure, and its terminology.

When multiple existing plans match and none can be selected safely, return
`blocked` with the matching candidates.

When the request targets a new plan, derive `plan_name` as a short kebab-case
slug of the change, and confirm it does not collide with an existing plan.

Resolve exactly one plan target per invocation.

### 2. Challenge the change

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

### 3. Run the clarification gate

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

Return `needs_clarification` with one to three targeted questions when any of
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

### 4. Inspect relevant context

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

### 5. Author the acceptance criteria

State how the finished plan is proven, before slicing tasks.

Each criterion describes observable behavior of the finished system and names
the check that proves it. Record repository-wide checks once under
`Full validation`, and the durable context the change must be reflected in
under `Context sync`.

`/validate` runs this section after the last task completes. It is the only
place a plan says how it is validated.

### 6. Author the task stack

Slice the work into sequential tasks `T01..T0N` using the task format and the
atomic slicing contract in `references/plan-template.md`.

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

A finished stack always leaves at least one incomplete task, so the invoking
workflow can always hand off to `/next-task`. When the request resolves to a
plan but produces no incomplete task, because the change is already implemented
or already covered by completed tasks, return `blocked` with category
`no_actionable_work` instead of writing the plan.

### 7. Write the plan

Write `context/plans/{plan_name}.md` using `references/plan-template.md`.

When updating an existing plan, keep completed tasks and their evidence intact,
and append or renumber new tasks without disturbing recorded history.

### 8. Return the result

Return exactly one structured result:

- `plan_ready`
- `needs_clarification`
- `blocked`

Return only the structured result. Do not add explanatory prose before or after
it.

## Tone

Every question and open question this skill writes is read by the user. Write
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

## Boundaries

Do not:

- Ask the user questions directly. Return `needs_clarification` and let the
  invoking workflow present the questions.
- Answer your own clarification questions.
- Write a plan file when returning `needs_clarification` or `blocked`.
- Implement any task in the plan.
- Modify application code or tests.
- Modify any file under `context/` outside `context/plans/`. Plan the context
  repair instead of performing it.
- Mark any task complete.
- Request implementation confirmation.
- Invoke task execution.
- Synchronize context.
- Run final validation.
- Author a validation, cleanup, or context-verification task. `/validate` owns
  that phase.
- Return `plan_ready` for a plan with no incomplete task.
- Create a Git commit.
- Author more than one plan.

## Completion

The skill is complete after:

- One plan target was resolved, or resolution failed and was reported.
- The plan file was written, or no file was written because the result is
  `needs_clarification` or `blocked`.
- One valid result matching `references/authoring-contract.yaml` was returned.
