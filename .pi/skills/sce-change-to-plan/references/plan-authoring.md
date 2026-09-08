# Plan authoring phase

Run this phase for step 2 of the workflow, and again for each revision in step 4.

Input: the change request, and the complete `loaded` brief from the context load
phase. Pass the brief verbatim; do not restate, summarize, or reinterpret it.

This phase owns the planning process:

- Resolve whether the request targets a new or an existing plan.
- Challenge the change and run the clarification gate.
- Derive plan-specific content from the request and loaded context.
- Decide task boundaries, dependencies, and ordering.
- Write or revise exactly one `context/plans/{plan_name}.md`.

`references/plan-template.md` is the sole owner of the persisted plan schema.
Read its `Acceptance criteria rules`, `Task rules` and `No validation task` rules,
and `Updating an existing plan` rules before authoring or revising. Apply those
rules rather than restating them here.

The workflow renders this phase's result as the summary defined in
`references/output.md`.

The change request may name a plan, describe a change to an existing plan, or
describe entirely new work. Resolving which applies is this phase's
responsibility.

The context brief is the durable memory this plan starts from. Treat its
`key_facts` as recorded current state, its `gaps` as areas with no durable
context, and its `drift` as recorded context that no longer matches the code.

When no brief is supplied, load the context named by the change request before
authoring, and follow the selection discipline in *Inspect relevant context*.

Answers the user gave to earlier clarification questions arrive as part of the
change request. Incorporate them into the plan.

A revision of a plan authored earlier in the session also arrives as the change
request, and it is usually terse: a task boundary the user disagrees with, an
ordering they want changed, work they want added or dropped. Read it against the
existing plan, which supplies the scope, criteria, and terminology it omits.
Terseness is not ambiguity. Do not set internal status `needs_clarification` for
detail the plan already carries; ask only when the revision itself is genuinely
undecidable.

## 2.1 Resolve the plan target

Determine whether the request targets a new plan or an existing plan in
`context/plans/`.

When it targets an existing plan, read that plan before authoring. Apply the
`Updating an existing plan` rules in `references/plan-template.md` when writing;
this step only resolves which plan is being revised.

When multiple existing plans match and none can be selected safely, return
`blocked` with the matching candidates.

When the request targets a new plan, derive `plan_name` as a short kebab-case
slug of the change, and confirm it does not collide with an existing plan.

Resolve exactly one plan target per invocation.

## 2.2 Challenge the change

Before planning how to build the change, work out whether it is worth building. A
plan is a commitment of someone's time; authoring one for work that should not
happen is worse than authoring none.

Interrogate the request:

- What breaks, or stays broken, if this is never built? If the answer is nothing
  concrete, say so.
- What problem is it actually solving, as opposed to what it proposes to do? A
  request that names only a solution has not stated a problem.
- Does the repository already do this, or most of it? The brief's `key_facts` are
  the first place to check.
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

Keep going regardless. Skepticism shapes the plan and the open questions; it does
not withhold the plan. The only value judgment that stops authoring is
`no_actionable_work`, when the change is already implemented.

## 2.3 Run the clarification gate

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

Set internal status `needs_clarification` with one to three targeted questions
when any of these would materially change the plan. Write no plan file in that
case.

Use repository conventions for ordinary local choices. Do not block on:

- Naming inferable from surrounding code.
- Established formatting or style.
- Reversible local implementation details.
- Details that do not change scope, acceptance criteria, or task ordering.

Record those choices under `assumptions`.

Do not silently invent missing requirements. When the user has explicitly allowed
assumptions, record them in the plan's `Assumptions` section instead of asking.

A justification that does not survive inspection is itself a critical unresolved
detail. "For consistency", "to make it cleaner", "we will need it later" name no
outcome and prove nothing; ask what the change is actually for before planning
around it. Do not treat confident phrasing as evidence.

## 2.4 Inspect relevant context

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

## 2.5 Author the acceptance criteria

Derive the plan-specific success outcomes and checks before slicing tasks, then
apply the `Acceptance criteria rules` and exact section shape in
`references/plan-template.md`. The template owns their generic validation
semantics and placement.

## 2.6 Author the task stack

Slice and order the plan-specific work after the acceptance criteria, applying the
`Task rules` and `No validation task` rules in `references/plan-template.md`.
Do not restate those generic task rules here.

A finished stack always leaves at least one incomplete task, so the workflow can
always hand off to `/next-task`. When the request resolves to a plan but produces
no incomplete task, because the change is already implemented or already covered
by completed tasks, set internal status `blocked` with category
`no_actionable_work` instead of writing the plan.

## 2.7 Write the plan

Write `context/plans/{plan_name}.md` by applying `references/plan-template.md`
exactly. For revisions, apply its `Updating an existing plan` rules.

## 2.8 Return the result

Return one internal result with one of these statuses:

- `plan_ready`
- `needs_clarification`
- `blocked`

Return only the internal result. Do not add explanatory prose before or after it.

A `plan_ready` result always names the next task in `next_task`, and carries the
`total_tasks` count and any open questions the summary needs. Step 3 renders those
without recomputing them.

## Plan authoring tone

Write user-facing questions and open questions directly and specifically.

- Ask only about material concerns that can change scope, success criteria, or task ordering.
- State the concern and the concrete evidence for it.
- Name a smaller or safer alternative when one is known.
- Do not invent concerns, add praise or reassurance, or repeat the same concern in several forms.
- When the user overrules a concern, record the decision and continue. Do not reintroduce it as a constraint, non-goal, or task.

## Plan authoring boundaries

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
- Write a plan that violates `references/plan-template.md`.
- Set internal status `plan_ready` for a plan with no incomplete task.
- Create a Git commit.
- Author more than one plan.
