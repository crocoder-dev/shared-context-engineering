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

- Catalog diagnostics are emitted on `stderr` as the redacted catalog message followed by a newline, without an `Error` label, `SCE-ERR-*` code, separator, `Try:` guidance, or ANSI styling. This is the terminal path for `CliError::User`.
- `CliError::Internal` diagnostics are emitted on `stderr` as the styled `Error [<code>]: <message>` wrapper.
- Before stderr emission, all `CliError` instances are logged via `Logger::log_cli_error()` with event ID `sce.error.{code}` and fields `error_code`, `error_class`.
- For `CliError::Internal`, if the rendered message does not already include `Try:`, runtime appends class-default remediation guidance; if it already contains `Try:`, runtime preserves the original remediation text and does not append a second one.
- For `CliError::User`, runtime renders the catalog message from `UserError` without technical source text or class-default `Try:` remediation. The `UserError::UnexpectedFailure` entry renders the fixed message `An unexpected error occurred. Check the log files for more details.` without dynamic path interpolation.
- Diagnostic text is still redaction-filtered through `services::security::redact_sensitive_text` before emission.

## Actionable parser/invocation guidance contract

- High-frequency parse/invocation failures use explicit `Try:` remediations instead of generic usage-only hints.
- Top-level unknown command/option messages include targeted retry guidance (`sce --help` and command-local `sce <command> --help`).
- Setup invocation validation failures (`--repo` without `--hooks`, mutually exclusive target flags, unexpected args) include concrete valid alternatives.
- Hooks invocation validation failures (missing hook subcommand, missing `commit-msg` message file, unknown subcommand) include command-form examples that are copyable for retry automation.
- This actionable-message normalization is owned by parser/validation paths in `cli/src/app.rs`, `cli/src/services/setup/mod.rs`, and `cli/src/services/hooks/mod.rs`.

## Ownership

- `FailureClass` in `cli/src/services/error.rs` owns class selection and stable code assignment (`FailureClass::code()`).
- `CliError::{User,Internal}` in `cli/src/services/error.rs` is the typed CLI-boundary error type; `CliError::code()`/`CliError::class()` delegate to the failure class. `CliError::User` carries a catalog `UserError` (`NotAuthenticated`, `AuthStorageUnavailable`, or `UnexpectedFailure`) for expected, deliberately-explained failures; `CliError::Internal` carries a live `anyhow::Error` source for every other failure. `CliError::User` may also carry an optional preserved technical `source`, kept for observability only and never rendered to the terminal.
- `UserError` in `cli/src/services/error.rs` is the closed catalog of deliberately presented terminal failures. It has no arbitrary-message variant (no `Message(String)`/`Custom(...)` escape hatch): every entry is a fixed, reviewed sentence returned by `UserError::message()`, keyed for structured logging by `UserError::key()`. `NotAuthenticated` (`auth.not_authenticated`) covers missing credentials from `sce auth logout`/`whoami` and Control Plane authentication failures from `whoami`; `AuthStorageUnavailable` (`auth.storage_unavailable`) is used by `sce sync` and the `sce auth login`, `logout`, and `whoami` command boundary for token-storage plus `AuthError::Io`/`Storage` failures. Both auth-command mappings preserve technical sources for observability. `UnexpectedFailure` (`general.unexpected_failure`) is used by `sce sync` for its default failure classification and by the auth-command boundary for all non-storage failures, including rendering, prompt, runtime, configuration, and unrelated Control Plane failures; it renders one fixed, user-safe diagnostic sentence and has no automatic `Try:` suffix or dynamic path input.
- Command and domain layers construct and return a `CliError`; they do not format terminal text or apply styling. Auth command orchestration classifies expected authentication and credential-storage failures into the existing `UserError` catalog by typed domain variants, never by string matching, and preserves the original technical chain as the optional user-error source. `app_support` is the sole owner of turning a `CliError` into the final stderr sentence.
- `Logger::log_cli_error` in `cli/src/services/observability.rs` owns structured error logging with `sce.error.{code}` event IDs.
- `write_error_diagnostic` in `cli/src/services/app_support.rs` owns final stderr rendering: it redacts and writes the catalog variant's message without a wrapper or styling, while `CliError::Internal` retains code-bearing rendering and styles its rendered chain through `services::style::error_text_with_color_policy` under the stderr TTY/`NO_COLOR` policy (`services::style::supports_color_stderr()`), independent of stdout's TTY state.
- `run_with_dependency_check_and_streams` in `cli/src/app.rs` owns error logging before stderr emission.
- The auth-command typed user-error boundary is an accepted system-wide contract; see [the auth-command decision](../decisions/2026-08-20-auth-command-typed-user-errors.md) and [the auth fallback decision](../decisions/2026-08-20-auth-command-unexpected-fallbacks.md).

## Determinism and testing

- Error code value is derived from failure class and is stable for a given class.
- Code-bearing stderr output and remediation presence are locked by `app::tests`.
