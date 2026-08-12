# CLI Error-Code Taxonomy

## Scope

This document defines the stable user-facing error-code contract for `sce` runtime diagnostics: `ClassifiedError` (`cli/src/services/error.rs`) carries the code, class, message, and optional hint; `cli/src/services/app_support.rs` renders it.
It complements the numeric process exit-code classes documented in `context/sce/cli-exit-code-contract.md`.

## Stable diagnostic code classes

- `SCE-ERR-PARSE`: top-level parse failures before command invocation.
- `SCE-ERR-VALIDATION`: invocation/argument validation failures after parsing.
- `SCE-ERR-RUNTIME`: runtime execution failures after successful parse + validation.
- `SCE-ERR-DEPENDENCY`: startup dependency failures before parsing/dispatch.

## Rendering contract

- User-facing diagnostics are emitted on `stderr` as: `Error [<code>]: <message>`.
- Before stderr emission, all `ClassifiedError` instances are logged via `Logger::log_classified_error()` with event ID `sce.error.{code}` and fields `error_code`, `error_class`.
- `write_error_diagnostic` in `cli/src/services/app_support.rs` selects remediation text by construction-time state only, never by inspecting the rendered message: an explicit `error.hint()` when the `ClassifiedError` carries one, otherwise `error.class().default_try_guidance()`.
- No renderer code path inspects `message()` for a `Try:` substring; remediation presence is decided entirely by whether `hint` was set when the error was constructed.
- Diagnostic text is still redaction-filtered through `services::security::redact_sensitive_text` before emission.

## Actionable parser/invocation guidance contract

- High-frequency parse/invocation failures use explicit `Try:` remediations instead of generic usage-only hints, attached via `ClassifiedError::with_hint(...)` at construction time.
- Top-level unknown command/option messages include targeted retry guidance (`sce --help` and command-local `sce <command> --help`).
- Setup invocation validation failures (`--repo` without `--hooks`, mutually exclusive target flags, unexpected args) include concrete valid alternatives.
- Hooks invocation validation failures (missing hook subcommand, missing `commit-msg` message file, unknown subcommand) include command-form examples that are copyable for retry automation.
- This actionable-message normalization is owned by parser/validation paths in `cli/src/services/parse/command_runtime.rs`, `cli/src/services/bash_policy.rs`, and `cli/src/services/setup/mod.rs`.

## Ownership

- `FailureClass` in `cli/src/services/error.rs` owns class selection and default remediation text (`default_try_guidance()`), used only when no explicit hint is set.
- `ClassifiedError` in `cli/src/services/error.rs` owns stable code assignment and optional hint data (`with_hint()` builder, `hint()` accessor), defaulting to `None` on every constructor (`parse`/`validation`/`runtime`/`dependency`).
- `write_error_diagnostic` in `cli/src/services/app_support.rs` owns final code-bearing stderr rendering and the hint-vs-class-default remediation choice — i.e. it owns `Try:` presentation, not remediation content.
- `Logger::log_classified_error` in `cli/src/services/observability.rs` owns structured error logging with `sce.error.{code}` event IDs.
- `app_support::render_run_outcome` (invoked from `cli/src/app.rs`) owns error logging before stderr emission.

### The anyhow-boundary exception

- `ClassifiedError::from_anyhow_text(message)` in `cli/src/services/error.rs` is the one intentional, construction-time exception to "remediation presence is not decided by inspecting message text." It runs once, at the point an already-formatted `anyhow::Error` (which may already carry a hand-composed trailing `" Try: ..."` clause from code that predates the hint field) is converted into a `ClassifiedError::runtime(...)`. It splits on the last `" Try: "` occurrence, attaching the tail as an explicit hint via `with_hint()` and keeping the remainder as `message()`. After this point, the resulting `ClassifiedError` is indistinguishable from one built with `with_hint()` directly — no message inspection occurs at render time. It is used at the shared anyhow-boundary `.map_err` conversion sites (`auth_command/command.rs`, `config/command.rs`, `version/command.rs`, `hooks/command.rs`, `setup/command.rs`, `trace/command.rs`, `doctor/command.rs`).
- `auth_command/mod.rs`'s `with_try_guidance` is a separate, out-of-scope helper that composes plain `anyhow::Error`/`String` text *upstream* of any `ClassifiedError` construction (e.g. before `AuthError` values reach the anyhow boundary above). Its own `message.contains("Try:")` check exists only to avoid double-composing `Try:` guidance inside that pre-boundary text; it does not affect `ClassifiedError` rendering and was intentionally left unchanged.

## Determinism and testing

- Error code value is derived from failure class and is stable for a given class.
- Code-bearing stderr output and remediation presence (hint-present vs. class-default) are locked by tests in `cli/src/services/error.rs` and `cli/src/services/app_support.rs`.
