---
description: "Run `sce-validation` to finish an SCE plan with validation and cleanup"
agent: "Shared Context Code"
---

## Purpose
- Run the final SCE validation phase by delegating to `sce-validation`.

## Inputs
- `$ARGUMENTS`: target plan name/path or change identifier.
- The plan's success criteria and current repository state.

## Preconditions
1. Resolve the target plan or completed change.
2. Confirm implementation is ready for final validation.

## Workflow
1. Load `sce-validation`.
2. Pass the target and let the skill discover project checks, capture evidence, clean temporary scaffolding, and verify context.
3. Return the pass/fail result and validation-report location.
4. Stop after reporting validation.

## Guardrails
- Keep this command thin; validation scope, command discovery, repairs, evidence, and report shape remain skill-owned.
- Do not convert failed validation into a success result.

## Outputs
- Validation status, commands and evidence summary, residual risks, and report location.

## Completion criteria
- `sce-validation` records a conclusive result against every success criterion.

## Failure handling
- Report unresolved failures and their evidence; do not close the plan while required checks remain failed or unevaluated.

## Related units
- `sce-validation` — sole owner of final validation behavior.
- `Shared Context Code` — default agent for this command.
