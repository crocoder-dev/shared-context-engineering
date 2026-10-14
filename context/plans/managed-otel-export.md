# Plan: managed-otel-export

## Change summary

Add managed OpenTelemetry export to the Rust `sce` CLI for traces and logs. Managed telemetry activates automatically when the encrypted auth DB contains a valid, unexpired telemetry credential. The existing telemetry-credential provisioning performed by `sce auth login` remains the credential source; exporter startup only reads the credential and never mints or refreshes one.

The CLI will hardcode one canonical OTLP/HTTP protobuf default set for the collector and all three signal paths. The repo-local `.sce/config.json` schema will accept an optional `otel` block for explicit declaration and future configuration, but its absence must never remove the code defaults. Traces and logs are implemented in this plan; the metrics endpoint is defined as a reserved default for the later metrics pipeline. The existing local logger, stdout/stderr contracts, hook fail-open behavior, and command lifecycle remain intact when export is unavailable or fails.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [x] AC1: Managed OTLP defaults are hardcoded in the CLI as `endpoint=https://otel.crocoderlab.dev`, `protocol=http/protobuf`, `traces_endpoint=https://otel.crocoderlab.dev/v1/traces`, `metrics_endpoint=https://otel.crocoderlab.dev/v1/metrics`, and `logs_endpoint=https://otel.crocoderlab.dev/v1/logs`; `.sce/config.json` may omit the optional `otel` block or any optional field without changing those defaults, traces and logs consume their signal-specific endpoints, and no metrics exporter is created by this plan.
  - Validate: Config-schema, resolver, and Rust tests assert optional-block/optional-field behavior and the exact code defaults; `sce config validate` accepts the repository config with the block absent; `sce config show --format json` reports code-default provenance when unset; source inspection confirms the metrics endpoint is reserved only.
- [x] AC2: A normal `sce` invocation automatically selects managed telemetry when `telemetry_credential_storage::load_telemetry_credential` returns a valid unexpired credential, uses its OpenBao `token` as an `Authorization: Bearer <token>` credential for OTLP requests, never transmits the stored `accessor`, and falls back to `NoopTelemetry` without minting or refreshing credentials when the row is absent, expired, malformed, or unavailable.
  - Validate: Credential-mode and storage tests cover valid, absent, expired, malformed, and storage-error cases; a controlled OTLP receiver asserts the bearer header and absence of the accessor; an issuance-call fixture proves exporter startup performs no credential issuance request.
- [x] AC3: Managed command spans are exported as OTLP/HTTP protobuf to `/v1/traces` with stable SCE service identity and allowlisted command/outcome/error attributes, while command stdout, stderr, exit status, and existing local log-file behavior remain unchanged.
  - Validate: A loopback receiver integration test decodes captured trace payloads and asserts endpoint, resource identity, span names, attributes, and output parity against telemetry-disabled execution.
- [x] AC4: Existing structured SCE logger records are exported as OTLP/HTTP protobuf logs to `/v1/logs`, retaining stable event identity, severity, message, and approved structured fields without exporting bearer credentials, accessors, raw command arguments, repository paths, hook payloads, or unrestricted error/source text; local file/stderr logging remains unchanged.
  - Validate: A loopback receiver integration test decodes captured log payloads, asserts the expected event/severity/field mapping, and injects secret-bearing values to prove they are rejected or redacted before export.
- [x] AC5: Managed exporter lifecycle is async-safe and fail-open: subscriber scope follows every command-future poll on Tokio worker migration, exporter work does not block protected CLI operations, shutdown/flush is bounded and occurs after command output and required persistence complete, and exporter/network failures never alter command results or hook permission decisions.
  - Validate: Focused lifecycle tests cover multi-poll/thread migration, concurrent commands, stalled/refused/redirecting receivers, bounded shutdown, lock isolation, and hook output/exit/decision parity.
