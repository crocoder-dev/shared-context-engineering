# CLI Observability Contract

## Scope

This document defines the implemented structured observability baseline for `sce` runtime execution.
It covers deterministic stderr logger controls, optional OpenTelemetry export bootstrap, and event emission boundaries in `cli/src/services/observability.rs` and `cli/src/app.rs`.

## Runtime controls

- `SCE_LOG_LEVEL` selects log threshold with allowed values `error`, `warn`, `info`, `debug`.
- `SCE_LOG_FORMAT` selects log format with allowed values `text`, `json`.
- Defaults are deterministic: `SCE_LOG_LEVEL=info` and `SCE_LOG_FORMAT=text` when env keys are unset.
- Invalid observability env values fail invocation validation with actionable error text.
- OpenTelemetry bootstrap is opt-in via `SCE_OTEL_ENABLED` (`true|false|1|0`, default `false`).
- When OpenTelemetry is enabled, exporter config is env-addressable:
  - `OTEL_EXPORTER_OTLP_ENDPOINT` (default `http://127.0.0.1:4317`, must be absolute `http(s)` URL)
  - `OTEL_EXPORTER_OTLP_PROTOCOL` (`grpc` or `http/protobuf`, default `grpc`)
- Invalid OTEL env values fail invocation validation with explicit remediation guidance.

## Emission contract

- Log output is emitted to `stderr` only; command result payloads remain on `stdout`.
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

## Ownership and verification

- `cli/src/services/observability.rs` owns env parsing, level filtering, and record rendering.
- `cli/src/services/observability.rs` also owns OTEL runtime setup (`TelemetryRuntime`) and deterministic endpoint/protocol validation.
- `cli/src/app.rs` owns lifecycle event emission around parse/dispatch success and failure paths and wraps dispatch inside the observability subscriber context.
- Contract behavior is covered by `services::observability::tests` and exercised in end-to-end app command tests.
