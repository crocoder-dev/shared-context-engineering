# Agent Trace hook rollout doctor

## Scope

Task `agent-trace-attribution-no-git-wrapper` `T07` adds local rollout validation through `sce doctor` so operators can verify hook readiness before enabling attribution enforcement.

## Command contract

- Entrypoint: `sce doctor`
- Service implementation: `cli/src/services/doctor.rs`
- Command dispatch: `cli/src/app.rs` (`Command::Doctor(DoctorRequest)`)
- Command surface status: implemented in `cli/src/command_surface.rs`

`sce doctor` supports deterministic dual output via `--format <text|json>`.

Text output includes:

- readiness verdict (`ready` or `not ready`)
- hook-path source (`default (.git/hooks)`, per-repo `core.hooksPath`, or global `core.hooksPath`)
- detected repository root and effective hooks directory
- default global/local config-file locations with `present` or `expected` state
- Agent Trace local DB location with `present` or `expected` state
- required hook checks for `pre-commit`, `commit-msg`, `post-commit`
- actionable diagnostics for missing or misconfigured hooks

JSON output includes stable top-level fields:

- `status`, `command`
- `readiness` (`ready` or `not_ready`)
- `hook_path_source` (`default`, `local_config`, `global_config`)
- `repository_root`, `hooks_directory`
- `config_paths[]` with `label`, `path`, `exists`, `state`
- `agent_trace_local_db` with `label`, `path`, `exists`, `state`
- `hooks[]` with `name`, `path`, `exists`, `executable`, `state`
- `diagnostics[]`

## Health validation rules

`sce doctor` resolves git state using CLI git commands:

- `git rev-parse --show-toplevel`
- `git rev-parse --git-path hooks`
- `git config --local --get core.hooksPath`
- `git config --global --get core.hooksPath`

Git command resolution is repository-root anchored for the inspected repo, and the effective hooks directory is normalized to an absolute path when git returns a relative hook path.

Config and DB location reporting uses deterministic local path resolution only:

- global config path: `${state_root}/sce/config.json`
- local config path: `<current working directory>/.sce/config.json`
- Agent Trace local DB path: `${state_root}/sce/agent-trace/local.db`

`state_root` resolves through `cli/src/services/local_db.rs` platform rules (`XDG_STATE_HOME` or the platform-equivalent user state root fallback).

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
- post-setup ready state after required hooks are installed
- post-setup ready state for per-repo custom `core.hooksPath`
- config/local-DB location reporting for present and absent cases
- request parsing defaults and `--format json` support
- JSON report shape contract (`status`, `command`, `readiness`, `hook_path_source`, `config_paths`, `agent_trace_local_db`, `hooks`, `diagnostics`)

`cli/src/app.rs` includes command-level routing/exit success coverage for `sce doctor`, including `--format json` routing.
