---
name: sce-validation
description: Use when user wants to Run final plan validation and cleanup with evidence capture.
compatibility: opencode
---

<!-- GENERATED FILE: DO NOT EDIT DIRECTLY. Update canonical sources under config/pkl/ and regenerate. -->

## When to use
- Use for the plan's final validation task.

## Validation checklist
1) Run full test suite (or best available full-project checks).
2) Run lint/format checks used by the repository.
3) Remove temporary scaffolding related to the change.
4) Verify context reflects final implemented behavior.
5) Confirm each success criterion has evidence.

## Validation report
Write to `context/plans/{plan_name}.md` including:
- Commands run
- Exit codes and key outputs
- Failed checks and follow-ups
- Success-criteria verification summary
- Residual risks, if any
