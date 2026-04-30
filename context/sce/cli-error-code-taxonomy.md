# CLI Error-Code Taxonomy

## Scope

This document defines the stable user-facing error-code contract rendered by `sce` runtime diagnostics in `cli/src/app.rs`.
It complements the numeric process exit-code classes documented in `context/sce/cli-exit-code-contract.md`.

## Stable diagnostic code classes

- `SCE-ERR-PARSE`: top-level parse failures before command invocation.
- `SCE-ERR-VALIDATION`: invocation/argument validation failures after parsing.
- `SCE-ERR-RUNTIME`: runtime execution failures after successful parse + validation.
- `SCE-ERR-DEPENDENCY`: startup dependency failures before parsing/dispatch.

## Rendering contract

- User-facing diagnostics are emitted on `stderr` as: `Error [<code>]: <message>`.
- Before stderr emission, all `ClassifiedError` instances are logged via `Logger::log_classified_error()` with event ID `sce.error.{code}` and fields `error_code`, `error_class`.
- If a diagnostic message does not already include `Try:`, runtime appends class-default remediation guidance.
- If the message already contains `Try:`, runtime preserves the original remediation text and does not append a second one.
- Diagnostic text is still redaction-filtered through `services::security::redact_sensitive_text` before emission.

## Actionable parser/invocation guidance contract

- High-frequency parse/invocation failures use explicit `Try:` remediations instead of generic usage-only hints.
- Top-level unknown command/option messages include targeted retry guidance (`sce --help` and command-local `sce <command> --help`).
- Setup invocation validation failures (`--repo` without `--hooks`, mutually exclusive target flags, unexpected args) include concrete valid alternatives.
- Hooks invocation validation failures (missing hook subcommand, missing `commit-msg` message file, unknown subcommand) include command-form examples that are copyable for retry automation.
- This actionable-message normalization is owned by parser/validation paths in `cli/src/app.rs`, `cli/src/services/setup/mod.rs`, and `cli/src/services/hooks/mod.rs`.

## Ownership

- `FailureClass` in `cli/src/services/error.rs` owns class selection.
- `ClassifiedError` in `cli/src/services/error.rs` owns stable code assignment.
- `Logger::log_classified_error` in `cli/src/services/observability.rs` owns structured error logging with `sce.error.{code}` event IDs.
- `write_error_diagnostic` in `cli/src/app.rs` owns final code-bearing stderr rendering.
- `run_with_dependency_check_and_streams` in `cli/src/app.rs` owns error logging before stderr emission.

## Determinism and testing

- Error code value is derived from failure class and is stable for a given class.
- Code-bearing stderr output and remediation presence are locked by `app::tests`.