- [x] AC6: Managed mode cannot exfiltrate the bearer token to an endpoint selected by repository config, inherited `OTEL_EXPORTER_OTLP_*` endpoint/header/protocol/certificate settings, or HTTP redirects; the managed collector remains the canonical `otel.crocoderlab.dev` destination and only the pinned OTLP/HTTP protobuf signal paths are accepted.
  - Validate: Security integration tests exercise non-canonical `otel` config values, malicious repository config, inherited endpoint/header/protocol overrides, redirect responses, and alternate receivers; invalid/non-canonical config is rejected or ignored before export and all tests assert zero credential-bearing requests outside the canonical collector path.
- [x] AC7: Current-state documentation records the managed OTEL runtime, encrypted-auth-DB credential boundary, canonical code/config defaults, traces/logs scope, reserved metrics endpoint, lifecycle/failure behavior, and operator-visible configuration semantics without claiming that OTEL is configurable or active when no valid credential exists.
  - Validate: Inspect the synchronized observability, auth/telemetry-credential, auth DB, config-schema/config-precedence, architecture, overview, glossary, and context-map documents against the implemented runtime and run the repository context/file checks in the full validation suite.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml observability`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth`
- Inspect the managed OTLP endpoint, bearer-token, redirect, config-parity, and output-parity integration evidence without contacting the production collector.

### Context sync

- `.sce/config.json`
- `config/pkl/base/sce-config-schema.pkl`
- `cli/src/services/config/schema.rs`
- `cli/src/services/config/resolver.rs`
- `cli/src/services/config/render.rs`
- `context/sce/cli-observability-contract.md`
- `context/sce/telemetry-credential.md`
- `context/sce/auth-db.md`
- `context/cli/config-precedence-contract.md`
- `context/cli/execution-glossary.md`
- `context/architecture.md`
- `context/overview.md`
- `context/glossary.md`
- `context/context-map.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`. A completed task must be `synced` before another task can start or the plan can finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition** beside the status. Never infer `synced` from conversation history; write every lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `.sce/config.json`; `config/pkl/base/sce-config-schema.pkl`; config DTO, resolver, rendering, and generated-schema parity; `cli/Cargo.toml` and lockfile; managed OTEL configuration/defaults; `cli/src/app.rs`; `cli/src/services/observability.rs` and focused observability submodules; `cli/src/services/telemetry_credential_storage.rs`; controlled OTLP receiver/test support; trace and log instrumentation boundaries; current-state context listed under Context sync.
- **Out of scope:** OTLP metrics production; telemetry credential issuance or refresh changes; Control Plane/server changes; a public telemetry command; repository-configurable collector selection; replacing local file/stderr logging; Agent Trace DB synchronization; arbitrary export of raw CLI arguments, paths, hook payloads, SQL, secrets, or full technical error chains.
- **Constraints:** Use the encrypted auth DB through the existing storage seam; use the bearer token only in managed OTLP request authentication; use OTLP/HTTP protobuf; pin compatible OpenTelemetry crates explicitly; hardcode the managed endpoint and signal-path defaults in Rust; keep the `.sce/config.json` `otel` block and each of its fields optional; do not make omission dependent on checked-in repo config; ignore ambient exporter endpoint/header/protocol/certificate overrides in managed mode; disable redirects; use future-aware subscriber attachment rather than a thread-local guard across `await`; keep exporter failures fail-open and bounded; run Cargo through the repository Nix wrapper.
- **Non-goal:** Defining or exporting metrics now. The `/v1/metrics` default is recorded so a later metrics implementation does not invent a competing endpoint contract.

## Assumptions

- Managed telemetry is automatic for every CLI invocation, including hook subprocesses, when a valid unexpired telemetry credential is available; hooks retain their existing fail-open behavior and exporter failures cannot change their decisions.
- The stored `TelemetryCredential::token` is the sole bearer secret sent to the collector. The stored `accessor` remains local metadata and is never sent unless a future server contract explicitly requires it.
- The canonical managed collector is pinned to `https://otel.crocoderlab.dev`; the standard `OTEL_EXPORTER_OTLP_*` names describe the built-in defaults but cannot redirect authenticated managed export. A separate user-selected standalone exporter mode is not part of this plan.
- The repo-local config schema accepts an optional `otel` object with optional `endpoint`, `protocol`, `traces_endpoint`, `metrics_endpoint`, and `logs_endpoint` keys. The code constants remain the source of defaults when the block or fields are absent; managed export still uses the canonical collector and does not allow repository or ambient configuration to redirect bearer-authenticated requests.
- The existing `sce auth login` best-effort provisioning and `sce auth logout` deletion behavior remain authoritative; exporter startup reads only an already-stored credential.

