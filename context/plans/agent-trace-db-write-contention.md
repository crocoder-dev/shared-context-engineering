# Plan: agent-trace-db-write-contention

## Change summary

Fixes ingestion loss in the repository-scoped Agent Trace DB under multiprocess-WAL writer contention, found while validating PR #297. Today every Turso connection runs with the default `BusyHandler::None`. A contended `BEGIN IMMEDIATE` (or any write) returns `Busy` immediately, and the shared `run_with_retry_sync` query policy in `cli/src/services/db/mod.rs` (`QUERY_RETRY_POLICY`: 5 attempts, 25..100 ms deterministic backoff, no jitter) is the only thing that serializes writers. The original investigation saw real `sce hooks codex` processes lose distinct events with 2–3 concurrent writers. The T01 baseline on the current host did not reproduce loss at N=2–4. It did reproduce loss deterministically when one writer holds `BEGIN IMMEDIATE` for ≥ ~300 ms, and under N=8 stress (1525 of 4000 distinct events lost). Database integrity is unaffected; the failure is ingestion availability.

This plan implements this contention contract for the Agent Trace DB:

```
multiprocess WAL                  cross-process correctness and locking
        ↓
Turso busy_timeout                Turso waits for the lock while it reports Busy (busy_timeout_ms)
        ↓
typed Busy/BusySnapshot still returned?
        ↓
small jittered SCE outer retry    write-capable operations only; at most one outer retry by default
        ↓
contention deadline               decides whether another outer retry may start (contention_deadline_ms)
```

The final implementation contract:

```
Agent Trace connection:
    multiprocess WAL + Turso busy_timeout

Agent Trace write-capable operation (whole retry unit known to be safe):
    Turso handles ordinary Busy waiting
    ↓
    if Busy/BusySnapshot escapes:
        at most one jittered outer retry, subject to contention_deadline_ms

Agent Trace read-only queries and everything else:
    existing SCE outer retry semantics, unchanged
```

**Turso's `busy_timeout` is connection-wide, so it naturally applies to any Turso statement on an Agent Trace DB connection that hits `Busy`. SCE's new outer retry policy is intentionally scoped to write-capable Agent Trace operations whose complete retry unit is known to be safe.** Read-only query APIs (`query`, `query_values`, `query_map`), `passive_checkpoint`, non-idempotent single-statement writes, and migrations keep their existing generic retry semantics in this PR.

**The contention deadline bounds SCE retry scheduling, not the execution time of an already-running Turso operation.** SCE will not begin another contention retry once the configured contention deadline has expired. An individual Turso operation already in progress may complete after that deadline, so this is a retry-scheduling bound, not a hard wall-clock operation timeout. The investigation already saw successful operations take about 1.3 s, 2.8 s and 6.4 s. Turso 0.8.1's Rust binding exposes `Connection::busy_timeout(...)` but no public query-timeout or cancellation API, so this plan cannot enforce a hard wall-clock bound on a running operation and does not claim one.

The plan also makes the hook-runtime open read-only for repository metadata that is already initialized. That removes one unnecessary write from the hook fast path.

Two Agent Trace-only settings are added: `policies.database_retry.agent_trace_db.busy_timeout_ms` and `policies.database_retry.agent_trace_db.contention_deadline_ms`. They are not accepted under `local_db` or `auth_db`; config validation rejects them there with the existing unknown-key error, so no ignored setting is silently accepted. The existing `query.timeout_ms` semantics are unchanged, and generic `RetryPolicy.timeout_ms` cleanup is a separate follow-up. `local_db` and `auth_db` keep their current retry configuration and behavior.

**Branch and base:** PR #299 is currently stacked on `mutation-trace-health-invariant` (PR #297) so it can reuse the contention investigation and test stabilization work. Its base is `mutation-trace-health-invariant` and its head is `agent-trace-db-write-contention`. After #297 merges, rebase the branch onto the updated `main` before final merge if necessary.

The investigation suite `cli/src/services/agent_trace_db/lock_contention_tests.rs` is version-controlled on this branch (committed by T01) and registered as a `#[cfg(test)]` module. It holds the pre-fix baseline and is the acceptance gate for later tasks.

**Pre-fix reproduction signals.** The N=2–4 concurrent suites are regression gates on this host, not a deterministic pre-fix reproduction. The deterministic held-lock boundary and the N=8 stress test are the pre-fix reproduction signals. The fix is not weakened because N=2–4 happened to pass on this machine before it.

## Acceptance criteria

- [x] AC1: Every Agent Trace DB connection opened by SCE has Turso's busy handler set to the resolved `busy_timeout_ms` (default 1000 ms; 500 ms before T08), and `experimental_multiprocess_wal(true)` stays enabled on every local open path.
  - Validate: the T02 busy-timeout tests pass (`nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml busy_timeout`), and inspection of `cli/src/services/db/mod.rs` shows `experimental_multiprocess_wal(true)` and the `busy_timeout` call on every local `TursoDb` open path.
- [x] AC2: Turso's busy handler covers the failing writer-lock acquisition. Turso 0.8.1's `Transaction::new_unchecked(conn, TransactionBehavior::Immediate)` runs `BEGIN IMMEDIATE` through `Connection::execute(...)` on the same connection. With connection A holding `BEGIN IMMEDIATE` for about 100 ms:
  - a control connection without a busy handler returns `Busy` promptly;
  - the production `insert_conversation_text_event` on a busy-timeout-configured Agent Trace DB connection waits and succeeds, with no test-level retry around the call.
  - Validate: the T02 direct busy-handler tests pass.
- [x] AC3: Agent Trace write-capable operations (the enumerated set in T04) use at most two outer attempts, retry only typed `Busy`/`BusySnapshot` failures, use bounded full jitter, and never start another outer attempt after the contention retry-start rule disallows it. A retry starts only when `remaining_deadline >= jittered_backoff + busy_timeout`, and never once the contention deadline has expired. Deterministic errors fail after exactly one attempt with no sleep. A ~100 ms lock hold succeeds with `attempts = 1` and `outer_retries = 0`. Read-only query APIs (`query`, `query_values`, `query_map`), `passive_checkpoint`, and Agent Trace writes outside the enumerated set retain their existing outer retry semantics.
  - Validate: the T04 unit tests (`... test --manifest-path cli/Cargo.toml agent_trace_db_write_contention_retry`) pass. They cover:
    - a seeded jitter seam;
    - retry-start rule boundary cases (remaining just above, equal to, and just below `backoff + busy_timeout`, and an expired deadline);
    - Busy/BusySnapshot vs non-Busy classification;
    - the attempt cap;
    - the 100 ms-hold single-attempt assertion;
    - a regression assertion that an Agent Trace `query`/`query_map` call still resolves the existing generic query retry policy.
- [x] AC4: When the contention policy is exhausted, the returned error carries `db_name`, `operation`, `attempts`, `busy_timeout_ms`, `contention_deadline_ms`, `elapsed_ms` and `cause`. Existing hook fail-open paths log that error through the configured SCE `Logger`. Log-file/stderr routing stays as it is today, stdout is unchanged, and hook fail-open behavior is unchanged.
  - The DB layer also emits one structured `tracing` event, `sce.agent_trace_db.contention_exhausted`, with the same contention fields.
  - This event is a telemetry instrumentation point. It reaches a sink only when a tracing subscriber is installed. Production currently runs with `NoopTelemetry`, and PR #299 does not add a production tracing subscriber.
  - Validate:
    - the T05 unit tests prove the error text and the shape of the structured `tracing` event, using a test-only capturing subscriber;
    - the existing hook/logger tests (`... test --manifest-path cli/Cargo.toml hooks`) and the existing hook fail-open `log.error(...)` paths show the returned error stays observable through the production `Logger` path;
    - inspection confirms no new stdout writes on hook paths.
- [x] AC5: Opening an already-initialized repository Agent Trace DB through the hook runtime issues zero write statements. The metadata guarantees still hold: repository-ID mismatch is an error, the source instance ID is stable and never overwritten, and concurrent first initialization converges on one ID. An initialized hook open succeeds while another connection holds `BEGIN IMMEDIATE`.
  - Validate: the T06 write-statement-count test passes with 0 writes; the converted hook-open-under-write-lock test passes; existing `verify_or_initialize_repository_metadata` tests pass.
- [x] AC6: `insert_conversation_text_event` and the mutation-trace CAS batch keep their whole-transaction semantics. Message and part commit together. The first delivery returns `Ok(true)` and a duplicate replay returns `Ok(false)`. An injected mid-transaction failure leaves no orphans. Every retry restarts the whole unit from `BEGIN IMMEDIATE`, never an individual statement.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db` and `... mutation_trace` pass.
- [x] AC7: The lock-budget boundary test characterizes busy timeout, outer retry and the contention retry-start deadline together. One connection holds `BEGIN IMMEDIATE` for 100, 250, 500, 750, 1000, 1500, 1750, 2000, 2250, 2500 or 3000 ms while another calls the real production API. Assertions use wide margins and are stated as outcome and retry policy, not wall-clock cutoffs:
  - holds ≤ 1000 ms succeed;
  - holds ≥ 3000 ms exhaust the configured contention policy and fail cleanly;
  - every failure leaves 0 partial rows, every sample makes at most 2 attempts, and `outer_retries + 1 == attempts`;
  - the middle holds (1500–2500 ms) are reported (outcome, latency, attempts, retry timeline), not asserted.
  - These thresholds come from the T08 10-run held-lock dataset for the 1000 / 2250 defaults. The T07 thresholds (≤ 250 ms succeed, ≥ 2000 ms exhaust) belonged to the superseded 500 / 1250 defaults.
  - Validate: `... test --manifest-path cli/Cargo.toml lock_budget_boundary -- --nocapture` passes and prints per-hold outcome, latency and attempts.
- [x] AC8: The Rust production-API suite (`insert_conversation_text_event`) passes the strict levels N=2–4: distinct-event 2×1000, 3×1000, 4×500, and duplicate-delivery 2×500, 3×500, 4×500. These levels are regression gates on the reference host; the T01 baseline already passes them before the fix. The test enforces:
  - always, per round: `messages == parts` (no orphan rows); the `Ok(true)` count equals the persisted message rows; distinct events never report `Ok(false)`; a duplicate-delivery round inserts at most once, and exactly once when at least one writer completed (no duplicate persisted events);
  - under `SCE_LOCK_CONTENTION_STRICT=1`, per writer level: 0 lock errors, 0 other errors and 0 lost distinct events.
  - 8 writers runs as non-strict stress/characterization only. If the fixed system makes N=8 reliable within acceptable latency, that result is reported, but N=8 is not a supported semantic requirement unless deliberately decided.
  - Validate: run these, reporting p50/p95/p99/max latency, outer retries and contention exhaustions (from test instrumentation), next to the T01 baseline:
    - `SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=2,3 SCE_LOCK_CONTENTION_ROUNDS=1000 nix develop -c ./scripts/run-cli-cargo.sh test --release --manifest-path cli/Cargo.toml concurrent_distinct_events -- --ignored --nocapture`;
    - the same with `WRITERS=4 ROUNDS=500`;
    - `concurrent_duplicate_delivery` with `WRITERS=2,3,4 ROUNDS=500`;
    - a non-strict `WRITERS=8` run.
- [x] AC9: Real release `sce hooks codex` `UserPromptSubmit` processes pass the strict levels 2×500, 3×500 and 4×200. These levels are regression gates on the reference host; the T01 baseline already passes them before the fix. `concurrent_real_codex_hook_processes_persist_every_distinct_event` enforces:
  - always, per round: `messages == parts` (no orphan rows);
  - always, per writer level: `persisted_messages <= expected` and `persisted_parts <= expected` (no over-persistence or duplicate events);
  - under `SCE_LOCK_CONTENTION_STRICT=1`, per writer level: `persisted_messages == expected`, `persisted_parts == expected`, and 0 non-zero hook exits;
  - under `SCE_LOCK_CONTENTION_STRICT=1`, across all levels: `total_lost == 0` and `total_nonzero_exits == 0`.
  - Hook stderr lines are recorded and reported but are not a strict invariant. Hooks fail open, and the contention fix may legitimately emit diagnostics through the configured logging path. "Lock-exhaustion failures" in hook processes show up as lost events, because the hook fails open; they are covered by the lost-event and persisted-count assertions.
  - Validate: `nix build .#default`, then `SCE_BIN=$PWD/result/bin/sce SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=2,3 SCE_LOCK_CONTENTION_ROUNDS=500 nix develop -c ./scripts/run-cli-cargo.sh test --release --manifest-path cli/Cargo.toml concurrent_real_codex_hook_processes -- --ignored --nocapture`, and the same with `WRITERS=4 ROUNDS=200`. Record p50/p95/p99/max per-event latency next to the T01 baseline. Hook processes are separate processes, so in-process counters are unavailable; report persisted-row outcomes and latency only, and no retry counts unless they are derived from the hook log file.
