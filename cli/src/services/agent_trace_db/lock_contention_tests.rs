use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;

use crate::services::db::{
    count_write_contention, count_write_statements, record_write_contention_timeline,
    WriteContentionCounts, WriteContentionTimelineEvent,
};

use super::repository::RepositoryAgentTraceDb;
use super::{InsertMessageInsert, InsertPartInsert, MessageRole, PartType};

const DATABASE_LOCKED_ERROR: &str = "database is locked";
const WRITE_CONTENTION_ERROR: &str = "under write contention";
const LOCK_HOLD_DURATIONS_MS: &[u64] = &[
    100, 250, 500, 750, 1_000, 1_500, 1_750, 2_000, 2_250, 2_500, 3_000,
];
const RELIABLY_WITHIN_CONTENTION_BUDGET_MS: u64 = 1_000;
const RELIABLY_BEYOND_CONTENTION_BUDGET_MS: u64 = 3_000;
const WRITE_CONTENTION_MAX_ATTEMPTS: u32 = 2;
const BUSY_TIMEOUT_PRODUCTION_HOLD_MS: u64 = 100;
const ROUNDS_ENV: &str = "SCE_LOCK_CONTENTION_ROUNDS";
const WRITERS_ENV: &str = "SCE_LOCK_CONTENTION_WRITERS";
const STRICT_ENV: &str = "SCE_LOCK_CONTENTION_STRICT";
const DEFAULT_ROUNDS: usize = 200;
const DEFAULT_DUPLICATE_WRITER_COUNTS: &[usize] = &[2, 3, 4, 8];
const DEFAULT_DISTINCT_WRITER_COUNTS: &[usize] = &[2, 3, 4, 8];
const MEASUREMENT_PREFIX: &str = "SCE_MEAS";
const SLOW_OPERATION_MS: f64 = 500.0;
const STALL_MONITOR_TICK: Duration = Duration::from_millis(5);
const STALL_MONITOR_REPORT_MS: f64 = 50.0;

fn unix_ms_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
        * 1_000.0
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn emit_measurement(record: &serde_json::Value) {
    eprintln!("{MEASUREMENT_PREFIX} {record}");
}

fn timeline_json(
    started_at: Instant,
    timeline: &[(Instant, WriteContentionTimelineEvent)],
) -> serde_json::Value {
    serde_json::Value::Array(
        timeline
            .iter()
            .map(|(at, event)| {
                let at_ms = millis(at.saturating_duration_since(started_at));
                match event {
                    WriteContentionTimelineEvent::AttemptStart => {
                        serde_json::json!({"event": "attempt_start", "at_ms": at_ms})
                    }
                    WriteContentionTimelineEvent::AttemptEnd => {
                        serde_json::json!({"event": "attempt_end", "at_ms": at_ms})
                    }
                    WriteContentionTimelineEvent::BackoffRequested(backoff) => serde_json::json!({
                        "event": "backoff_requested",
                        "at_ms": at_ms,
                        "backoff_ms": millis(*backoff),
                    }),
                    WriteContentionTimelineEvent::BackoffSlept => {
                        serde_json::json!({"event": "backoff_slept", "at_ms": at_ms})
                    }
                }
            })
            .collect(),
    )
}

struct StallMonitor {
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<Vec<(f64, f64)>>,
}

impl StallMonitor {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut gaps = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let before = Instant::now();
                    thread::sleep(STALL_MONITOR_TICK);
                    let late_ms = millis(before.elapsed()) - millis(STALL_MONITOR_TICK);
                    if late_ms >= STALL_MONITOR_REPORT_MS {
                        gaps.push((unix_ms_now(), late_ms));
                    }
                }
                gaps
            })
        };
        Self { stop, handle }
    }

    fn finish(self) -> Vec<(f64, f64)> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.join().expect("stall monitor should not panic")
    }
}

fn stall_gaps_json(gaps: &[(f64, f64)]) -> serde_json::Value {
    serde_json::Value::Array(
        gaps.iter()
            .map(|(ended_unix_ms, late_ms)| {
                serde_json::json!({"ended_unix_ms": ended_unix_ms, "late_ms": late_ms})
            })
            .collect(),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WriteOutcome {
    Inserted,
    AlreadyPresent,
    Locked(String),
    Other(String),
}

impl WriteOutcome {
    fn from_result(result: Result<bool>) -> Self {
        match result {
            Ok(true) => Self::Inserted,
            Ok(false) => Self::AlreadyPresent,
            Err(error) => {
                let message = format!("{error:#}");
                if message.contains(DATABASE_LOCKED_ERROR) {
                    Self::Locked(message)
                } else {
                    Self::Other(message)
                }
            }
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Inserted => "Ok(true)",
            Self::AlreadyPresent => "Ok(false)",
            Self::Locked(_) => "database is locked",
            Self::Other(_) => "other error",
        }
    }
}

fn unique_test_db_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!(
            "sce-lock-contention-{label}-{}-{nonce}",
            std::process::id()
        ))
        .join("agent-trace.db")
}

fn remove_test_db(db_path: &Path) {
    if let Some(parent) = db_path.parent() {
        fs::remove_dir_all(parent).expect("test DB directory should be removed");
    }
}

fn create_repository_db(db_path: &Path) {
    RepositoryAgentTraceDb::new_at(db_path).expect("repository DB should be created up front");
}

fn open_production_connection(db_path: &Path) -> RepositoryAgentTraceDb {
    RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(db_path)
        .expect("production hook connection should open")
}

