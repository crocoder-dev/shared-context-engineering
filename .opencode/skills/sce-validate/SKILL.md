---
name: sce-validate
description: >
  Validate one completed SCE plan and record final validation evidence
compatibility: opencode
---

# SCE Validate

## Execution contract

Own this workflow from input through its terminal user-visible response.
Follow its steps, gates, and stops in order; do not add, skip, reorder, or merge them.
Keep internal phase results private and continue immediately until a defined wait or stop.
Resume user waits in this same skill and session.
Render user-visible output only from the named workflow layouts or phase reports.
Do not expose raw internal state or add text around a rendered layout or report.
Non-SCE helpers may assist, but must return to the active step without changing
phase order, gates, waits, writes, validation, stops, or terminal output.
Do not invoke another SCE skill, package, or workflow command.

## Phase references

Each numbered step below dispatches to a phase whose steps and boundaries live in
a reference file. This document holds the control flow — which phase runs, what it
receives, and how its result branches — and each reference holds the phase itself.

| Step | Read before running the phase |
|---|---|
| 1 | `references/validation.md` |

`references/validation-report.md` defines the `## Validation Report` section
written into the plan file. Step 1 points to it at the moment it is needed, on a
`validated` or `failed` outcome only.

Read the reference before taking any action for step 1, not after.

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
the path carried by the **Validation phase** in its Markdown report (`Plan:`, or a
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

`blocked` -> Print the blocked Markdown report as returned. Do not rephrase it
into a different layout. Stop.

`failed` -> Print the failed Markdown report as returned. It is already a session
handoff: self-contained, actionable, and ending with `/validate {plan-path}` after
repairs.

Do not rewrite it into a shorter summary. Do not drop the retry command. Do not
add an alternate continuation that replaces `/validate`. Stop.

`validated` -> Print the complete validated Markdown result as returned.
Continue to the next step.

### 2. Report completion

Return exactly one completion block. Do not start another workflow.

Render the **Completion** layout from `references/output.md`.

Stop.

## Rules

- Validate at most one plan per invocation.
- Read each phase's reference before running that phase.
- Do not duplicate the internal instructions of embedded phases.
- Do not run final validation when implementation tasks remain; the **Validation phase**
  returns `blocked`, and this workflow stops.
- On `failed`, print the handoff Markdown as returned and stop. Preserve the
  retry `/validate {plan-path}` instruction.
- Do not implement remaining plan tasks from this workflow unless the user
  explicitly continues in-session after a failed handoff.
- Do not create a Git commit or push changes.
- Do not mark the plan archived or delete the plan.
- Do not execute a follow-up `/next-task`, `/change-to-plan`, or `/validate`
  yourself.
- Do not infer success when an embedded phase returns a non-success status.
