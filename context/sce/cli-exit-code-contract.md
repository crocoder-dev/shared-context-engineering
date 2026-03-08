# CLI Exit-Code Contract

## Scope

This document defines the stable `sce` process exit-code classes used by `cli/src/app.rs`.
The contract is intentionally class-based so automation can branch on failure category without parsing free-form error text.

## Exit-code classes

- `0` (`success`): command completed successfully.
- `2` (`parse_failure`): top-level CLI parsing failed (for example unknown top-level command/option or malformed command token).
- `3` (`validation_failure`): command/subcommand arguments parsed but failed invocation validation (for example incompatible or missing command-local arguments).
- `4` (`runtime_failure`): command invocation was valid but runtime execution failed (filesystem/process/environment/runtime operation errors).
- `5` (`dependency_failure`): startup dependency checks failed before command parsing/dispatch.

## Classification ownership

- `cli/src/app.rs` owns classification via `FailureClass` and maps it to numeric codes through `FailureClass::exit_code`.
- Top-level parse failures are classified in `parse_command`/`parse_subcommand`.
- Command-local argument validation failures are classified in `parse_config_subcommand`, `parse_setup_subcommand`, `parse_hooks_subcommand`, and `parse_non_setup_subcommand`.
- Runtime failures are classified in `dispatch` when service execution fails after valid parsing/validation.
- Dependency failures are classified in startup via the dependency-check closure passed to `run_with_dependency_check`.

## Determinism requirements

- Exit code is derived only from failure class and is stable for a given failure category.
- Error text remains on `stderr`; exit-code class is independent from message wording.
- Representative class mapping is locked by `app::tests`.
