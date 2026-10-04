# Plan: agent-trace-db-write-contention

## Change summary

Fixes ingestion loss in the repository-scoped Agent Trace DB under multiprocess-WAL writer contention, found while validating PR #297. Today every Turso connection runs with the default `BusyHandler::None`. A contended `BEGIN IMMEDIATE` (or any write) returns `Busy` immediately, and the shared `run_with_retry_sync` query policy in `cli/src/services/db/mod.rs` (`QUERY_RETRY_POLICY`: 5 attempts, 25..100 ms deterministic backoff, no jitter) is the only thing that serializes writers. Real `sce hooks codex` processes lose distinct events with only 2–3 concurrent writers. Database integrity is unaffected; the failure is ingestion availability.

This plan implements this contention contract for the Agent Trace DB:

```
multiprocess WAL                  cross-process correctness and locking
        ↓
Turso busy_timeout                Turso waits for the lock while it reports Busy (busy_timeout_ms)
        ↓
Busy/BusySnapshot still returned?
        ↓
small jittered SCE outer retry    at most one outer retry by default
        ↓
contention deadline               decides whether another outer retry may start (contention_deadline_ms)
```

**The contention deadline bounds SCE retry scheduling, not the execution time of an already-running Turso operation.** SCE will not begin another contention retry once the configured contention deadline has expired. An individual Turso operation already in progress may complete after that deadline, so this is a retry-scheduling bound, not a hard wall-clock operation timeout. The investigation already saw successful operations take about 1.3 s, 2.8 s and 6.4 s. Turso 0.8.1's Rust binding exposes `Connection::busy_timeout(...)` but no public query-timeout or cancellation API, so this plan cannot enforce a hard wall-clock bound on a running operation and does not claim one.

The plan also makes the hook-runtime open read-only for repository metadata that is already initialized. That removes one unnecessary write from the hook fast path.

Two settings are added under the existing per-database `policies.database_retry` hierarchy: `busy_timeout_ms` and `contention_deadline_ms`. The existing `query.timeout_ms` semantics are unchanged, and generic `RetryPolicy.timeout_ms` cleanup is a separate follow-up. Other databases (`local_db`, `auth_db`) keep their current retry behavior.

**Branch and base:** PR #299 is currently stacked on `mutation-trace-health-invariant` (PR #297) so it can reuse the contention investigation and test stabilization work. Its base is `mutation-trace-health-invariant` and its head is `agent-trace-db-write-contention`. After #297 merges, rebase the branch onto the updated `main` before final merge if necessary.

The investigation suite `cli/src/services/agent_trace_db/lock_contention_tests.rs` was never committed and is no longer in the working tree. T01 restores it, wires it into the build, and records the baseline before the fix. It is then the acceptance gate.

## Acceptance criteria

- [ ] AC1: Every Agent Trace DB connection opened by SCE has Turso's busy handler set to the resolved `busy_timeout_ms` (default 500 ms), and `experimental_multiprocess_wal(true)` stays enabled on every local open path.
  - Validate: the T02 busy-timeout tests pass (`nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml busy_timeout`), and inspection of `cli/src/services/db/mod.rs` shows `experimental_multiprocess_wal(true)` and the `busy_timeout` call on every local `TursoDb` open path.
- [ ] AC2: Turso's busy handler covers the failing writer-lock acquisition. Turso 0.8.1's `Transaction::new_unchecked(conn, TransactionBehavior::Immediate)` runs `BEGIN IMMEDIATE` through `Connection::execute(...)` on the same connection. With connection A holding `BEGIN IMMEDIATE` for about 100 ms:
  - a control connection without a busy handler returns `Busy` promptly;
  - the production `insert_conversation_text_event` on a busy-timeout-configured Agent Trace DB connection waits and succeeds, with no test-level retry around the call.
  - Validate: the T02 direct busy-handler tests pass.
