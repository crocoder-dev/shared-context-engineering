# CLI stdout/stderr contract

## Scope

This document defines the implemented stream contract for CLI command payload and diagnostics in `cli/src/app.rs`.

## Contract

- Command success payloads are emitted to `stdout` only through app-level stream handling.
- User-facing diagnostics and failures are emitted to `stderr` only.
- Failure diagnostics are prefixed with `Error:` and passed through shared redaction (`services::security::redact_sensitive_text`) before emission.
- Command handlers now return payload strings to the app dispatcher; the app owns stream selection and final emission.

## Implementation surface

- `run_with_dependency_check_and_streams(...)` is the app-level stream boundary for production and tests.
- `try_run_with_dependency_check(...)` performs parse + dispatch and returns payload text or classified errors.
- `dispatch(...)` returns payload text for each command path rather than writing directly to process streams.
- `write_stdout_payload(...)` handles success payload writes.
- `write_error_diagnostic(...)` handles redacted error writes.

## Determinism notes

- Stream routing is centralized in one app-level path to avoid per-command stream drift.
- Exit code class mapping remains unchanged (`parse`, `validation`, `runtime`, `dependency`).
- Observability lifecycle logs remain on `stderr` by contract and are independent from command payload output.
