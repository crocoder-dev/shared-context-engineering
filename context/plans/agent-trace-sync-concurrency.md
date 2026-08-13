# Plan: agent-trace-sync-concurrency

## Change summary

Refactor `sce trace sync` so its four independent Agent Trace ingestion streams (`messages`, `parts`, `diff_traces`, and `agent_traces`) execute as concurrent async state machines after one authoritative `/agent-trace/ingestion/state` request. The current synchronous `sync_stream` callbacks and nested `runtime.block_on(...)` calls serialize the HTTP requests; the change will move stream ingest and cursor-refresh callbacks to awaitable futures, keep cursor-dependent batches sequential within each stream, and use one outer runtime boundary around the overall sync operation.

Preserve the existing cursor validation, batching, `409` and ambiguous-failure reconciliation, terminal-error classification, progress/reporting, output, and no-local-sync-state behavior. Add refresh single-flight coordination so simultaneous expired-token requests share one refresh while ordinary valid-token requests remain concurrent. Extend the test HTTP helper with a concurrent, delayed-batch mode that records global and per-stream in-flight maxima, then add regressions proving four-way overlap and single-stream ordering.

## Acceptance criteria

- [x] AC1: One initial `/agent-trace/ingestion/state` response supplies all four starting cursors, and with one pending batch in each stream the test control plane observes `max_in_flight == 4` concurrent `/batch` requests.
  - Validate: `trace::sync` concurrency regression test using the concurrent delayed test server; assert exactly one state request and four overlapping batch requests.
- [x] AC2: Batches within an individual stream remain sequential and cursor-safe: a stream never has two batch requests in flight, and batch N+1 uses the cursor returned by batch N.
  - Validate: the delayed server's per-stream in-flight maximum and captured expected-cursor assertions; existing `agent_trace_sync::tests` cursor, batching, and reconciliation tests.
- [x] AC3: Existing reconciliation and terminal behavior remains unchanged for `409`, ambiguous transport/5xx/invalid-2xx outcomes, `/state` refreshes, terminal errors, invalid cursors, and reporting/output.
  - Validate: the existing `agent_trace_sync::` and `trace::sync::` test suites, including full/incremental, conflict, malformed-response, terminal-error, auth, cursor-validation, and progress tests, through the CLI flake test check.
- [x] AC4: Concurrent requests with an expired stored token cause at most one refresh operation for the shared client; all callers use the resulting token, while valid-token requests do not serialize behind refresh coordination unnecessarily.
  - Validate: control-plane concurrency tests with a delayed refresh response and a counting credential store assert one refresh/save and successful requests from all four callers; valid-token reuse tests continue to assert zero refresh/save calls.
- [x] AC5: No nested `runtime.block_on(...)` remains in per-stream ingest or reconciliation callbacks; the synchronous CLI boundary performs at most one `block_on` around the complete async sync operation.
  - Validate: source inspection of `cli/src/services/trace/sync.rs` plus the async orchestration tests.
