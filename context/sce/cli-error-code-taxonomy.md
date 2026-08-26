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
- Before stderr emission, all `CliError` instances are logged via `Logger::log_cli_error()` with event ID `sce.error.{code}` and fields `error_code`, `error_class`.
- For `CliError::Internal`, if the rendered message does not already include `Try:`, runtime appends class-default remediation guidance; if it already contains `Try:`, runtime preserves the original remediation text and does not append a second one.
- For `CliError::User`, runtime renders the catalog message from `UserError` verbatim, with no class-default `Try:` appended.
- Diagnostic text is still redaction-filtered through `services::security::redact_sensitive_text` before emission.

## Actionable parser/invocation guidance contract

- High-frequency parse/invocation failures use explicit `Try:` remediations instead of generic usage-only hints.
- Top-level unknown command/option messages include targeted retry guidance (`sce --help` and command-local `sce <command> --help`).
- Setup invocation validation failures (`--repo` without `--hooks`, mutually exclusive target flags, unexpected args) include concrete valid alternatives.
- Setup repository preflight failures use `UserError::NotGitRepository` and payload-bearing `UserError::NotGitRemote { remote_name }` messages with `git init` and `git remote add <name> <url>` remediation only for Git's explicit `not a git repository` result and an actually missing/empty configured remote URL. The configured remote name appears in the missing-URL explanation and remediation, while the URL itself is never rendered. Git launch, permission, bare/malformed-repository, configuration, and remote-lookup execution failures remain `CliError::Internal` runtime errors with their technical sources.
- Hooks invocation validation failures (missing hook subcommand, missing `commit-msg` message file, unknown subcommand) include command-form examples that are copyable for retry automation.
- This actionable-message normalization is owned by parser/validation paths in `cli/src/app.rs`, `cli/src/services/setup/mod.rs`, `cli/src/services/setup/command.rs`, and `cli/src/services/hooks/mod.rs`.

## Ownership

- `FailureClass` in `cli/src/services/error.rs` owns class selection and stable code assignment (`FailureClass::code()`).
- `CliError::{User,Internal}` in `cli/src/services/error.rs` is the typed CLI-boundary error type; `CliError::code()`/`CliError::class()` delegate to the failure class. `CliError::User` carries a catalog `UserError` (`NotAuthenticated`, `NotGitRepository`, or `NotGitRemote { remote_name }`) for expected, deliberately-explained failures; `CliError::Internal` carries a live `anyhow::Error` source for every other failure, including non-classifiable setup preflight failures. `CliError::User` may also carry an optional preserved technical `source`, kept for observability only and never rendered to the terminal; the named remote payload contains no URL.
- `UserError` in `cli/src/services/error.rs` is the closed catalog of deliberately presented terminal failures. It has no arbitrary-message variant (no `Message(String)`/`Custom(...)` escape hatch): every entry returns a fixed reviewed sentence or reviewed payload-derived message from `UserError::message()`, keyed for structured logging by `UserError::key()`.
- Command and domain layers construct and return a `CliError`; they do not format terminal text, apply styling, or decide authentication/user-error semantics from string matching. `app_support` is the sole owner of turning a `CliError` into the final stderr sentence.
- `Logger::log_cli_error` in `cli/src/services/observability.rs` owns structured error logging with `sce.error.{code}` event IDs.
- `write_error_diagnostic` in `cli/src/services/app_support.rs` owns final code-bearing stderr rendering, including styling `CliError::User`'s catalog message and `CliError::Internal`'s rendered chain through `services::style::error_text_with_color_policy` under the stderr TTY/`NO_COLOR` policy (`services::style::supports_color_stderr()`), independent of stdout's TTY state.
- `run_with_dependency_check_and_streams` in `cli/src/app.rs` owns error logging before stderr emission.

## Determinism and testing

- Error code value is derived from failure class and is stable for a given class.
- Code-bearing stderr output and remediation presence are locked by `app::tests`.
