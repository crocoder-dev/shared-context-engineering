# Plan: sce-observability-end-to-end

## Change summary

Audit and complete observability coverage for the `sce` CLI runtime, hook command paths, and Agent Trace / diff-trace flows. The implementation should verify current behavior first, then fill concrete gaps so lifecycle logs, stable event IDs, config/file logging, optional OTEL export, and operator-facing verification surfaces work consistently across the scoped SCE runtime paths.

## Success criteria

- Current observability coverage is audited across CLI app startup/dispatch, `sce hooks` subcommands, Agent Trace DB setup/doctor/lifecycle integration, `diff-trace` intake, post-commit intersection, and the OpenCode plugin handoff into `sce hooks diff-trace`.
- Every major in-scope runtime path either emits stable observability events with deterministic `event_id` values or has an explicit documented reason for not emitting a separate event.
- Hook and Agent Trace / diff-trace success and failure paths include useful structured fields for operators and automation without leaking sensitive values.
- Existing stdout/stderr contracts remain intact: command payloads stay on stdout, diagnostics/logs stay on stderr and optional file sinks.
- Operators can verify observability through `sce config show`, `sce doctor`, configured log-file output, and optional OTEL export behavior.
- Tests cover the audit-backed observability contract for in-scope paths, including at least one hook/diff-trace path and one operator verification path.
- `nix run .#pkl-check-generated` and `nix flake check` pass before plan completion.

## Constraints and non-goals

- Scope is limited to the CLI runtime, hooks, and Agent Trace / diff-trace flows. Broad generated-agent behavior, release workflows, unrelated setup UX, and non-trace OpenCode plugin behavior are out of scope.
- Do not introduce a new observability backend, tracing framework, external service, or dependency unless a later human decision explicitly approves it.
- Preserve current config precedence and defaults from `cli/src/services/config/mod.rs` and `cli/src/services/observability.rs`.
- Preserve hook command behavior, Agent Trace DB schema semantics, artifact persistence, and post-commit intersection semantics except where observability-only instrumentation is required.
- Preserve stable CLI help text, exit codes, user-facing error classes, and machine-readable output fields unless directly required for the accepted observability contract.
- Do not log raw patch contents, commit-message contents, tokens, config secrets, or full user payloads.

## Assumptions

- “Everything mentioned” means all four acceptance signals named during clarification: existing observability compile/test health, stable event IDs for major paths, operator verification through `sce config show` and `sce doctor`, and verification through log-file output plus optional OTEL export.
- The OpenCode agent-trace plugin is included only at its handoff boundary into `sce hooks diff-trace`; broader OpenCode runtime observability is outside this plan.
- If T01 finds an already-complete path, the implementation task for that path should record the evidence and avoid unnecessary code churn.

## Task stack

- [x] T01: Audit in-scope observability coverage (status:done)
  - Task ID: T01
  - Goal: Produce a code-truth observability gap matrix for CLI startup/dispatch, hook subcommands, Agent Trace DB lifecycle/runtime paths, diff-trace intake, post-commit intersection, and OpenCode plugin handoff.
  - Boundaries (in/out of scope): In - inspect current code and tests; update this plan with an audit section or gap matrix; classify gaps as runtime event, operator verification, test coverage, or documentation/context gap. Out - runtime code changes, new event IDs, or behavior changes.
  - Done when: The plan records each in-scope path, current evidence, missing coverage if any, and the exact follow-up task that owns each gap; any already-complete paths have verification evidence instead of speculative work.
  - Verification notes (commands or checks): Inspect relevant files under `cli/src/app.rs`, `cli/src/services/observability*`, `cli/src/services/hooks*`, `cli/src/services/agent_trace_db*`, `config/lib/agent-trace-plugin/`, and existing tests; run narrow compile/check only if needed for audit confidence.
  - Completed: 2026-05-13
  - Files changed: `context/plans/sce-observability-end-to-end.md`
  - Evidence: Code-truth audit recorded in "T01 audit matrix" below; plan-only Markdown change, so no compile check was required.