- [x] AC10: `busy_timeout_ms` and `contention_deadline_ms` are Agent Trace-only settings. They are accepted, validated and documented only under `policies.database_retry.agent_trace_db`:
  - `policies.database_retry.local_db` and `policies.database_retry.auth_db` keep their existing keys (`connection_open`, `query`) only, and reject `busy_timeout_ms` and `contention_deadline_ms` through the existing config-validation path. The generated-schema check runs first and fails with the existing stable `Config file '<path>' failed schema validation against generated schema '<schema>': …` error, naming the offending key. The Rust per-DB key check (`validate_object_keys`, allowed keys `connection_open, query` for `local_db`/`auth_db`) stays as a backstop with its existing `contains unknown key` wording;
  - the generated config schema publishes the two keys only on the `agent_trace_db` object; the `local_db`/`auth_db` objects keep `additionalProperties = false` with only `connection_open` and `query`;
  - both are non-negative integers with upper bounds;
  - `busy_timeout_ms = 0` disables the Turso busy handler;
  - `contention_deadline_ms = 0` means no outer retry is started;
  - both are rendered by `sce config show` in text and JSON and published in the generated config schema;
  - defaults (1000 / 2250; 500 / 1250 before T08) apply when they are unset;
  - existing `query.timeout_ms` behavior and rendering are unchanged.
  - Validate: the T03 config tests (`... test --manifest-path cli/Cargo.toml database_retry`) pass, including the `local_db`/`auth_db` rejection tests for both keys and an unchanged-`query.timeout_ms` regression assertion; `nix run .#pkl-check-generated` passes.

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
  - **outer write-contention retry:** SCE-level retry after Turso exhausts its wait, scoped to the enumerated write-capable Agent Trace operations; read-only queries keep the generic retry;
  - **`contention_deadline_ms`:** cutoff for launching another outer retry, explicitly not a hard operation timeout;
  - **`query.timeout_ms`:** existing generic retry setting, unchanged.
- `context/sce/agent-trace-db.md`: the hook-runtime read-only metadata fast path, the contention contract (including the not-a-hard-timeout statement), and a summary of the measured contention evidence.
- `context/cli/config-precedence-contract.md`: the Agent Trace-only `busy_timeout_ms` and `contention_deadline_ms` keys, their validation, the meaning of zero, and their rejection under `local_db`/`auth_db`.
- `context/glossary.md`: "busy timeout" and "contention deadline" entries, if the glossary covers retry vocabulary.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/db/mod.rs` (connection open, Agent Trace write-contention retry routing for transactional primitives and the opt-in idempotent write entrypoint, test seams), `cli/src/services/mutation_trace/store.rs` (only switching the three `…_IF_ABSENT` writes to the opt-in entrypoint), `cli/src/services/resilience.rs` (only if a small deadline/jitter helper belongs there), `cli/src/services/agent_trace_db/{repository.rs,mod.rs,lock_contention_tests.rs}`, `cli/src/services/agent_trace_storage/mod.rs` (hook-runtime open path), `cli/src/services/config/{schema.rs,resolver.rs,render.rs}`, `config/pkl/base/sce-config-schema.pkl`, and the durable context listed above.
- **Out of scope:**
  - retry behavior and config keys of `local_db` and `auth_db`;
  - outer retry semantics of Agent Trace read-only queries (`query`/`query_values`/`query_map`), `passive_checkpoint`, and non-idempotent Agent Trace writes;
  - changing `query.timeout_ms` semantics, and generic `RetryPolicy.timeout_ms` redesign for all databases (follow-up);
  - Codex/Claude/OpenCode/Pi hook fail-open semantics;
  - any change to PR #297's health-invariant behavior;
  - production-wide retry/exhaustion metrics.
- **Constraints:**
  - keep `experimental_multiprocess_wal(true)`;
  - stay on Turso 0.8.1 and use the existing `rand` dependency for jitter (no new crates);
  - retries always wrap whole transaction units, restarting from `BEGIN IMMEDIATE`; a single-statement write is retried as the complete statement, and only where replay is safe by its SQL semantics;
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
- Historical (T01, complete): T01 recovered the original investigation harness from a local Nix flake-source snapshot and committed it to the branch. `lock_contention_tests.rs` is now version-controlled and registered as a `#[cfg(test)]` module in `agent_trace_db/mod.rs`. No later task depends on a Nix store path.
- Initial defaults, to be tuned only from measured evidence:
  - `busy_timeout_ms = 500`;
  - `contention_deadline_ms = 1250`;
  - outer `max_attempts = 2`;
  - full-jitter backoff `random(0..=100 ms)`.
- Current defaults, tuned in T08 from the failed final validation run and the T08 measurement campaign:
  - `busy_timeout_ms = 1000`;
  - `contention_deadline_ms = 2250`;
  - outer `max_attempts = 2` (unchanged);
  - full-jitter backoff `random(0..=100 ms)` (unchanged).
- Retry-start rule: after a typed `Busy`/`BusySnapshot`, compute `backoff = jitter(0..=cap)` and `remaining = contention_deadline - (now - operation_start)`. Launch the next attempt only if attempts remain and `remaining >= backoff + busy_timeout`; otherwise fail. With the current defaults, a second attempt is possible only if the first one returned within about 1150–1250 ms (650–750 ms under the initial 500 / 1250 defaults).
- Upper bounds: `busy_timeout_ms <= 10_000` and `contention_deadline_ms <= 30_000`.
- Config contract (decided): `busy_timeout_ms` and `contention_deadline_ms` are Agent Trace DB contention settings, supported only as `policies.database_retry.agent_trace_db.{busy_timeout_ms,contention_deadline_ms}`, beside that object's existing `connection_open`/`query` keys:

  ```json
  {
    "policies": {
      "database_retry": {
        "agent_trace_db": {
          "busy_timeout_ms": 1000,
          "contention_deadline_ms": 2250
        }
      }
    }
  }
  ```

  `local_db` and `auth_db` keep their existing `connection_open`/`query` keys only and reject the two new keys; ignored configuration is never silently accepted. This specialization is feasible within the current architecture: the Pkl schema adds a dedicated `agentTraceDbRetrySchema` object (the shared `perDbRetrySchema` properties plus the two keys) used only for `agent_trace_db`, and the Rust `build_per_db` closure in `config/schema.rs` passes a per-DB allowed-key list to the existing `validate_object_keys`. If T03 finds this specialization disproportionately complex, it stops and records the blocker; it must not broaden the public config contract to `local_db`/`auth_db`.
- The outer `max_attempts` and backoff cap for the Agent Trace write-contention retry are fixed constants in this plan, not new config keys. `query.*` overrides keep their current meaning for the generic retry path, which remains the retry path for every Agent Trace operation outside the T04 write set.
- Write-contention retry scope (decided). Turso's `busy_timeout` is connection-wide; the SCE outer write-contention retry applies only to write-capable Agent Trace operations whose complete retry unit is known to be safe:
  - the transactional insert-pair primitive used by `insert_conversation_text_event`. Retry unit: `BEGIN IMMEDIATE` → check message → insert message → insert part → `COMMIT`;
  - the transactional mutation-trace CAS batch. Retry unit: `BEGIN IMMEDIATE` → CAS statements → `COMMIT`;
  - single-statement Agent Trace writes whose replay is safe by existing SQL semantics, invoked through an explicit opt-in write entrypoint (not by changing generic `execute`):
    - `INSERT_REPOSITORY_METADATA_SQL` (`ON CONFLICT (id) DO NOTHING`);
    - `CLAIM_SOURCE_INSTANCE_ID_SQL` (guarded `UPDATE … WHERE source_instance_id = ''`);
    - mutation-trace `INSERT_WORKTREE_IF_ABSENT_SQL`, `INSERT_SCOPE_IF_ABSENT_SQL` and `INSERT_SCOPE_PROVENANCE_IF_ABSENT_SQL` (`ON CONFLICT … DO NOTHING`).

  The repository-metadata writes keep the outer retry only on the initialization path that still writes after T06. A retry never re-runs an individual statement inside a transaction unit (for example only the second insert or only `COMMIT`); it restarts the whole unit from `BEGIN IMMEDIATE`.
