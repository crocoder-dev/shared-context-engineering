# Plan: agent-trace-sync-state-timeout

## Change summary

`sce trace sync`'s `POST /agent-trace/ingestion/state` call currently runs
through `resilience::run_with_retry` with `STATE_RETRY_MAX_ATTEMPTS = 3` and a
10-second per-attempt timeout (`STATE_RETRY_TIMEOUT_MS`), defined in
`cli/src/services/agent_trace_sync/control_plane.rs`. This is the exact call
failing in the field: `Operation 'agent_trace_sync.ingestion_state' failed
after 3 attempt(s) (timeout=10000ms, backoff=250..2000ms)`.

This plan removes the retry loop for that call (a single attempt instead of
up to three) and raises its per-attempt timeout from 10s to 60s. Both values
are already externalized as named constants consumed by one `RetryPolicy`
literal, so this is a narrow constant change plus the test coverage needed to
prove the new single-attempt-at-60s behavior, not a new mechanism.

`POST /agent-trace/ingestion/batch` (`post_batch`) is untouched: it already
has no client-side timeout or retry wrapper today (batch reconciliation is
handled by the sync engine's own bounded `409`/ambiguous-failure logic, not
by this HTTP-layer policy), and this request is out of scope for the change.

## Acceptance criteria

- [x] AC1: A transient `5xx` response from `POST /agent-trace/ingestion/state` fails the call immediately after exactly one HTTP attempt instead of retrying up to three times.
  - Validate: `cargo test --manifest-path cli/Cargo.toml --lib agent_trace_sync::control_plane -- ingestion_state`
- [x] AC2: `ingestion_state`'s per-attempt timeout is 60 seconds (`STATE_RETRY_TIMEOUT_MS == 60_000`), and the existing 401-refresh-and-retry-once authentication path (a separate mechanism from this policy) is unaffected.
  - Validate: inspect `STATE_RETRY_MAX_ATTEMPTS`/`STATE_RETRY_TIMEOUT_MS` in `cli/src/services/agent_trace_sync/control_plane.rs`; `unexpected_401_refreshes_once_and_retries_once_on_success` and `unexpected_401_twice_fails_without_a_third_attempt` still pass unmodified.

### Full validation

