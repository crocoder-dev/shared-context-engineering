# CLI Error-Code Taxonomy

## Scope

This document defines the stable user-facing error-code contract rendered by `sce` runtime diagnostics in `cli/src/app.rs`.
It complements the numeric process exit-code classes documented in `context/sce/cli-exit-code-contract.md`.

## Stable diagnostic code classes

- `SCE-ERR-PARSE`: top-level parse failures before command invocation.
- `SCE-ERR-VALIDATION`: invocation/argument validation failures after parsing.
- `SCE-ERR-RUNTIME`: runtime execution failures after successful parse + validation.
- `SCE-ERR-DEPENDENCY`: startup dependency failures before parsing/dispatch.

## Optional user-facing presentation

- `ClassifiedError` may carry a `UserFacingPresentation` containing a caller-provided message, including any presentation styling, and an optional separate semantic reason key.
- The presentation is distinct from the technical diagnostic message, `FailureClass`, stable `SCE-ERR-*` code, and numeric exit code.
- Existing constructors leave the presentation absent. The top-level `sync` command is the current command-specific adoption: typed `MissingCredentials` and `AuthenticationFailed` control-plane failures attach a concise login presentation, while other command mappings retain the classified fallback.

## Rendering contract

- Errors with no `UserFacingPresentation` are emitted on `stderr` as: `Error [<code>]: <message>`.
- When a `UserFacingPresentation` is present, the app emits its redacted message on `stderr` without applying renderer-owned styling, and without the `Error [<code>]` header, technical diagnostic, or automatic class-default `Try:` guidance; any caller-provided styling and message structure are preserved.
- The presentation does not change the classified exit code or structured error logging. For `sce sync` authentication failures, it renders `You are not logged in. Please log in using the sce auth login command.` in color-disabled output, with only the `sce auth login` segment caller-styled when styling is enabled; other command mappings retain the classified fallback.
- Before stderr emission, all `ClassifiedError` instances are logged via `Logger::log_classified_error()` with event ID `sce.error.{code}` and fields `error_code`, `error_class`. The app passes `true` for fallback errors so the configured logger stderr record remains visible, and `false` for an explicit presentation so only that logger stderr record is suppressed; tracing/file observability remains active.
- If a fallback diagnostic message does not already include `Try:`, runtime appends class-default remediation guidance.
- If the message already contains `Try:`, runtime preserves the original remediation text and does not append a second one.
- Both presentation and fallback diagnostic text are redaction-filtered through `services::security::redact_sensitive_text` before emission; only fallback diagnostics receive renderer-owned stderr styling.

## Actionable parser/invocation guidance contract

- High-frequency parse/invocation failures use explicit `Try:` remediations instead of generic usage-only hints.
- Top-level unknown command/option messages include targeted retry guidance (`sce --help` and command-local `sce <command> --help`).
- Setup invocation validation failures (`--repo` without `--hooks`, mutually exclusive target flags, unexpected args) include concrete valid alternatives.
- Hooks invocation validation failures (missing hook subcommand, missing `commit-msg` message file, unknown subcommand) include command-form examples that are copyable for retry automation.
- This actionable-message normalization is owned by parser/validation paths in `cli/src/app.rs`, `cli/src/services/setup/mod.rs`, and `cli/src/services/hooks/mod.rs`.

## Ownership

- `FailureClass` in `cli/src/services/error.rs` owns class selection.
- `ClassifiedError` in `cli/src/services/error.rs` owns stable code assignment, technical diagnostics, and optional `UserFacingPresentation` metadata.
- `Logger::log_classified_error` in `cli/src/services/observability.rs` owns structured error logging with `sce.error.{code}` event IDs.
- `write_error_diagnostic` in `cli/src/services/app_support.rs` owns final stderr rendering, selecting either the exact optional presentation or the code-bearing fallback.
- `run_with_dependency_check_and_streams` in `cli/src/app.rs` owns error logging before stderr emission.

## Determinism and testing

- Error code value is derived from failure class and is stable for a given class.
- Code-bearing stderr output, exact presentation rendering, stdout isolation, and remediation presence are locked by `services::app_support::tests`.
- Technical classified-error logging remains independently covered by `services::observability::tests::classified_error_logging_keeps_technical_data_separate_from_presentation`.