## Task stack

- [x] T01: `Define managed OTLP defaults, config surface, and credential-backed runtime selection` (status:done)
  - Task ID: T01
  - Scope: In — hardcode the exact OTLP base/protocol/traces/metrics/logs defaults in Rust; add an optional `otel` object with optional fields to the Pkl-authored config schema, typed config parsing/resolution/rendering, and repo-local config contract; preserve generated-schema parity; add the managed signal configuration seam and runtime mode resolution that loads one valid telemetry credential from the encrypted auth DB and otherwise selects `NoopTelemetry`. Add pure tests for optional config shape, code-default fallback, endpoint derivation, credential validity decisions, bearer-only credential projection, and no-issuance behavior. Out — exporter construction, span/log emission, metrics pipeline, Control Plane changes.
  - Dependencies: none
  - Done when: Code-owned defaults work with no `otel` config block, optional config parses and renders without becoming a required source of defaults, valid credentials select managed mode, invalid/unavailable credentials select no-op behavior, and no startup path mints or refreshes a credential.
  - Verify: `nix run .#pkl-check-generated`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry_credential`; targeted config-schema, config-resolution, managed-default, and runtime-mode tests.
  - Completed: 2026-10-14
  - Files changed: `config/pkl/base/sce-config-schema.pkl`, `cli/src/services/config/schema.rs`, `cli/src/services/config/resolver.rs`, `cli/src/services/config/render.rs`, `cli/src/services/mod.rs`, `cli/src/services/managed_telemetry.rs`, `context/plans/managed-otel-export.md`
  - Result: Added hardcoded managed OTLP defaults and pure credential-backed mode selection (`managed_telemetry`; only `token` projected as bearer, no issuance path), an optional `otel` object with optional fields in the Pkl schema, config parsing, layered resolution with code-default provenance, and `config show` text/JSON rendering. Metrics endpoint is reserved only; no exporter is built. `resolve_managed_telemetry_mode` is not yet wired into app startup (T02).
  - Verify results: `nix run .#pkl-check-generated` passed (142 files); `telemetry_credential` tests passed (18); `managed_telemetry` (4), `otel` (5), and `config::` (45) tests passed; `cargo clippy --all-targets -D warnings` clean via nix develop.
  - Context impact: important — config schema gains optional `otel` block and `config show` gains `otel.*` fields; managed defaults module added.
  - Context synchronization: synced

- [x] T02: `Implement managed OTLP trace export and command lifecycle spans` (status:done)
  - Task ID: T02
  - Scope: In — add the pinned OpenTelemetry trace/provider/exporter runtime, OTLP/HTTP protobuf `/v1/traces` transport, bearer authentication, stable SCE resource identity, allowlisted command spans, private future-aware subscriber dispatch, bounded batch/shutdown lifecycle, and app-runtime wiring after T01. Out — OTLP logs, metrics, public configuration, raw logger-event export, and production collector verification.
  - Dependencies: T01
  - Done when: Credential-backed non-hook command execution emits only approved trace data to the canonical traces endpoint, no-op execution remains behaviorally identical without a credential, and exporter activity is bounded and isolated from command/persistence critical paths.
  - Verify: Focused trace lifecycle/endpoint tests, controlled loopback OTLP trace receiver tests, and `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry`.
  - Completed: 2026-10-14
  - Files changed: `cli/Cargo.toml`, `cli/Cargo.lock`, `cli/src/app.rs`, `cli/src/services/managed_telemetry.rs`, `cli/src/services/managed_telemetry/trace_export.rs`, `context/plans/managed-otel-export.md`
  - Result: Added pinned OpenTelemetry 0.33.1 OTLP/HTTP protobuf trace export. `ManagedTraceTelemetry` wraps each command future with a future-aware dispatch and an allowlisted `sce.command` span (command name, outcome, error code; logger events filtered out). A pinned HTTP client sends only the stored token as bearer to the canonical traces endpoint with redirects disabled and bounded timeouts. `RuntimeTelemetry` is resolved after startup (skipped for hook invocations), and bounded shutdown runs after command output is rendered.
  - Verify results: `telemetry` filter tests passed (30, including loopback receiver bearer/allowlist, redirect, and unreachable-collector tests); `app::` tests passed (11); clippy `--all-targets -D warnings` clean; `cargo fmt` applied.
  - Context impact: important — managed trace export runtime, new OpenTelemetry dependencies, lifecycle ordering in app runtime.
  - Context synchronization: synced