- [ ] AC3: The default Agent Trace DB outer retry is the initial attempt plus at most one retry, triggered only by a typed Turso `Busy`/`BusySnapshot`. Backoff is bounded full jitter. Deterministic errors fail after exactly one attempt with no sleep. A retry starts only when `remaining_deadline >= jittered_backoff + busy_timeout`, and never once the contention deadline has expired. A ~100 ms lock hold succeeds with `attempts = 1` and `outer_retries = 0`.
  - Validate: the T04 unit tests (`... test --manifest-path cli/Cargo.toml agent_trace_db_contention_retry`) pass. They cover:
    - a seeded jitter seam;
    - retry-start rule boundary cases (remaining just above, equal to, and just below `backoff + busy_timeout`, and an expired deadline);
    - Busy/BusySnapshot vs non-Busy classification;
    - the attempt cap;
    - the 100 ms-hold single-attempt assertion.
- [ ] AC4: When the contention policy is exhausted, the returned error and one structured `tracing` event `sce.agent_trace_db.contention_exhausted` carry `db_name`, `operation`, `attempts`, `busy_timeout_ms`, `contention_deadline_ms`, `elapsed_ms` and `cause`. The event goes only to the configured observability path (log file / stderr per logger config), never stdout. Hook fail-open behavior is unchanged.
  - Validate: the T05 test asserts the error text and the fields of the captured `tracing` event. Inspection confirms no new stdout writes on hook paths.
- [ ] AC5: Opening an already-initialized repository Agent Trace DB through the hook runtime issues zero write statements. The metadata guarantees still hold: repository-ID mismatch is an error, the source instance ID is stable and never overwritten, and concurrent first initialization converges on one ID. An initialized hook open succeeds while another connection holds `BEGIN IMMEDIATE`.
  - Validate: the T06 write-statement-count test passes with 0 writes; the converted hook-open-under-write-lock test passes; existing `verify_or_initialize_repository_metadata` tests pass.
- [ ] AC6: `insert_conversation_text_event` and the mutation-trace CAS batch keep their whole-transaction semantics. Message and part commit together. The first delivery returns `Ok(true)` and a duplicate replay returns `Ok(false)`. An injected mid-transaction failure leaves no orphans. Every retry restarts the whole unit from `BEGIN IMMEDIATE`, never an individual statement.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db` and `... mutation_trace` pass.
- [ ] AC7: The lock-budget boundary test characterizes busy timeout, outer retry and the contention retry-start deadline together. One connection holds `BEGIN IMMEDIATE` for 100, 250, 500, 750, 1000, 1500 or 2000 ms while another calls the real production API. Assertions use wide margins and are stated as outcome and retry policy, not wall-clock cutoffs:
  - holds ≤ 250 ms succeed;
  - holds ≥ 2000 ms exhaust the configured contention policy and fail cleanly;
  - every failure leaves 0 partial rows;
  - the middle holds are reported (outcome, latency, attempts), not asserted.
  - Validate: `... test --manifest-path cli/Cargo.toml lock_budget_boundary -- --nocapture` passes and prints per-hold outcome, latency and attempts.
- [ ] AC8: Under `SCE_LOCK_CONTENTION_STRICT=1` with N=2–4, the Rust production-API suite records 0 lock-exhaustion failures, 0 lost distinct events, 0 orphan rows and 0 duplicate persisted events. The required levels are distinct-event 2×1000, 3×1000, 4×500 and duplicate-delivery 2×500, 3×500, 4×500. 8 writers runs as non-strict stress/characterization only.
  - Validate: run these, reporting p50/p95/p99/max latency, outer retries and contention exhaustions (from test instrumentation), next to the T01 baseline:
    - `SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=2,3 SCE_LOCK_CONTENTION_ROUNDS=1000 nix develop -c ./scripts/run-cli-cargo.sh test --release --manifest-path cli/Cargo.toml concurrent_distinct_events -- --ignored --nocapture`;
    - the same with `WRITERS=4 ROUNDS=500`;
    - `concurrent_duplicate_delivery` with `WRITERS=2,3,4 ROUNDS=500`;
    - a non-strict `WRITERS=8` run.
- [ ] AC9: Under strict mode, real release `sce hooks codex` processes persist every distinct event for 2×500, 3×500 and 4×200, with 0 lock-exhaustion failures, 0 lost events, 0 orphans and 0 duplicates.
  - Validate: `nix build .#default`, then `SCE_BIN=$PWD/result/bin/sce SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=2,3 SCE_LOCK_CONTENTION_ROUNDS=500 nix develop -c ./scripts/run-cli-cargo.sh test --release --manifest-path cli/Cargo.toml concurrent_real_codex_hook_processes -- --ignored --nocapture`, and the same with `WRITERS=4 ROUNDS=200`. Record p50/p95/p99/max per-event latency next to the T01 baseline. Hook processes are separate processes, so in-process counters are unavailable; report persisted-row outcomes and latency only, and no retry counts unless they are derived from the hook log file.
