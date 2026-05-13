# CLI Observability Contract

## Scope

This document defines the structured observability baseline and in-scope event contract for `sce` runtime execution.
It covers deterministic stderr logger controls, the current logger and telemetry trait boundaries, optional OpenTelemetry export bootstrap, config-backed runtime resolution, startup degradation behavior for invalid discovered config, and event emission boundaries in `cli/src/services/observability.rs`, `cli/src/services/config/mod.rs`, `cli/src/app.rs`, `cli/src/services/hooks/mod.rs`, and `cli/src/services/agent_trace_db/mod.rs`.

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

## Hook and Agent Trace event taxonomy

The hook and Agent Trace / diff-trace event contract extends the app-level lifecycle signal set without changing command stdout payloads or hook behavior.
Events below are the stable target contract for hook and Agent Trace observability work; paths not yet instrumented must either emit these IDs or retain the no-event rationale listed here until implemented by a follow-up task.

### Hook dispatch events

- `sce.hooks.dispatch_start` (debug): emitted after the hook subcommand is parsed and before subcommand-specific work begins.
  - Required safe fields: `component=hooks`, `hook_subcommand`, `repo_root_present`.
- `sce.hooks.dispatch_end` (debug): emitted after a hook subcommand succeeds.
  - Required safe fields: `component=hooks`, `hook_subcommand`, `outcome`.
- `sce.hooks.dispatch_error` (error): emitted when a hook subcommand returns a runtime error after parse/validation.
  - Required safe fields: `component=hooks`, `hook_subcommand`, `failure_class` or `error_kind` when available.
- Hook dispatch events must not include raw stdin, raw patch text, commit-message content, auth/config secrets, or full filesystem payloads.

### No-op hook paths

- `pre-commit` and `post-rewrite` currently remain deterministic no-op paths.
- They should emit only the shared hook dispatch start/end/error events unless a future runtime behavior makes path-specific fields useful.
- No separate path-specific event is required while their only behavior is no-op status rendering.
- `post-rewrite` must not log raw stdin received from Git.

### Commit-message attribution hook

- `sce.hooks.commit_msg.attribution_checked` (debug): emitted after attribution gate evaluation and trailer-policy processing.
  - Required safe fields: `component=hooks`, `hook_subcommand=commit-msg`, `attribution_enabled`, `sce_disabled`, `policy_gate_passed`, `trailer_applied`.
  - Optional safe fields: `message_file_path_present=true` only; do not log the path if it may contain sensitive user directory names unless passed through existing redaction policy.
- The event must not include raw commit-message contents, existing trailer text, author identity extracted from the message, or full message-file paths unless redacted.

### Diff-trace intake events

- `sce.hooks.diff_trace.received` (debug): emitted after stdin JSON is parsed and required fields are validated.
  - Required safe fields: `component=hooks`, `hook_subcommand=diff-trace`, `session_id_present`, `diff_bytes`, `time_ms_present`.
  - `sessionID` itself must not be logged unless a future explicit decision classifies it as safe.
- `sce.hooks.diff_trace.artifact_write_start` (debug): emitted before writing the collision-safe `context/tmp/*-diff-trace.json` artifact.
  - Required safe fields: `component=hooks`, `artifact_category=context_tmp`, `attempt` when retrying collision-safe names.
- `sce.hooks.diff_trace.artifact_write_end` (debug): emitted after artifact persistence succeeds.
  - Required safe fields: `component=hooks`, `artifact_category=context_tmp`, `attempt`.
- `sce.hooks.diff_trace.agent_trace_db_write_start` (debug): emitted before inserting the diff trace into Agent Trace DB.
  - Required safe fields: `component=agent_trace_db`, `hook_subcommand=diff-trace`.
- `sce.hooks.diff_trace.agent_trace_db_write_end` (debug): emitted after the DB insert succeeds.
  - Required safe fields: `component=agent_trace_db`, `hook_subcommand=diff-trace`, `time_ms`.
- Existing failure events remain part of the contract and should gain structured safe fields where practical:
  - `sce.hooks.diff_trace.error`
  - `sce.hooks.diff_trace.agent_trace_db_time_invalid`
  - `sce.hooks.diff_trace.agent_trace_db_write_failed`
- Diff-trace events must never include raw patch text, the full JSON payload, raw `sessionID`, or absolute artifact/database paths.

