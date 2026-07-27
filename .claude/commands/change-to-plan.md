---
description: "Run `sce-context-load` -> `sce-plan-authoring` to turn a change request into a scoped SCE plan"
argument-hint: "<describe changes you want to introduce>"
allowed-tools: Task, Read, Glob, Grep, Edit, Write, Question, Skill
---

SCE CHANGE TO PLAN `$ARGUMENTS`

## Input

`$ARGUMENTS` is the change request, in free-form prose.

- The change request is required.
- It may describe a new plan or a change to an existing plan. Do not resolve which one applies; `sce-plan-authoring` owns that decision.

When `$ARGUMENTS` is empty, report that a change request is required, state the expected argument, and stop. Do not infer a change request from the repository state or the conversation.

Pass the change request to `sce-plan-authoring` unmodified. Do not restate, summarize, or pre-scope it.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is the path resolved by `sce-plan-authoring` (`plan.path`, or an entry of `candidates`), so every emitted command is directly runnable.

## Workflow

### 1. Load durable context

Invoke `sce-context-load` with the change request as the focus.

`context/` is durable AI-first memory describing current state. Load it before planning so the plan starts from recorded truth. Where context and code disagree, the code is the source of truth.

The skill must return a result matching its context brief contract.

Branch on `status`:

`bootstrap_required` -> `context/` does not exist. Do not create it, and do not plan without it. Return:

```

-------------------------------------

# This repository has no durable context.

Bootstrap it, then continue in this session:

`sce setup --bootstrap-context`
```

Wait for the user. When they report the command ran, invoke `sce-context-load` again and continue in this session. Do not restart planning, and do not ask for the change request again.

`loaded` -> Continue to the next step.

Do not read `context/` yourself. Do not repair drift or stale context; the brief reports it and the plan may schedule the repair.

### 2. Author the plan

Invoke `sce-plan-authoring` with the change request and the complete `loaded` brief from `sce-context-load`.

Pass the brief verbatim. Do not restate, summarize, or reinterpret it.

`sce-plan-authoring` exclusively owns:

- Resolving whether the request targets a new or an existing plan.
- The clarification gate.
- Normalizing the change summary, acceptance criteria, constraints, and non-goals.
- Slicing the task stack into one-task/one-atomic-commit units.
- Writing `context/plans/{plan_name}.md`.

Do not duplicate any of it. Do not write or edit the plan file yourself.

The skill must return a result matching its authoring contract.

Branch on `status`:

`needs_clarification` -> No plan was written. Present the result as prose. Do not print the raw result. Return:

```

-------------------------------------

# Clarification needed.

No plan was written.

Answer each question below.  

## {question-id} · {category}

{question}

Why this blocks planning: {why_blocking}
```

Render one `##` block per entry in `questions`, in result order. Use the question's `id`, `category`, `question`, and `why_blocking` fields exactly as returned.

Do not answer the questions. Do not assume answers. Do not write a plan. Stop and wait.

`blocked` -> No plan was written. Present the result as prose. Do not print the raw result. Present:

- Each issue in `issues`: its problem, its impact, and the decision it requires.
- When `candidates` is present, the candidate plan paths, and that naming the intended `{candidate-path}` in the change request resolves the ambiguity.

Stop.

`plan_ready` -> Continue to the next step.

### 3. Determine the continuation

Render the `plan_ready` result as the summary defined by `sce-plan-authoring` in `references/plan-summary.md`. Follow that layout exactly. Do not print the raw result.

Take the next task from `next_task`. A `plan_ready` result always names one. Do not evaluate its dependencies; `sce-plan-review` checks them when the emitted command runs and returns `blocked` if they are unmet.

The continuation invites revision. The plan was written from one prose request, so its assumptions are guesses about what the user meant, its scope is one reading of the request, and its task boundaries are the author's judgement. The user has seen none of it until now, and every one of those is cheaper to correct here than after a task has been built on it. A user who does not know revision is on the table will implement a plan they would have changed.

Write `task` rather than `tasks` when `total_tasks` is 1.

Offer revision, but do not gate the handoff on it, do not manufacture concerns, and do not ask the user to confirm the plan. When the summary lists open questions, leave them in the summary only — do not restate them in the continuation, do not answer them, and do not block the handoff on them. Blocking questions belong in `needs_clarification` (step 2), not here.

Return:

```

-------------------------------------

# Plan {plan-name} is ready.

{total-tasks} tasks planned.

This plan is a draft. State a correction and it will be updated.

Next up:

{next-task-id} — {next-task-title}

`/next-task {plan-path} {next-task-id}`
```

Then stop and wait. Do not implement, and do not run the handoff yourself.

### 4. Revise the plan on request

When the user answers clarification questions from step 2, answers open questions listed in the summary, or answers with changes to the plan, revise it in this session. Do not ask them to rerun `/change-to-plan`, and do not ask for the original change request again.

Invoke `sce-plan-authoring` with their answer or correction and the same `loaded` brief from step 1. The brief still holds; durable context did not change because the user disagreed with a task boundary. Do not reload it.

An answer that resolves a doubt removes that open question. An answer that does not resolve it leaves the question standing; do not drop it because the user replied to it. If the reply raises a new doubt, the revised plan carries a new open question.

Pass the correction as written. Do not restate, soften, or pre-scope it. `sce-plan-authoring` owns resolving it against the existing plan, and owns preserving completed tasks and their evidence.

Branch on `status` exactly as in step 2. A revision may legitimately return `needs_clarification` or `blocked`.

On `plan_ready`, render the summary again and the continuation exactly as in step 3, replacing `is ready` with `revised` in the heading.

Revise as many times as the user asks. Each revision is one invocation of `sce-plan-authoring` against the same plan.

When the user signals the plan is good, or asks to begin, return the handoff without re-authoring the plan. Say so plainly if questions are still open: the user may proceed over an unresolved doubt, and that is their call, but do not record it as resolved.

Stop.

## Rules

- Plan at most one change request per invocation. Revisions to the plan that request produced are part of the same invocation, not a second request.
- Always tell the user the plan can be revised, and always name its assumptions as the first thing worth checking.
- Do not gate the handoff on open questions listed in the plan summary. Blocking questions return `needs_clarification` before any plan is written. Offering revision is not the same as demanding it, and inventing doubts to justify a review gate is not allowed.
- Do not suppress, soften, or answer an open question or clarification question on the user's behalf.
- Do not defer the user's revision to a rerun of `/change-to-plan`, and do not defer it to the implementation phase. Revise the plan here.
- Do not narrow, expand, or reinterpret a revision the user asked for. Pass it to `sce-plan-authoring` as written.
- Do not duplicate the internal instructions of invoked skills.
- Do not plan before durable context is loaded.
- Do not bootstrap `context/` yourself. `sce setup --bootstrap-context` owns that.
- Do not modify any file under `context/` outside `context/plans/`.
- Do not implement any part of the plan.
- Do not ask for implementation confirmation.
- Do not run task execution, context synchronization, or full-plan validation.
- Do not emit a `/validate` command. This workflow always hands off to `/next-task`.
- Do not answer the skill's clarification questions on the user's behalf.
- Do not execute the continuation returned at the end.
- Do not infer success when `sce-plan-authoring` returns a non-`plan_ready` status.