- Explicitly unchanged in this PR (keep the existing generic `run_with_retry_sync` query policy):
  - read-only `query`, `query_values` and `query_map`, because there is no evidence the read paths need the write-contention policy;
  - `passive_checkpoint`. It is documented as never blocking on readers or writers, it is best-effort post-commit maintenance whose failure is already logged and swallowed (`sce.agent_trace_db.passive_checkpoint_failed`), and no investigation evidence shows it losing work to writer contention;
  - append-only single-statement writes with no conflict clause (`INSERT_DIFF_TRACE_SQL`, `INSERT_POST_COMMIT_PATCH_INTERSECTION_SQL`, `INSERT_AGENT_TRACE_SQL`, multi-row `insert_parts`), the last-writer `UPSERT_CLAUDE_MODEL_STATE_SQL`, the sync/export `insert_messages` batches, and migration statements. Widening the policy to them needs separate proof of replay safety.
- Production observability of contention exhaustion is the returned structured error. Existing hook fail-open handlers log it through the configured SCE `Logger` (log file / stderr per logger config), and the plan does not claim it is otherwise user-visible. The structured `tracing` event `sce.agent_trace_db.contention_exhausted` is kept as a telemetry instrumentation point for future telemetry work. Production runs with `NoopTelemetry` and has no tracing subscriber, so the raw event is not persisted during normal CLI execution. No `Logger` dependency is added to `TursoDb`, and no telemetry runtime or tracing-to-`Logger` bridge is added in this PR. Process-local atomic counters (attempts, outer retries, exhaustions) are test instrumentation only. They are meaningful inside the in-process Rust suite, not across hook processes, and are not production-wide telemetry.
- Turso 0.8.1 does not expose how often or how long its busy handler waited, so no busy-handler wait count is reported. Measurements cover outer attempts and retries, contention exhaustions and latency only.
- The only existing statement-count seam is the `#[cfg(test)]` `count_read_statements`. T06 adds a parallel `count_write_statements` seam with the same thread-local pattern rather than asserting on SQL strings.
- `hook_open_metadata_write_exhausts_retry_budget_while_write_lock_is_held` characterizes the bug being fixed. T06 converts it into a positive test (an initialized hook open succeeds under a held write lock) instead of deleting it.

## Task stack

