---
description: "<Action-oriented outcome>"
agent: "<Projected execution-profile carrier name>"
entry-skill: "<primary-skill>"
skills:
  - "<primary-skill>"
subtask: false
---

## Purpose
- State the user-invoked action and delegated skill chain.

## Inputs
- `$ARGUMENTS`: define required and optional positional or free-form input.

## Preconditions
1. State argument, repository-state, and approval gates.

## Workflow
1. Load the entry skill.
2. Pass normalized inputs.
3. Preserve skill-owned gates.
4. Return the result and stop.

## Guardrails
- Keep the workflow thin; never duplicate execution-profile policy or detailed skill behavior.
- A workflow capability allow-set may only narrow its execution-profile ceiling.
- State mode switches and workflow-owned side effects only.

## Outputs
- State the exact response or artifact shape.

## Completion criteria
- State how the workflow knows the delegated procedure completed.

## Failure handling
- State argument errors, delegated blockers, and side-effect failures.

## Related units
- `<entry-skill>` — sole owner of the primary detailed procedure.
- `<execution-profile>` — invocation-wide role policy bound by this workflow.

## Reference
<!--
Required canonical declaration fields: slug, title, description, body, executionProfile,
entrySkill, requiredSkills, and toolPolicy. entrySkill must occur in requiredSkills, all skills
must resolve through the bound profile, and workflow capabilities must be a profile subset.
-->

## Examples
<!-- Optional. Keep examples last. -->
