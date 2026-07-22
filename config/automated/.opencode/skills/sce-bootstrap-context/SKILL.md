---
name: sce-bootstrap-context
description: |
  Use when user wants to Bootstrap SCE baseline context directory when missing.
compatibility: opencode
---

## Purpose
- Enforce the automated-profile rule that baseline context must be created manually before automation runs.

## Inputs
- Repository root and `context/` existence state.

## Preconditions
1. Invoke only when `context/` is missing or baseline integrity is being checked.

## Workflow
1. Inspect whether the required baseline exists.
2. When it is missing, emit `Automated profile requires existing context/. Run manual bootstrap first.`
3. List the required baseline paths for the manual bootstrap session.
4. Stop without creating or modifying files.

## Guardrails
- Do not auto-bootstrap.
- Do not create placeholders or infer project context.

## Outputs
- A deterministic blocking error and required-path list.

## Completion criteria
- Automation stops before planning or implementation when baseline context is absent.

## Failure handling
- Treat a partial baseline as missing and report every absent path.

## Related units
- Manual `sce-bootstrap-context` — creates the baseline after human approval.
- `Shared Context Plan` — consumes the baseline in automated planning.

## Reference
Required paths are the same as the manual profile: root overview, architecture, patterns, glossary, context map, and the plans, handovers, decisions, and tmp directories with `context/tmp/.gitignore`.
