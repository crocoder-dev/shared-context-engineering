# CLI stdout/stderr contract

## Scope

This document defines the implemented stream contract for CLI command payload and diagnostics in `cli/src/app.rs`.

## Contract

- Command success payloads are emitted to `stdout` only through app-level stream handling.
- User-facing diagnostics and failures are emitted to `stderr` only.
- Classified fallback diagnostics are emitted as `Error [<code>]: ...` on `stderr`, where `<code>` is the stable class-based `SCE-ERR-*` identifier from `ClassifiedError`; an explicit `UserFacingPresentation` instead emits only its redacted configured message without renderer-owned styling. Both paths pass through shared redaction (`services::security::redact_sensitive_text`) before emission, while caller-provided presentation styling is preserved.
- Command handlers now return payload strings to the app dispatcher; the app owns stream selection and final emission.

## Implementation surface

- `run_with_dependency_check_and_streams(...)` is the app-level stream boundary for production and tests.
- `try_run_with_dependency_check(...)` performs parse + dispatch and returns payload text or classified errors.
- `dispatch(...)` returns payload text for each command path rather than writing directly to process streams.
- `write_stdout_payload(...)` handles success payload writes.
- `write_error_diagnostic(...)` handles redacted fallback or explicit user-facing error writes.

See also: `context/sce/cli-error-code-taxonomy.md` for the canonical error-code classes and `Try:` remediation injection rules.

## Determinism notes

- Stream routing is centralized in one app-level path to avoid per-command stream drift.
- Exit code class mapping remains unchanged (`parse`, `validation`, `runtime`, `dependency`).
- Observability lifecycle logs use the logger's tracing/file sinks and configured stderr sink. Classified fallback errors retain the logger stderr record, while explicit user-facing presentations suppress only that per-call logger record; intentional user-facing diagnostics and command-owned progress remain on `stderr` independently from command payload output.
- Text-mode `sce sync` emits its aligned four-row `indicatif` progress display on `stderr` before accepted batches begin: rows start at zero with independent steady spinners, accepted batches update only the corresponding cumulative count, and each stream receives a styled completion check at its own future boundary. Redirected/non-TTY output stays plain and free of terminal-control sequences, while `NO_COLOR` disables styling. The final text report remains the command result without repository or source-instance identifiers; JSON-mode sync emits no human progress text and keeps its JSON-only payload on `stdout`, also without those identifiers.

The durable trace-sync stream choice is recorded in [Trace-sync progress stream contract](../decisions/2026-08-13-trace-sync-progress-stream-contract.md).
