---
name: sce-validation
description: Runs the final validation phase of a project plan by executing the full test suite, lint and format checks, removing temporary scaffolding, and writing a structured validation report with command outputs and success-criteria evidence to `context/plans/{plan_name}.md`. Use when the user wants to verify a completed implementation, confirm all success criteria are met, wrap up a plan, finalize a feature or fix, or sign off on a change before closing it out.
compatibility: claude
---

## When to use
- Use for the plan's final validation task after implementation is complete.
- Triggered by requests like "validate the plan", "run final checks", "confirm everything passes", "wrap up the task", or "sign off on this change".

## Validation checklist
1) **Run full test suite** - discover and run the project's primary test command (e.g., `pytest`, `npm test`, `go test ./...`, `cargo test`, `make test`). Check `package.json`, `Makefile`, `pyproject.toml`, or CI config files to find the right command.
2) **Run lint/format checks** - discover and run the project's lint and format tools (e.g., `eslint`, `ruff`, `golangci-lint`, `cargo clippy`, `make lint`). Check project config files such as `.eslintrc`, `pyproject.toml`, or `.golangci.yml`.
3) **Remove temporary scaffolding** - delete any debug code, temporary files, or intermediate artifacts introduced during the change.
4) **Verify context reflects final implemented behavior** - confirm that plan context and notes match the actual final state of the implementation.
5) **Confirm each success criterion has evidence** - for every success criterion defined in the plan, record concrete evidence (command output, exit code, screenshot reference, or file path).

### If checks fail
- **Fixable failures**: fix the issue, re-run the failing check, and update the report with the corrected output.
- **Non-trivial failures**: document the failure, the attempted fix, and the blocker in the report under "Failed checks and follow-ups". Escalate to the user before closing out.
- **Lint/format auto-fixes**: if the tool supports auto-fix (e.g., `ruff --fix`, `eslint --fix`), apply it, then re-run to confirm clean output.

## Validation report
Write to `context/plans/{plan_name}.md` including:
- Commands run
- Exit codes and key outputs
- Failed checks and follow-ups
- Success-criteria verification summary
- Residual risks, if any

### Example report entry
```
## Validation Report

### Commands run
- `npm test` -> exit 0 (42 tests passed, 0 failed)
- `eslint src/` -> exit 0 (no warnings)
- Removed: `src/debug_patch.js` (temporary scaffolding)

### Success-criteria verification
- [x] All API endpoints return 200 for valid input -> confirmed via test output line 34
- [x] Error responses include structured JSON -> confirmed via `test_error_format.js`

### Residual risks
- None identified.
```
