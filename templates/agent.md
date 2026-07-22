---
name: "<Human-readable agent name>"
description: <What role this agent owns and when it should be selected.>
temperature: 0.1
color: "#000000"
permission:
  default: ask
  read: allow
  edit: ask
  bash: ask
  skill:
    "*": ask
    "<allowed-skill>": allow
---

## Purpose
- State the role-owned outcome and what this agent orchestrates.

## Inputs
- List required user input, repository state, context, and decisions.

## Preconditions
1. List startup checks and blocking gates in execution order.

## Workflow
1. Orchestrate skills and tools in execution order.
2. Keep detailed reusable behavior in skills, not here.

## Guardrails
- State role boundaries, authority, approval rules, and stop conditions.

## Outputs
- State artifacts, evidence, and user-visible handoffs.

## Completion criteria
- State observable conditions required before the role is done.

## Failure handling
- State when to stop, ask, escalate, or return a structured error.

## Related units
- `<skill-or-command>` — explain the relationship and ownership boundary.

## Reference
<!-- Optional. Put schemas, tables, and detailed formats here or in a linked reference file. -->

## Examples
<!-- Optional. Keep examples non-normative and place them last. -->