- [x] T02: Freeze the in-scope observability event contract (status:done)
  - Task ID: T02
  - Goal: Update the current observability context contract with the audit-backed event taxonomy and operator verification expectations for hooks and Agent Trace / diff-trace paths.
  - Boundaries (in/out of scope): In - `context/sce/cli-observability-contract.md` and `context/context-map.md` updates if needed; define stable event IDs, required metadata fields, non-logged sensitive fields, and no-event justifications. Out - runtime code changes and broad generated-agent documentation.
  - Done when: Future implementation tasks have a concrete contract to target, including expected events for `diff-trace` intake and post-commit intersection, plus operator checks through config, doctor, logs, and OTEL.
  - Verification notes (commands or checks): Context review against T01 gap matrix; no shell command required unless the implementing agent chooses to run formatting/validation after context edits.
  - Completed: 2026-05-13
  - Files changed: `context/sce/cli-observability-contract.md`, `context/context-map.md`, `context/glossary.md`, `context/plans/sce-observability-end-to-end.md`
  - Evidence: Contract-only context update defining hook dispatch, no-op hook, commit-msg, diff-trace intake, post-commit intersection, Agent Trace DB boundary, operator verification, and sensitive-field exclusion expectations; context sync added the root glossary pointer for the event-contract term and reviewed shared root context.

- [ ] T03: Instrument hook command observability gaps (status:todo)
  - Task ID: T03
  - Goal: Add missing stable hook-level observability events and fields for `sce hooks` subcommand execution without changing hook behavior.
  - Boundaries (in/out of scope): In - hook dispatch/start/end/error events, subcommand names, attribution gate state where safe, post-commit summary counts, and deterministic redacted failure metadata. Out - modifying co-author policy, patch algorithms, database schema, or hook stdout messages except tests that prove they remain stable.
  - Done when: Each active or no-op hook path has either emitted contract events or a documented no-event rationale from T02, and existing hook behavior/output remains unchanged.
  - Verification notes (commands or checks): Targeted Rust tests for hook observability behavior where practical; prefer `nix flake check` for full validation when changes are complete.

- [ ] T04: Instrument Agent Trace and diff-trace persistence observability gaps (status:todo)
  - Task ID: T04
  - Goal: Add missing structured observability around Agent Trace DB initialization, diff-trace insert, recent-patch query/skip accounting, artifact persistence, and post-commit intersection persistence.
  - Boundaries (in/out of scope): In - safe counts, timings/window bounds, database path category or redacted path where existing policy allows, loaded/skipped row counts, and persistence failure classification. Out - schema changes, retry queues, artifact backfill, raw patch logging, or changing success/failure semantics.
  - Done when: Diff-trace and post-commit persistence success/failure paths provide enough observability to diagnose setup, DB, artifact, and malformed-row issues while preserving current command contracts.
  - Verification notes (commands or checks): Targeted unit/integration tests around Agent Trace DB helpers or hook paths; confirm no raw patch content appears in emitted logs.

- [ ] T05: Add operator-facing observability verification coverage (status:todo)
  - Task ID: T05
  - Goal: Ensure operators can verify the observability setup through `sce config show`, `sce doctor`, file logging, and optional OTEL bootstrap checks.
  - Boundaries (in/out of scope): In - tests or small behavior fixes for config provenance display, doctor Agent Trace DB visibility, log-file write mode behavior, and OTEL enablement validation. Out - new CLI commands, new config keys, external collector dependencies, or broad doctor layout redesign.
  - Done when: The configured observability values are visible through existing operator surfaces, doctor still reports Agent Trace DB health, log-file output can be verified in tests, and OTEL configuration errors/success bootstrap behavior remain deterministic.
  - Verification notes (commands or checks): Targeted app/config/doctor observability tests; `nix flake check` before marking complete if multiple surfaces are touched.

- [ ] T06: Add end-to-end hook/diff-trace observability tests (status:todo)
  - Task ID: T06
  - Goal: Add regression tests that exercise the full in-scope observability path from a diff-trace-producing handoff into `sce hooks diff-trace` and through post-commit intersection accounting where practical.
  - Boundaries (in/out of scope): In - temp repo/state tests, JSON or text log assertions for stable event IDs and safe metadata, preservation of stdout/stderr contracts, and plugin payload-shape handoff coverage if the existing plugin test harness supports it. Out - real OpenCode runtime execution, real OTEL collector integration, network calls, or non-deterministic timing assertions.
  - Done when: Tests fail if key hook/diff-trace observability events disappear, unsafe patch payload logging is introduced, or command output streams regress.
  - Verification notes (commands or checks): Narrow Rust/Bun tests for touched areas where available; then `nix flake check` for repo-level confidence.