- [x] AC6: Human text progress and final text/JSON output remain deterministic and contract-compatible despite concurrent stream completion.
  - Validate: existing progress/rendering tests and the full CLI test check; final stream rendering remains in the documented fixed stream order.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/cli/agent-trace-sync-command.md` — document concurrent stream execution, per-stream sequential ordering, and refresh single-flight behavior.
- `context/cli/trace-command.md` — update the sync behavior description so fixed ordering applies to reporting/contract order, not serial execution.
- `context/overview.md` and `context/architecture.md` — update the current Agent Trace sync data-flow/architecture claims where they describe the four streams as fixed-order execution.

## Constraints and non-goals

- **In scope:** async conversion of `cli/src/services/agent_trace_sync/mod.rs` and `cli/src/services/trace/sync.rs`; refresh coordination in `cli/src/services/agent_trace_sync/control_plane.rs`; concurrent test-server support and regression tests in the existing Agent Trace sync test modules; the listed current-state context updates.
- **Out of scope:** wire-protocol changes, local database schema or persistence changes, local sync cursors, control-plane server changes, unrelated CLI command or rendering redesign, and background/daemon synchronization.
- **Constraints:** fetch `/state` once before starting streams; preserve one in-flight batch per stream and server-cursor ordering within each stream; use the existing single-thread Tokio runtime without introducing OS threads for production concurrency; preserve public synchronous CLI behavior, authentication semantics, error classification, progress routing, and output shapes; keep dependencies unchanged unless the existing Tokio/reqwest APIs cannot provide the required seam.
- **Non-goal:** making batches from one stream concurrent, replacing authoritative `/state` reconciliation with local state, or redesigning authentication/token storage beyond a narrowly scoped refresh single-flight mechanism.

## Assumptions

- `tokio::join!` or an equivalent structured-concurrency combinator will run the four stream futures on the existing current-thread runtime; reqwest's asynchronous socket waits provide HTTP overlap without application-created OS threads.
- The outer progress-sink API remains synchronous. Internal stream progress delivery may use a small shared event-emission seam or equivalent coordination, but events must still be reported as batches complete and final reports must retain the documented stream order.
- Refresh single-flight coordination will guard only refresh-and-save operations and will re-check stored credentials after acquiring the guard, so normal requests holding a valid token can proceed concurrently and callers do not repeat a refresh another caller just completed.
- The existing sequential `TestHttpServer::start()` behavior may remain for order-sensitive legacy tests; a concurrent server mode or replacement helper will be used for overlap tests, with route-aware/dynamic batch responses where queued response order would otherwise be nondeterministic.

## Task stack

- [x] T01: `Refactor stream synchronization to async concurrent orchestration` (status:done)
  - Task ID: T01
  - Goal: Make `sync_stream` an async state machine with awaitable read, ingest, and cursor-refresh callbacks, make `sync_one_stream` async without nested runtime bridging, and run all four stream futures concurrently after the single initial `/state` call while preserving per-stream batch sequencing and existing reconciliation/reporting semantics.
  - Boundaries (in/out of scope): In — `cli/src/services/agent_trace_sync/mod.rs`, `cli/src/services/trace/sync.rs`, the outer synchronous runtime boundary, async-compatible progress/event coordination, and updates required to compile and preserve the existing engine/orchestration tests. Out — token refresh coordination, concurrent HTTP test-server implementation, wire changes, and database/persistence changes.
  - Dependencies: none
  - Done when: the initial state is awaited once; four stream futures are joined; each stream awaits its own next read/ingest/refresh before advancing; all existing reconciliation, cursor-validation, progress, and incremental-sync tests pass; no per-stream callback calls `runtime.block_on(...)`.
  - Implementation evidence: `sync_stream` is now an awaitable state machine; `run_sync_async` fetches `/state` once, joins four stream futures with per-stream sequential awaits, and emits completion reports in fixed stream order. `cli/src/services/trace/sync.rs` has one production `runtime.block_on(...)` around the complete async operation; batch ingestion and reconciliation callbacks are awaitable.
  - Verification notes (commands or checks): `nix build .#checks.x86_64-linux.cli-tests` passed; focused `agent_trace_sync` and `trace::sync::tests` suites passed via `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml ...`; `git diff --check` passed; source inspection confirmed the single production runtime bridge.

- [x] T02: `Coalesce concurrent access-token refreshes` (status:done)
  - Task ID: T02
  - Goal: Add a small client-owned single-flight refresh guard and double-check logic so concurrent expired-token resolution, and concurrent forced refreshes when applicable, do not issue duplicate WorkOS refresh/save operations while preserving valid-token request concurrency and the existing one-refresh/one-retry `401` contract.
  - Boundaries (in/out of scope): In — `AuthenticatedControlPlaneClient` refresh coordination, credential re-checking, and focused control-plane tests with delayed/counting fake credentials and refresh responses. Out — changing token formats, credential storage, WorkOS endpoints, ordinary request scheduling, or sync-stream logic.
  - Dependencies: T01
  - Done when: four concurrent client operations sharing an expired credential perform one refresh and one save, all complete with the refreshed access token, valid credentials still produce zero refresh/save calls, and existing 401 success/failure behavior remains unchanged.
  - Implementation evidence: `AuthenticatedControlPlaneClient` now owns a Tokio refresh mutex; expired-token resolution re-loads credentials after acquiring it, and unexpected-`401` refreshes reuse a token saved for the rejected access token instead of refreshing again. Focused concurrency coverage uses four coordinated expired-token callers and asserts one WorkOS refresh, one credential save, and four successful requests with the refreshed token.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_sync::control_plane::tests` passed (28 tests); `nix build .#checks.x86_64-linux.cli-tests` passed; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::trace::sync::tests -- --test-threads=1` passed (7 tests); `git diff --check` passed.

