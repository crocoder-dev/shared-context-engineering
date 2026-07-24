---
name: "<Human-readable execution-profile name>"
description: <What invocation-wide role this profile owns and when it should be selected.>
mode: primary
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
- State the broad invocation-wide role outcome without duplicating a workflow.

## Inputs
- List required user intent, repository state, context, and human decisions.

## Preconditions
1. List profile-wide startup checks and blocking gates.

## Workflow
1. State the broad operating posture for workflows bound to this profile.
2. Keep user-invoked sequencing in workflows and reusable procedures in skills.

## Guardrails
- State role boundaries, authority, approval rules, and stop conditions.

## Outputs
- State profile-wide evidence and user-visible handoff expectations.

## Completion criteria
- State observable conditions shared by invocations of this profile.

## Failure handling
- State when profile-bound work must stop, ask, escalate, or return an error.

## Related units
- `<allowed-skill>` — explain why the profile permits this reusable procedure.

## Reference
<!--
Required canonical declaration fields: slug, title, and policy. ProfilePolicy must contain
body, allowedSkills, and toolPolicy. ToolPolicy contains harness-neutral allowedCapabilities
and approvalRequiredCapabilities; target renderers translate those capabilities to native tools.
-->

## Examples
<!-- Optional. Keep examples non-normative and place them last. -->
