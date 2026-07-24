---
name: sce-validation
description: |
  Runs the final validation phase of a project plan by executing the full test suite, lint and format checks, removing temporary scaffolding, and writing a structured validation report with command outputs and success-criteria evidence to `context/plans/{plan_name}.md`. Use when the user wants to verify a completed implementation, confirm all success criteria are met, wrap up a plan, finalize a feature or fix, or sign off on a change before closing it out.
compatibility: claude
---

## Purpose
- Run final validation and cleanup for a completed SCE plan or change.
- Produce evidence for every success criterion and a conclusive pass/fail report.

## Inputs
- Target plan name/path, success criteria, implemented repository state, and existing task evidence.

## Preconditions
1. Resolve the target plan and confirm implementation tasks are complete enough for final validation.
2. Discover authoritative project commands from repository configuration and CI files rather than guessing.

## Workflow
1. Run the project's full test suite.
2. Run lint, format, static-analysis, and build checks required by the repository.
3. Remove temporary scaffolding, debug code, and intermediate artifacts introduced by the change.
4. Verify durable context reflects final implemented behavior.
5. Map concrete evidence to every plan success criterion.
6. Apply supported, in-scope auto-fixes for lint/format failures and rerun the affected check.
7. Append a structured validation report to `context/plans/{plan_name}.md`.
8. Report pass/fail status and residual risks.

## Guardrails
- Do not invent commands, outputs, exit codes, screenshots, or passing results.
- Do not hide flaky, skipped, or unevaluated criteria.
- Escalate non-trivial failures instead of broadening scope silently.
- Preserve evidence sufficient for another session to reproduce the result.

## Outputs
- A validation report with commands, exit codes, key output, failed checks/follow-ups, criterion evidence, and residual risks.
- An explicit overall pass/fail result.

## Completion criteria
- Every required check has a recorded outcome.
- Every success criterion has concrete evidence or is explicitly unresolved.
- Temporary scaffolding is removed and context is synchronized.

## Failure handling
- Fix and rerun failures only when the fix is clearly in scope.
- For non-trivial failures, record the command, evidence, attempted fix, blocker, and required follow-up; do not close the plan as passed.

## Related units
- `/validate` — thin command entrypoint.
- `sce-context-sync` — verifies final context truth.
- `sce-task-execution` — supplies task-level evidence.

## Reference
Append a report to the target plan using this shape:

```markdown
## Validation Report

### Commands run
- `command` -> exit 0 (key result)

### Failed checks and follow-ups
- None.

### Success-criteria verification
- [x] Criterion -> evidence

### Residual risks
- None identified.
```