### Post-commit intersection events

- `sce.hooks.post_commit.intersection_start` (debug): emitted before the post-commit intersection flow captures/query/persist work.
  - Required safe fields: `component=hooks`, `hook_subcommand=post-commit`, `window_days`.
- `sce.hooks.post_commit.patch_capture_end` (debug): emitted after current commit patch capture succeeds.
  - Required safe fields: `component=hooks`, `commit_present`, `patch_file_count`.
  - `commit` may be logged only in the same short/stable form already rendered in hook stdout; raw patch content must not be logged.
- `sce.hooks.post_commit.recent_diff_traces_loaded` (debug): emitted after querying recent diff traces.
  - Required safe fields: `component=agent_trace_db`, `window_start_ms`, `window_end_ms`, `loaded_count`, `skipped_count`.
- `sce.hooks.post_commit.intersection_persisted` (debug): emitted after persisting the post-commit patch intersection.
  - Required safe fields: `component=agent_trace_db`, `loaded_count`, `skipped_count`, `intersection_files`.
- `sce.hooks.post_commit.intersection_error` (error): emitted when the post-commit intersection flow fails after dispatch begins.
  - Required safe fields: `component=hooks` or `component=agent_trace_db`, `error_kind` when available.
- Post-commit events must not include raw current-commit patches, stored diff-trace patches, serialized intersections, or trace artifact contents.

### Agent Trace DB adapter boundaries

- Agent Trace DB setup/doctor lifecycle remains primarily operator-facing through `sce doctor` and `sce setup`; no separate lifecycle event is required while lifecycle providers already report DB path/health through command output.
- Runtime DB work should emit at caller boundaries for diff-trace intake and post-commit intersection rather than logging every low-level SQL statement.
- Safe Agent Trace DB fields are counts, inclusive query-window bounds, path category (`state_root` / `agent_trace_db`), migration/setup outcome categories, and failure classifications.
- Unsafe Agent Trace DB fields are raw patch JSON, raw SQL parameters, full user paths unless redacted, session IDs, tokens, and config secret values.

## Operator verification contract

- `sce config show` is the canonical operator surface for resolved observability settings and provenance: `log_level`, `log_format`, `log_file`, `log_file_mode`, `otel.enabled`, `otel.exporter_otlp_endpoint`, and `otel.exporter_otlp_protocol`.
- `sce config validate` confirms observability config validity without duplicating the full provenance display.
- `sce doctor` is the canonical operator surface for Agent Trace DB readiness and health, including path resolution, parent-directory readiness, DB initialization health, and fix-mode bootstrap where supported.
- Configured log-file output verifies the same `event_id` records that are emitted on stderr; file logging must preserve stdout payload separation and line-oriented deterministic rendering.
- Optional OTEL export verifies that logger events are mirrored into tracing events under the app subscriber context; tests should validate bootstrap/config behavior without requiring a real external collector.
- Operators should be able to verify hook and Agent Trace observability by enabling debug-level logging with a file sink, running hook/diff-trace/post-commit paths in a disposable repository, and checking for event IDs plus safe metadata without raw patch or secret leakage.

## Sensitive-field exclusion contract

- Never log raw patch contents, serialized patch/intersection JSON, complete diff-trace stdin payloads, commit-message contents, tokens, config secrets, or private signing keys.
- Do not log raw `sessionID` values unless a future decision explicitly classifies them as operator-safe.
- Avoid absolute filesystem paths in hook/Agent Trace events unless they pass through the existing redaction policy or are represented as path categories.
- Prefer counts, booleans, enum-like status values, millisecond timestamps/window bounds, command names, hook subcommand names, and safe outcome categories.
- Any new observability event added to hook or Agent Trace paths must declare its sensitive-field exclusions in this contract before implementation.

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
- The same traits module exposes object-safe `services::observability::traits::Telemetry` with the current app subscriber boundary: `with_default_subscriber` for command-lifecycle execution.
- The concrete `services::observability::TelemetryRuntime` implements the telemetry trait by delegating to its existing inherent method, preserving OTEL subscriber behavior.
- `cli/src/app.rs` stores production logger and telemetry runtime instances behind `Arc<dyn Logger>` and `Arc<dyn Telemetry>` in `AppContext`, then passes that context through command execution without changing emitted events or OTEL behavior.

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
