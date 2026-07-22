# CLI Observability Contract

## Scope

This document defines the implemented structured observability baseline for `sce` runtime execution.
It covers deterministic stderr logger controls, optional log-directory routing with bounded retention, the current logger and telemetry trait boundaries, config-backed runtime resolution, startup degradation behavior for invalid discovered config, and event emission boundaries in `cli/src/services/observability.rs`, `cli/src/services/config/mod.rs`, and `cli/src/app.rs`.

Runtime observability now consumes the shared resolved observability config from `cli/src/services/config/mod.rs`: env values still win, config-file values act as fallback, and defaults apply when both are absent. When default-discovered config files are invalid JSON, fail schema validation, or are not top-level JSON objects, observability resolution now skips those files, collects the failure text in `validation_errors`, and continues with defaults; explicit `--config` / `SCE_CONFIG_FILE` selections remain fatal. Startup therefore keeps running with degraded observability defaults instead of turning discovered invalid config into a startup failure. Those resolved values are surfaced to operators through `sce config show`; `sce config validate` uses the same validation path but now reports only validation status plus any errors or warnings.

## Runtime controls

- `SCE_LOG_LEVEL` selects log threshold with allowed values `error`, `warn`, `info`, `debug`.
- `SCE_LOG_FORMAT` selects log format with allowed values `text`, `json`.
- `SCE_LOG_DIR` configures the optional log-directory value used by the logger configuration surface.
- Defaults are deterministic: `log_level=error` and `log_format=text` when higher-precedence env/config inputs are unset.
- `log_dir` remains unset when no env/config value is present.
- Invalid observability env values still fail invocation validation with actionable error text.
- Invalid default-discovered observability config files no longer block runtime config resolution by themselves; they are skipped and resolution falls back to defaults.
- After degraded observability config is constructed, startup emits one `warn`-level log per skipped discovered-file failure before command dispatch continues.
## Repository-local default in this repo

- This repository now ships a repo-local config at `.sce/config.json`.
- The local config sets `log_level=error` and `log_dir=context/tmp`.
- Running `sce` commands from this repository resolves `context/tmp` as the configured log directory unless `SCE_LOG_DIR` overrides it.

## Emission contract

- Log output is always emitted to `stderr`; command result payloads remain on `stdout`.
- When `log_dir` is configured, each enabled or forced log operation appends the redacted rendered record to a file selected at emit time from the machine-local date and optional caller-provided session ID.
- Sessionless file logs route to `<log_dir>/sce-<dd_mm_yyyy>.log`; session-aware file logs route to `<log_dir>/sce-<dd_mm_yyyy>-<sanitized_session_id>.log`.
- Session filename sanitization preserves ASCII letters, digits, `-`, and `_`; percent-encodes every other UTF-8 byte as uppercase `%HH`; and represents an explicitly empty `Some("")` session ID with the reserved `%EMPTY` token.
- `sce hooks diff-trace` and `conversation-trace` pass producer-native session context into this existing routing argument when available. Diff-trace logging never uses the AgentTraceDb-only `oc_`/`cc_`/`pi_` prefix; skipped conversation items use their own session; batch-wide conversation insert failures use the first valid insert's session. Agent Trace DB open failures use hook-specific error events (`sce.hooks.diff_trace.agent_trace_db_open_failed` and `sce.hooks.conversation_trace.agent_trace_db_open_failed`) and do not also emit their broader write/intake events for the same failure. Session IDs remain absent from rendered record fields unless separately supplied as fields.
- File routing creates the configured directory when needed, uses owner-only create permissions on Unix, serializes writes per path, and fails open by keeping stderr logging active when file creation/write/flush fails.
- After a successful write to a newly created sessionless or session-aware SCE log file, the logger runs one best-effort retention pass over direct regular `*.log` children of `log_dir`. Existing-file appends do not scan or delete files. Retention keeps at most 10 `.log` files, ordered newest-first by filesystem modification time with path/name ordering as the deterministic tie-break, and removes older `.log` files regardless of whether their names are SCE-owned.
- Retention is non-recursive and ignores non-regular entries plus non-`.log` files such as directories, symlinks, database files, and extensionless artifacts. Directory scan, metadata, or deletion failures do not fail the completed write; cleanup emits a redacted direct-stderr diagnostic without re-entering `Logger::log` and leaves entries it cannot safely process intact.
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
- App runtime initializes tracing subscriber context before parse/dispatch and shuts down tracer provider on process exit.
- Tracing event emission checks the `sce` target and requested tracing level before constructing serialized `fields` payloads; disabled or filtered tracing events return without building field JSON while enabled events preserve the same `event_id`, `event_message`, and `fields` payload shape.