- [x] T01: `Restore and wire the lock-contention suite and record the baseline` (status:done)
  - Task ID: T01
  - Scope: In:
    - restore `cli/src/services/agent_trace_db/lock_contention_tests.rs` unchanged from the investigation's local Nix flake-source snapshot (historical; done, and the file is now version-controlled);
    - register `#[cfg(test)] mod lock_contention_tests;` in `agent_trace_db/mod.rs`;
    - extend the suite's reporting with per-write p50/p95/p99/max latency for the Rust API and hook-process tests;
    - run the AC8/AC9 commands against current behavior and record the baseline numbers in this task's completion record.
    Out — any production code change; relaxing or removing any existing assertion.
  - Dependencies: none
  - Done when: the suite is tracked and compiles as part of the test build; the non-ignored tests pass on unfixed code; the baseline table (N, rounds, lost events, lock errors, orphans, duplicates, latency percentiles) is recorded for the Rust API and hook-process runs.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_contention`; the AC8/AC9 commands in non-strict mode to capture the baseline.
  - Completed: 2026-10-04
  - Files changed:
    - `cli/src/services/agent_trace_db/lock_contention_tests.rs` (restored verbatim from the Nix store copy, sha256 `e94912a71fc594af…`, then extended with latency reporting; staged in git so flake builds include it)
    - `cli/src/services/agent_trace_db/mod.rs` (`#[cfg(test)] mod lock_contention_tests;`)
    - `context/plans/agent-trace-db-write-contention.md` (this record)
  - Result:
    - The suite compiles in the test build and in `nix build .#default` / the flake clippy and fmt checks.
    - Added a nearest-rank `latency_percentiles` helper (with the unit test `lock_contention_latency_percentiles_use_nearest_rank`) and p50/p95/p99/max columns to the in-process level report and to the hook-process report.
    - Hook-process latency is now measured per process, from stdin release to process exit; each child is awaited on its own feeder thread.
    - No production code changed and no existing assertion was changed.
    - Deviation: the baseline did **not** reproduce ingestion loss at N=2–4, either in-process or with real hook processes, on this host (release builds, the plan's round counts). Loss reproduces at N=8: 1525 of 4000 distinct events lost. The single-writer lock-budget boundary confirms the root cause: any `BEGIN IMMEDIATE` hold of ≥300 ms exhausts `QUERY_RETRY_POLICY` after about 280 ms with no busy handler. The N=2–4 strict gates in AC8/AC9 already pass before the fix, so they act as regression gates; the 8-writer and boundary rows are the before-fix signal.
  - Baseline (before fix, release build, non-strict, 2026-10-04, HEAD `5f77cf2d` + T01 test changes):
    - Lock-budget boundary (debug build, single writer vs. a `BEGIN IMMEDIATE` holder):

      | hold ms | outcome | elapsed ms | rows (msg/part) |
      | --- | --- | --- | --- |
      | 50 | Ok(true) | 91 | 1/1 |
      | 100 | Ok(true) | 194 | 1/1 |
      | 200 | Ok(true) | 294 | 1/1 |
      | 300 | database is locked (5 attempts) | 279 | 0/0 |
      | 500 | database is locked (5 attempts) | 280 | 0/0 |
      | 1000 | database is locked (5 attempts) | 280 | 0/0 |

    - Hook-open metadata under a 500 ms write lock: the schema check passes, and the metadata `INSERT … ON CONFLICT DO NOTHING` fails with `database is locked` after 5 attempts (276 ms).
    - Rust production API (`insert_conversation_text_event`, per-write latency):

      | mode | N | rounds | lost events | lock errors | orphans | duplicates | p50 ms | p95 ms | p99 ms | max ms |
      | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
      | distinct | 2 | 1000 | 0 | 0 | 0 | 0 | 37 | 107 | 138 | 228 |
      | distinct | 3 | 1000 | 0 | 0 | 0 | 0 | 68 | 207 | 231 | 372 |
      | distinct | 4 | 500 | 0 | 0 | 0 | 0 | 87 | 212 | 318 | 384 |
      | duplicate | 2 | 500 | 0 | 0 | 0 | 0 | 20 | 33 | 34 | 39 |
      | duplicate | 3 | 500 | 0 | 0 | 0 | 0 | 44 | 135 | 207 | 2303 |
      | duplicate | 4 | 500 | 0 | 0 | 0 | 0 | 83 | 190 | 286 | 347 |
      | distinct (stress) | 8 | 500 | 1525 | 1525 (500/500 rounds) | 0 | 0 | 275 | 288 | 298 | 410 |
      | duplicate (stress) | 8 | 500 | 0 | 1545 (500/500 rounds) | 0 | 0 | 275 | 284 | 296 | 351 |

    - Real release `sce hooks codex` processes (`UserPromptSubmit`, distinct events, per-process latency):

      | N | rounds | expected | persisted | lost | orphans | non-zero exits | stderr lines | p50 ms | p95 ms | p99 ms | max ms |
      | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
      | 2 | 500 | 1000 | 1000 | 0 | 0 | 0 | 0 | 35 | 45 | 49 | 51 |
      | 3 | 500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 45 | 105 | 212 | 245 |
      | 4 | 200 | 800 | 800 | 0 | 0 | 0 | 0 | 50 | 196 | 197 | 231 |

    - Orphans and duplicates are enforced by the suite's per-round assertions (messages == parts; Ok(true) count == persisted rows; ≤1 insert per duplicate round). All runs passed them.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_contention`: passed (3 passed, 3 ignored).
    - AC8 commands (non-strict, release): `concurrent_distinct_events` with `WRITERS=2,3 ROUNDS=1000` and with `WRITERS=4 ROUNDS=500`; `concurrent_duplicate_delivery` with `WRITERS=2,3,4 ROUNDS=500`; `WRITERS=8 ROUNDS=500` for both. All ran and passed; results are in the table above.
    - AC9 commands (non-strict, release): `nix build .#default`, then `concurrent_real_codex_hook_processes` with `WRITERS=2` / `3` and `ROUNDS=500`, and with `WRITERS=4 ROUNDS=200`. All ran and passed; results are in the table above.
    - Additional: `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` passed.
  - Post-completion amendment (plan/harness correction before T02): the strict hook-process assertions now match AC9. Per level, `persisted_messages`/`persisted_parts <= expected` is always asserted. Under strict mode, both must equal `expected`, non-zero exits must be 0, and `total_lost == 0` and `total_nonzero_exits == 0` hold across all levels. The in-process strict check now also asserts `lost_events == 0` per level. Two unit tests cover the hook-level assertion helper (`lock_contention_hook_level_assertions_*`). No baseline number changed.
  - Context impact: none to durable context. This is a test-only change plus plan evidence. The baseline numbers feed the `context/sce/agent-trace-db.md` measured-evidence summary planned for later tasks.
  - Context synchronization: synced

- [x] T02: `Configure Turso busy timeout on Agent Trace DB connections` (status:done)
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
  - Completed: 2026-10-04
  - Files changed:
    - `cli/src/services/db/mod.rs`
    - `cli/src/services/agent_trace_db/lock_contention_tests.rs`
    - `context/plans/agent-trace-db-write-contention.md` (this record)
  - Result:
    - Added `AGENT_TRACE_DB_BUSY_TIMEOUT_MS = 500` and the single resolver `resolve_busy_timeout::<M>()`. It returns 500 ms for `agent_trace_db` and zero for every other `DbSpec`. T03 feeds config into this resolver.
    - `apply_busy_timeout` calls `Connection::busy_timeout(...)` on the connection returned by `connect()` inside `TursoDb::open_without_migrations_at`, the only local `TursoDb` open path (`new`, `new_at` and `open_without_migrations` all delegate to it). A zero timeout leaves the handler unset. `experimental_multiprocess_wal(true)` is unchanged.
    - The resolver's doc comment explains how multiprocess WAL differs from the busy timeout.
    - The encrypted `EncryptedTursoDb::new` path is `auth_db` only and does not use multiprocess WAL; it is untouched. `local_db` and `auth_db` resolve to zero, so their behavior is unchanged.
    - Tests:
      - `busy_timeout_resolves_default_for_agent_trace_db_and_zero_for_other_dbs`.
      - `busy_timeout_unset_connection_returns_busy_promptly_on_begin_immediate` (control): a raw `BEGIN IMMEDIATE` with no SCE retry gets `turso::Error::Busy` in under half the hold.
      - `busy_timeout_agent_trace_connection_waits_for_begin_immediate_holder`: a raw `BEGIN IMMEDIATE` on an Agent Trace connection waits for the holder and succeeds.
      - `busy_timeout_production_insert_waits_for_begin_immediate_holder`: under a 100 ms hold, `insert_conversation_text_event` succeeds with no test-level retry, writes 1/1 rows, and its elapsed time is at least half the hold.
    - Deviation: the two raw-connection tests hold the lock for 300 ms instead of ~100 ms. 300 ms is beyond the pre-fix generic retry budget, so the waiting test fails without the busy handler.
    - Deviation (needed to keep existing non-ignored tests valid): the T01 characterization tests assumed the ~280 ms pre-fix budget. With a 500 ms busy timeout under the unchanged generic 5-attempt query retry, the interim worst case is about 2.8 s. So `LOCK_HOLD_DURATIONS_MS` gains a 4 000 ms hold and `RELIABLY_BEYOND_RETRY_BUDGET_MS` becomes 4 000. `hook_open_metadata_write_exhausts_retry_budget_while_write_lock_is_held` therefore holds for 4 s. Both tests are still recalibrated or converted by T07/T06. No assertion was removed or weakened in kind.
    - Interim lock-budget boundary (debug build):

      | hold ms | outcome | elapsed ms | rows (msg/part) |
      | --- | --- | --- | --- |
      | 50 | Ok(true) | 70 | 1/1 |
      | 100 | Ok(true) | 119 | 1/1 |
      | 200 | Ok(true) | 244 | 1/1 |
      | 300 | Ok(true) | 344 | 1/1 |
      | 500 | Ok(true) | 521 | 1/1 |
      | 1000 | Ok(true) | 1043 | 1/1 |
      | 4000 | database is locked (5 attempts) | 2784 | 0/0 |

      Before the fix, 300, 500 and 1000 ms holds failed at about 280 ms.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml busy_timeout`: passed (4 passed).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db`: passed (49 passed, 3 ignored).
    - Additional: `... test --manifest-path cli/Cargo.toml lock_contention -- --nocapture` passed (6 passed, 3 ignored) and produced the table above. `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` passed.
  - Context impact: localized behavior change. Agent Trace DB connections now carry a 500 ms Turso busy timeout. The planned `context/sce/shared-turso-db.md` layering text (multiprocess WAL vs. `busy_timeout_ms`) applies; the config key and outer-retry layers arrive in T03/T04.
  - Context synchronization: synced

- [x] T03: `Expose busy_timeout_ms and contention_deadline_ms in database_retry config` (status:done)
  - Task ID: T03
  - Scope: In:
    - add `busy_timeout_ms` and `contention_deadline_ms` to the `agent_trace_db` database_retry config document and resolved config only (for example Agent Trace-only optional fields, so `local_db`/`auth_db` documents cannot carry them);
    - in `config/pkl/base/sce-config-schema.pkl`, add a dedicated `agentTraceDbRetrySchema` (the `perDbRetrySchema` `connection_open`/`query` properties plus the two keys, `additionalProperties = false`) used only for `["agent_trace_db"]`; `local_db`/`auth_db` keep `perDbRetrySchema` unchanged;
    - in `config/schema.rs` `build_per_db`, pass a per-DB allowed-key list to the existing `validate_object_keys`: `connection_open, busy_timeout_ms, contention_deadline_ms, query` for `agent_trace_db`, and `connection_open, query` for `local_db`/`auth_db`;
    - validate the values (non-negative integers, upper bounds 10_000 / 30_000, documented zero semantics) with stable error text;
    - feed `busy_timeout_ms` into the T02 resolver;
    - put the `contention_deadline_ms` default constant (`1250`) in one place for T04 to consume;
    - render both in `sce config show` text and JSON for `agent_trace_db` only;
    - resolver/schema/render tests:
      - `agent_trace_db` accepts both keys and overrides the defaults;
      - out-of-range and wrong-type values are rejected;
      - `local_db.busy_timeout_ms`, `local_db.contention_deadline_ms`, `auth_db.busy_timeout_ms` and `auth_db.contention_deadline_ms` are each rejected with the existing `failed schema validation` error naming the key;
      - the generated schema's `local_db`/`auth_db` objects list only `connection_open`/`query`;
      - `query.timeout_ms` parsing and rendering are unchanged (regression).
    Out — any change to `query.timeout_ms`/`connection_open` semantics; new top-level config keys; outer-retry behavior; any `local_db`/`auth_db` key additions. If per-DB specialization proves disproportionately complex, stop and record the blocker instead of broadening the contract.
  - Dependencies: T02
  - Done when: AC10 holds; configured values override the defaults for Agent Trace DB connections; invalid values are rejected; `local_db`/`auth_db` reject both keys.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml database_retry`; `nix run .#pkl-check-generated`.
  - Completed: 2026-10-04
  - Files changed:
    - `config/pkl/base/sce-config-schema.pkl`
    - `cli/src/services/config/types.rs`
    - `cli/src/services/config/schema.rs`
    - `cli/src/services/config/render.rs`
    - `cli/src/services/db/mod.rs`
    - `context/plans/agent-trace-db-write-contention.md` (this record)
  - Result:
    - Pkl: a new `agentTraceDbRetrySchema` (`connection_open`, `query`, `busy_timeout_ms` 0..=10000 default 500, `contention_deadline_ms` 0..=30000 default 1250, `additionalProperties = false`) is used only for `agent_trace_db`. `local_db`/`auth_db` keep `perDbRetrySchema` unchanged.
    - Types: `DatabaseRetryConfig.agent_trace_db` is now `AgentTraceDbRetryConfig { retry: PerDbRetryConfig, busy_timeout_ms, contention_deadline_ms }`. `PerDbRetryConfig` (used by `local_db`/`auth_db`) cannot carry the new keys. Upper-bound constants are `AGENT_TRACE_DB_BUSY_TIMEOUT_MAX_MS = 10_000` and `AGENT_TRACE_DB_CONTENTION_DEADLINE_MAX_MS = 30_000`.
    - Parsing: `map_database_retry_config` passes a per-DB allowed-key list to `validate_object_keys`: `connection_open, busy_timeout_ms, contention_deadline_ms, query` for `agent_trace_db`, and `connection_open, query` for `local_db`/`auth_db`. A Rust bounds backstop rejects values above the maximum with `Config key 'policies.database_retry.agent_trace_db.<key>' in '<path>' must be <= <max>.`. Generated-schema validation runs first and catches out-of-range, negative and wrong-type values with the existing `failed schema validation` error.
    - Resolution (`db/mod.rs`): `resolve_busy_timeout` now reads the configured `busy_timeout_ms` and falls back to 500; 0 leaves the busy handler unset. Added `AGENT_TRACE_DB_CONTENTION_DEADLINE_MS = 1_250` and `resolve_contention_deadline::<M>()` (zero for other DBs) for T04 to consume. It is `#[cfg_attr(not(test), allow(dead_code))]` until T04 wires it in. Pure `*_from_config` helpers make the override testable without the global `OnceLock`.
    - Rendering: `sce config show` text and JSON output include `busy_timeout_ms` / `contention_deadline_ms` for `agent_trace_db` when they are configured. Like the existing per-DB overrides, they are omitted when unset. `query`/`connection_open` rendering is unchanged.
    - Tests (all prefixed `database_retry`): accepts both keys; accepts 0 and the upper bounds; omitted keys stay unset; rejects out-of-range values; rejects wrong-type values; `local_db`/`auth_db` reject both keys with the `failed schema validation` error naming the key and path; the generated schema lists only `connection_open`/`query` for `local_db`/`auth_db`; the Rust per-DB key backstop keeps its `contains unknown key` wording; a regression test shows `query.timeout_ms` parsing is unchanged; resolver override, zero and default cases; JSON/text render for `agent_trace_db` only.
    - Assumption: the versioned release schema snapshots under `schema/v*/config.json` are written at release bump and are left untouched.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml database_retry`: passed (14 passed).
    - `nix run .#pkl-check-generated`: passed (ephemeral Pkl generation passed, 142 files).
    - Additional: `... test --manifest-path cli/Cargo.toml busy_timeout` passed (5 passed); `... test --manifest-path cli/Cargo.toml config` passed (116 passed); `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` passed.
  - Context impact: localized config-contract change. There are two new Agent Trace-only keys under `policies.database_retry.agent_trace_db`, and `local_db`/`auth_db` reject them. This affects `context/cli/config-precedence-contract.md` and `context/sce/shared-turso-db.md` (config layer for `busy_timeout_ms`/`contention_deadline_ms`). Outer-retry behavior is not changed yet (T04).
  - Context synchronization: synced

- [x] T04: `Add Agent Trace DB write-contention retry for safe write units` (status:done)
  - Task ID: T04
  - Scope: In — an Agent Trace DB write-contention retry seam, applied only when `M::db_config_key() == "agent_trace_db"` and only to the write-capable operations enumerated under "Write-contention retry scope" in Assumptions:
    - `execute_transactional_insert_pair_if_absent` (used by `insert_conversation_text_event`); the retry unit is the whole `BEGIN IMMEDIATE` → check message → insert message → insert part → `COMMIT` transaction;
    - `execute_transactional_cas_batch`; the retry unit is the whole `BEGIN IMMEDIATE` → CAS statements → `COMMIT` transaction;
    - a new explicit opt-in single-statement write entrypoint (for example `execute_idempotent_write`) that retries the complete statement. Only the enumerated replay-safe writes switch to it: repository-metadata `INSERT … ON CONFLICT DO NOTHING` and the guarded source-instance claim `UPDATE`, plus the mutation-trace `INSERT_WORKTREE_IF_ABSENT_SQL`, `INSERT_SCOPE_IF_ABSENT_SQL` and `INSERT_SCOPE_PROVENANCE_IF_ABSENT_SQL`. Generic `execute` keeps the generic retry for every other caller.

    Policy:
    - default `max_attempts = 2`;
    - full-jitter backoff `random(0..=cap)` behind an injectable jitter source (seeded/fixed in tests);
    - the retry-start rule from Assumptions (`remaining >= backoff + busy_timeout`, never after deadline expiry), measured from operation start so time spent in Turso's busy wait counts;
    - only typed `Busy`/`BusySnapshot` retried via `is_retryable_turso_error`, with the opt-in single-statement wrapper keeping the typed `turso::Error` long enough to classify before converting to `anyhow`;
    - deterministic errors fail once with no sleep;
    - a retry never re-runs an individual statement of a transaction unit (never only the second insert, never only `COMMIT`);
    - `#[cfg(test)]`-oriented process-local counters for attempts, outer retries and exhaustions.

    Doc comments state that Turso's `busy_timeout` is connection-wide while the outer write-contention retry is scoped to safe write units, and that the deadline schedules retries and does not interrupt a running operation.

    Unit tests (prefix `agent_trace_db_write_contention_retry`):
    - each policy property;
    - the retry-start boundary cases;
    - the ~100 ms-hold case asserting `attempts = 1` and `outer_retries = 0`;
    - a regression assertion that Agent Trace `query`/`query_map` still use the generic query retry policy.

    Out:
    - `local_db`/`auth_db` retry behavior;
    - generic `run_with_retry_sync` semantics;
    - read-only `query`/`query_values`/`query_map`;
    - `passive_checkpoint` (it stays on its existing behavior unless concrete writer/checkpoint-lock evidence appears, which would need its own justification and test);
    - append-only or last-writer single-statement writes;
    - the exhaustion error/event shape (T05).
  - Dependencies: T03
  - Done when: AC3 holds; the default never yields more than 2 attempts on the enumerated write operations and never starts an attempt in violation of the retry-start rule; reads and non-enumerated writes keep their existing retry policy; existing `agent_trace_db` and `mutation_trace` tests pass unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db_write_contention_retry`; `... agent_trace_db`; `... mutation_trace`; `... resilience`.
  - Completed: 2026-10-04
  - Files changed:
    - `cli/src/services/db/mod.rs`
    - `cli/src/services/agent_trace_db/repository.rs`
    - `cli/src/services/mutation_trace/store.rs`
    - `cli/src/services/agent_trace_db/lock_contention_tests.rs`
    - `context/plans/agent-trace-db-write-contention.md` (this record)
  - Result:
    - Policy: `WriteContentionPolicy` holds `max_attempts`, `backoff_cap`, `busy_timeout` and `contention_deadline`. `write_contention_policy::<M>()` returns `Some` only for `agent_trace_db`. It uses the fixed constants `AGENT_TRACE_DB_WRITE_CONTENTION_MAX_ATTEMPTS = 2` and `AGENT_TRACE_DB_WRITE_CONTENTION_BACKOFF_CAP_MS = 100`, plus the T02/T03 resolvers for `busy_timeout` and `contention_deadline`. `resolve_contention_deadline` is now live, and its `dead_code` allowances are removed.
    - `run_with_write_contention_retry` is the production wrapper. It uses `rand::thread_rng()` for full-jitter backoff `0..=cap`, `std::thread::sleep`, and a monotonic `std::time::Instant` started at the operation start. It delegates to the private `run_with_write_contention_retry_using`, which takes the backoff source, the sleep function and an elapsed-time clock (`FnMut() -> Duration`) as injected parameters. This seam is not public.
    - Retry admission is checked at two points, with `remaining = contention_deadline - elapsed` and `elapsed` measured from the operation start:
      1. Before the backoff sleep, `write_contention_retry_may_sleep` requires enough budget for the backoff plus a full busy-timeout wait: `remaining >= backoff + busy_timeout`.
      2. After the backoff sleep has actually returned, with elapsed time recomputed, `write_contention_retry_may_start_now` requires enough budget for a full busy-timeout wait: `remaining >= busy_timeout`. Equality still admits the attempt.
      
      Neither check admits anything once the deadline has expired, and failing either one exhausts the policy. Scheduler oversleep therefore cannot start a new attempt outside the admission contract. A running attempt is never interrupted.
    - `outer_retries` is incremented only after the post-sleep check succeeds, so it counts additional attempts actually admitted.
      - retries only `WriteAttemptFailure::Retryable`, which is typed `Busy`/`BusySnapshot` via `is_retryable_turso_error`; deterministic failures return after one attempt with no sleep.
    - `CasBatchFailure` is renamed `WriteAttemptFailure` and gains `into_error`.
    - Retry units:
      - `execute_insert_pair_if_absent_body` now classifies each typed `turso::Error` with `classify_turso_error`, keeping the existing message wording.
      - `execute_transactional_insert_pair_if_absent` and `execute_transactional_cas_batch` build one attempt closure that runs the whole `BEGIN IMMEDIATE` → `COMMIT` unit. They route it through the contention retry for Agent Trace; every other DB stays on the unchanged generic `run_with_retry_sync` path.
      - New `TursoDb::execute_idempotent_write` runs the complete statement under the contention policy on Agent Trace and delegates to `execute` everywhere else.
      - Only the five enumerated writes switch to `execute_idempotent_write`: `INSERT_REPOSITORY_METADATA_SQL` and `CLAIM_SOURCE_INSTANCE_ID_SQL` in `repository.rs`, and `INSERT_WORKTREE_IF_ABSENT_SQL`, `INSERT_SCOPE_IF_ABSENT_SQL` and `INSERT_SCOPE_PROVENANCE_IF_ABSENT_SQL` in `store.rs`.
    - Reads, `passive_checkpoint`, generic `execute` and migrations are unchanged.
    - Deviation (user request): the code carries no comments. The planned doc comments were removed, and the `busy_timeout`-scope and deadline-semantics statements live in `context/sce/shared-turso-db.md` instead.
    - Interim exhaustion error (T05 reshapes it): `Operation '<op>' failed after <n> attempt(s) under write contention (busy_timeout=…ms, contention_deadline=…ms, elapsed=…ms). Last error: <cause>. Try: <hint>`. The cause keeps Turso's `database is locked` text.
    - Test instrumentation: `#[cfg(test)]` `WriteContentionCounts { attempts, outer_retries, exhaustions }` read through `count_write_contention`.
    - Tests (prefix `agent_trace_db_write_contention_retry`):
      - seeded jitter is reproducible and stays within the cap;
      - retry-start boundaries: remaining just above, equal to and just below `backoff + busy_timeout`, plus an expired deadline and a zero deadline;
      - post-sleep admission boundaries: remaining just above, equal to and just below `busy_timeout`, plus an expired deadline;
      - an oversleep regression on a fake clock (`busy_timeout = 500`, `contention_deadline = 1250`, Busy at 690 ms, backoff 50 ms, sleep ends at 810 ms): attempt 2 never runs, giving `attempts = 1`, `outer_retries = 0`, `exhaustions = 1` and a contention-exhaustion error;
      - the adjacent case where the sleep ends at 750 ms, so post-sleep remaining equals `busy_timeout`: attempt 2 is admitted, giving `attempts = 2` and `outer_retries = 1`;
      - `Busy` and `BusySnapshot` are each retried once;
      - deterministic errors (`Constraint`, `Misuse`, `Readonly`) fail once and never sleep;
      - the attempt cap of 2 under persistent `Busy`;
      - no retry where the rule disallows one;
      - the policy applies only to `agent_trace_db`;
      - regression: Agent Trace `query`/`query_values`/`query_map`/`passive_checkpoint`/`execute` resolve `QUERY_RETRY_POLICY` and record zero contention attempts;
      - the 100 ms-hold production `insert_conversation_text_event` succeeds with `attempts = 1`, `outer_retries = 0`, `exhaustions = 0`.
    - Deviation: the counters are thread-local, following the `count_read_statements` pattern, not global atomics. Parallel tests cannot skew each other, and T07 writer threads can read their own counts.
    - Interim lock-budget boundary (debug build; T07 recalibrates it):

      | hold ms | outcome | elapsed ms | rows (msg/part) |
      | --- | --- | --- | --- |
      | 50 | Ok(true) | 81 | 1/1 |
      | 100 | Ok(true) | 128 | 1/1 |
      | 200 | Ok(true) | 260 | 1/1 |
      | 300 | Ok(true) | 350 | 1/1 |
      | 500 | Ok(true) | 533 | 1/1 |
      | 1000 | Ok(true) | 1081 | 1/1 |
      | 4000 | database is locked (2 attempts) | 1007 | 0/0 |

      The 4 000 ms hold now fails cleanly after 2 attempts in about 1.0 s; after T02 it took about 2.8 s and 5 attempts. The hook-open metadata write under a 4 000 ms hold now fails after 2 attempts in 1 054 ms.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db_write_contention_retry`: passed (9 passed).
    - `... agent_trace_db`: passed (66 passed, 3 ignored).
    - `... mutation_trace`: passed (389 passed).
    - `... resilience`: passed (6 passed).
    - Additional: `... lock_contention -- --nocapture` passed (7 passed, 3 ignored); `... services::db::` passed (35 passed); `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` passed.
  - Context impact: localized behavior change in the shared Turso DB layer. Agent Trace write units now have the outer write-contention retry layer, and there is a new opt-in `execute_idempotent_write` entrypoint. This affects `context/sce/shared-turso-db.md` (the outer-retry layer and its scope; the stale "5 attempts" Agent Trace text) and `context/sce/agent-trace-db.md` (the contention contract).
  - Context synchronization: synced

- [x] T05: `Make exhausted Agent Trace DB contention failures observable` (status:done)
  - Task ID: T05
  - Scope: In — when the T04 policy is exhausted:
    - return an error carrying `db_name`, `operation`, `attempts`, `busy_timeout_ms`, `contention_deadline_ms`, `elapsed_ms` and `cause` (database busy / busy timeout exhausted), worded so the deadline is described as a retry-scheduling cutoff;
    - emit one structured `tracing::warn!` event `sce.agent_trace_db.contention_exhausted` with the same fields, as a telemetry instrumentation point (never stdout; it has no production sink until a tracing subscriber is installed);
    - increment the test exhaustion counter.
    Add a test asserting the error fields and the captured event. Out — changing hook fail-open behavior; a metrics system; claims of user visibility when logging is not configured.
  - Dependencies: T04
  - Done when: AC4 holds; the stdout of the hook commands is unchanged.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db_contention_exhausted`; `... test --manifest-path cli/Cargo.toml hooks`.
  - Completed: 2026-10-04
  - Files changed:
    - `cli/src/services/db/mod.rs`
    - `context/plans/agent-trace-db-write-contention.md` (this record)
  - Result:
    - `WriteContentionPolicy` now carries `db_name` (from `M::db_name()`).
    - Exhaustion goes through a new `contention_exhausted_error` helper. It computes `elapsed` once, increments the `#[cfg(test)]` exhaustion counter, emits the event, and returns the error.
    - Error text: `Operation '<op>' failed after <n> attempt(s) under write contention (db_name=<db>, operation=<op>, attempts=<n>, busy_timeout_ms=<ms>, contention_deadline_ms=<ms> [no retry is scheduled past this cutoff], elapsed_ms=<ms>, cause=database busy (busy timeout exhausted)). Last error: <turso error>. Try: <hint>`.
      - The T04 prefix is unchanged.
      - The deadline is worded as a retry-scheduling cutoff.
      - Turso's `database is locked` text stays in `Last error`.
    - Event: one `tracing::warn!(target: "sce", event_id = "sce.agent_trace_db.contention_exhausted", …)` with the fields `db_name`, `operation`, `attempts`, `busy_timeout_ms`, `contention_deadline_ms`, `elapsed_ms`, `cause` and `last_error`, following the `sce.resilience.retry` pattern. There are no stdout writes, and hook fail-open behavior is unchanged.
    - The constants `CONTENTION_EXHAUSTED_EVENT_ID` and `CONTENTION_EXHAUSTED_CAUSE` hold the stable strings.
    - Tests (prefix `agent_trace_db_contention_exhausted`). They use a minimal in-test capturing `tracing::Subscriber` installed with `tracing::subscriber::with_default`; no new crates.
      - The exact error text and every event field, plus target `sce` and level `WARN`, on the fake-clock oversleep scenario (`elapsed_ms=810`).
      - Exactly one event, with `attempts=2`, after the attempt cap.
      - No event for success-after-retry or for deterministic errors.
    - The T04 oversleep test now asserts `elapsed_ms=810`; it previously asserted the interim `elapsed=810ms`.
    - Observability scope boundary (amended 2026-10-05):
      - The `tracing` event is kept as an instrumentation point for future telemetry/OTEL integration.
      - Production currently uses `NoopTelemetry`, so the raw event itself is not persisted during normal CLI execution. `sce.resilience.retry` behaves the same way.
      - The returned error carries every AC4 field. Existing hook fail-open handlers already pass it to the configured SCE `Logger` through `log.error(...)` (for example `sce.hooks.codex.error` and the conversation-trace hook).
      - No `Logger` dependency was added to `TursoDb`, and no telemetry runtime was added in this PR.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db_contention_exhausted`: passed (3 passed).
    - `... test --manifest-path cli/Cargo.toml hooks`: passed (806 passed, 1 ignored).
    - Additional:
      - `... agent_trace_db`: passed (72 passed, 3 ignored).
      - `... mutation_trace`: passed (389 passed).
      - `... services::db::`: passed (41 passed).
      - `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt`: passed.
      - Inspection: the diff adds no `print!`/`println!`/stdout writes.
  - Amendment (2026-10-05): AC4, the observability assumption and this task's scope were reworded to match the production observability above. T05 runtime code is unchanged. `context/sce/cli-observability-contract.md` was corrected so it no longer claims that app runtime installs a production tracing subscriber.
  - Context impact: localized observability change. The exhaustion error shape and the `sce.agent_trace_db.contention_exhausted` event affect `context/sce/shared-turso-db.md` and `context/sce/agent-trace-db.md` (the contention contract: what gets reported on exhaustion and where it is visible).
  - Context synchronization: synced

- [x] T06: `Make hook-runtime repository metadata open read-only when initialized` (status:done)
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
  - Completed: 2026-10-05
  - Files changed:
    - `cli/src/services/agent_trace_db/repository.rs`
    - `cli/src/services/agent_trace_db/lock_contention_tests.rs`
    - `cli/src/services/db/mod.rs`
    - `context/plans/agent-trace-db-write-contention.md` (this record)
  - Result:
    - `verify_or_initialize_repository_metadata` now reads the metadata row first. If the row exists, a `repository_id` mismatch errors with no write. If the `source_instance_id` is valid, the method returns with no write.
    - Only a missing row or an empty/invalid `source_instance_id` falls through to the unchanged path: `INSERT … ON CONFLICT DO NOTHING` → re-read → mismatch check → atomic claim → re-read.
    - The mismatch check moved into a private `ensure_repository_id_matches` helper. The error text is unchanged.
    - `db/mod.rs` gains a `#[cfg(test)]` `count_write_statements` seam that mirrors `count_read_statements`. It counts once per logical call, before any retry wrapper, in `TursoDb::execute`, `execute_idempotent_write`, `execute_transactional_insert_pair_if_absent` and `execute_transactional_cas_batch`. When `execute_idempotent_write` falls back to `execute`, the call is still counted once. `EncryptedTursoDb` is not instrumented.
    - New tests in `repository.rs`:
      - `initialized_repository_metadata_hook_runtime_open_issues_no_writes`: a hook-runtime open (`open_for_hooks_without_migrations_at` → `ensure_schema_ready_for_hooks` → verify) issues 0 writes and returns the original metadata; the first initialization issues more than 0 writes.
      - `mismatched_repository_metadata_errors_without_writes`
      - `repository_metadata_with_empty_source_instance_id_is_claimed_once`: an empty placeholder is claimed, and the claimed ID is not overwritten afterwards (0 writes on the next open).
    - The existing tests still pass: stable ID across reopen, mismatch, concurrent convergence, and baseline-only migration.
    - `hook_open_metadata_write_exhausts_retry_budget_while_write_lock_is_held` became `initialized_hook_open_succeeds_while_write_lock_is_held`. With another connection holding `BEGIN IMMEDIATE` for 4000 ms, the schema check and the metadata open both succeed, return the initialized metadata, issue 0 writes, and finish before the lock is released (observed: 1 ms).
    - Per user instruction, the generated code carries no new comments.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml repository_metadata`: passed (5 passed).
    - `... agent_trace_db`: passed (75 passed, 3 ignored).
    - `... agent_trace_storage`: passed (14 passed).
    - Additional:
      - `... initialized_hook_open_succeeds -- --nocapture`: passed (`writes=0 after 1ms` under a 4000 ms lock).
      - `... concurrent_initialization_converges`: passed.
      - `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt`: passed.
  - Context impact: localized behavior change. Opening an already-initialized repository Agent Trace DB is now read-only, so it no longer contends for the write lock; this affects `context/sce/agent-trace-db.md` (hook-runtime read-only metadata fast path). There is no config, schema or public CLI contract change.
  - Context synchronization: synced

- [x] T07: `Recalibrate the lock-budget boundary test to the new contention contract` (status:done)
  - Task ID: T07
  - Scope: In:
    - Deterministic contention contract (primary proof): set `LOCK_HOLD_DURATIONS_MS` to 100/250/500/750/1000/1500/2000 and align the budget constants with the T03/T04 contract. Assert by outcome and retry policy with wide margins: holds ≤ 250 ms succeed, holds ≥ 2000 ms exhaust the contention policy and fail cleanly, and every failure leaves zero message/part rows. Middle holds are characterization only.
    - Concurrent regression/stress: N=2–4 strict levels (AC8/AC9) must show zero loss and zero exhaustion. They are regression gates, not the pre-fix reproduction. N=8 is reported as stress characterization. If the fix makes N=8 reliable within acceptable latency, report it, but do not turn N=8 into a supported requirement without a deliberate decision.
    - make the strict in-process concurrent tests report outer retries and contention exhaustions from the T04/T05 test counters next to the latency percentiles;
    - run the full AC8/AC9 strict matrix plus the N=8 stress runs and record the after-fix measurements next to the T01 baseline in this task's completion record;
    - tune the defaults only if the evidence requires it, recording any tuning and its reason.
    Out — weakening, skipping or deleting any strict assertion; wall-clock cutoff assertions that assume a running operation is interrupted; raising attempts or budgets beyond what the measurements justify.
  - Dependencies: T03, T04, T05, T06
  - Done when: AC7 holds deterministically; the strict N=2–4 matrix passes every AC8/AC9 strict assertion; before-vs-after measurements (p50, p95, p99, max, outer retries and exhaustions where measurable, plus the 8-writer stress run) are recorded.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_budget_boundary -- --nocapture`; the AC8 and AC9 strict commands.
  - Completed: 2026-10-05
  - Files changed:
    - `cli/src/services/agent_trace_db/lock_contention_tests.rs`
    - `context/plans/agent-trace-db-write-contention.md` (this record)
  - Result:
    - Lock-budget boundary:
      - `LOCK_HOLD_DURATIONS_MS` is now 100/250/500/750/1000/1500/2000. The budget constants are `RELIABLY_WITHIN_CONTENTION_BUDGET_MS = 250` and `RELIABLY_BEYOND_CONTENTION_BUDGET_MS = 2_000`, and `WRITE_CONTENTION_MAX_ATTEMPTS = 2` mirrors the T04 cap.
      - Each sample wraps the production insert in `count_write_contention`. It reports and asserts attempts, outer retries and exhaustions.
      - For every hold: messages equal parts; a failure leaves 0/0 rows; `1 <= attempts <= 2`; `outer_retries + 1 == attempts`.
      - Holds of 250 ms or less: `Ok(true)` with 0 exhaustions.
      - Holds of 2000 ms or more: a `database is locked` contention-exhaustion error (text contains `under write contention`) with exactly 1 exhaustion.
      - The previous `elapsed < hold` wall-clock assertion is removed.
      - Middle holds are reported only.
    - Concurrent in-process suites:
      - Each writer's insert is wrapped in `count_write_contention`. The level report adds attempts, outer-retry and exhaustion columns next to the latency percentiles.
      - Every round asserts that no writer exceeds the attempt cap.
      - Strict mode additionally asserts 0 exhaustions per level.
      - The stale "no outer retry" titles are replaced, and the test functions are renamed to `concurrent_distinct_events_persist_every_event_under_write_contention`, `concurrent_duplicate_delivery_persists_each_event_once_under_write_contention`. The AC8 filters still match.
      - The stale `T05` prefix is dropped from the hook-process report title.
    - `initialized_hook_open_succeeds_while_write_lock_is_held` now uses a 2000 ms hold through the renamed constant (previously 4000 ms).
    - No strict assertion was weakened, skipped or deleted.
    - Defaults are not tuned in T07: `busy_timeout_ms = 500`, `contention_deadline_ms = 1250`, max attempts = 2, backoff cap = 100 ms. The T07 measurement campaign below did not justify a policy change. **Superseded by T08:** final validation then failed AC8 under these defaults, and T08 tuned them to 1000 / 2250.
  - After-fix measurement campaign (authoritative T07 evidence):
    - Detailed environment, methodology, raw run tables, aggregate statistics, retry timelines, host correlation and policy conclusions are recorded in `context/sce/agent-trace-db-write-contention-evidence.md`. That doc is the canonical measurement source; this record only summarizes it.
    - Method: frozen release artifacts plus test-only retry-timeline instrumentation (`record_write_contention_timeline`) and scheduler-gap monitors.
    - Contract (asserted by tests):
      - lock-budget boundary: holds `<= 250 ms` succeed; holds `>= 2000 ms` exhaust cleanly with 0/0 partial rows; no writer exceeds 2 attempts. Holds of 500–1500 ms are characterization only;
      - strict N=2–4 in-process and real-hook gates (AC8/AC9): 0 lock errors, 0 exhaustions, 0 lost distinct events, no orphan or duplicate rows, 0 non-zero hook exits.
    - Observed on reference host (not guarantees):
      - 10 full held-lock boundary runs passed the contract (70/70 samples).
      - 5/5 complete supported N=2–4 strict in-process matrices passed: 57,500 writes, 0 lock errors, 0 contention exhaustions, 0 lost distinct events.
      - Real release hooks, N=2–4: 16,500 events, 0 lost events, 0 fail-open persistence losses, 0 non-zero exits.
      - N=8 remains stress characterization only, not a supported requirement:
        - distinct: 20,000 writes, 0 exhaustions, 0 loss;
        - duplicate: 20,000 writes, 16 contention exhaustions in one of five runs, 0 logical-event loss because another concurrent writer persisted each affected duplicate.
      - Retry timelines show those N=8 failures were genuine bounded DB-policy exhaustion while another transaction held the writer lock for more than about 1 s. No scheduler stall was observed during the failure. It correlated with a device-level IO burst; the source of that burst was not proven.
    - Conclusion at T07: no policy change was recommended from this dataset; the policy looked adequate for supported N=2–4 and borderline only at unsupported N=8 stress. A later validation run contradicted the supported-load part of that conclusion (see T08).
  - Preliminary / superseded measurement: an earlier single-run campaign on the same day (HEAD `c513278d` + T07 test changes) recorded a strict distinct-event failure that it attributed to host-wide stalls, and reported N=8 as clean. It is not the final T07 evidence. Its host-stall attribution is withdrawn because the full campaign above observed no scheduler stall and no strict N=2–4 failure.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_budget_boundary -- --nocapture`: passed in every run, printing per-hold outcome, latency, attempts, outer retries and exhaustions.
    - AC8 strict `concurrent_distinct_events` (`WRITERS=2,3 ROUNDS=1000`, `WRITERS=4 ROUNDS=500`) and `concurrent_duplicate_delivery` (`WRITERS=2,3,4 ROUNDS=500`): 5/5 complete matrices passed in the measurement campaign.
    - AC8 N=8 stress (distinct and duplicate, 500 rounds each, 5 runs each): results above; reported as characterization only.
    - AC9: `nix build .#default` passed; strict `concurrent_real_codex_hook_processes` with `WRITERS=2,3 ROUNDS=500` and `WRITERS=4 ROUNDS=200` passed in every campaign run.
    - Additional: `... lock_contention` passed (7 passed, 3 ignored); `nix build .#checks.x86_64-linux.cli-clippy .#checks.x86_64-linux.cli-fmt` passed.
  - Context impact: none to code contracts or durable architecture; this is a test-only change plus plan evidence. The measured evidence lives in `context/sce/agent-trace-db-write-contention-evidence.md`.
  - Context synchronization: synced

- [x] T08: `Tune Agent Trace contention defaults after the supported-load validation failure` (status:done)
  - Task ID: T08
  - Trigger: final validation run 1 (500 / 1250 defaults) failed AC8. Strict distinct N=4×500 lost 1 event, and strict duplicate N=3×500 recorded 2 lock exhaustions. In both failing rounds a writer held the writer lock for 1616–1667 ms, and waiters exhausted after 1006–1090 ms.
  - Scope: In:
    - test the larger bounded two-attempt policy `busy_timeout_ms = 1000`, `contention_deadline_ms = 2250`, max attempts 2, backoff cap 100 ms;
    - extend the held-lock experiment to 100–3000 ms and set the AC7 thresholds from a 10-run dataset;
    - rerun the full strict AC8/AC9 campaign (5 matrices each) and N=8 stress (5 runs each) on frozen release artifacts with host evidence;
    - update config defaults, schema, tests and durable context.
    Out — weakening AC8; a third attempt; a queue, spool or daemon; any change to retry classification, retry units, attempt cap, jitter, admission re-check, deterministic-error handling, hook fail-open, the metadata fast path or exhaustion observability.
  - Dependencies: T07, validation run 1
  - Done when: the candidate is either adopted with all supported strict matrices passing, or rejected with the architectural conclusion recorded.
  - Completed: 2026-10-05
  - Files changed:
    - `cli/src/services/db/mod.rs`: `AGENT_TRACE_DB_BUSY_TIMEOUT_MS` 500 → 1_000; `AGENT_TRACE_DB_CONTENTION_DEADLINE_MS` 1_250 → 2_250; the default-pinning test assertion.
    - `config/pkl/base/sce-config-schema.pkl`: `busy_timeout_ms` default 500 → 1000; `contention_deadline_ms` default 1250 → 2250.
    - `cli/src/services/config/schema.rs`: generated-schema default assertions.
    - `cli/src/services/agent_trace_db/lock_contention_tests.rs`: `LOCK_HOLD_DURATIONS_MS` = 100/250/500/750/1000/1500/1750/2000/2250/2500/3000; `RELIABLY_WITHIN_CONTENTION_BUDGET_MS` 250 → 1_000; `RELIABLY_BEYOND_CONTENTION_BUDGET_MS` 2_000 → 3_000 (also lengthens the `initialized_hook_open_succeeds_while_write_lock_is_held` hold to 3000 ms).
    - `context/sce/agent-trace-db-write-contention-evidence.md`, `context/sce/shared-turso-db.md`, `context/cli/config-precedence-contract.md`, `context/glossary.md`, `context/context-map.md`, this plan.
  - Result (decision A, candidate passes; detailed tables, timelines and host evidence in `context/sce/agent-trace-db-write-contention-evidence.md`):
    - Contract (asserted): holds ≤ 1000 ms succeed, holds ≥ 3000 ms exhaust cleanly; failure leaves 0/0 rows; ≤ 2 attempts; `outer_retries + 1 == attempts`; 1500–2500 ms is characterization only.
    - Observed on reference host (65 runs on frozen release artifacts, all exit 0, nothing rerun or dropped):
      - Held lock, 10 runs × 11 holds: 110/110 samples met the contract with 0 invariant violations. Holds of 100–1000 ms succeeded on attempt 1 and 1500–2000 ms on attempt 2; 2250–3000 ms exhausted at 2001–2094 ms. The transition lies between a holder release of 2013 ms (succeeded) and 2254 ms (exhausted).
      - Strict N=2–4 in-process: 5/5 matrices passed; 57,500 writes, 0 lock errors, 0 exhaustions, 0 outer retries, 0 lost of 35,000 distinct events, 0 orphan or duplicate rows, 0 operations ≥ 500 ms.
      - Strict real hooks: 5/5 matrices passed; 16,500 events, 0 lost, 0 fail-open losses, 0 non-zero exits, 0 stderr records.
      - N=8 stress (characterization only): distinct 20,000 writes and duplicate 20,000 writes, 0 exhaustions, 0 loss; worst max 859 ms (1834 ms under 500 / 1250).
      - Host: 0 external sleep gaps ≥ 50 ms in 50.6 minutes; no post-failure snapshot was needed.
    - Latency cost: an exhausting write now blocks its hook for about 2.0–2.1 s instead of 1.0–1.1 s (about +1 s). Supported-load percentiles did not get worse (distinct p99 ranges 102–135 / 134–202 / 99–167 ms for N=2 / 3 / 4).
    - Caveat: the T08 campaign never produced a supported-load holder of 1.6–1.7 s. Coverage of that holder rests on the held-lock experiment (1500–2000 ms holds succeeded 30/30), with about 300 ms of slack.
    - The 500 / 1250 T07 campaign and validation run 1 remain in the evidence doc and this plan as historical evidence.
  - Verify:
    - `nix run .#pkl-check-generated`: passed.
    - `... test --manifest-path cli/Cargo.toml` with `database_retry` (14), `busy_timeout` (7), `agent_trace_db_write_contention_retry` (12), `agent_trace_db` (75, 3 ignored), `agent_trace_storage` (14), `resilience` (6): passed.
    - `... lock_budget_boundary -- --nocapture` with the final thresholds: passed; `initialized_hook_open` passed under the 3000 ms hold.
    - Campaign A–D on frozen release artifacts: every run exit 0 (results above).
  - Context impact: changes the default Agent Trace contention budget. Durable context (`shared-turso-db.md`, `config-precedence-contract.md`, `glossary.md`, the evidence doc) now states 1000 / 2250. `agent-trace-db.md` does not mention the defaults and needed no change.
  - Context synchronization: synced

## Open questions

- The current 1000 / 2250 / 2 policy exhausts when a transaction holds the writer lock for more than about 2 s. Neither the T08 campaign (supported N=2–4 and N=8 stress) nor the held-lock experiment showed such a holder at supported load. If one is observed, do not tune timeouts further: bounded synchronous hook latency, no durable queue or spool, and unbounded writer-lock duration together mean zero-loss ingestion cannot be guaranteed. That needs a product/architecture decision between accepting occasional fail-open ingestion loss and introducing eventual persistence (spool or queue). Why the 1.6–1.7 s holders occurred in the failed validation run is also still unexplained (no host evidence was captured).

- PR #297's branch carries test-only stabilizers for this same contention (`c6cbc94a` retries locked SQLite writes in concurrent repository tests; `94dd0937` stabilizes convergence checks). Once this fix lands, those test-side retries may be redundant and could mask a regression. Should they be revisited in a follow-up after both PRs merge? This plan leaves them alone.
- Follow-up, not in this plan: proper CLI telemetry / OTEL integration, as a separate future telemetry effort. It would:
  - replace production `NoopTelemetry` with a real telemetry runtime;
  - install a tracing subscriber during command execution;
  - export structured tracing events through OTEL, covering `sce.resilience.retry` and `sce.agent_trace_db.contention_exhausted`;
  - define trace/session/repository correlation;
  - avoid `Logger` → tracing → `Logger` feedback or duplicate events;
  - keep the existing file/stderr `Logger` behavior during migration.
- Follow-up, not in this plan: generic `RetryPolicy.timeout_ms` is still documented and rendered as a per-attempt timeout for every database, even though `run_with_retry_sync` only checks elapsed time after a synchronous call returns. Should the generic cleanup become its own plan?

## Validation history

Earlier `/validate` runs, kept as evidence. The next `/validate` writes a fresh `## Validation Report` below.

### Validation run 1 (2026-10-05, 500 / 1250 defaults): failed

**Status:** failed  
**Date:** 2026-10-05

#### Commands run

- `nix flake check` -> exit 0 (all checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 142 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db` -> exit 0 (75 passed, 3 ignored)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` -> exit 0 (389 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml resilience` -> exit 0 (6 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml database_retry` -> exit 0 (14 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml busy_timeout` -> exit 0 (7 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db_write_contention_retry` -> exit 0 (12 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks` -> exit 0 (806 passed, 1 ignored)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml contention_exhausted` -> exit 0 (3 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml repository_metadata` -> exit 0 (5 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml initialized_hook_open` -> exit 0 (1 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_budget_boundary -- --nocapture` -> exit 0 (100/250/500 ms attempt 1; 750/1000 ms attempt 2; 1500/2000 ms exhausted after 2 attempts at about 1.0 s with 0/0 rows)
- AC8 strict `concurrent_distinct_events`, `WRITERS=2,3 ROUNDS=1000` -> exit 0 (0 lost, 0 exhaustions; p50/p95/p99/max N=2 30/93/135/1138 ms, N=3 45/105/151/236 ms)
- AC8 strict `concurrent_distinct_events`, `WRITERS=4 ROUNDS=500` -> exit 101 (1 lock error, 1 exhaustion, 1 lost distinct event in round 384; p50/p95/p99/max 45/173/254/1666 ms)
- AC8 strict `concurrent_duplicate_delivery`, `WRITERS=2,3,4 ROUNDS=500` -> exit 101 (N=3: 2 lock errors, 2 exhaustions in round 338, 0 lost logical events; N=2 and N=4 clean)
- AC8 non-strict `concurrent_distinct_events`, `WRITERS=8 ROUNDS=500` -> exit 0 (stress: 2 exhaustions, 2 lost of 4000, max 2134 ms)
- AC8 non-strict `concurrent_duplicate_delivery`, `WRITERS=8 ROUNDS=500` -> exit 0 (stress: 0 exhaustions, 0 lost)
- `nix build .#default` -> exit 0
- AC9 strict `concurrent_real_codex_hook_processes`, `WRITERS=2,3 ROUNDS=500` -> exit 0 (2500/2500 persisted, 0 non-zero exits; p99 43/166 ms)
- AC9 strict `concurrent_real_codex_hook_processes`, `WRITERS=4 ROUNDS=200` -> exit 0 (800/800 persisted, 0 non-zero exits; p99 73 ms)

#### Success-criteria verification

- [x] AC1: busy handler on every Agent Trace DB connection, multiprocess WAL kept -> `busy_timeout` tests passed; inspection of `cli/src/services/db/mod.rs`: the single `TursoDb::open` local path calls `.experimental_multiprocess_wal(true)` then `apply_busy_timeout`; the other `new_local` call is `EncryptedTursoDb` (auth DB), outside AC1
- [x] AC2: busy handler covers `BEGIN IMMEDIATE` -> T02 direct busy-handler tests passed within the `busy_timeout` filter
- [x] AC3: bounded write-contention retry -> `agent_trace_db_write_contention_retry` 12 passed
- [x] AC4: exhausted contention is observable -> `contention_exhausted` 3 passed; `hooks` 806 passed; inspection of `git diff main...HEAD -- cli/src` found no new stdout writes (only `eprintln!` in the test-only `lock_contention_tests.rs`)
- [x] AC5: initialized hook open issues zero writes -> `initialized_repository_metadata_hook_runtime_open_issues_no_writes`, `initialized_hook_open_succeeds_while_write_lock_is_held` and metadata tests passed
- [x] AC6: whole-transaction semantics -> `agent_trace_db` and `mutation_trace` passed
- [x] AC7: lock-budget boundary -> `lock_budget_boundary` passed, printing per-hold outcome, latency and attempts
- [ ] AC8: strict N=2–4 production-API levels -> failed: strict distinct N=4×500 lost 1 event to contention exhaustion, and strict duplicate N=3×500 recorded 2 lock errors
- [x] AC9: strict real-hook levels -> 2×500, 3×500 and 4×200 passed with 0 lost and 0 non-zero exits
- [x] AC10: Agent Trace-only config keys -> `database_retry` 14 passed; `pkl-check-generated` passed

#### Failed checks and follow-ups

- AC8 strict distinct `WRITERS=4 ROUNDS=500`: 1 lost distinct event; evidence: round 384, one writer's single attempt held the writer lock for 1666 ms (`Ok(true)`), and a waiter exhausted after 2 attempts at 1006 ms (`database is locked`, `busy_timeout_ms=500`, `contention_deadline_ms=1250`); required: decide whether the 500/1250/2-attempt policy must cover a lock holder of about 1.6 s at supported N=4, or whether AC8's strict gate needs a different acceptance basis, then fix in a normal work session.
- AC8 strict duplicate `WRITERS=2,3,4 ROUNDS=500`: 2 lock errors at N=3 (0 logical events lost); evidence: round 338, one holder's attempt took 1616 ms, and two waiters exhausted at 1064 and 1089 ms; required: same decision as above.
- The failure shape matches the documented N=8 exhaustion mode in `context/sce/agent-trace-db-write-contention-evidence.md` (holder transaction above about 1.05 s), but here it occurred at supported N=3 and N=4. The evidence doc's claim of 0 strict N=2–4 failures holds only for its recorded campaign.

#### Residual risks

- Supported N=2–4 writes can lose a distinct event when one transaction holds the writer lock for more than about 1.05 s. Host IO state during this validation run was not captured.
- Real hook processes passed, but they ran at lower round counts than the in-process suite.

#### Retry

After repairs, rerun:

`/validate context/plans/agent-trace-db-write-contention.md`

## Validation Report

**Status:** validated  
**Date:** 2026-10-05

### Commands run

- `nix flake check` -> exit 0 (all checks passed)
- `nix run .#pkl-check-generated` -> exit 0 (ephemeral Pkl generation passed: 142 files)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db` -> exit 0 (75 passed, 3 ignored)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace` -> exit 0 (389 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml resilience` -> exit 0 (6 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml database_retry` -> exit 0 (14 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml busy_timeout` -> exit 0 (7 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace_db_write_contention_retry` -> exit 0 (12 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks` -> exit 0 (806 passed, 1 ignored)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml contention_exhausted` -> exit 0 (3 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml repository_metadata` -> exit 0 (5 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml initialized_hook_open` -> exit 0 (1 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_budget_boundary -- --nocapture` -> exit 0 (100–1000 ms holds succeeded on attempt 1; 1500/1750/2000 ms succeeded on attempt 2; 2250/2500/3000 ms exhausted after 2 attempts at 2047–2063 ms with 0/0 rows)
- AC8 strict `concurrent_distinct_events`, `WRITERS=2,3 ROUNDS=1000` -> exit 0 (0 lock errors, 0 lost, 0 exhaustions, 0 outer retries; p50/p95/p99/max N=2 31/95/134/806 ms, N=3 52/119/175/285 ms)
- AC8 strict `concurrent_distinct_events`, `WRITERS=4 ROUNDS=500` -> exit 0 (0 lock errors, 0 lost, 0 exhaustions, 1 outer retry; p50/p95/p99/max 45/157/232/1246 ms)
- AC8 strict `concurrent_duplicate_delivery`, `WRITERS=2,3,4 ROUNDS=500` -> exit 0 (0 lock errors, 0 exhaustions, exactly 500 inserts per level; p99/max N=2 28/38, N=3 154/1412, N=4 191/299 ms)
- AC8 non-strict `concurrent_distinct_events`, `WRITERS=8 ROUNDS=500` -> exit 0 (stress: 0 lost of 4000, 0 exhaustions, 2 outer retries; p99/max 505/1067 ms)
- AC8 non-strict `concurrent_duplicate_delivery`, `WRITERS=8 ROUNDS=500` -> exit 0 (stress: 0 exhaustions, 500 inserts; p99/max 345/844 ms)
- `nix build .#default` -> exit 0
- AC9 strict `concurrent_real_codex_hook_processes`, `WRITERS=2,3 ROUNDS=500` -> exit 0 (1000/1000 and 1500/1500 persisted, 0 non-zero exits, 0 stderr lines; p50/p95/p99/max N=2 26/38/40/46 ms, N=3 46/108/155/629 ms)
- AC9 strict `concurrent_real_codex_hook_processes`, `WRITERS=4 ROUNDS=200` -> exit 0 (800/800 persisted, 0 non-zero exits, 0 stderr lines; p50/p95/p99/max 44/74/77/81 ms)

### Success-criteria verification

- [x] AC1: busy handler on every Agent Trace DB connection, multiprocess WAL kept -> `busy_timeout` 7 passed; inspection of `cli/src/services/db/mod.rs`: the single `TursoDb` local open path calls `.experimental_multiprocess_wal(true)` and then `apply_busy_timeout`; the only other `new_local` call is `EncryptedTursoDb` (auth DB), outside AC1
- [x] AC2: busy handler covers `BEGIN IMMEDIATE` -> T02 direct busy-handler tests passed within the `busy_timeout` filter
- [x] AC3: bounded write-contention retry -> `agent_trace_db_write_contention_retry` 12 passed; the boundary run showed the 100 ms hold at `attempts = 1`, `outer_retries = 0`
- [x] AC4: exhausted contention is observable -> `contention_exhausted` 3 passed; `hooks` 806 passed; inspection of `git diff main...HEAD -- cli/src` found no new stdout writes (new `eprintln!` lines appear only in the test-only `lock_contention_tests.rs`)
- [x] AC5: initialized hook open issues zero writes -> `repository_metadata` 5 passed and `initialized_hook_open` passed under the 3000 ms held write lock
- [x] AC6: whole-transaction semantics -> `agent_trace_db` 75 passed and `mutation_trace` 389 passed; strict runs showed no orphan or duplicate rows
- [x] AC7: lock-budget boundary -> `lock_budget_boundary` passed: ≤ 1000 ms succeeded, ≥ 3000 ms exhausted cleanly, every sample ≤ 2 attempts with `outer_retries + 1 == attempts` and 0/0 rows on failure
- [x] AC8: strict N=2–4 production-API levels -> distinct 2×1000, 3×1000, 4×500 and duplicate 2×500, 3×500, 4×500 passed with 0 lock errors, 0 other errors and 0 lost events; N=8 stress reported with 0 loss
- [x] AC9: strict real-hook levels -> 2×500, 3×500 and 4×200 passed with 0 lost and 0 non-zero exits
- [x] AC10: Agent Trace-only config keys -> `database_retry` 14 passed; `pkl-check-generated` passed

### Failed checks and follow-ups

- None.

### Residual risks

- Writer-lock holders longer than about 2 s still exhaust the 1000 / 2250 / 2-attempt policy and lose the event through hook fail-open; no such holder occurred at supported load in this run. The open question on accepting occasional loss vs. eventual persistence still applies.
- The unexplained 1.6–1.7 s holders from validation run 1 did not recur here, but this run captured no host IO evidence. Coverage of such holders rests on the held-lock boundary results (1500–2000 ms holds succeeded on attempt 2).
- Strict contention gates are timing-sensitive and were measured on one reference host.
