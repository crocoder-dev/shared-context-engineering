# Agent Trace DB write-contention evidence

The measured behavior of the Agent Trace DB contention contract (Turso `busy_timeout` → at most one jittered outer retry → `contention_deadline_ms` cutoff), and the test suite that measures it. The contract itself is defined in [shared-turso-db.md](shared-turso-db.md); the repository adapter is described in [agent-trace-db.md](agent-trace-db.md).

## Lock-contention suite

`cli/src/services/agent_trace_db/lock_contention_tests.rs` is a `#[cfg(test)]` module that drives the real production API (`insert_conversation_text_event` on hook-runtime connections) and the real release `sce hooks codex` binary against a temporary repository Agent Trace DB.

| Test | Kind | What it proves |
| --- | --- | --- |
| `lock_budget_boundary_characterizes_single_writer_blocked_by_begin_immediate_holder` | default | One connection holds `BEGIN IMMEDIATE` for 100–2000 ms while another inserts. Holds ≤ 250 ms succeed with no exhaustion. Holds ≥ 2000 ms fail with the `under write contention` exhaustion error and exactly one exhaustion. Every hold makes 1–2 attempts, and every failure leaves 0 message/part rows. Middle holds are reported only. |
| `concurrent_distinct_events_persist_every_event_under_write_contention` | `#[ignore]` | N writers insert one distinct event each per round. |
| `concurrent_duplicate_delivery_persists_each_event_once_under_write_contention` | `#[ignore]` | N writers deliver the same event per round. |
| `concurrent_real_codex_hook_processes_persist_every_distinct_event` | `#[ignore]` | N real `sce hooks codex` `UserPromptSubmit` processes per round (needs `SCE_BIN`). |

Assertions:

- Every in-process round always asserts:
  - `messages == parts`;
  - the `Ok(true)` count equals the persisted rows;
  - no writer exceeds the 2-attempt cap.
- Every duplicate round always asserts at most one insert.
- With `SCE_LOCK_CONTENTION_STRICT=1`, each level additionally asserts 0 lock errors, 0 other errors, 0 lost events and 0 contention exhaustions.

The ignored tests take `SCE_LOCK_CONTENTION_WRITERS` (comma-separated) and `SCE_LOCK_CONTENTION_ROUNDS`. They print latency percentiles and the per-level `count_write_contention` attempt, outer-retry and exhaustion totals. Those counters are thread-local test instrumentation and are unavailable for the hook-process test.

Run them with `--release`, `-- --ignored --nocapture`, through `nix develop -c ./scripts/run-cli-cargo.sh test`.

The strict gates are N=2–4:

- distinct-event 2×1000, 3×1000, 4×500;
- duplicate-delivery 2×500, 3×500, 4×500;
- hook processes 2×500, 3×500, 4×200.

N=8 is stress characterization only, not a supported requirement.

## Measured behavior (reference host, defaults 500 / 1250 ms)

Single writer against a held `BEGIN IMMEDIATE`:

| Hold | Result |
| --- | --- |
| ≤ 500 ms | succeeds on the first attempt; Turso's busy wait absorbs it |
| 750–1000 ms | succeeds after one admitted outer retry |
| ≥ 1500 ms | exhausts after 2 attempts in about 1.0–1.1 s, with 0 rows |

Before the contract, any hold of 300 ms or more failed after about 280 ms (5 generic attempts, no busy handler).

Concurrent writers:

- At N=2–4, strict runs have 0 lost events and 0 exhaustions. p50 is 18–53 ms and p99 is 26–234 ms, mostly at or below the pre-contract baseline (occasional single-write maxima above 1 s are host stalls; see below).
- N=8 stress went from 1525 of 4000 distinct events lost (every round exhausted) to 0 lost, with 18 outer retries in 4000 writes and p99 of about 0.5 s.

## Known failure mode: host stalls

The contract is bounded on purpose, with no indefinite hook waits. A lock holder that stalls for more than about 1.1 s therefore exhausts the waiting writer's policy. That writer gets the contention-exhaustion error, and a fail-open hook loses that event.

On a loaded host, such stalls have been observed intermittently: 1–2 s where every writer stalls at once, not tied to WAL checkpoints. They made one strict N=2–4 distinct-event run record 1–3 exhaustions in about 2000 writes, while reruns passed.

Treat an isolated strict-gate failure with a max latency above 1 s as host noise to rerun on a quiet host, not as a policy regression. Whether to raise `contention_deadline_ms` is open and has not been decided from this evidence.