## Format contract

- `text` format emits single-line key/value records with fixed key ordering: `timestamp`, `log_format`, `level`, `event_id`, `message`, then optional fields.
- `json` format emits a single-line object with fixed top-level keys: `timestamp`, `log_format`, `level`, `event_id`, `message`, `fields`.
- Timestamps are UTC ISO8601 with millisecond precision (e.g., `2026-03-20T14:30:00.123Z`) generated via `chrono::Utc::now()`.
- Logger threshold behavior is deterministic and severity-based (`error < warn < info < debug`).
- Startup invalid-config diagnostics use an explicit warn-emission path so the warning is still rendered even when degraded defaults resolve to `log_level=error`.
- Rendered records remain deterministic line-based strings on `stderr`; optional log-directory files contain the same redacted rendered lines, do not add session IDs to the record schema automatically, and are bounded by creation-triggered `*.log` retention.

## Observability trait boundaries

- `cli/src/services/observability/traits.rs` exposes the `services::observability::traits::Logger` trait with the current logging API: `info`, `debug`, `warn`, `error`, and `log_classified_error`, each accepting `Option<&str>` session context used only for optional file routing.
- The concrete `services::observability::Logger` implements the trait while retaining the existing inherent methods and behavior.
- `NoopLogger` is available from the same traits module for tests and future dependency-injected services that need a logger without side effects.
- The same traits module exposes object-safe `services::observability::traits::Telemetry` with the current app subscriber boundary: `with_default_subscriber` for command-lifecycle execution.
- The concrete `services::observability::TelemetryRuntime` implements the telemetry trait by delegating to its existing inherent method.
- `cli/src/app.rs` stores the production logger and telemetry runtime as concrete `AppRuntime` fields, creates borrowed `AppContext` views for command execution, and exposes logger/telemetry access through associated-type context accessors instead of owned `Arc<dyn ...>` fields or object-erased accessor return values.
- Final stream rendering uses `RunOutcome<L: Logger>` in `cli/src/services/app_support.rs`, so classified-error and stdout-write-failure logging depends on the logger trait boundary rather than the concrete production logger type.
- `run_command_lifecycle` expects the telemetry subscriber action to execute command dispatch at most once; if a telemetry implementation invokes the action again, the app returns a `SCE-ERR-RUNTIME` classified error rather than panicking or reparsing consumed arguments.

## Log directory config safety contract

- `log_dir` config-file values are schema-validated as non-empty strings.
- `SCE_LOG_DIR` env values are resolved with env-over-config precedence and rejected when explicitly empty.
- Logger construction validates configured `log_dir` as non-empty without opening files; directory creation, per-operation local-date file selection, append writes, Unix owner-only create permissions, and creation-triggered retention happen at log emission time.

## Ownership and verification

- `cli/src/services/config/mod.rs` owns shared observability value resolution, config-file discovery/merge, and env-over-config precedence for runtime inputs.
- `cli/src/services/observability.rs` owns runtime logger construction from resolved values, `log_dir` non-empty validation, level filtering, tracing-event enablement checks, record rendering, local-date/session file-name selection, session filename sanitization, append file writes, and best-effort `.log` retention; `cli/src/services/observability/traits.rs` owns the logger and telemetry trait boundaries plus the no-op logger implementation.
- `cli/src/app.rs` owns lifecycle event emission around parse/dispatch success and failure paths, resolves observability config before command dispatch, emits startup invalid-config warning events for skipped discovered config files, wraps dispatch inside the observability subscriber context, and guards the single-use command-dispatch action against repeated telemetry invocation with a runtime-classified error. `cli/src/services/app_support.rs` owns final stdout/stderr rendering and generic logger-backed classified-error logging.
- Contract behavior is exercised by app command tests and the root flake check suite.
