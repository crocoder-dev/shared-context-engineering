---
name: sce-validate
description: >
  Validate one completed SCE plan and synchronize its durable context
compatibility: opencode
---

# SCE Validate

## Purpose

Own this workflow from input parsing through its terminal user-visible response.
Execute the phases below directly and in order. Phase statuses are internal state,
not inter-skill handoffs. Do not invoke another SCE skill, sibling package, or
workflow command except `sce-decision`, and invoke `sce-decision` only from the
successful context-synchronization decision gate. Follow the canonical workflow's steps, gates,
and stops exactly as written: never invent, skip, reorder, or merge a step.

## Phase references

Each numbered step below dispatches to a phase whose steps and boundaries live in
a reference file. This document holds the control flow — which phase runs, what it
receives, and how its result branches — and each reference holds the phase itself.

| Step | Read before running the phase |
|---|---|
| 1 | `references/validation.md` |
| 2 | `references/context-sync.md` |

`references/validation-report.md` defines the `## Validation Report` section
written into the plan file. Step 1 points to it at the moment it is needed, on a
`validated` or `failed` outcome only.

Read a step's reference before taking any action for that step, not after. Read
only the reference for the step you have reached: a run that stops at a `blocked`
or `failed` validation never enters step 2, which is why they are separate files.

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

`$ARGUMENTS` is the plan name or plan path.

- The plan name or path is required.
- Resolve exactly one plan. Do not invent a plan from the conversation or from
  incomplete nearby work.

When `$ARGUMENTS` is empty, report that a plan name or path is required, state
the expected argument, and stop. Do not infer the plan from repository state or
the conversation.

Pass the plan name or path to the **Validation phase** unmodified. Do not restate,
summarize, or pre-scope it.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is
the path carried by the **Validation phase** in its Markdown result (`Plan:`, or a
candidate path), so every emitted command is directly runnable.

## Workflow

### 1. Validate the plan

Read `references/validation.md`, then run the **Validation phase** with the plan
name or path.

This phase measures finished work and never repairs it: it does not modify tests,
application code, or configuration to make a failing check pass. That property is
load-bearing, so reach it through the reference rather than acting from this
summary.

Do not write the Validation Report yourself.

Branch on the report's `Status:`.

`blocked` -> Do not run context synchronization. Print the blocked Markdown
report as returned. Do not rephrase it into a different layout. Stop.

`failed` -> Do not run context synchronization. Print the failed Markdown
report as returned. It is already a session handoff: self-contained, actionable,
and ending with `/validate {plan-path}` after repairs.

Do not rewrite it into a shorter summary. Do not drop the retry command. Do not
add an alternate continuation that replaces `/validate`.

Stop. Do not mark the plan finished. Do not continue to context synchronization.
Do not start the repair work in this workflow unless the user explicitly asks
to continue here; the default is that the handoff can leave this session.

`validated` -> Pass the complete validated Markdown result to the **Plan context synchronization phase**.

Do not reconstruct, summarize, or reinterpret the validation result before
passing it.

### 2. Synchronize plan context

Read `references/context-sync.md`, then run the **Plan context synchronization
phase** with the `Status: validated` Markdown result from the **Validation
phase**.

Do not run the **Plan context synchronization phase** for `failed` or `blocked`. Those are not
success states.

Pass the validated result verbatim. It is the authoritative handoff, and the **Plan context synchronization phase**
owns reading the plan path, required context paths, validation evidence, and
reported context impact out of it.

Do not restate, summarize, or reconstruct any part of the validation result.

This phase verifies the five root context files on every invocation, whatever the
reported impact, and must account for every path in the plan's `Context sync`
section, so it is never correct to skip it as unnecessary.

Branch on the synchronization result.

`blocked` -> Validation itself succeeded and is already recorded in the plan.
Render the **Context synchronization blocked** layout from
`references/output.md`. Nothing records the skipped synchronization, so it is
lost once this session ends.

Stop.

`synced` | `no_context_change` -> Print out the report returned by the **Plan context synchronization phase**.
Continue to the next step.

### 3. Report completion

Return exactly one completion block. Do not start another workflow.

Render the **Completion** layout from `references/output.md`.

When the synchronization status was `no_context_change`, keep the same
completion block. "Synchronized" here means the final context pass finished
successfully, including the case where no edit was warranted.

Stop.

## Rules

- Validate at most one plan per invocation.
- Read each phase's reference before running that phase.
- Do not duplicate the internal instructions of embedded phases.
- The only permitted sibling-skill invocation is `sce-decision`, and only the
  successful context-synchronization decision gate may invoke it.
- Do not run final validation when implementation tasks remain; the **Validation phase**
  returns `blocked`, and this workflow stops.
- Run the **Plan context synchronization phase** only when the **Validation phase** returned
  `Status: validated`. Do not run it for `failed` or `blocked`.
- On `failed`, print the handoff Markdown as returned and stop. Preserve the
  retry `/validate {plan-path}` instruction. Do not synchronize context.
- Do not implement remaining plan tasks from this workflow unless the user
  explicitly continues in-session after a failed handoff.
- Do not create a Git commit or push changes.
- Do not mark the plan archived or delete the plan.
- Do not execute a follow-up `/next-task`, `/change-to-plan`, or `/validate`
  yourself.
- Do not infer success when an embedded phase returns a non-success status.
- Preserve validation evidence already written to the plan when context
  synchronization fails.
