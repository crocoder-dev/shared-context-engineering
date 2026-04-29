# CLI Observability Contract

## Scope

This document defines the implemented structured observability baseline for `sce` runtime execution.
It covers deterministic stderr logger controls, the current logger and telemetry trait boundaries, optional OpenTelemetry export bootstrap, config-backed runtime resolution, startup degradation behavior for invalid discovered config, and event emission boundaries in `cli/src/services/observability.rs`, `cli/src/services/config/mod.rs`, and `cli/src/app.rs`.

Runtime observability now consumes the shared resolved observability config from `cli/src/services/config/mod.rs`: env values still win, config-file values act as fallback, and defaults apply when both are absent. When default-discovered config files are invalid JSON, fail schema validation, or are not top-level JSON objects, observability resolution now skips those files, collects the failure text in `validation_errors`, and continues with defaults; explicit `--config` / `SCE_CONFIG_FILE` selections remain fatal. Startup therefore keeps running with degraded observability defaults instead of turning discovered invalid config into a startup failure. Those resolved values are surfaced to operators through `sce config show`; `sce config validate` uses the same validation path but now reports only validation status plus any errors or warnings.

## Runtime controls

- `SCE_LOG_LEVEL` selects log threshold with allowed values `error`, `warn`, `info`, `debug`.
- `SCE_LOG_FORMAT` selects log format with allowed values `text`, `json`.
- `SCE_LOG_FILE` optionally enables a file log sink at the provided file path.
- `SCE_LOG_FILE_MODE` controls file-write policy with allowed values `truncate` and `append`.
- `SCE_LOG_FILE_MODE` requires `SCE_LOG_FILE`.
- Defaults are deterministic: `log_level=error` and `log_format=text` when higher-precedence env/config inputs are unset.
- When file logging is enabled and `SCE_LOG_FILE_MODE` is unset, default policy is `truncate`.
- Invalid observability env values still fail invocation validation with actionable error text.
- Invalid default-discovered observability config files no longer block runtime config resolution by themselves; they are skipped and resolution falls back to defaults.
- After degraded observability config is constructed, startup emits one `warn`-level log per skipped discovered-file failure before command dispatch continues.
- OpenTelemetry bootstrap is opt-in via resolved `otel.enabled` / `SCE_OTEL_ENABLED` (`true|false|1|0`, default `false`).
- When OpenTelemetry is enabled, exporter config resolves from env first and config-file fallback second:
  - `OTEL_EXPORTER_OTLP_ENDPOINT` (default `http://127.0.0.1:4317`, must be absolute `http(s)` URL)
  - `OTEL_EXPORTER_OTLP_PROTOCOL` (`grpc` or `http/protobuf`, default `grpc`)
- Invalid OTEL env values fail invocation validation with explicit remediation guidance.

## Repository-local default in this repo

- This repository now ships a repo-local config at `.sce/config.json`.
- The local config sets `log_level=debug`, `log_file=context/tmp/sce.log`, and `log_file_mode=append`.
- Running `sce` commands from this repository therefore mirrors lifecycle logs into `context/tmp/sce.log` unless higher-precedence flag or env inputs override those values.

## Emission contract

- Log output is emitted to `stderr` only; command result payloads remain on `stdout`.
- When `SCE_LOG_FILE` is set, the same rendered log lines are also mirrored to the configured file sink.
- Each emitted record includes a stable `event_id`.
- Current app-level event identifiers:
  - `sce.app.start`
  - `sce.config.invalid_config` (warn level - emitted once per skipped invalid discovered config file during startup)
  - `sce.config.file_discovered` (debug level - logged for each discovered config file)
  - `sce.command.raw_args` (debug level - logged at command parsing entry)
  - `sce.command.parsed`
  - `sce.command.dispatch_start` (debug level - logged before dispatch)
  - `sce.command.dispatch_end` (debug level - logged after successful dispatch)
  - `sce.command.completed`
- Error logging uses the pattern `sce.error.{code}` where `{code}` is the classified error code (e.g., `sce.error.SCE-ERR-RUNTIME`).
- All `ClassifiedError` instances are logged via `Logger::log_classified_error()` before user-facing stderr diagnostics are written.
- Event records include deterministic metadata keys used by automation (`command`, `failure_class`, `component` when applicable).
- Error log records include `error_code` and `error_class` fields for structured observability.
- Logger events are mirrored into tracing events so OTEL export can observe the same lifecycle signal set when enabled.
- App runtime initializes tracing subscriber context before parse/dispatch and shuts down tracer provider on process exit.

## Format contract

- `text` format emits single-line key/value records with fixed key ordering: `timestamp`, `log_format`, `level`, `event_id`, `message`, then optional fields.
- `json` format emits a single-line object with fixed top-level keys: `timestamp`, `log_format`, `level`, `event_id`, `message`, `fields`.
- Timestamps are UTC ISO8601 with millisecond precision (e.g., `2026-03-20T14:30:00.123Z`) generated via `chrono::Utc::now()`.
- Logger threshold behavior is deterministic and severity-based (`error < warn < info < debug`).
- Startup invalid-config diagnostics use an explicit warn-emission path so the warning is still rendered even when degraded defaults resolve to `log_level=error`.
- File sink writes are deterministic line-based writes with immediate flush after each record.

## Observability trait boundaries

- `cli/src/services/observability/traits.rs` exposes the `services::observability::traits::Logger` trait with the current logging API: `info`, `debug`, `warn`, `error`, and `log_classified_error`.
- The concrete `services::observability::Logger` implements the trait while retaining the existing inherent methods and behavior.
- `NoopLogger` is available from the same traits module for tests and future dependency-injected services that need a logger without side effects.
- The same traits module exposes `services::observability::traits::Telemetry` with the current subscriber boundary: `with_default_subscriber`.
- The concrete `services::observability::TelemetryRuntime` implements the telemetry trait by delegating to its existing inherent method, preserving OTEL subscriber behavior.
- Current production call sites still use the concrete logger and telemetry runtime directly; broader call-site migration is deferred to the app-context DI tasks.

## File sink safety contract

- On file-sink initialization, parent directories are created when missing.
- On Unix, log file permissions are tightened to owner-only (`0600`) when group/other bits are present.
- File open failures include actionable remediation guidance (verify writable path or unset `SCE_LOG_FILE`).
- File write failures are reported to `stderr` as diagnostics and do not alter command `stdout` payload contracts.

## Ownership and verification

- `cli/src/services/config/mod.rs` owns shared observability value resolution, config-file discovery/merge, and env-over-config precedence for runtime inputs.
- `cli/src/services/observability.rs` owns runtime logger construction from resolved values, level filtering, record rendering, optional file sink lifecycle/permission enforcement, and OTEL runtime setup (`TelemetryRuntime`); `cli/src/services/observability/traits.rs` owns the logger and telemetry trait boundaries plus the no-op logger implementation.
- `cli/src/app.rs` owns lifecycle event emission around parse/dispatch success and failure paths, resolves observability config before command dispatch, emits startup invalid-config warning events for skipped discovered config files, and wraps dispatch inside the observability subscriber context.
- Contract behavior is covered by `services::observability::tests` and exercised in end-to-end app command tests.
