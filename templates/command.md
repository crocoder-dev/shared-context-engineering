---
description: "<Action-oriented outcome>"
agent: "<Agent display name>"
entry-skill: "<primary-skill>"
skills:
  - "<primary-skill>"
---

## Purpose
- State the user-facing action and delegated skill chain.

## Inputs
- `$ARGUMENTS`: define required and optional positional or free-form input.

## Preconditions
1. State argument, repository-state, and approval gates.

## Workflow
1. Load the primary skill.
2. Pass normalized inputs.
3. Preserve skill-owned gates.
4. Return the result and stop.

## Guardrails
- Keep the command thin; never duplicate detailed skill behavior.
- State mode switches and command-owned side effects only.

## Outputs
- State the exact response or artifact shape.

## Completion criteria
- State how the command knows the delegated workflow completed.

## Failure handling
- State argument errors, delegated blockers, and side-effect failures.

## Related units
- `<skill>` — sole owner of detailed behavior.
- `<agent>` — default execution role.

## Reference
<!-- Optional. Document argument syntax or modes. -->

## Examples
<!-- Optional. Keep examples last. -->
