# CLI Observability Contract

## Scope

This document defines the implemented structured observability baseline for `sce` runtime execution.
It covers deterministic stderr logger controls, optional OpenTelemetry export bootstrap, and event emission boundaries in `cli/src/services/observability.rs` and `cli/src/app.rs`.

## Runtime controls

- `SCE_LOG_LEVEL` selects log threshold with allowed values `error`, `warn`, `info`, `debug`.
- `SCE_LOG_FORMAT` selects log format with allowed values `text`, `json`.
- `SCE_LOG_FILE` optionally enables a file log sink at the provided file path.
- `SCE_LOG_FILE_MODE` controls file-write policy with allowed values `truncate` and `append`.
- `SCE_LOG_FILE_MODE` requires `SCE_LOG_FILE`.
- Defaults are deterministic: `SCE_LOG_LEVEL=info` and `SCE_LOG_FORMAT=text` when env keys are unset.
- When file logging is enabled and `SCE_LOG_FILE_MODE` is unset, default policy is `truncate`.
- Invalid observability env values fail invocation validation with actionable error text.
- OpenTelemetry bootstrap is opt-in via `SCE_OTEL_ENABLED` (`true|false|1|0`, default `false`).
- When OpenTelemetry is enabled, exporter config is env-addressable:
  - `OTEL_EXPORTER_OTLP_ENDPOINT` (default `http://127.0.0.1:4317`, must be absolute `http(s)` URL)
  - `OTEL_EXPORTER_OTLP_PROTOCOL` (`grpc` or `http/protobuf`, default `grpc`)
- Invalid OTEL env values fail invocation validation with explicit remediation guidance.

## Emission contract

- Log output is emitted to `stderr` only; command result payloads remain on `stdout`.
- When `SCE_LOG_FILE` is set, the same rendered log lines are also mirrored to the configured file sink.
- Each emitted record includes a stable `event_id`.
- Current app-level event identifiers:
  - `sce.app.start`
  - `sce.command.parsed`
  - `sce.command.completed`
  - `sce.command.parse_failed`
  - `sce.command.failed`
- Event records include deterministic metadata keys used by automation (`command`, `failure_class`, `component` when applicable).
- Logger events are mirrored into tracing events so OTEL export can observe the same lifecycle signal set when enabled.
- App runtime initializes tracing subscriber context before parse/dispatch and shuts down tracer provider on process exit.

## Format contract

- `text` format emits single-line key/value records with fixed key ordering: `log_format`, `level`, `event_id`, `message`, then optional fields.
- `json` format emits a single-line object with fixed top-level keys: `log_format`, `level`, `event_id`, `message`, `fields`.
- Logger threshold behavior is deterministic and severity-based (`error < warn < info < debug`).
- File sink writes are deterministic line-based writes with immediate flush after each record.

## File sink safety contract

- On file-sink initialization, parent directories are created when missing.
- On Unix, log file permissions are tightened to owner-only (`0600`) when group/other bits are present.
- File open failures include actionable remediation guidance (verify writable path or unset `SCE_LOG_FILE`).
- File write failures are reported to `stderr` as diagnostics and do not alter command `stdout` payload contracts.

## Ownership and verification

- `cli/src/services/observability.rs` owns env parsing, level filtering, record rendering, and optional file sink lifecycle/permission enforcement.
- `cli/src/services/observability.rs` also owns OTEL runtime setup (`TelemetryRuntime`) and deterministic endpoint/protocol validation.
- `cli/src/app.rs` owns lifecycle event emission around parse/dispatch success and failure paths and wraps dispatch inside the observability subscriber context.
- Contract behavior is covered by `services::observability::tests` and exercised in end-to-end app command tests.