- [x] T03: `Add concurrent overlap and ordering regression coverage` (status:done)
  - Task ID: T03
  - Goal: Extend or replace the test HTTP helper with a concurrent connection mode that delays batch responses and tracks global/per-stream in-flight counts, then add end-to-end regressions for four-stream overlap and same-stream sequential batches while retaining the existing order-sensitive tests and update the affected durable sync documentation.
  - Boundaries (in/out of scope): In — `cli/src/services/agent_trace_sync/test_http_server.rs`, `cli/src/services/trace/sync.rs` tests and any focused control-plane tests needed for the helper, plus the listed context files; dynamic batch response validation based on request stream/cursor and deterministic overlap assertions. Out — production server behavior, new test dependencies, changes to wire payloads, and unrelated context cleanup.
  - Dependencies: T01, T02
  - Done when: a seeded one-row-per-stream test asserts one `/state` request and `max_in_flight == 4`; a multi-batch stream test asserts that stream's in-flight maximum is one and expected cursors advance batch-by-batch; the full existing sync/auth/reconciliation/progress/output suite remains green; current-state context describes concurrency accurately.
  - Implementation evidence: Added `ConcurrentBatchTestServer` with delayed, per-stream/cursor-routed responses, global and per-stream in-flight maxima, and captured expected cursors. Added end-to-end regressions proving one authoritative `/state` request, four-way batch overlap, and sequential same-stream cursor advancement. Preserved the sequential helper for existing order-sensitive tests and made terminal-failure request-count assertions tolerant of early structured-concurrency cancellation while still proving no refresh or resend.
  - Verification notes (commands or checks): `nix build .#checks.x86_64-linux.cli-tests` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed; focused `services::trace::sync::tests` (9 tests) and `services::agent_trace_sync` (37 tests) passed; `git diff --check` passed.

## Open questions

None. The request identifies the broken serialization mechanism, the required concurrency boundaries, authentication race behavior, test observables, and explicit non-goals. The current code confirms the premise: `sync_stream` and its callbacks are synchronous, `trace/sync.rs` calls `runtime.block_on(...)` per stream operation, and the existing test server handles connections serially; no clarification would materially change the scope or acceptance criteria.

## Validation Report

**Status:** validated  
**Date:** 2026-08-13

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (ephemeral generated-output parity passed)
- `nix flake check` -> exit 0 (all flake checks passed)
- `nix build .#checks.x86_64-linux.cli-tests` -> exit 0 (CLI test check passed)
- `nix develop -c sh -c 'set -e; ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::trace::sync::tests -- --test-threads=1; ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::agent_trace_sync -- --test-threads=1'` -> exit 0 (9 trace sync and 37 Agent Trace sync tests passed)
- `nix shell nixpkgs#ripgrep -c rg -n "runtime\\.block_on|block_on" cli/src/services/trace/sync.rs cli/src/services/agent_trace_sync/mod.rs; nix shell nixpkgs#ripgrep -c rg -n "run_sync_async|join!|join_all" cli/src/services/trace/sync.rs` -> exit 0 (one production runtime bridge; async joined orchestration confirmed)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: One initial `/agent-trace/ingestion/state` response supplies all four starting cursors, and with one pending batch in each stream the test control plane observes `max_in_flight == 4` concurrent `/batch` requests. -> `concurrent_sync_overlaps_all_four_stream_batches_after_one_state_request` passed; observed one state request and four-way overlap.
- [x] AC2: Batches within an individual stream remain sequential and cursor-safe: a stream never has two batch requests in flight, and batch N+1 uses the cursor returned by batch N. -> `concurrent_sync_keeps_batches_sequential_within_one_stream` passed; per-stream maximum was one and captured cursors advanced correctly. Existing cursor, batching, and reconciliation tests also passed.
- [x] AC3: Existing reconciliation and terminal behavior remains unchanged for `409`, ambiguous transport/5xx/invalid-2xx outcomes, `/state` refreshes, terminal errors, invalid cursors, and reporting/output. -> 37 Agent Trace sync and 9 trace sync tests passed, including reconciliation, classification, cursor, terminal, and output/progress coverage.
- [x] AC4: Concurrent requests with an expired stored token cause at most one refresh operation for the shared client; all callers use the resulting token, while valid-token requests do not serialize behind refresh coordination unnecessarily. -> `concurrent_expired_tokens_share_one_refresh_and_save` and valid-token reuse tests passed in the 37-test Agent Trace sync suite.
- [x] AC5: No nested `runtime.block_on(...)` remains in per-stream ingest or reconciliation callbacks; the synchronous CLI boundary performs at most one `block_on` around the complete async sync operation. -> Source inspection found one production `runtime.block_on` in `trace/sync.rs`; orchestration and async stream tests passed.
- [x] AC6: Human text progress and final text/JSON output remain deterministic and contract-compatible despite concurrent stream completion. -> Progress/final-report tests passed in the trace sync suite; fixed stream-order reporting is retained by `run_sync_async`.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.
