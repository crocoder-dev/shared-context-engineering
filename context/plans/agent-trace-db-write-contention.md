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

- [ ] AC1: Every Agent Trace DB connection opened by SCE has Turso's busy handler set to the resolved `busy_timeout_ms` (default 500 ms), and `experimental_multiprocess_wal(true)` stays enabled on every local open path.
  - Validate: the T02 busy-timeout tests pass (`nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml busy_timeout`), and inspection of `cli/src/services/db/mod.rs` shows `experimental_multiprocess_wal(true)` and the `busy_timeout` call on every local `TursoDb` open path.
- [ ] AC2: Turso's busy handler covers the failing writer-lock acquisition. Turso 0.8.1's `Transaction::new_unchecked(conn, TransactionBehavior::Immediate)` runs `BEGIN IMMEDIATE` through `Connection::execute(...)` on the same connection. With connection A holding `BEGIN IMMEDIATE` for about 100 ms:
  - a control connection without a busy handler returns `Busy` promptly;
  - the production `insert_conversation_text_event` on a busy-timeout-configured Agent Trace DB connection waits and succeeds, with no test-level retry around the call.
  - Validate: the T02 direct busy-handler tests pass.
- [ ] AC3: Agent Trace write-capable operations (the enumerated set in T04) use at most two outer attempts, retry only typed `Busy`/`BusySnapshot` failures, use bounded full jitter, and never start another outer attempt after the contention retry-start rule disallows it. A retry starts only when `remaining_deadline >= jittered_backoff + busy_timeout`, and never once the contention deadline has expired. Deterministic errors fail after exactly one attempt with no sleep. A ~100 ms lock hold succeeds with `attempts = 1` and `outer_retries = 0`. Read-only query APIs (`query`, `query_values`, `query_map`), `passive_checkpoint`, and Agent Trace writes outside the enumerated set retain their existing outer retry semantics.
  - Validate: the T04 unit tests (`... test --manifest-path cli/Cargo.toml agent_trace_db_write_contention_retry`) pass. They cover:
    - a seeded jitter seam;
    - retry-start rule boundary cases (remaining just above, equal to, and just below `backoff + busy_timeout`, and an expired deadline);
    - Busy/BusySnapshot vs non-Busy classification;
    - the attempt cap;
    - the 100 ms-hold single-attempt assertion;
    - a regression assertion that an Agent Trace `query`/`query_map` call still resolves the existing generic query retry policy.
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
- [ ] AC8: The Rust production-API suite (`insert_conversation_text_event`) passes the strict levels N=2–4: distinct-event 2×1000, 3×1000, 4×500, and duplicate-delivery 2×500, 3×500, 4×500. These levels are regression gates on the reference host; the T01 baseline already passes them before the fix. The test enforces:
  - always, per round: `messages == parts` (no orphan rows); the `Ok(true)` count equals the persisted message rows; distinct events never report `Ok(false)`; a duplicate-delivery round inserts at most once, and exactly once when at least one writer completed (no duplicate persisted events);
  - under `SCE_LOCK_CONTENTION_STRICT=1`, per writer level: 0 lock errors, 0 other errors and 0 lost distinct events.
  - 8 writers runs as non-strict stress/characterization only. If the fixed system makes N=8 reliable within acceptable latency, that result is reported, but N=8 is not a supported semantic requirement unless deliberately decided.
  - Validate: run these, reporting p50/p95/p99/max latency, outer retries and contention exhaustions (from test instrumentation), next to the T01 baseline:
    - `SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=2,3 SCE_LOCK_CONTENTION_ROUNDS=1000 nix develop -c ./scripts/run-cli-cargo.sh test --release --manifest-path cli/Cargo.toml concurrent_distinct_events -- --ignored --nocapture`;
    - the same with `WRITERS=4 ROUNDS=500`;
    - `concurrent_duplicate_delivery` with `WRITERS=2,3,4 ROUNDS=500`;
    - a non-strict `WRITERS=8` run.
- [ ] AC9: Real release `sce hooks codex` `UserPromptSubmit` processes pass the strict levels 2×500, 3×500 and 4×200. These levels are regression gates on the reference host; the T01 baseline already passes them before the fix. `concurrent_real_codex_hook_processes_persist_every_distinct_event` enforces:
  - always, per round: `messages == parts` (no orphan rows);
  - always, per writer level: `persisted_messages <= expected` and `persisted_parts <= expected` (no over-persistence or duplicate events);
  - under `SCE_LOCK_CONTENTION_STRICT=1`, per writer level: `persisted_messages == expected`, `persisted_parts == expected`, and 0 non-zero hook exits;
  - under `SCE_LOCK_CONTENTION_STRICT=1`, across all levels: `total_lost == 0` and `total_nonzero_exits == 0`.
  - Hook stderr lines are recorded and reported but are not a strict invariant. Hooks fail open, and the contention fix may legitimately emit diagnostics through the configured logging path. "Lock-exhaustion failures" in hook processes show up as lost events, because the hook fails open; they are covered by the lost-event and persisted-count assertions.
  - Validate: `nix build .#default`, then `SCE_BIN=$PWD/result/bin/sce SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=2,3 SCE_LOCK_CONTENTION_ROUNDS=500 nix develop -c ./scripts/run-cli-cargo.sh test --release --manifest-path cli/Cargo.toml concurrent_real_codex_hook_processes -- --ignored --nocapture`, and the same with `WRITERS=4 ROUNDS=200`. Record p50/p95/p99/max per-event latency next to the T01 baseline. Hook processes are separate processes, so in-process counters are unavailable; report persisted-row outcomes and latency only, and no retry counts unless they are derived from the hook log file.
