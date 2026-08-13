# Validate output layouts

Use only the applicable layout. Values come from internal workflow state.

## Context synchronization blocked

State that plan `{plan-path}` passed final validation and its Validation
Report is written; report the context contradiction or synchronization
failure, any context edits the report says were preserved, the action
required to resolve the problem, and the retry condition. State that
durable context is now out of date relative to the validated
implementation and must be synchronized before treating the plan as fully
closed.

## Completion

```markdown
-------------------------------------

# Plan {plan-name} validated.

All implementation tasks were already complete.
Final validation passed.
Durable context is synchronized.

Validation report: {plan-path}
```
