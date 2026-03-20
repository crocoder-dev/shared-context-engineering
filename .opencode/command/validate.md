---
description: "Run `sce-validation` to finish an SCE plan with validation and cleanup"
agent: "Shared Context Code"
entry-skill: "sce-validation"
skills:
  - "sce-validation"
---

Load and follow the `sce-validation` skill.

Input:
`$ARGUMENTS`

Behavior:
- Keep this command as thin orchestration; validation scope, command selection, cleanup, and evidence formatting stay owned by `sce-validation`.
- Run `sce-validation` to execute the full validation phase for the targeted plan or change, including required checks, evidence capture, and cleanup expected by the skill.
- Let `sce-validation` decide pass/fail status and record any residual risks or unmet criteria.
- Stop after reporting the validation outcome and the location of any written validation evidence.
