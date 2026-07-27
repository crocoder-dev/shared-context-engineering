---
description: "Run `sce-validation` -> `sce-plan-context-sync` to finish an SCE plan"
argument-hint: "<plan-name>"
---

SCE VALIDATE `$ARGUMENTS`

## Input

`$ARGUMENTS` is the plan name or plan path.

- The plan name or path is required.
- Resolve exactly one plan. Do not invent a plan from the conversation or from
  incomplete nearby work.

When `$ARGUMENTS` is empty, report that a plan name or path is required, state
the expected argument, and stop. Do not infer the plan from repository state or
the conversation.

Pass the plan name or path to `sce-validation` unmodified. Do not restate,
summarize, or pre-scope it.

Every `{plan-path}` and `{candidate-path}` emitted anywhere in this workflow is
the path carried by `sce-validation` in its Markdown result (`Plan:`, or a
candidate path), so every emitted command is directly runnable.

## Workflow

### 1. Validate the plan

Invoke `sce-validation` with the plan name or path.

`sce-validation` exclusively owns:

- Resolving one plan.
- Confirming every implementation task is complete.
- Running full validation and acceptance-criteria checks.
- Removing temporary scaffolding.
- Writing the Validation Report into the plan.
- Returning one Markdown validation result.

Do not duplicate any of it. Do not write the Validation Report yourself.

The skill must return a Markdown result matching its validation-result contract.
Branch on the report's `Status:`.

`blocked` -> Do not invoke context synchronization. Print the blocked Markdown
report as returned. Do not rephrase it into a different layout. Stop.

`failed` -> Do not invoke context synchronization. Print the failed Markdown
report as returned. It is already a session handoff: self-contained, actionable,
and ending with `/validate {plan-path}` after repairs.

Do not rewrite it into a shorter summary. Do not drop the retry command. Do not
add an alternate continuation that replaces `/validate`.

Stop. Do not mark the plan finished. Do not continue to context synchronization.
Do not start the repair work in this workflow unless the user explicitly asks
to continue here; the default is that the handoff can leave this session.

`validated` -> Pass the complete validated Markdown result to
`sce-plan-context-sync`.

Do not reconstruct, summarize, or reinterpret the validation result before
passing it.

### 2. Synchronize plan context

Invoke `sce-plan-context-sync` only with a `Status: validated` Markdown result
from `sce-validation`.

Do not invoke `sce-plan-context-sync` for `failed` or `blocked`. Those are not
success states.

Pass the validated result verbatim. It is the authoritative handoff, and
`sce-plan-context-sync` owns reading the plan path, required context paths,
validation evidence, and reported context impact out of it.

Do not restate, summarize, or reconstruct any part of the validation result.

Branch on the synchronization result.

`blocked` -> Validation itself succeeded and is already recorded in the plan.
Present:

- That plan `{plan-path}` passed final validation and its Validation Report is
  written.
- The context contradiction or synchronization failure.
- Any context edits the report says were preserved.
- The action required to resolve the problem.
- The retry condition stated by the report.

State that durable context is now out of date relative to the validated
implementation, and that plan context synchronization must be resolved before
treating the plan as fully closed. Nothing records the skipped synchronization,
so it is lost once this session ends.

Stop.

`synced` | `no_context_change` -> Print out the report
`sce-plan-context-sync` returned. Continue to the next step.

### 3. Report completion

Return exactly one completion block. Do not start another workflow.

```

-------------------------------------

# Plan {plan-name} validated.

All implementation tasks were already complete.
Final validation passed.
Durable context is synchronized.

Validation report: {plan-path}
```

When the synchronization status was `no_context_change`, keep the same
completion block. "Synchronized" here means the final context pass finished
successfully, including the case where no edit was warranted.

Stop.

## Rules

- Validate at most one plan per invocation.
- Do not duplicate the internal instructions of invoked skills.
- Do not run final validation when implementation tasks remain; `sce-validation`
  returns `blocked`, and this workflow stops.
- Invoke `sce-plan-context-sync` only when `sce-validation` returned
  `Status: validated`. Do not invoke it for `failed` or `blocked`.
- On `failed`, print the handoff Markdown as returned and stop. Preserve the
  retry `/validate {plan-path}` instruction. Do not synchronize context.
- Do not implement remaining plan tasks from this workflow unless the user
  explicitly continues in-session after a failed handoff.
- Do not create a Git commit or push changes.
- Do not mark the plan archived or delete the plan.
- Do not execute a follow-up `/next-task`, `/change-to-plan`, or `/validate`
  yourself.
- Do not infer success when an invoked skill returns a non-success status.
- Preserve validation evidence already written to the plan when context
  synchronization fails.