- [ ] AC10: `busy_timeout_ms` and `contention_deadline_ms` are Agent Trace-only settings. They are accepted, validated and documented only under `policies.database_retry.agent_trace_db`:
  - `policies.database_retry.local_db` and `policies.database_retry.auth_db` keep their existing keys (`connection_open`, `query`) only, and reject `busy_timeout_ms` and `contention_deadline_ms` through the existing config-validation path. The generated-schema check runs first and fails with the existing stable `Config file '<path>' failed schema validation against generated schema '<schema>': …` error, naming the offending key. The Rust per-DB key check (`validate_object_keys`, allowed keys `connection_open, query` for `local_db`/`auth_db`) stays as a backstop with its existing `contains unknown key` wording;
  - the generated config schema publishes the two keys only on the `agent_trace_db` object; the `local_db`/`auth_db` objects keep `additionalProperties = false` with only `connection_open` and `query`;
  - both are non-negative integers with upper bounds;
  - `busy_timeout_ms = 0` disables the Turso busy handler;
  - `contention_deadline_ms = 0` means no outer retry is started;
  - both are rendered by `sce config show` in text and JSON and published in the generated config schema;
  - defaults (500 / 1250) apply when they are unset;
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
- Initial defaults, to be tuned only from T07 evidence:
  - `busy_timeout_ms = 500`;
  - `contention_deadline_ms = 1250`;
  - outer `max_attempts = 2`;
  - full-jitter backoff `random(0..=100 ms)`.
- Retry-start rule: after a typed `Busy`/`BusySnapshot`, compute `backoff = jitter(0..=cap)` and `remaining = contention_deadline - (now - operation_start)`. Launch the next attempt only if attempts remain and `remaining >= backoff + busy_timeout`; otherwise fail. With the defaults, a second attempt is normally possible only if the first one returned within about 650–750 ms.
- Upper bounds: `busy_timeout_ms <= 10_000` and `contention_deadline_ms <= 30_000`.
- Config contract (decided): `busy_timeout_ms` and `contention_deadline_ms` are Agent Trace DB contention settings, supported only as `policies.database_retry.agent_trace_db.{busy_timeout_ms,contention_deadline_ms}`, beside that object's existing `connection_open`/`query` keys:

  ```json
  {
    "policies": {
      "database_retry": {
        "agent_trace_db": {
          "busy_timeout_ms": 500,
          "contention_deadline_ms": 1250
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
- Production observability is the structured `tracing` event `sce.agent_trace_db.contention_exhausted` through the existing logger. It is available only where logging is configured (log file / stderr), and the plan does not claim it is otherwise user-visible. Process-local atomic counters (attempts, outer retries, exhaustions) are test instrumentation only. They are meaningful inside the in-process Rust suite, not across hook processes, and are not production-wide telemetry.
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

- [ ] T04: `Add Agent Trace DB write-contention retry for safe write units` (status:todo)
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
    - Deterministic contention contract (primary proof): set `LOCK_HOLD_DURATIONS_MS` to 100/250/500/750/1000/1500/2000 and align the budget constants with the T03/T04 contract. Assert by outcome and retry policy with wide margins: holds ≤ 250 ms succeed, holds ≥ 2000 ms exhaust the contention policy and fail cleanly, and every failure leaves zero message/part rows. Middle holds are characterization only.
    - Concurrent regression/stress: N=2–4 strict levels (AC8/AC9) must show zero loss and zero exhaustion. They are regression gates, not the pre-fix reproduction. N=8 is reported as stress characterization. If the fix makes N=8 reliable within acceptable latency, report it, but do not turn N=8 into a supported requirement without a deliberate decision.
    - make the strict in-process concurrent tests report outer retries and contention exhaustions from the T04/T05 test counters next to the latency percentiles;
    - run the full AC8/AC9 strict matrix plus the N=8 stress runs and record the after-fix measurements next to the T01 baseline in this task's completion record;
    - tune the defaults only if the evidence requires it, recording any tuning and its reason.
    Out — weakening, skipping or deleting any strict assertion; wall-clock cutoff assertions that assume a running operation is interrupted; raising attempts or budgets beyond what the measurements justify.
  - Dependencies: T03, T04, T05, T06
  - Done when: AC7 holds deterministically; the strict N=2–4 matrix passes every AC8/AC9 strict assertion; before-vs-after measurements (p50, p95, p99, max, outer retries and exhaustions where measurable, plus the 8-writer stress run) are recorded.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml lock_budget_boundary -- --nocapture`; the AC8 and AC9 strict commands.
  - Context synchronization: pending

## Open questions

- PR #297's branch carries test-only stabilizers for this same contention (`c6cbc94a` retries locked SQLite writes in concurrent repository tests; `94dd0937` stabilizes convergence checks). Once this fix lands, those test-side retries may be redundant and could mask a regression. Should they be revisited in a follow-up after both PRs merge? This plan leaves them alone.
- Follow-up, not in this plan: generic `RetryPolicy.timeout_ms` is still documented and rendered as a per-attempt timeout for every database, even though `run_with_retry_sync` only checks elapsed time after a synchronous call returns. Should the generic cleanup become its own plan?