- [ ] AC10: `policies.database_retry.agent_trace_db.busy_timeout_ms` and `policies.database_retry.agent_trace_db.contention_deadline_ms` are accepted, validated and documented:
  - both are non-negative integers with upper bounds;
  - `busy_timeout_ms = 0` disables the Turso busy handler;
  - `contention_deadline_ms = 0` means no outer retry is started;
  - both are rendered by `sce config show` in text and JSON and published in the generated config schema;
  - defaults (500 / 1250) apply when they are unset;
  - existing `query.timeout_ms` behavior and rendering are unchanged.
  - Validate: the T03 config tests (`... test --manifest-path cli/Cargo.toml database_retry`) pass, including an unchanged-`query.timeout_ms` regression assertion; `nix run .#pkl-check-generated` passes.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml resilience`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml database_retry`
- The strict contention commands from AC8 and AC9, reported as before-vs-after measurements

### Context sync

- `context/sce/shared-turso-db.md`: clearly distinguish these layers, and replace the stale "5 attempts / ≤2_000 ms" Agent Trace default text:
  - **multiprocess WAL:** cross-process correctness and locking;
  - **`busy_timeout_ms`:** Turso's wait/retry policy for `Busy`, covering `BEGIN IMMEDIATE` via `Transaction::new_unchecked` → `Connection::execute`;
  - **outer contention retry:** SCE-level retry after Turso exhausts its wait;
  - **`contention_deadline_ms`:** cutoff for launching another outer retry, explicitly not a hard operation timeout;
  - **`query.timeout_ms`:** existing generic retry setting, unchanged.
- `context/sce/agent-trace-db.md`: the hook-runtime read-only metadata fast path, the contention contract (including the not-a-hard-timeout statement), and a summary of the measured contention evidence.
- `context/cli/config-precedence-contract.md`: the `busy_timeout_ms` and `contention_deadline_ms` keys, their validation, and the meaning of zero.
- `context/glossary.md`: "busy timeout" and "contention deadline" entries, if the glossary covers retry vocabulary.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/db/mod.rs` (connection open, Agent Trace query/transaction retry routing, test seams), `cli/src/services/resilience.rs` (only if a small deadline/jitter helper belongs there), `cli/src/services/agent_trace_db/{repository.rs,mod.rs,lock_contention_tests.rs}`, `cli/src/services/agent_trace_storage/mod.rs` (hook-runtime open path), `cli/src/services/config/{schema.rs,resolver.rs,render.rs}`, `config/pkl/base/sce-config-schema.pkl`, and the durable context listed above.
- **Out of scope:**
  - retry behavior of `local_db` and `auth_db`;
  - changing `query.timeout_ms` semantics, and generic `RetryPolicy.timeout_ms` redesign for all databases (follow-up);
  - Codex/Claude/OpenCode/Pi hook fail-open semantics;
  - any change to PR #297's health-invariant behavior;
  - production-wide retry/exhaustion metrics.
- **Constraints:**
  - keep `experimental_multiprocess_wal(true)`;
  - stay on Turso 0.8.1 and use the existing `rand` dependency for jitter (no new crates);
  - retries always wrap whole transaction units, restarting from `BEGIN IMMEDIATE`;
  - classify retryability only from typed `turso::Error` through the existing `is_retryable_turso_error`, never by `anyhow` string matching;
  - no hook diagnostics on stdout;
  - keep existing user-facing error prefixes stable;
  - run Cargo only through `scripts/run-cli-cargo.sh` under `nix develop`.
- **Non-goal:**
  - no spool, daemon, background worker, queue or custom cross-process lock;
  - no Turso replacement and no disabling of multiprocess WAL;
  - no indefinite hook waits;
  - no blind increases to retry counts or budgets to make tests pass;
  - no change to mutation-trace semantics;
  - no claim of a hard wall-clock operation timeout.

## Assumptions

- PR #299 (`agent-trace-db-write-contention`) is stacked on `mutation-trace-health-invariant` (PR #297). After #297 merges, the branch is rebased onto the updated `main` before final merge if necessary.
- `lock_contention_tests.rs` was never committed and is absent from the working tree. Three identical 781-line copies (sha256 prefix `e94912a71fc594af`) exist in Nix store flake-source snapshots, for example `/nix/store/knhb7i1sc5ymy3nkvvjgg09fgxamf28z-source/cli/src/services/agent_trace_db/lock_contention_tests.rs`. T01 restores it unchanged from there. It is not registered in `agent_trace_db/mod.rs` today, so T01 registers it as a `#[cfg(test)]` module.
- Initial defaults, to be tuned only from T07 evidence:
  - `busy_timeout_ms = 500`;
  - `contention_deadline_ms = 1250`;
  - outer `max_attempts = 2`;
  - full-jitter backoff `random(0..=100 ms)`.