- [ ] T07: Validate, cleanup, and sync context (status:todo)
  - Task ID: T07
  - Goal: Run final repo validation, remove temporary scaffolding, and sync durable context with the implemented observability state.
  - Boundaries (in/out of scope): In - `nix run .#pkl-check-generated`, `nix flake check`, cleanup of temporary test/log artifacts, and updates to current-state context files for any accepted observability contract changes. Out - new runtime features beyond gaps already covered by earlier tasks.
  - Done when: Full validation passes, context reflects code truth for CLI/hooks/Agent Trace observability, and this plan contains final evidence for success criteria.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect `context/tmp/` for accidental committed runtime artifacts.

## T01 audit matrix

| In-scope path | Current evidence | Current coverage | Gaps | Follow-up owner |
| --- | --- | --- | --- | --- |
| CLI app startup and dispatch | `cli/src/app.rs` resolves observability config before runtime initialization, builds `Logger`/`TelemetryRuntime`, emits `sce.app.start`, parses under `TelemetryRuntime::with_default_subscriber`, and delegates dispatch through `app_support::execute_command_phase`; `cli/src/services/app_support.rs` emits `sce.config.file_discovered`, `sce.config.invalid_config`, `sce.command.dispatch_start`, `sce.command.dispatch_end`, `sce.command.completed`, and classified `sce.error.{code}` records. | Runtime events and stdout/stderr separation are implemented for app lifecycle. `context/sce/cli-observability-contract.md` already lists the app-level event IDs and file/OTEL behavior. | Test coverage should explicitly assert configured file logging and OTEL bootstrap/operator verification behavior where practical. | T05 |
| Logger/file sink/OTEL runtime | `cli/src/services/observability.rs` renders deterministic text/JSON records with `event_id`, redacts emitted lines, mirrors to optional file sink, creates file parents, tightens Unix permissions, emits tracing events, and initializes OTLP gRPC or HTTP/protobuf exporters when enabled. `cli/src/services/observability/traits.rs` exposes injectable logger/telemetry traits and `NoopLogger`. | Runtime primitives are implemented and documented; file sink and OTEL are available to the app runtime. | End-to-end/operator tests should verify log-file output and deterministic OTEL configuration failure/success bootstrap boundaries without a real collector. | T05 |
| `sce config show` operator verification | `cli/src/services/config/mod.rs` formats `log_level`, `log_format`, `log_file`, `log_file_mode`, `otel.enabled`, `otel.exporter_otlp_endpoint`, and `otel.exporter_otlp_protocol` with provenance in text and JSON output. | Operator verification surface exists for resolved observability values. | Add/confirm tests that lock the observability provenance fields used by operators. | T05 |
| `sce doctor` / Agent Trace DB operator verification | `cli/src/services/doctor/mod.rs` aggregates lifecycle providers including Agent Trace DB; `cli/src/services/agent_trace_db/lifecycle.rs` diagnoses path resolution and DB path health, can bootstrap the DB parent in fix mode, and initializes `AgentTraceDb::new()` in setup. | Doctor/setup lifecycle coverage exists for Agent Trace DB health but does not emit separate observability events. | No runtime event gap for T03/T04 unless T02 explicitly requires lifecycle events; operator-facing Agent Trace DB visibility should be test-locked. | T05 |
| Hook command routing overall | `cli/src/services/hooks/mod.rs` routes `pre-commit`, `commit-msg`, `post-commit`, `post-rewrite`, and `diff-trace`. Only `diff-trace` receives `Option<&dyn Logger>`; other hook paths do not receive a logger and therefore emit no structured events. | Hook behavior/output is implemented, but hook-level observability is partial. | Define the hook event contract, then instrument start/end/error or no-event rationale for each subcommand without changing hook stdout. | T02 then T03 |
| `pre-commit` and `post-rewrite` no-op hooks | `run_pre_commit_subcommand` and `run_post_rewrite_subcommand` resolve hook runtime state and return deterministic no-op text. `post-rewrite` reads stdin before no-op handling. | No structured event is emitted; current rationale is attribution-only/no-op behavior. | T02 must decide whether no-op hooks need explicit events or documented no-event rationale; T03 implements any required events. | T02 then T03 |
| `commit-msg` attribution hook | `run_commit_msg_subcommand_in_repo` validates the message file, resolves attribution gate state, applies idempotent SCE co-author trailer only when enabled, and reports `policy_gate_passed`/`trailer_applied` in stdout. | Behavior has safe summary fields internally but no structured event, and raw commit-message content is not logged. | T02 should define safe fields; T03 should instrument gate state and trailer-applied outcome if required while preserving no raw message logging. | T02 then T03 |
| `diff-trace` intake | `run_diff_trace_subcommand` reads stdin, parses required `sessionID`/`diff`/`time`, persists pretty JSON to `context/tmp`, and writes to Agent Trace DB. It emits only `sce.hooks.diff_trace.error`, `sce.hooks.diff_trace.agent_trace_db_time_invalid`, and `sce.hooks.diff_trace.agent_trace_db_write_failed`; success paths have no event and current warn/error fields are empty or message-only. | Partial failure observability exists; success/count/path-category observability is missing. Raw patch payload is persisted as required but not logged. | T02 should define success/failure event IDs and safe metadata. T04 should add structured events for artifact persistence, DB insert success/failure, time conversion, and no raw patch logging. T06 should add regression tests. | T02, T04, T06 |
| Agent Trace DB insert/query/skip accounting | `cli/src/services/agent_trace_db/mod.rs` owns migrations, parameterized `insert_diff_trace`, `insert_post_commit_patch_intersection`, and `recent_diff_trace_patches`; malformed recent patch rows are skipped with reason strings and exposed through `RecentDiffTracePatches::{loaded_count, skipped_count}`. | Persistence/query behavior exposes counts to callers, but the DB adapter itself emits no observability events. | T04 should instrument at caller boundaries or adapter seams with safe counts/window bounds and failure classification, avoiding raw patch/session payload leakage. | T04 |
| Post-commit intersection | `run_post_commit_intersection_flow` opens Agent Trace DB, captures current commit patch from git, queries recent diff traces over a 7-day window, combines/intersects patches, persists the serialized intersection, and returns summary stdout with commit, loaded, skipped, and `intersection_files`. Unit coverage in `hooks` tests verifies query/persist window consistency, loaded/skipped counts, serialized intersection, and stdout. `run_post_commit_subcommand_with_trace` also writes a hook trace artifact in `context/tmp` containing git-derived input/outcome. | Behavioral accounting exists and has a targeted test, but structured observability events are missing. Existing trace artifact can include patch content and is not an observability log stream. | T02 should define safe post-commit event fields; T03/T04 should instrument dispatch/intersection/persistence outcomes without logging raw patches; T06 should add event-presence and stdout/stderr contract tests. | T02, T03, T04, T06 |
| OpenCode plugin handoff into `sce hooks diff-trace` | `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts` listens only to `session.diff`, extracts non-empty patch entries, builds `{ sessionID, diff, time: Date.now() }`, spawns `sce hooks diff-trace` in the repo root, writes JSON to stdin, ignores stdout, inherits stderr, and rejects on non-zero exit/signal. Generated plugin outputs mirror this source. | Handoff boundary exists and lets Rust hook runtime own persistence/logging. No plugin-local observability is expected beyond subprocess stderr inheritance. | T06 should add or confirm plugin payload-shape/handoff tests if a harness is available; broader OpenCode runtime observability remains out of scope. | T06 |
| Existing context contract | `context/sce/cli-observability-contract.md` documents app-level events, config/file/OTEL controls, and trait boundaries, but it does not yet include hook/diff-trace/Agent Trace event taxonomy. | App contract is current; hook/Agent Trace observability contract is incomplete. | Freeze audit-backed event taxonomy, safe fields, sensitive-field exclusions, and no-event rationales before runtime instrumentation. | T02 |

T01 classification summary:

- Runtime event gaps: hook subcommand start/end/error coverage, diff-trace success/failure metadata, Agent Trace DB persistence/query accounting, and post-commit intersection events (T02 -> T03/T04).
- Operator verification gaps: test-lock `sce config show`, `sce doctor` Agent Trace DB visibility, file logging, and OTEL bootstrap behavior (T05).
- Test coverage gaps: hook/diff-trace event regression, stdout/stderr preservation, unsafe payload non-logging, and plugin handoff payload shape where supported (T06).
- Documentation/context gaps: extend `context/sce/cli-observability-contract.md` with hook/Agent Trace event taxonomy and no-event rationales (T02).

## Open questions

None — clarification answers are incorporated above.