- `nix flake check` (runs `cli-tests`, `cli-clippy`, `cli-fmt` among the repo's other checks)

### Context sync

- None. `context/cli/agent-trace-sync-command.md`'s recovery-semantics section describes `401`/`409`/ambiguous-batch-failure behavior qualitatively and names no specific attempt count or timeout value for `/state`, so it needs no edit for this change.

## Constraints and non-goals

- **In scope:** `STATE_RETRY_MAX_ATTEMPTS` and `STATE_RETRY_TIMEOUT_MS` in `cli/src/services/agent_trace_sync/control_plane.rs`, and the `control_plane.rs` unit tests needed to prove the new behavior.
- **Out of scope:** `post_batch`/`POST /agent-trace/ingestion/batch` (no existing client-side timeout to change); the `409`/ambiguous-failure reconciliation engine; the control-plane server (separate repository).
- **Constraints:** Reuse the existing `resilience::RetryPolicy`/`run_with_retry` mechanism as-is (`max_attempts: 1` still enforces the per-attempt timeout via `tokio::time::timeout`, per `resilience.rs`); do not introduce a new timeout/retry primitive for one call site.
- **Non-goal:** Making the retry/timeout values configurable (e.g. via `sce/config.json`'s `database_retry`-style namespace). Nothing in the request asked for that, and the existing `policies.database_retry` config namespace is scoped to local Turso databases, not control-plane HTTP calls.

## Assumptions

- "Remove retry attempt" means the transient-failure retry loop gated by `STATE_RETRY_MAX_ATTEMPTS` (currently 3, backed by exponential backoff) — not the unrelated, already-single-retry `401`-refresh-and-retry-once mechanism in `execute_authenticated`, which the user's error output never mentions and which this change leaves untouched.
- "Timeout... increased to 60 sec" means `STATE_RETRY_TIMEOUT_MS`, the only existing timeout knob on this call path (10,000 → 60,000).
- With `max_attempts = 1`, `STATE_RETRY_INITIAL_BACKOFF_MS`/`STATE_RETRY_MAX_BACKOFF_MS` become dead values (the backoff branch in `run_with_retry` never executes for a single-attempt policy). Left in place rather than removed: `RetryPolicy` requires all four fields, and `run_with_retry`'s failure message still reports the configured backoff window, matching the existing shape other `RetryPolicy` call sites use.

## Task stack

- [x] T01: `Make ingestion_state a single 60s attempt` (status:complete)
  - Task ID: T01
  - Goal: Change `STATE_RETRY_MAX_ATTEMPTS` to `1` and `STATE_RETRY_TIMEOUT_MS` to `60_000` in `cli/src/services/agent_trace_sync/control_plane.rs`, and add test coverage proving a transient `5xx` on `/agent-trace/ingestion/state` is no longer retried.
  - Boundaries (in/out of scope): In — the two constants, and one new (or adapted) unit test in `control_plane.rs`'s existing test module asserting single-attempt behavior on a transient failure (e.g. queue one `503` response and assert `server.call_count() == 1` and the call fails, mirroring the existing `TestHttpServer`/`CannedResponse` test scaffolding already used by the `401` tests in the same file). Out — `post_batch`, the sync engine's `409`/reconciliation logic, and any server-side change.
  - Dependencies: none
  - Done when: `STATE_RETRY_MAX_ATTEMPTS == 1` and `STATE_RETRY_TIMEOUT_MS == 60_000`; the new/adapted test demonstrates a transient failure fails after exactly one HTTP call; `unexpected_401_refreshes_once_and_retries_once_on_success` and `unexpected_401_twice_fails_without_a_third_attempt` still pass unchanged, showing the `401` path is independent of this policy.
  - Implementation evidence: Set the state retry policy to one 60-second attempt, updated its API documentation, and added `transient_state_failure_fails_after_one_http_attempt`, which queues a `503`, asserts failure, and verifies exactly one HTTP call.
  - Verification notes (commands or checks): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_sync::control_plane` passed (27 tests, including both unchanged 401 tests); `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml -- -D warnings` passed; `nix develop -c cargo fmt --manifest-path cli/Cargo.toml -- --check` passed.

## Open questions

This is a client-side mitigation, not a fix for the diagnosed root cause. Earlier in this session, the actual `ingestion_state` timeouts were traced to the control-plane server (`packages/agent-trace/src/batch-store.ts` in the `control-plane` repository): `POST /agent-trace/ingestion/batch` holds a SQLite write-lock transaction for 75-115+ seconds because it awaits one network round-trip per row inside the transaction instead of batching them, and `/state`'s own write (`findOrCreateSource`) blocks behind that lock. A 60-second single-attempt timeout still fails whenever a concurrent batch upload runs longer than 60s — which was observed up to 115s — and removing the retry removes the one behavior (backoff + reattempt) that occasionally let a request through once the lock released. If the batch-store fix (batching the per-row inserts server-side) is in scope soon, consider whether this client-side change is still worth doing now versus after that fix lands, since post-fix the 10s timeout would likely stop firing on its own. Proceeding as requested either way.

## Validation Report

**Status:** validated  
**Date:** 2026-08-13

### Commands run

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_sync::control_plane` -> exit 0 (27 control-plane tests passed, including both unchanged 401 tests and the transient state-failure test)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml --lib agent_trace_sync::control_plane -- ingestion_state` -> exit 101 (the package has no library target)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_sync::control_plane -- ingestion_state` -> exit 0 (27 control-plane tests passed)
- `nix flake check` -> exit 0 (all flake checks passed)

### Scaffolding removed

- None.

### Success-criteria verification

- [x] AC1: A transient `5xx` response from `POST /agent-trace/ingestion/state` fails the call immediately after exactly one HTTP attempt instead of retrying up to three times. -> `transient_state_failure_fails_after_one_http_attempt` passed and verified exactly one HTTP call.
- [x] AC2: `ingestion_state`'s per-attempt timeout is 60 seconds (`STATE_RETRY_TIMEOUT_MS == 60_000`), and the existing 401-refresh-and-retry-once authentication path (a separate mechanism from this policy) is unaffected. -> Inspection confirmed `STATE_RETRY_TIMEOUT_MS == 60_000` and `STATE_RETRY_MAX_ATTEMPTS == 1`; both named 401 tests passed unmodified.

### Failed checks and follow-ups

- None.

### Residual risks

- The 60-second client timeout remains vulnerable to control-plane batch transactions observed to exceed 60 seconds, as described in Open questions.