- [x] T03: `Implement managed OTLP log export from the existing logger boundary` (status:done)
  - Task ID: T03
  - Scope: In — add the OTLP logs provider/exporter for `/v1/logs`, bridge approved SCE logger events into OpenTelemetry log records, preserve local file/stderr rendering and stable event semantics, apply the shared security/allowlist boundary, attach the bearer token only to the canonical logs endpoint, and integrate logs with the bounded lifecycle from T02. Out — metrics export, changing local log formats, exporting unrestricted tracing events or technical error chains.
  - Dependencies: T02
  - Done when: Managed executions export approved SCE log records to the canonical logs endpoint with correct severity/resource/auth fields, while local logging and command output are unchanged and sensitive/unbounded fields never enter the remote payload.
  - Verify: Controlled loopback OTLP log receiver tests, secret/path/argument rejection tests, local-log parity tests, and focused `observability`/`telemetry` Rust tests.
  - Completed: 2026-10-14
  - Files changed: `cli/Cargo.toml`, `cli/src/services/managed_telemetry.rs`, `cli/src/services/managed_telemetry/trace_export.rs`, `cli/src/services/managed_telemetry/log_export.rs`, `context/plans/managed-otel-export.md`
  - Result: Added OTLP/HTTP protobuf log export to the canonical `/v1/logs` endpoint. A `LogBridgeLayer` converts `target: "sce"` logger events (already gated by the local log level) into OTel log records with severity, event ID, and a body limited to safe messages; only allowlisted field keys (`command`, `component`, `failure_class`, `error_code`, `error_class`, `error_surface`, `user_error`) with path/secret-free bounded values are attached. Logs share the pinned bearer client, redirect refusal, bounded batch queue, and bounded shutdown with traces; local logging is untouched.
  - Verify results: `telemetry` tests passed (32, including loopback bearer/log mapping, secret/path/argument rejection, redirect, unreachable-collector); `observability` tests passed (20); `app::` tests passed (11); clippy `--all-targets -D warnings` clean; `cargo fmt` applied.
  - Context impact: important — managed runtime now exports logs; observability contract must record logs scope and allowlist.
  - Context synchronization: synced

- [x] T04: `Prove managed exporter security, failure isolation, and hook parity` (status:done)
  - Task ID: T04
  - Scope: In — add the end-to-end controlled receiver and security regressions for bearer headers, endpoint pinning, ignored ambient overrides, redirect refusal, absent/expired credentials, stalled/refused receivers, bounded shutdown, concurrent execution, lock isolation, and hook output/exit/permission parity. Out — production collector access, metrics, credential provisioning changes, and a separate standalone telemetry mode.
  - Dependencies: T03
  - Done when: All managed exporter failure and exfiltration cases are covered by deterministic local tests, and every failure mode preserves existing CLI and hook behavior.
  - Verify: Named integration-test filters for managed export, endpoint security, shutdown, and hook parity; no test contacts a non-loopback collector.
  - Completed: 2026-10-14
  - Files changed: `cli/src/app.rs`, `cli/src/services/managed_telemetry/trace_export.rs`, `context/plans/managed-otel-export.md`
  - Result: Added loopback-only regressions: ambient `OTEL_EXPORTER_OTLP_*` endpoint/header/protocol overrides cannot redirect export or inject headers (alternate receiver never contacted, bearer-only on pinned paths); stalled receiver leaves command latency unaffected and shutdown bounded; concurrent commands keep isolated results and spans; hook invocations (including with leading flags) never resolve managed telemetry and disallowed resolution yields `Noop`. Existing tests already cover bearer/no-accessor, redirect refusal, refused receiver, and absent/expired/blank credentials.
  - Verify results: `telemetry` filter tests passed (37); `app::` tests passed (13); clippy `--all-targets -D warnings` clean; `cargo fmt` applied. No test contacts a non-loopback collector.
  - Context impact: minor — test-only additions; no runtime behavior change.
  - Context synchronization: synced