- Retry-start rule: after a typed `Busy`/`BusySnapshot`, compute `backoff = jitter(0..=cap)` and `remaining = contention_deadline - (now - operation_start)`. Launch the next attempt only if attempts remain and `remaining >= backoff + busy_timeout`; otherwise fail. With the defaults, a second attempt is normally possible only if the first one returned within about 650–750 ms.
- Upper bounds: `busy_timeout_ms <= 10_000` and `contention_deadline_ms <= 30_000`.
- The new keys sit beside the existing `connection_open`/`query` keys as `policies.database_retry.<db>.{busy_timeout_ms,contention_deadline_ms}`, consistent with the current per-DB schema. Only `agent_trace_db` gets non-zero defaults. `local_db`/`auth_db` either reject the keys or treat them as disabled; T03 picks whichever the per-DB schema supports more cleanly, and their runtime behavior is unchanged either way.
- The outer `max_attempts` and backoff cap for the Agent Trace contention retry are fixed constants in this plan, not new config keys. `query.*` overrides keep their current meaning for the generic retry path.
- Production observability is the structured `tracing` event `sce.agent_trace_db.contention_exhausted` through the existing logger. It is available only where logging is configured (log file / stderr), and the plan does not claim it is otherwise user-visible. Process-local atomic counters (attempts, outer retries, exhaustions) are test instrumentation only. They are meaningful inside the in-process Rust suite, not across hook processes, and are not production-wide telemetry.
- Turso 0.8.1 does not expose how often or how long its busy handler waited, so no busy-handler wait count is reported. Measurements cover outer attempts and retries, contention exhaustions and latency only.
- The only existing statement-count seam is the `#[cfg(test)]` `count_read_statements`. T06 adds a parallel `count_write_statements` seam with the same thread-local pattern rather than asserting on SQL strings.
- `hook_open_metadata_write_exhausts_retry_budget_while_write_lock_is_held` characterizes the bug being fixed. T06 converts it into a positive test (an initialized hook open succeeds under a held write lock) instead of deleting it.

## Task stack

- [ ] T01: `Restore and wire the lock-contention suite and record the baseline` (status:todo)
  - Task ID: T01
  - Scope: In:
    - restore `cli/src/services/agent_trace_db/lock_contention_tests.rs` unchanged from the Nix store copy named in Assumptions;
    - register `#[cfg(test)] mod lock_contention_tests;` in `agent_trace_db/mod.rs`;
    - extend the suite's reporting with per-write p50/p95/p99/max latency for the Rust API and hook-process tests;
    - run the AC8/AC9 commands against current behavior and record the baseline numbers in this task's completion record.
    Out — any production code change; relaxing or removing any existing assertion.
  - Dependencies: none
  - Done when: the suite is tracked and compiles as part of the test build; the non-ignored tests pass on unfixed code; the baseline table (N, rounds, lost events, lock errors, orphans, duplicates, latency percentiles) is recorded for the Rust API and hook-process runs.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_contention`; the AC8/AC9 commands in non-strict mode to capture the baseline.
  - Context synchronization: pending

- [ ] T02: `Configure Turso busy timeout on Agent Trace DB connections` (status:todo)
  - Task ID: T02
  - Scope: In:
    - one named default constant (`AGENT_TRACE_DB_BUSY_TIMEOUT_MS = 500`) resolved per `DbSpec` through a single resolver;
    - call `connection.busy_timeout(...)` on the connection returned by `connect()` on every local `TursoDb` open path, so the same connection later runs `Transaction::new_unchecked(.., Immediate)` → `BEGIN IMMEDIATE`, keeping `experimental_multiprocess_wal(true)`;
    - zero leaves the handler unset;
    - a doc comment explaining the distinction between multiprocess WAL and busy timeout.
    Behavioral tests (Turso 0.8.1 has no busy-timeout getter):
    - (a) connection A holds `BEGIN IMMEDIATE` ~100 ms; a control connection with busy timeout 0 gets `Busy` promptly;
    - (b) under the same hold, a production `insert_conversation_text_event` on an Agent Trace DB connection waits and succeeds, with no test-level retry around the call and an elapsed time showing it waited for the holder.
    Out — config-file surface (T03); outer-retry redesign and attempt counters (T04); `local_db`/`auth_db` defaults.
  - Dependencies: T01
  - Done when: AC1 and AC2 hold; Agent Trace DB writes wait for transient lock contention inside Turso; the other DBs are unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml busy_timeout`; `... test --manifest-path cli/Cargo.toml agent_trace_db`.
  - Context synchronization: pending

