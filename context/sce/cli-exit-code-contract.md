# CLI Exit-Code Contract

## Scope

This document defines the stable `sce` process exit-code classes used by `cli/src/app.rs`.
The contract is intentionally class-based so automation can branch on failure category without parsing free-form error text.

## Exit-code classes

- `0` (`success`): command completed successfully.
- `2` (`parse_failure`): CLI parsing failed for an invocation that is not an unknown command/subcommand help request (for example an unknown option or malformed command token). Unknown top-level commands and unknown subcommands instead resolve to successful help with exit code `0`.
- `3` (`validation_failure`): command/subcommand arguments parsed but failed invocation validation (for example incompatible or missing command-local arguments).
- `4` (`runtime_failure`): command invocation was valid but runtime execution failed (filesystem/process/environment/runtime operation errors).
- `5` (`dependency_failure`): startup dependency checks failed before command parsing/dispatch.

## Classification ownership

- `cli/src/services/error.rs` owns `FailureClass`, its numeric `exit_code` mapping, and `CliError::class()` for internal and typed user errors.
- The synchronous `services::parse::command_runtime::parse_runtime_command` owns clap conversion/classification; command-local parsers retain invocation validation. `app::parse_command_phase` delegates to that boundary.
- Service-owned command methods return classified runtime errors through directly awaited `RuntimeCommand::execute_with_stderr` and `app_support::execute_command_phase`.
- `app::perform_dependency_check` classifies startup dependency failures before parsing, using the closure passed to `run_with_dependency_check`.
- After awaited execution completes, synchronous `app_support::render_run_outcome` / `exit_with_error` renders diagnostics and maps `error.class().exit_code()` to the process result. The application runtime changes execution ownership, not numeric classes.

## Determinism requirements

- Exit code is derived only from failure class and is stable for a given failure category.
- Error text remains on `stderr`; exit-code class is independent from message wording.
- Representative class mapping is locked by `app::tests`.