- [x] T05: `Record the managed OTEL current-state and config contract` (status:done)
  - Task ID: T05
  - Scope: In — synchronize the observability, telemetry credential, auth DB, config schema/precedence, execution, architecture, overview, glossary, and context-map documents plus the optional repo-local config declaration with the implemented managed runtime, hardcoded defaults, signal scope, credential boundary, lifecycle guarantees, and metrics deferral. Out — rewriting historical plan records or presenting future metrics as implemented.
  - Dependencies: T04
  - Done when: Durable current-state context and the optional repo-local config contract consistently distinguish code-owned managed traces/logs defaults from the reserved metrics endpoint, name the encrypted auth DB bearer source, and match the final code and tests.
  - Verify: Context-owner inspection plus the focused repository context/file checks required by `/validate`.
  - Completed: 2026-10-14
  - Files changed: `context/sce/telemetry-credential.md`, `context/sce/auth-db.md`, `context/sce/cli-observability-contract.md`, `context/cli/execution-glossary.md`, `context/architecture.md`, `context/overview.md`, `context/glossary.md`, `context/context-map.md`, `context/plans/managed-otel-export.md`
  - Result: Current-state context now records the managed OTLP trace and log runtime, the encrypted-auth-DB bearer-token boundary (accessor never sent), code-owned canonical defaults with the optional `otel` config block, fail-open bounded lifecycle, hook `NoopTelemetry` behavior, and the reserved metrics endpoint. `config-precedence-contract.md` already carried the optional `otel` block contract from T01 and was verified unchanged; `.sce/config.json` intentionally keeps the optional `otel` block absent.
  - Verify results: Context-owner inspection against `managed_telemetry` code and plan T01-T04 records passed; touched files keep their prior size (pre-existing `architecture.md` length unchanged in scope); no stale "foundation only"/trace-only/`NoopTelemetry`-only statements remain in the edited files.
  - Context impact: important — documentation-only synchronization of the managed OTEL current state.
  - Context synchronization: synced

## Open questions

None. The plan implements automatic managed export for traces and logs, hardcodes the defaults in the CLI, keeps the `.sce/config.json` OTEL block optional, uses the encrypted auth DB's OpenBao bearer token, pins the collector, and reserves metrics for a later implementation.

## Validation Report

**Status:** validated  
**Date:** 2026-10-14

### Commands run

- `nix flake check` -> exit 0 (all checks passed; cli-tests, clippy, fmt green)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed, 142 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml observability` -> exit 0 (20 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml telemetry` -> exit 0 (37 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml auth` -> exit 0 (16 passed)
- Inspection of loopback integration evidence and context docs -> bearer, redirect, ambient-override, stalled/refused, and parity tests present and passing; docs match runtime

### Success-criteria verification

- [x] AC1: Managed defaults and optional `otel` block -> config/managed-default tests and flake check pass; `.sce/config.json` has no `otel` block; metrics endpoint reserved only
- [x] AC2: Credential-backed selection, bearer-only auth, no issuance -> telemetry/auth tests pass
- [x] AC3: Trace export to `/v1/traces` with output parity -> loopback trace tests pass under flake check
- [x] AC4: Log export mapping and redaction -> loopback log tests pass
- [x] AC5: Async-safe, fail-open, bounded lifecycle -> stalled/unreachable/concurrent tests pass
- [x] AC6: Endpoint pinning, ambient override and redirect refusal -> security tests pass
- [x] AC7: Current-state documentation -> context docs inspected against runtime; context checks pass

### Failed checks and follow-ups

- None.

### Residual risks

- Production client TLS root-store behavior on hosts lacking system CA certificates is unverified (sandbox tests now use an explicit empty root store).
