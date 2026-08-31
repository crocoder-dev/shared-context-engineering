# CLI stdout/stderr contract

## Scope

This document defines the implemented stream contract for CLI command payload and diagnostics in `cli/src/app.rs`.

## Contract

- Command success payloads are emitted to `stdout` only through app-level stream handling.
- User-facing diagnostics and failures are emitted to `stderr` only.
- Failure diagnostics are emitted as `Error [<code>]: ...` on `stderr`, where `<code>` is the stable class-based `SCE-ERR-*` identifier from `CliError` in `cli/src/services/error.rs`; diagnostics are passed through shared redaction (`services::security::redact_sensitive_text`) before emission.
- The diagnostic body differs by `CliError` variant: `CliError::Internal` renders the real `anyhow` source chain (`format!("{source:#}")`) plus class-default `Try:` remediation; `CliError::User` renders its catalog `UserError` template, with non-authentication automatic-sync payload reasons included only in that reviewed message and no technical source chain or `Try:` suffix. Both bodies are styled through the same stderr TTY/`NO_COLOR` policy before redaction and emission.
- Command handlers now return payload strings to the app dispatcher; the app owns stream selection and final emission.
- The post-commit `sce sync --format json` child keeps stdout null so a successful automatic run produces no hook payload, while inheriting stderr so its single app-rendered typed failure diagnostic remains observable. The launcher waits for terminal child completion, making child lifetime and inherited-stderr closure deterministic while adding child synchronization latency to post-commit execution. It ignores a non-zero child exit after that diagnostic and renders executable/spawn/wait failures through the same stderr diagnostic writer in the parent; all remain fail-open.
- Automatic child failures render exactly one `Error [SCE-ERR-RUNTIME]: Automatic synchronization failed: ...` diagnostic on inherited stderr. Authentication exposes login-plus-manual-sync recovery while its technical reason stays in observability; non-authentication reasons and recovery guidance are included in that one diagnostic, with no generic duplicate `Try:` suffix.

## Implementation surface

- `run_with_dependency_check_and_streams(...)` is the app-level stream boundary for production and tests.
- `try_run_with_dependency_check(...)` performs parse + dispatch and returns payload text or classified errors.
- `dispatch(...)` returns payload text for each command path rather than writing directly to process streams.
- `write_stdout_payload(...)` handles success payload writes.
- `write_error_diagnostic(...)` handles redacted error writes.

See also: `context/sce/cli-error-code-taxonomy.md` for the canonical error-code classes and `Try:` remediation injection rules.

## Determinism notes

- Stream routing is centralized in one app-level path to avoid per-command stream drift.
- Exit code class mapping remains unchanged (`parse`, `validation`, `runtime`, `dependency`).
- Observability lifecycle logs remain on `stderr` by contract and are independent from command payload output.
- Text-mode `sce sync` emits its aligned four-row `indicatif` progress display on `stderr` before accepted batches begin: rows start at zero with independent steady spinners, accepted batches update only the corresponding cumulative count, and each stream receives a styled completion check at its own future boundary. Redirected/non-TTY output stays plain and free of terminal-control sequences, while `NO_COLOR` disables styling. The final text report remains the command result without repository or source-instance identifiers; JSON-mode sync emits no human progress text and keeps its JSON-only payload on `stdout`, also without those identifiers.

The durable trace-sync stream choice is recorded in [Trace-sync progress stream contract](../decisions/2026-08-13-trace-sync-progress-stream-contract.md).