- [ ] T03: `Expose busy_timeout_ms and contention_deadline_ms in database_retry config` (status:todo)
  - Task ID: T03
  - Scope: In:
    - add `busy_timeout_ms` and `contention_deadline_ms` to the per-DB `database_retry` config document and resolved config;
    - validate them (non-negative integers, upper bounds 10_000 / 30_000, documented zero semantics) with stable error text;
    - feed `busy_timeout_ms` into the T02 resolver;
    - put the `contention_deadline_ms` default constant (`1250`) in one place for T04 to consume;
    - render both in `sce config show` text and JSON;
    - add both to `config/pkl/base/sce-config-schema.pkl`;
    - resolver/schema/render tests, including a regression test that `query.timeout_ms` parsing and rendering are unchanged.
    Out — any change to `query.timeout_ms`/`connection_open` semantics; new top-level config keys; outer-retry behavior.
  - Dependencies: T02
  - Done when: AC10 holds; configured values override the defaults for Agent Trace DB connections; invalid values are rejected.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml database_retry`; `nix run .#pkl-check-generated`.
  - Context synchronization: pending

- [ ] T04: `Replace Agent Trace DB query retry with jittered deadline-scheduled outer retry` (status:todo)
  - Task ID: T04
  - Scope: In — an Agent Trace DB-specific contention retry seam used by `execute`/`query`/`query_values`/`query_map`/`passive_checkpoint` and both transactional primitives when `M::db_config_key() == "agent_trace_db"`:
    - default `max_attempts = 2`;
    - full-jitter backoff `random(0..=cap)` behind an injectable jitter source (seeded/fixed in tests);
    - the retry-start rule from Assumptions (`remaining >= backoff + busy_timeout`, never after deadline expiry), measured from operation start so time spent in Turso's busy wait counts;
    - only typed `Busy`/`BusySnapshot` retried via `is_retryable_turso_error`, with single-statement wrappers keeping the typed `turso::Error` long enough to classify before converting to `anyhow`;
    - deterministic errors fail once with no sleep;
    - transactional primitives keep retrying the whole `BEGIN IMMEDIATE` unit;
    - `#[cfg(test)]`-oriented process-local counters for attempts, outer retries and exhaustions.
    Unit tests (prefix `agent_trace_db_contention_retry`) for each property, the retry-start boundary cases, and the ~100 ms-hold case asserting `attempts = 1`, `outer_retries = 0`. Doc comments state that the deadline schedules retries and does not interrupt a running operation. Out — `local_db`/`auth_db` retry behavior; generic `run_with_retry_sync` semantics; the exhaustion error/event shape (T05).
  - Dependencies: T03
  - Done when: AC3 holds; the default never yields more than 2 attempts and never starts an attempt in violation of the retry-start rule; existing `agent_trace_db` and `mutation_trace` tests pass unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db_contention_retry`; `... agent_trace_db`; `... mutation_trace`; `... resilience`.
  - Context synchronization: pending

- [ ] T05: `Make exhausted Agent Trace DB contention failures observable` (status:todo)
  - Task ID: T05
  - Scope: In — when the T04 policy is exhausted:
    - return an error carrying `db_name`, `operation`, `attempts`, `busy_timeout_ms`, `contention_deadline_ms`, `elapsed_ms` and `cause` (database busy / busy timeout exhausted), worded so the deadline is described as a retry-scheduling cutoff;
    - emit one structured `tracing::warn!` event `sce.agent_trace_db.contention_exhausted` with the same fields through the existing logger only (never stdout);
    - increment the test exhaustion counter.
    Add a test asserting the error fields and the captured event. Out — changing hook fail-open behavior; a metrics system; claims of user visibility when logging is not configured.
  - Dependencies: T04
  - Done when: AC4 holds; the stdout of the hook commands is unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db_contention_exhausted`; `... test --manifest-path cli/Cargo.toml hooks`.
  - Context synchronization: pending

