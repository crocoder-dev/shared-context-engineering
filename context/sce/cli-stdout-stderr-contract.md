# CLI stdout/stderr contract

## Scope

This document defines the implemented stream contract for CLI command payload and diagnostics in `cli/src/app.rs`.

## Contract

- Command success payloads are emitted to `stdout` only through app-level stream handling.
- User-facing diagnostics and failures are emitted to `stderr` only.
- `CliError::Internal` failure diagnostics are emitted as `Error [<code>]: ...` on `stderr`, where `<code>` is the stable class-based `SCE-ERR-*` identifier from `CliError` in `cli/src/services/error.rs`. `CliError::User` failures emit only their redacted message and trailing newline on `stderr`, without the wrapper, code, guidance, or ANSI styling. All emitted diagnostic text is passed through shared redaction (`services::security::redact_sensitive_text`) before emission.
- The diagnostic body differs by `CliError` variant: `CliError::Internal` renders the real `anyhow` source chain (`format!("{source:#}")`) plus class-default `Try:` remediation and applies the stderr TTY/`NO_COLOR` styling policy; the catalog variant renders its `UserError` message after redaction, with no low-level technical text, wrapper, styling, or `Try:` suffix.
- Command handlers now return payload strings to the app dispatcher; the app owns stream selection and final emission.

## Implementation surface

- Async `app::run_with_dependency_check_and_streams(...)` borrows the output writers until execution and rendering finish.
- `app::try_run_with_dependency_check(...)` completes synchronous startup and directly awaits `run_command_lifecycle`, returning a `RunOutcome<L>` containing payload text or a classified error, logger and optional startup diagnostic.
- `app_support::execute_command_phase(...)` awaits static `RuntimeCommand::execute_with_stderr(...)`, which returns the completed payload. Sync borrows the app's stderr writer for format-gated progress through its await.
- Synchronous `app_support::render_run_outcome(...)` owns final emission after execution completes; `write_stdout_payload(...)` handles success payload writes and `write_error_diagnostic(...)` handles redacted error writes in the same module.

See also: `context/sce/cli-error-code-taxonomy.md` for the canonical error-code classes and `Try:` remediation injection rules.

## Determinism notes

- Stream routing is centralized in one app-level path to avoid per-command stream drift.
- Exit code class mapping remains unchanged (`parse`, `validation`, `runtime`, `dependency`).
- Observability logger records are independent from command payload output: with `log_to_file=true`, normal records are written to the configured log file and not emitted to `stderr`; with `log_to_file=false`, only error-level logger records are emitted to `stderr`. User-facing diagnostics, direct file-write diagnostics, and text-mode sync progress remain on `stderr`.
- Text-mode `sce sync` emits its aligned three-row `indicatif` progress display on `stderr` before accepted batches begin: rows start at zero with independent steady spinners, accepted batches update only the corresponding cumulative count, and each stream receives a styled completion check at its own future boundary. Redirected/non-TTY output stays plain and free of terminal-control sequences, while `NO_COLOR` disables styling. The final text report remains the command result without repository or source-instance identifiers; JSON-mode sync emits no human progress text and keeps its JSON-only payload on `stdout`, also without those identifiers.

The durable trace-sync stream choice is recorded in [Trace-sync progress stream contract](../decisions/2026-08-13-trace-sync-progress-stream-contract.md).
