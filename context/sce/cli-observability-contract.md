# CLI Observability Contract

## Scope

This document defines the implemented structured observability baseline for `sce` runtime execution.
It covers deterministic stderr logger controls, optional OpenTelemetry export bootstrap, config-backed runtime resolution, and event emission boundaries in `cli/src/services/observability.rs`, `cli/src/services/config.rs`, and `cli/src/app.rs`.

Runtime observability now consumes the shared resolved observability config from `cli/src/services/config.rs`: env values still win, config-file values act as fallback, and defaults apply when both are absent. The same resolved values are now surfaced to operators through `sce config show|validate`, with deterministic text output plus text/JSON `source` and `config_source` provenance for the flat logging keys and nested `otel` keys.

## Runtime controls

- `SCE_LOG_LEVEL` selects log threshold with allowed values `error`, `warn`, `info`, `debug`.
- `SCE_LOG_FORMAT` selects log format with allowed values `text`, `json`.
- `SCE_LOG_FILE` optionally enables a file log sink at the provided file path.
- `SCE_LOG_FILE_MODE` controls file-write policy with allowed values `truncate` and `append`.
- `SCE_LOG_FILE_MODE` requires `SCE_LOG_FILE`.
- Defaults are deterministic: `log_level=error` and `log_format=text` when higher-precedence env/config inputs are unset.
- When file logging is enabled and `SCE_LOG_FILE_MODE` is unset, default policy is `truncate`.
- Invalid observability env or config-backed values fail invocation validation with actionable error text.
- OpenTelemetry bootstrap is opt-in via resolved `otel.enabled` / `SCE_OTEL_ENABLED` (`true|false|1|0`, default `false`).
- When OpenTelemetry is enabled, exporter config resolves from env first and config-file fallback second:
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

- `cli/src/services/config.rs` owns shared observability value resolution, config-file discovery/merge, and env-over-config precedence for runtime inputs.
- `cli/src/services/observability.rs` owns runtime logger construction from resolved values, level filtering, record rendering, optional file sink lifecycle/permission enforcement, and OTEL runtime setup (`TelemetryRuntime`).
- `cli/src/app.rs` owns lifecycle event emission around parse/dispatch success and failure paths, resolves observability config before command dispatch, and wraps dispatch inside the observability subscriber context.
- Contract behavior is covered by `services::observability::tests` and exercised in end-to-end app command tests.