- [ ] T06: `Make hook-runtime repository metadata open read-only when initialized` (status:todo)
  - Task ID: T06
  - Scope: In:
    - restructure `verify_or_initialize_repository_metadata` to `SELECT` first: when the row exists with a matching `repository_id` and a valid `source_instance_id`, return with no write; a repository-ID mismatch stays an error;
    - only a missing row or an empty/invalid `source_instance_id` runs the existing `INSERT … ON CONFLICT DO NOTHING` / atomic-claim `UPDATE … WHERE source_instance_id = ''` path, followed by a re-read;
    - add a `#[cfg(test)]` `count_write_statements` seam in `db/mod.rs` that mirrors `count_read_statements`.
    Tests:
    - an initialized hook-runtime open issues 0 writes;
    - a mismatch still errors;
    - an existing valid ID is never overwritten;
    - concurrent first opens converge;
    - `hook_open_metadata_write_exhausts_retry_budget_while_write_lock_is_held` is converted into a test that an initialized hook open succeeds while another connection holds `BEGIN IMMEDIATE`.
    Out — schema/migration changes; setup-path behavior beyond what the shared method changes.
  - Dependencies: T01
  - Done when: AC5 holds; all existing metadata tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml repository_metadata`; `... agent_trace_db`; `... agent_trace_storage`.
  - Context synchronization: pending

- [ ] T07: `Recalibrate the lock-budget boundary test to the new contention contract` (status:todo)
  - Task ID: T07
  - Scope: In:
    - set `LOCK_HOLD_DURATIONS_MS` to 100/250/500/750/1000/1500/2000 and align the budget constants with the T03/T04 contract;
    - assert by outcome and retry policy with wide margins: holds ≤ 250 ms succeed, holds ≥ 2000 ms exhaust the contention policy and fail cleanly, and every failure leaves zero message/part rows; middle holds are characterization only;
    - make the strict in-process concurrent tests report outer retries and contention exhaustions from the T04/T05 test counters next to the latency percentiles;
    - run the full AC8/AC9 strict matrix and record the after-fix measurements next to the T01 baseline in this task's completion record;
    - tune the defaults only if the evidence requires it, recording any tuning and its reason.
    Out — weakening, skipping or deleting any strict assertion; wall-clock cutoff assertions that assume a running operation is interrupted; raising attempts or budgets beyond what the measurements justify.
  - Dependencies: T03, T04, T05, T06
  - Done when: AC7 holds deterministically; the strict N=2–4 matrix records zero lock-exhaustion failures, lost events, orphans and duplicates; before-vs-after measurements (p50, p95, p99, max, outer retries and exhaustions where measurable, plus the 8-writer stress run) are recorded.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_budget_boundary -- --nocapture`; the AC8 and AC9 strict commands.
  - Context synchronization: pending

## Open questions

- PR #297's branch carries test-only stabilizers for this same contention (`c6cbc94a` retries locked SQLite writes in concurrent repository tests; `94dd0937` stabilizes convergence checks). Once this fix lands, those test-side retries may be redundant and could mask a regression. Should they be revisited in a follow-up after both PRs merge? This plan leaves them alone.
- Follow-up, not in this plan: generic `RetryPolicy.timeout_ms` is still documented and rendered as a per-attempt timeout for every database, even though `run_with_retry_sync` only checks elapsed time after a synchronous call returns. Should the generic cleanup become its own plan?