fn conversation_text_event(
    session_id: &str,
    message_id: &str,
) -> (InsertMessageInsert, InsertPartInsert) {
    (
        InsertMessageInsert {
            session_id: session_id.to_string(),
            message_id: message_id.to_string(),
            role: MessageRole::User,
            generated_at_unix_ms: 1_000,
        },
        InsertPartInsert {
            part_type: PartType::Text,
            text: format!("text for {message_id}"),
            session_id: session_id.to_string(),
            message_id: message_id.to_string(),
            generated_at_unix_ms: 1_000,
        },
    )
}

fn session_row_count(db: &RepositoryAgentTraceDb, table: &str, session_id: &str) -> i64 {
    db.query_map(
        &format!("SELECT COUNT(*) FROM {table} WHERE session_id = ?1"),
        (session_id,),
        |row| row.get::<i64>(0).map_err(Into::into),
    )
    .expect("count query should succeed")
    .into_iter()
    .next()
    .expect("count row should exist")
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn env_writer_counts(default: &[usize]) -> Vec<usize> {
    std::env::var(WRITERS_ENV)
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|count| count.trim().parse().ok())
                .collect::<Vec<usize>>()
        })
        .filter(|counts| !counts.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

fn strict_mode() -> bool {
    std::env::var(STRICT_ENV).is_ok_and(|value| value == "1")
}

struct LockBoundarySample {
    hold: Duration,
    outcome: WriteOutcome,
    elapsed: Duration,
    holder_released_after: Duration,
    messages: i64,
    parts: i64,
    contention: WriteContentionCounts,
    timeline: serde_json::Value,
}

fn insert_while_write_lock_is_held(hold: Duration) -> LockBoundarySample {
    let db_path = unique_test_db_path(&format!("boundary-{}ms", hold.as_millis()));
    create_repository_db(&db_path);
    let session_id = "cx_lock-boundary";

    let lock_acquired = Arc::new(Barrier::new(2));
    let holder = {
        let db_path = db_path.clone();
        let lock_acquired = Arc::clone(&lock_acquired);
        thread::spawn(move || {
            let holder = open_production_connection(&db_path);
            holder
                .execute("BEGIN IMMEDIATE", ())
                .expect("holder should acquire the write lock");
            lock_acquired.wait();
            let held_since = Instant::now();
            thread::sleep(hold);
            holder
                .execute("COMMIT", ())
                .expect("holder should release the write lock");
            held_since.elapsed()
        })
    };

    let writer = open_production_connection(&db_path);
    lock_acquired.wait();
    let (message, part) = conversation_text_event(session_id, "cx:turn-1:user");
    let started_at = Instant::now();
    let ((result, contention), timeline) = record_write_contention_timeline(|| {
        count_write_contention(|| writer.insert_conversation_text_event(message, part))
    });
    let elapsed = started_at.elapsed();
    let outcome = WriteOutcome::from_result(result);
    let timeline = timeline_json(started_at, &timeline);

    let holder_released_after = holder.join().expect("holder thread should not panic");

    let verifier = open_production_connection(&db_path);
    let messages = session_row_count(&verifier, "messages", session_id);
    let parts = session_row_count(&verifier, "parts", session_id);
    drop((writer, verifier));
    remove_test_db(&db_path);

    LockBoundarySample {
        hold,
        outcome,
        elapsed,
        holder_released_after,
        messages,
        parts,
        contention,
        timeline,
    }
}

#[test]
fn lock_budget_boundary_characterizes_single_writer_blocked_by_begin_immediate_holder() {
    let samples: Vec<LockBoundarySample> = LOCK_HOLD_DURATIONS_MS
        .iter()
        .map(|hold_ms| insert_while_write_lock_is_held(Duration::from_millis(*hold_ms)))
        .collect();

    eprintln!(
        "\nlock-budget boundary (Agent Trace busy_timeout + write-contention retry + contention deadline)"
    );
    eprintln!(
        "hold_ms | outcome            | elapsed_ms | holder_released_ms | attempts | outer_retries | exhaustions | messages | parts"
    );
    for sample in &samples {
        eprintln!(
            "{:>7} | {:<18} | {:>10} | {:>18} | {:>8} | {:>13} | {:>11} | {:>8} | {:>5}",
            sample.hold.as_millis(),
            sample.outcome.label(),
            sample.elapsed.as_millis(),
            sample.holder_released_after.as_millis(),
            sample.contention.attempts,
            sample.contention.outer_retries,
            sample.contention.exhaustions,
            sample.messages,
            sample.parts,
        );
        if let WriteOutcome::Locked(message) | WriteOutcome::Other(message) = &sample.outcome {
            eprintln!("        error: {message}");
        }
        emit_measurement(&serde_json::json!({
            "kind": "boundary_sample",
            "hold_ms": sample.hold.as_millis(),
            "outcome": sample.outcome.label(),
            "error": match &sample.outcome {
                WriteOutcome::Locked(message) | WriteOutcome::Other(message) => Some(message),
                WriteOutcome::Inserted | WriteOutcome::AlreadyPresent => None,
            },
            "elapsed_ms": millis(sample.elapsed),
            "holder_released_ms": millis(sample.holder_released_after),
            "attempts": sample.contention.attempts,
            "outer_retries": sample.contention.outer_retries,
            "exhaustions": sample.contention.exhaustions,
            "messages": sample.messages,
            "parts": sample.parts,
            "timeline": sample.timeline,
        }));
    }

    for sample in &samples {
        let hold_ms = u64::try_from(sample.hold.as_millis()).expect("hold should fit in u64");
        assert_eq!(
            sample.messages, sample.parts,
            "hold {hold_ms}ms left a partial message/part pair"
        );
        match &sample.outcome {
            WriteOutcome::Inserted => assert_eq!(sample.messages, 1),
            WriteOutcome::Locked(_) | WriteOutcome::Other(_) => assert_eq!(sample.messages, 0),
            WriteOutcome::AlreadyPresent => panic!("hold {hold_ms}ms reported a phantom replay"),
        }
        assert!(
            (1..=WRITE_CONTENTION_MAX_ATTEMPTS).contains(&sample.contention.attempts),
            "hold {hold_ms}ms made {} attempt(s), outside the contention policy",
            sample.contention.attempts
        );
        assert_eq!(
            sample.contention.outer_retries + 1,
            sample.contention.attempts,
            "hold {hold_ms}ms: every attempt after the first must be an admitted outer retry"
        );
        if hold_ms <= RELIABLY_WITHIN_CONTENTION_BUDGET_MS {
            assert_eq!(
                sample.outcome,
                WriteOutcome::Inserted,
                "a {hold_ms}ms lock is well inside the contention budget"
            );
            assert_eq!(
                sample.contention.exhaustions, 0,
                "a {hold_ms}ms lock must not exhaust the contention policy"
            );
        }
        if hold_ms >= RELIABLY_BEYOND_CONTENTION_BUDGET_MS {
            match &sample.outcome {
                WriteOutcome::Locked(message) => assert!(
                    message.contains(WRITE_CONTENTION_ERROR),
                    "a {hold_ms}ms lock should fail with a contention-exhaustion error, got {message}"
                ),
                outcome => panic!(
                    "a {hold_ms}ms lock is expected to exhaust the contention policy, got {outcome:?}"
                ),
            }
            assert_eq!(
                sample.contention.exhaustions, 1,
                "a {hold_ms}ms lock should exhaust the contention policy exactly once"
            );
        }
    }
}

#[test]
fn busy_timeout_production_insert_waits_for_begin_immediate_holder() {
    let hold = Duration::from_millis(BUSY_TIMEOUT_PRODUCTION_HOLD_MS);

    let sample = insert_while_write_lock_is_held(hold);

    assert_eq!(
        sample.outcome,
        WriteOutcome::Inserted,
        "insert_conversation_text_event should wait out a {}ms holder",
        hold.as_millis()
    );
    assert_eq!((sample.messages, sample.parts), (1, 1));
    assert!(
        sample.elapsed >= hold / 2,
        "the insert should have waited for the holder, took {:?}",
        sample.elapsed
    );
}

#[test]
fn agent_trace_db_write_contention_retry_hundred_ms_hold_succeeds_on_the_first_attempt() {
    let hold = Duration::from_millis(BUSY_TIMEOUT_PRODUCTION_HOLD_MS);

    let (sample, counts) = count_write_contention(|| insert_while_write_lock_is_held(hold));

    assert_eq!(sample.outcome, WriteOutcome::Inserted);
    assert_eq!((sample.messages, sample.parts), (1, 1));
    assert_eq!(
        (counts.attempts, counts.outer_retries, counts.exhaustions),
        (1, 0, 0),
        "Turso's busy timeout should absorb a {}ms hold without an outer retry",
        hold.as_millis()
    );
}

struct LatencyPercentiles {
    p50: Duration,
    p95: Duration,
    p99: Duration,
    max: Duration,
}

fn nearest_rank(sorted: &[Duration], percentile: usize) -> Duration {
    let rank = (percentile * sorted.len()).div_ceil(100).max(1);
    sorted[rank - 1]
}

fn latency_percentiles(latencies: &[Duration]) -> LatencyPercentiles {
    if latencies.is_empty() {
        return LatencyPercentiles {
            p50: Duration::ZERO,
            p95: Duration::ZERO,
            p99: Duration::ZERO,
            max: Duration::ZERO,
        };
    }
    let mut sorted = latencies.to_vec();
    sorted.sort_unstable();
    LatencyPercentiles {
        p50: nearest_rank(&sorted, 50),
        p95: nearest_rank(&sorted, 95),
        p99: nearest_rank(&sorted, 99),
        max: sorted[sorted.len() - 1],
    }
}

#[test]
fn lock_contention_latency_percentiles_use_nearest_rank() {
    let latencies: Vec<Duration> = (1..=100).rev().map(Duration::from_millis).collect();
    let percentiles = latency_percentiles(&latencies);
    assert_eq!(percentiles.p50, Duration::from_millis(50));
    assert_eq!(percentiles.p95, Duration::from_millis(95));
    assert_eq!(percentiles.p99, Duration::from_millis(99));
    assert_eq!(percentiles.max, Duration::from_millis(100));

    let single = latency_percentiles(&[Duration::from_millis(7)]);
    assert_eq!(single.p50, Duration::from_millis(7));
    assert_eq!(single.p99, Duration::from_millis(7));

    let empty = latency_percentiles(&[]);
    assert_eq!(empty.max, Duration::ZERO);
}

struct SlowOperation {
    writer_index: usize,
    started_unix_ms: f64,
    elapsed: Duration,
    outcome: WriteOutcome,
    contention: WriteContentionCounts,
    timeline: serde_json::Value,
}

struct RoundResult {
    slow_operations: Vec<SlowOperation>,
    outcomes: Vec<WriteOutcome>,
    latencies: Vec<Duration>,
    contention: Vec<WriteContentionCounts>,
    max_elapsed: Duration,
    messages: i64,
    parts: i64,
}

fn run_concurrent_round(
    db_path: &Path,
    writers: usize,
    session_id: &str,
    distinct_events: bool,
) -> RoundResult {
    let start_together = Arc::new(Barrier::new(writers));
    let handles: Vec<_> = (0..writers)
        .map(|writer_index| {
            let db_path = db_path.to_path_buf();
            let start_together = Arc::clone(&start_together);
            let session_id = session_id.to_string();
            thread::spawn(move || {
                let db = open_production_connection(&db_path);
                let message_id = if distinct_events {
                    format!("cx:turn-{writer_index}:user")
                } else {
                    String::from("cx:turn-shared:user")
                };
                let (message, part) = conversation_text_event(&session_id, &message_id);
                start_together.wait();
                let started_unix_ms = unix_ms_now();
                let started_at = Instant::now();
                let ((result, contention), timeline) = record_write_contention_timeline(|| {
                    count_write_contention(|| db.insert_conversation_text_event(message, part))
                });
                let elapsed = started_at.elapsed();
                let outcome = WriteOutcome::from_result(result);
                let slow = (millis(elapsed) >= SLOW_OPERATION_MS).then(|| SlowOperation {
                    writer_index,
                    started_unix_ms,
                    elapsed,
                    outcome: outcome.clone(),
                    contention,
                    timeline: timeline_json(started_at, &timeline),
                });
                (outcome, elapsed, contention, slow)
            })
        })
        .collect();

    let mut slow_operations = Vec::new();
    let mut outcomes = Vec::with_capacity(writers);
    let mut latencies = Vec::with_capacity(writers);
    let mut contention = Vec::with_capacity(writers);
    let mut max_elapsed = Duration::ZERO;
    for handle in handles {
        let (outcome, elapsed, counts, slow) =
            handle.join().expect("writer thread should not panic");
        slow_operations.extend(slow);
        outcomes.push(outcome);
        latencies.push(elapsed);
        contention.push(counts);
        max_elapsed = max_elapsed.max(elapsed);
    }

    let verifier = open_production_connection(db_path);
    RoundResult {
        slow_operations,
        outcomes,
        latencies,
        contention,
        max_elapsed,
        messages: session_row_count(&verifier, "messages", session_id),
        parts: session_row_count(&verifier, "parts", session_id),
    }
}

#[derive(Default)]
struct LevelSummary {
    writers: usize,
    rounds: usize,
    clean_rounds: usize,
    rounds_with_lock_exhaustion: usize,
    inserted: usize,
    already_present: usize,
    lock_errors: usize,
    other_errors: usize,
    lost_events: i64,
    orphan_rows: i64,
    duplicate_rows: i64,
    persisted_rows: i64,
    attempts: u64,
    outer_retries: u64,
    exhaustions: u64,
    max_elapsed: Duration,
    latencies: Vec<Duration>,
    first_lock_error: Option<String>,
    first_other_error: Option<String>,
}

impl LevelSummary {
    fn total_errors(&self) -> usize {
        self.lock_errors + self.other_errors
    }
}

#[allow(clippy::too_many_lines)]
fn run_contention_level(writers: usize, rounds: usize, distinct_events: bool) -> LevelSummary {
    let mode = if distinct_events {
        "distinct"
    } else {
        "duplicate"
    };
    let db_path = unique_test_db_path(&format!("{mode}-{writers}w"));
    create_repository_db(&db_path);

    let mut summary = LevelSummary {
        writers,
        rounds,
        ..LevelSummary::default()
    };

    let monitor = StallMonitor::start();
    let level_started_unix_ms = unix_ms_now();
    for round in 0..rounds {
        let session_id = format!("cx_{mode}-{writers}w-round-{round}");
        let result = run_concurrent_round(&db_path, writers, &session_id, distinct_events);

        for slow in &result.slow_operations {
            emit_measurement(&serde_json::json!({
                "kind": "slow_operation",
                "mode": mode,
                "writers": writers,
                "round": round,
                "writer_index": slow.writer_index,
                "started_unix_ms": slow.started_unix_ms,
                "elapsed_ms": millis(slow.elapsed),
                "outcome": slow.outcome.label(),
                "attempts": slow.contention.attempts,
                "outer_retries": slow.contention.outer_retries,
                "exhaustions": slow.contention.exhaustions,
                "timeline": slow.timeline,
            }));
        }
        summary.orphan_rows += (result.messages - result.parts).abs();
        let expected_max_rows = if distinct_events { writers } else { 1 };
        summary.duplicate_rows +=
            (result.messages - i64::try_from(expected_max_rows).expect("count fits i64")).max(0);
        summary.persisted_rows += result.messages;

        assert_eq!(
            result.messages, result.parts,
            "{mode} round {round} with {writers} writers left orphaned rows"
        );

        let inserted = result
            .outcomes
            .iter()
            .filter(|outcome| **outcome == WriteOutcome::Inserted)
            .count();
        let already_present = result
            .outcomes
            .iter()
            .filter(|outcome| **outcome == WriteOutcome::AlreadyPresent)
            .count();
        let mut round_lock_errors = 0;
        let mut round_other_errors = 0;
        for outcome in &result.outcomes {
            match outcome {
                WriteOutcome::Locked(message) => {
                    round_lock_errors += 1;
                    summary
                        .first_lock_error
                        .get_or_insert_with(|| message.clone());
                }
                WriteOutcome::Other(message) => {
                    round_other_errors += 1;
                    summary
                        .first_other_error
                        .get_or_insert_with(|| message.clone());
                }
                WriteOutcome::Inserted | WriteOutcome::AlreadyPresent => {}
            }
        }

        assert_eq!(
            i64::try_from(inserted).expect("count fits i64"),
            result.messages,
            "{mode} round {round}: Ok(true) count must equal persisted message rows"
        );

        if distinct_events {
            assert_eq!(
                already_present, 0,
                "distinct events must never report Ok(false)"
            );
            summary.lost_events +=
                i64::try_from(writers).expect("count fits i64") - result.messages;
        } else {
            assert!(inserted <= 1, "duplicate delivery inserted more than once");
            if round_lock_errors + round_other_errors < writers {
                assert_eq!(
                    inserted, 1,
                    "a duplicate round with at least one completed writer must persist the event"
                );
            }
        }

        let expected_rows = if distinct_events { writers } else { 1 };
        if round_lock_errors + round_other_errors == 0
            && usize::try_from(result.messages).expect("count fits usize") == expected_rows
        {
            summary.clean_rounds += 1;
        }
        if round_lock_errors > 0 {
            summary.rounds_with_lock_exhaustion += 1;
        }
        summary.inserted += inserted;
        summary.already_present += already_present;
        summary.lock_errors += round_lock_errors;
        summary.other_errors += round_other_errors;
        for counts in &result.contention {
            assert!(
                counts.attempts <= WRITE_CONTENTION_MAX_ATTEMPTS,
                "{mode} round {round}: a writer made {} attempt(s), beyond the contention policy",
                counts.attempts
            );
            summary.attempts += u64::from(counts.attempts);
            summary.outer_retries += u64::from(counts.outer_retries);
            summary.exhaustions += u64::from(counts.exhaustions);
        }
        summary.max_elapsed = summary.max_elapsed.max(result.max_elapsed);
        summary.latencies.extend(result.latencies);
    }

    let stall_gaps = monitor.finish();
    let latency = latency_percentiles(&summary.latencies);
    emit_measurement(&serde_json::json!({
        "kind": "concurrent_level",
        "mode": mode,
        "writers": writers,
        "rounds": rounds,
        "started_unix_ms": level_started_unix_ms,
        "ended_unix_ms": unix_ms_now(),
        "total_writes": writers * rounds,
        "expected_rows": if distinct_events { writers * rounds } else { rounds },
        "persisted_rows": summary.persisted_rows,
        "lost_events": summary.lost_events,
        "lock_errors": summary.lock_errors,
        "other_errors": summary.other_errors,
        "orphan_rows": summary.orphan_rows,
        "duplicate_rows": summary.duplicate_rows,
        "inserted": summary.inserted,
        "already_present": summary.already_present,
        "attempts": summary.attempts,
        "outer_retries": summary.outer_retries,
        "exhaustions": summary.exhaustions,
        "p50_ms": millis(latency.p50),
        "p95_ms": millis(latency.p95),
        "p99_ms": millis(latency.p99),
        "max_ms": millis(latency.max),
        "first_lock_error": summary.first_lock_error,
        "first_other_error": summary.first_other_error,
        "stall_gaps": stall_gaps_json(&stall_gaps),
    }));

    remove_test_db(&db_path);
    summary
}

fn report_levels(title: &str, summaries: &[LevelSummary]) {
    eprintln!("\n{title}");
    eprintln!(
        "writers | rounds | clean rounds | rounds w/ lock exhaustion | lock errors | other errors | Ok(true) | Ok(false) | lost events | attempts | outer retries | exhaustions | max elapsed ms | p50 ms | p95 ms | p99 ms | max ms"
    );
    for summary in summaries {
        let latency = latency_percentiles(&summary.latencies);
        eprintln!(
            "{:>7} | {:>6} | {:>12} | {:>25} | {:>11} | {:>12} | {:>8} | {:>9} | {:>11} | {:>8} | {:>13} | {:>11} | {:>14} | {:>6} | {:>6} | {:>6} | {:>6}",
            summary.writers,
            summary.rounds,
            summary.clean_rounds,
            summary.rounds_with_lock_exhaustion,
            summary.lock_errors,
            summary.other_errors,
            summary.inserted,
            summary.already_present,
            summary.lost_events,
            summary.attempts,
            summary.outer_retries,
            summary.exhaustions,
            summary.max_elapsed.as_millis(),
            latency.p50.as_millis(),
            latency.p95.as_millis(),
            latency.p99.as_millis(),
            latency.max.as_millis(),
        );
    }
    for summary in summaries {
        if let Some(message) = &summary.first_lock_error {
            eprintln!("{} writers first lock error: {message}", summary.writers);
        }
        if let Some(message) = &summary.first_other_error {
            eprintln!("{} writers first other error: {message}", summary.writers);
        }
    }
}

fn assert_strict_if_requested(summaries: &[LevelSummary]) {
    if !strict_mode() {
        return;
    }
    for summary in summaries {
        assert_eq!(
            summary.total_errors(),
            0,
            "{} concurrent writers produced {} lock error(s) and {} other error(s)",
            summary.writers,
            summary.lock_errors,
            summary.other_errors
        );
        assert_eq!(
            summary.lost_events, 0,
            "{} concurrent writers lost {} distinct event(s)",
            summary.writers, summary.lost_events
        );
        assert_eq!(
            summary.exhaustions, 0,
            "{} concurrent writers exhausted the contention policy {} time(s)",
            summary.writers, summary.exhaustions
        );
    }
}

#[test]
#[ignore = "lock-contention characterization; run with --ignored --nocapture"]
fn concurrent_duplicate_delivery_persists_each_event_once_under_write_contention() {
    let rounds = env_usize(ROUNDS_ENV, DEFAULT_ROUNDS);
    let summaries: Vec<LevelSummary> = env_writer_counts(DEFAULT_DUPLICATE_WRITER_COUNTS)
        .into_iter()
        .map(|writers| run_contention_level(writers, rounds, false))
        .collect();

    report_levels(
        "concurrent duplicate delivery (same logical event, Agent Trace write-contention policy)",
        &summaries,
    );
    assert_strict_if_requested(&summaries);
}

#[test]
#[ignore = "lock-contention characterization; run with --ignored --nocapture"]
fn concurrent_distinct_events_persist_every_event_under_write_contention() {
    let rounds = env_usize(ROUNDS_ENV, DEFAULT_ROUNDS);
    let summaries: Vec<LevelSummary> = env_writer_counts(DEFAULT_DISTINCT_WRITER_COUNTS)
        .into_iter()
        .map(|writers| run_contention_level(writers, rounds, true))
        .collect();

    report_levels(
        "concurrent distinct events (one unique event per writer, Agent Trace write-contention policy)",
        &summaries,
    );
    assert_strict_if_requested(&summaries);
}

const SCE_BIN_ENV: &str = "SCE_BIN";

struct HookLevelSummary {
    writers: usize,
    expected: i64,
    persisted_messages: i64,
    persisted_parts: i64,
    nonzero_exits: usize,
}

fn assert_hook_levels(levels: &[HookLevelSummary], strict: bool) {
    for level in levels {
        assert!(
            level.persisted_messages <= level.expected,
            "{} hook writers persisted {} messages for {} distinct events",
            level.writers,
            level.persisted_messages,
            level.expected
        );
        assert!(
            level.persisted_parts <= level.expected,
            "{} hook writers persisted {} parts for {} distinct events",
            level.writers,
            level.persisted_parts,
            level.expected
        );
        if strict {
            assert_eq!(
                level.persisted_messages, level.expected,
                "{} hook writers lost message rows",
                level.writers
            );
            assert_eq!(
                level.persisted_parts, level.expected,
                "{} hook writers lost part rows",
                level.writers
            );
            assert_eq!(
                level.nonzero_exits, 0,
                "{} hook writers had non-zero exits",
                level.writers
            );
        }
    }

    if strict {
        let total_lost: i64 = levels
            .iter()
            .map(|level| level.expected - level.persisted_messages)
            .sum();
        let total_nonzero_exits: usize = levels.iter().map(|level| level.nonzero_exits).sum();
        assert_eq!(total_lost, 0, "real hook processes lost distinct events");
        assert_eq!(
            total_nonzero_exits, 0,
            "real hook processes exited with a non-zero status"
        );
    }
}

#[test]
fn lock_contention_hook_level_assertions_reject_over_persistence() {
    let over_persisted = [HookLevelSummary {
        writers: 2,
        expected: 4,
        persisted_messages: 5,
        persisted_parts: 5,
        nonzero_exits: 0,
    }];
    let result = std::panic::catch_unwind(|| assert_hook_levels(&over_persisted, false));
    assert!(
        result.is_err(),
        "over-persistence must fail even outside strict mode"
    );
}

#[test]
fn lock_contention_hook_level_assertions_enforce_strict_loss_and_exit_status() {
    let lost = [HookLevelSummary {
        writers: 2,
        expected: 4,
        persisted_messages: 3,
        persisted_parts: 3,
        nonzero_exits: 0,
    }];
    assert_hook_levels(&lost, false);
    assert!(std::panic::catch_unwind(|| assert_hook_levels(&lost, true)).is_err());

    let nonzero_exit = [HookLevelSummary {
        writers: 2,
        expected: 4,
        persisted_messages: 4,
        persisted_parts: 4,
        nonzero_exits: 1,
    }];
    assert_hook_levels(&nonzero_exit, false);
    assert!(std::panic::catch_unwind(|| assert_hook_levels(&nonzero_exit, true)).is_err());

    let clean = [HookLevelSummary {
        writers: 3,
        expected: 6,
        persisted_messages: 6,
        persisted_parts: 6,
        nonzero_exits: 0,
    }];
    assert_hook_levels(&clean, true);
}
const DEFAULT_PROCESS_ROUNDS: usize = 50;
const DEFAULT_PROCESS_WRITER_COUNTS: &[usize] = &[2, 3, 4];

struct HookRound {
    nonzero_exits: usize,
    stderr_lines: Vec<String>,
    latencies: Vec<Duration>,
    slow_processes: Vec<(usize, f64, Duration)>,
}

struct HookProcessHarness {
    sce: PathBuf,
    work: PathBuf,
    repo: PathBuf,
    db_path: PathBuf,
}

impl HookProcessHarness {
    fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(&self.sce);
        command
            .current_dir(&self.repo)
            .env("XDG_STATE_HOME", self.work.join("state"))
            .env("XDG_CONFIG_HOME", self.work.join("config"))
            .env("XDG_DATA_HOME", self.work.join("data"));
        command
    }

    fn create(sce: PathBuf, label: &str) -> Self {
        let work = unique_test_db_path(label)
            .parent()
            .expect("unique path has a parent")
            .to_path_buf();
        let repo = work.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/sce/lock-contention.git",
            ],
            vec![
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        ] {
            let status = std::process::Command::new("git")
                .args(&args)
                .current_dir(&repo)
                .status()
                .expect("git should spawn");
            assert!(status.success(), "git {args:?} should succeed");
        }

        let mut harness = Self {
            sce,
            work,
            repo,
            db_path: PathBuf::new(),
        };

        let setup = harness
            .command()
            .args(["setup", "--codex", "--non-interactive", "--hooks"])
            .output()
            .expect("sce setup should spawn");
        assert!(
            setup.status.success(),
            "sce setup failed: {}",
            String::from_utf8_lossy(&setup.stderr)
        );

        let doctor = harness
            .command()
            .args(["doctor", "--format", "json"])
            .output()
            .expect("sce doctor should spawn");
        let report: serde_json::Value =
            serde_json::from_slice(&doctor.stdout).expect("doctor should print JSON");
        harness.db_path = PathBuf::from(
            report["agent_trace_db"]["path"]
                .as_str()
                .expect("doctor should report agent_trace_db.path"),
        );
        assert!(
            harness.db_path.is_file(),
            "repository Agent Trace DB should exist"
        );
        harness
    }

    fn run_round(&self, writers: usize, session_id: &str) -> HookRound {
        use std::io::Write;
        use std::process::Stdio;

        let children: Vec<_> = (0..writers)
            .map(|_| {
                self.command()
                    .args(["hooks", "codex"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("sce hooks codex should spawn")
            })
            .collect();

        let release = Arc::new(Barrier::new(writers));
        let feeders: Vec<_> = children
            .into_iter()
            .enumerate()
            .map(|(writer_index, mut child)| {
                let mut stdin = child.stdin.take().expect("stdin should be piped");
                let release = Arc::clone(&release);
                let payload = serde_json::json!({
                    "hook_event_name": "UserPromptSubmit",
                    "session_id": session_id,
                    "turn_id": format!("w{writer_index}"),
                    "prompt": format!("prompt {session_id} w{writer_index}"),
                })
                .to_string();
                thread::spawn(move || {
                    release.wait();
                    let started_unix_ms = unix_ms_now();
                    let started_at = Instant::now();
                    stdin
                        .write_all(payload.as_bytes())
                        .expect("payload should be written");
                    drop(stdin);
                    let output = child
                        .wait_with_output()
                        .expect("hook process should finish");
                    (output, started_unix_ms, started_at.elapsed())
                })
            })
            .collect();

        let mut nonzero_exits = 0;
        let mut stderr_lines = Vec::new();
        let mut latencies = Vec::with_capacity(writers);
        let mut slow_processes = Vec::new();
        for (writer_index, feeder) in feeders.into_iter().enumerate() {
            let (output, started_unix_ms, elapsed) =
                feeder.join().expect("feeder thread should not panic");
            latencies.push(elapsed);
            if millis(elapsed) >= SLOW_OPERATION_MS {
                slow_processes.push((writer_index, started_unix_ms, elapsed));
            }
            if !output.status.success() {
                nonzero_exits += 1;
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if !stderr.is_empty() {
                stderr_lines.push(stderr);
            }
        }
        HookRound {
            nonzero_exits,
            stderr_lines,
            latencies,
            slow_processes,
        }
    }
}

#[test]
#[ignore = "end-to-end hook-process lock contention; set SCE_BIN and run with --ignored --nocapture"]
#[allow(clippy::too_many_lines)]
fn concurrent_real_codex_hook_processes_persist_every_distinct_event() {
    let Some(sce) = std::env::var_os(SCE_BIN_ENV).map(PathBuf::from) else {
        eprintln!("{SCE_BIN_ENV} is not set; skipping end-to-end hook-process reproduction");
        return;
    };
    let rounds = env_usize(ROUNDS_ENV, DEFAULT_PROCESS_ROUNDS);

    eprintln!("\nreal `sce hooks codex` UserPromptSubmit processes (distinct events)");
    eprintln!(
        "writers | rounds | expected | persisted msgs | persisted parts | lost | rounds w/ loss | non-zero exits | stderr lines | p50 ms | p95 ms | p99 ms | max ms"
    );
    let mut levels = Vec::new();
    for writers in env_writer_counts(DEFAULT_PROCESS_WRITER_COUNTS) {
        let harness = HookProcessHarness::create(sce.clone(), &format!("hooks-{writers}w"));
        let mut persisted_messages = 0;
        let mut persisted_parts = 0;
        let mut rounds_with_loss = 0;
        let mut nonzero_exits = 0;
        let mut stderr_samples = Vec::new();
        let mut latencies = Vec::with_capacity(writers * rounds);
        let mut fail_open_lost = 0;
        let monitor = StallMonitor::start();
        let level_started_unix_ms = unix_ms_now();

        for round in 0..rounds {
            let session_id = format!("lock-contention-{writers}w-r{round}");
            let HookRound {
                nonzero_exits: round_nonzero,
                stderr_lines: round_stderr,
                latencies: round_latencies,
                slow_processes,
            } = harness.run_round(writers, &session_id);
            for (writer_index, started_unix_ms, elapsed) in slow_processes {
                emit_measurement(&serde_json::json!({
                    "kind": "slow_hook_process",
                    "writers": writers,
                    "round": round,
                    "writer_index": writer_index,
                    "started_unix_ms": started_unix_ms,
                    "elapsed_ms": millis(elapsed),
                }));
            }
            for stderr in &round_stderr {
                emit_measurement(&serde_json::json!({
                    "kind": "hook_stderr",
                    "writers": writers,
                    "round": round,
                    "stderr": stderr,
                }));
            }
            nonzero_exits += round_nonzero;
            stderr_samples.extend(round_stderr);
            latencies.extend(round_latencies);

            let verifier = open_production_connection(&harness.db_path);
            let prefixed = format!("cx_{session_id}");
            let messages = session_row_count(&verifier, "messages", &prefixed);
            let parts = session_row_count(&verifier, "parts", &prefixed);
            assert_eq!(messages, parts, "hook round left orphaned rows");
            if usize::try_from(messages).expect("count fits usize") != writers {
                rounds_with_loss += 1;
                let round_lost = i64::try_from(writers).expect("count fits i64") - messages;
                let round_fail_open =
                    (round_lost - i64::try_from(round_nonzero).expect("count fits i64")).max(0);
                fail_open_lost += round_fail_open;
                emit_measurement(&serde_json::json!({
                    "kind": "hook_round_loss",
                    "writers": writers,
                    "round": round,
                    "lost": round_lost,
                    "nonzero_exits": round_nonzero,
                    "fail_open_lost": round_fail_open,
                    "ended_unix_ms": unix_ms_now(),
                }));
            }
            persisted_messages += messages;
            persisted_parts += parts;
        }

        let expected = i64::try_from(writers * rounds).expect("count fits i64");
        let lost = expected - persisted_messages;
        let latency = latency_percentiles(&latencies);
        eprintln!(
            "{:>7} | {:>6} | {:>8} | {:>14} | {:>15} | {:>4} | {:>14} | {:>14} | {:>12} | {:>6} | {:>6} | {:>6} | {:>6}",
            writers,
            rounds,
            expected,
            persisted_messages,
            persisted_parts,
            lost,
            rounds_with_loss,
            nonzero_exits,
            stderr_samples.len(),
            latency.p50.as_millis(),
            latency.p95.as_millis(),
            latency.p99.as_millis(),
            latency.max.as_millis(),
        );
        if let Some(sample) = stderr_samples.first() {
            eprintln!("        first stderr: {sample}");
        }
        let stall_gaps = monitor.finish();
        emit_measurement(&serde_json::json!({
            "kind": "hook_level",
            "writers": writers,
            "rounds": rounds,
            "started_unix_ms": level_started_unix_ms,
            "ended_unix_ms": unix_ms_now(),
            "expected": expected,
            "persisted_messages": persisted_messages,
            "persisted_parts": persisted_parts,
            "lost": lost,
            "fail_open_lost": fail_open_lost,
            "rounds_with_loss": rounds_with_loss,
            "nonzero_exits": nonzero_exits,
            "stderr_outputs": stderr_samples.len(),
            "stderr_lines": stderr_samples.iter().map(|sample| sample.lines().count()).sum::<usize>(),
            "p50_ms": millis(latency.p50),
            "p95_ms": millis(latency.p95),
            "p99_ms": millis(latency.p99),
            "max_ms": millis(latency.max),
            "stall_gaps": stall_gaps_json(&stall_gaps),
        }));
        fs::remove_dir_all(&harness.work).expect("hook harness dir should be removed");
        levels.push(HookLevelSummary {
            writers,
            expected,
            persisted_messages,
            persisted_parts,
            nonzero_exits,
        });
    }

    assert_hook_levels(&levels, strict_mode());
}

#[test]
fn initialized_hook_open_succeeds_while_write_lock_is_held() {
    let db_path = unique_test_db_path("hook-open-metadata");
    create_repository_db(&db_path);
    let repository_id = "lock-contention-repository";
    let initialized = open_production_connection(&db_path)
        .verify_or_initialize_repository_metadata(repository_id)
        .expect("metadata should initialize before contention");

    let hold = Duration::from_millis(RELIABLY_BEYOND_CONTENTION_BUDGET_MS);
    let lock_acquired = Arc::new(Barrier::new(2));
    let holder = {
        let db_path = db_path.clone();
        let lock_acquired = Arc::clone(&lock_acquired);
        thread::spawn(move || {
            let holder = open_production_connection(&db_path);
            holder
                .execute("BEGIN IMMEDIATE", ())
                .expect("holder should acquire the write lock");
            lock_acquired.wait();
            thread::sleep(hold);
            holder
                .execute("COMMIT", ())
                .expect("holder should release the write lock");
        })
    };

    let hook = open_production_connection(&db_path);
    lock_acquired.wait();
    let started_at = Instant::now();
    let ((schema_ready, metadata), writes) = count_write_statements(|| {
        (
            hook.ensure_schema_ready_for_hooks(),
            hook.verify_or_initialize_repository_metadata(repository_id),
        )
    });
    let elapsed = started_at.elapsed();
    holder.join().expect("holder thread should not panic");

    eprintln!(
        "\nhook-open under a {}ms write lock: schema_ready={:?} metadata={} writes={writes} after {}ms",
        hold.as_millis(),
        schema_ready.as_ref().map_err(|error| format!("{error:#}")),
        match &metadata {
            Ok(_) => String::from("Ok"),
            Err(error) => format!("Err({error:#})"),
        },
        elapsed.as_millis()
    );

    assert!(
        schema_ready.is_ok(),
        "the read-only schema check should not need the write lock"
    );
    let metadata = metadata.expect("an initialized hook open should not need the write lock");
    assert_eq!(metadata, initialized);
    assert_eq!(writes, 0, "an initialized hook open must issue no writes");
    assert!(
        elapsed < hold,
        "the initialized hook open must not wait for the held write lock"
    );
    drop(hook);
    remove_test_db(&db_path);
}
