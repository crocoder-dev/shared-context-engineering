# Agent Trace hook rollout doctor

## Scope

Task `agent-trace-attribution-no-git-wrapper` `T07` adds local rollout validation through `sce doctor` so operators can verify hook readiness before enabling attribution enforcement.

## Command contract

- Entrypoint: `sce doctor`
- Service implementation: `cli/src/services/doctor.rs`
- Command dispatch: `cli/src/app.rs` (`Command::Doctor`)
- Command surface status: implemented in `cli/src/command_surface.rs`

`sce doctor` always returns a deterministic text report with:

- readiness verdict (`ready` or `not ready`)
- hook-path source (`default (.git/hooks)`, per-repo `core.hooksPath`, or global `core.hooksPath`)
- detected repository root and effective hooks directory
- required hook checks for `pre-commit`, `commit-msg`, `post-commit`
- actionable diagnostics for missing or misconfigured hooks

## Health validation rules

`sce doctor` resolves git state using CLI git commands:

- `git rev-parse --show-toplevel`
- `git rev-parse --git-path hooks`
- `git config --local --get core.hooksPath`
- `git config --global --get core.hooksPath`

Readiness is `not ready` when any required check fails:

- hooks directory cannot be resolved
- hooks directory is missing
- any required hook file is missing
- any required hook exists but is not executable

If no diagnostics are present, readiness is `ready`.

## Verification coverage

`cli/src/services/doctor.rs` includes explicit doctor output tests for:

- healthy state (all required hooks present and executable)
- missing state (required hook absent)
- misconfigured state (required hook present but non-executable)

`cli/src/app.rs` includes command-level routing/exit success coverage for `sce doctor`.
