---
name: sce-change-to-plan
description: >
  Turn one change request into a scoped SCE plan in one self-contained workflow
compatibility: opencode
---

# SCE Change to Plan

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-SCE workflow handoffs. Do not invoke another SCE skill, sibling SCE
package, or SCE workflow command. Follow the canonical workflow's steps, gates,
and stops exactly as written: never invent, skip, reorder, or merge a step.

## Phase references

Each numbered step below dispatches to a phase whose steps and boundaries live in
a reference file. This document holds the control flow — which phase runs, what it
receives, and how its result branches — and each reference holds the phase itself.

| Step | Read before running the phase |
|---|---|
| 1 | `references/context-load.md` |
| 2 and 4 | `references/plan-authoring.md` |

`references/plan-template.md` defines the plan file written to disk. The plan
authoring phase points to it at the moment a plan is actually written, which never
happens on a `needs_clarification` or `blocked` result.

Read a step's reference before taking any action for that step, not after. Read
only the reference for the step you have reached: a run that stops at the
bootstrap gate never authors a plan, which is why they are separate files.

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

Relevant non-SCE skills may be used as helper capabilities during the active step.
They are not workflow handoffs: when a helper returns, control returns to the active
step. Helper use must preserve the canonical phase order, gates, waits, writes,
validation, stops, and terminal user-visible output.

## Input

`$ARGUMENTS` is the change request, in free-form prose.

- The change request is required.
- It may describe a new plan or a change to an existing plan. Do not resolve which one applies; step 2 owns that decision.

When `$ARGUMENTS` is empty, report that a change request is required, state the expected argument, and stop. Do not infer a change request from the repository state or the conversation.

Pass the change request to step 2 unmodified. Do not restate, summarize, or pre-scope it.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is the path resolved in step 2 (`plan.path`, or an entry of `candidates`), so every emitted command is directly runnable.

## Workflow

### 1. Load durable context

Read `references/context-load.md`, then run the **Context load phase** with the
change request as the focus.

`context/` is durable AI-first memory describing current state. Load it before planning so the plan starts from recorded truth. Where context and code disagree, the code is the source of truth.

Branch on `status`:

`bootstrap_required` -> `context/` does not exist. Do not create it, and do not plan without it. Render the **Missing context bootstrap gate** layout from `references/output.md`.

Wait for the user. When they report the command ran, run the **Context load phase** again and continue in this session. Do not restart planning, and do not ask for the change request again.

`loaded` -> Continue to the next step.

Do not read `context/` yourself. Do not repair drift or stale context; the brief reports it and the plan may schedule the repair.

### 2. Author the plan

Read `references/plan-authoring.md`, then run the **Plan authoring phase** with
the change request and the complete `loaded` brief from the **Context load
phase**.

Pass the brief verbatim. Do not restate, summarize, or reinterpret it.

This phase challenges whether the change is worth building before planning how to
build it, and it decides on its own whether to stop at the clarification gate.
Both shape what reaches the user, so reach them through the reference rather than
acting from this summary.

Do not write or edit the plan file yourself.

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
- Read each phase's reference before running that phase.
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
