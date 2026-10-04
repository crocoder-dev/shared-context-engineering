//! Shared Turso database infrastructure.
//!
//! Provides a generic `TursoDb` adapter that wraps Turso connection
//! management, tokio runtime bridging, and embedded migration execution for
//! service-specific database specs.

use std::{
    fs,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use turso::Value as TursoValue;

use crate::services::config::{AgentTraceDbRetryConfig, DatabaseRetryConfig};
use crate::services::lifecycle::{
    HealthCategory, HealthFixability, HealthProblem, HealthProblemKind, HealthSeverity,
};
use crate::services::resilience::{run_with_retry_sync, RetryPolicy};

const MIGRATIONS_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS __sce_migrations (
    id TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)";
const SELECT_MIGRATION_SQL: &str = "SELECT id FROM __sce_migrations WHERE id = ?1 LIMIT 1";
const INSERT_MIGRATION_SQL: &str = "INSERT INTO __sce_migrations (id) VALUES (?1)";
const ENCRYPTION_CIPHER_AEGIS256: &str = "aegis256";
const CONNECTION_OPEN_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 3,
    timeout_ms: 1_000,
    initial_backoff_ms: 25,
    max_backoff_ms: 200,
};
const CONNECTION_OPEN_RETRY_HINT: &str = "retry after the database lock clears; if the issue persists, stop other SCE processes using this database and rerun the command";
const QUERY_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 5,
    timeout_ms: 200,
    initial_backoff_ms: 25,
    max_backoff_ms: 100,
};
const QUERY_RETRY_HINT: &str = "retry after the database lock clears; if the issue persists, stop other SCE processes using this database and rerun the command";
const AGENT_TRACE_DB_CONFIG_KEY: &str = "agent_trace_db";
const AGENT_TRACE_DB_BUSY_TIMEOUT_MS: u64 = 500;
const AGENT_TRACE_DB_CONTENTION_DEADLINE_MS: u64 = 1_250;
const AGENT_TRACE_DB_WRITE_CONTENTION_MAX_ATTEMPTS: u32 = 2;
const AGENT_TRACE_DB_WRITE_CONTENTION_BACKOFF_CAP_MS: u64 = 100;

pub mod encryption_key;

/// Service-specific Turso database configuration.
pub trait DbSpec {
    /// Human-readable database name used in diagnostics.
    fn db_name() -> &'static str;

    /// Canonical database file path.
    fn db_path() -> Result<PathBuf>;

    /// Ordered embedded migration SQL files as `(id, sql)` pairs.
    fn migrations() -> &'static [(&'static str, &'static str)];

    /// Config-file lookup key under `policies.database_retry`.
    /// One of `"local_db"`, `"agent_trace_db"`, `"auth_db"`.
    fn db_config_key() -> &'static str;
}

/// Collect common filesystem health problems for a Turso database path.
pub fn collect_db_path_health(db_name: &str, db_path: &Path, problems: &mut Vec<HealthProblem>) {
    let db_name_title = sentence_case(db_name);

    let Some(parent) = db_path.parent() else {
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveStateRoot,
            category: HealthCategory::GlobalState,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
            summary: format!(
                "Unable to resolve parent directory for {db_name} path '{}'.",
                db_path.display()
            ),
            remediation: String::from("Verify that the current platform exposes a writable SCE state directory before rerunning 'sce doctor'."),
            next_action: "manual_steps",
        });
        return;
    };

    if !parent.exists() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveStateRoot,
            category: HealthCategory::GlobalState,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::AutoFixable,
            summary: format!(
                "{db_name_title} parent directory '{}' does not exist.",
                parent.display()
            ),
            remediation: format!(
                "Run 'sce doctor --fix' to create the canonical {db_name} parent directory at '{}'.",
                parent.display()
            ),
            next_action: "doctor_fix",
        });
    } else if !parent.is_dir() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveStateRoot,
            category: HealthCategory::GlobalState,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
            summary: format!(
                "{db_name_title} parent path '{}' is not a directory.",
                parent.display()
            ),
            remediation: format!(
                "Replace '{}' with a writable directory before rerunning 'sce doctor'.",
                parent.display()
            ),
            next_action: "manual_steps",
        });
    }

    if db_path.exists() && !db_path.is_file() {
        problems.push(HealthProblem {
            kind: HealthProblemKind::UnableToResolveStateRoot,
            category: HealthCategory::GlobalState,
            severity: HealthSeverity::Error,
            fixability: HealthFixability::ManualOnly,
            summary: format!(
                "{db_name_title} path '{}' is not a file.",
                db_path.display()
            ),
            remediation: format!(
                "Replace '{}' with a writable {db_name} file path before rerunning 'sce doctor'.",
                db_path.display()
            ),
            next_action: "manual_steps",
        });
    }
}

/// Create the parent directory for a Turso database path.
pub fn bootstrap_db_parent(db_name: &str, db_path: &Path) -> Result<PathBuf> {
    let parent = db_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{db_name} path has no parent: {}", db_path.display()))?;

    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create {db_name} parent directory: {}",
            parent.display()
        )
    })?;

    Ok(parent.to_path_buf())
}

fn sentence_case(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    first.to_uppercase().collect::<String>() + chars.as_str()
}

fn ensure_db_parent_dir(db_name: &str, db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create {db_name} parent directory: {}",
                parent.display()
            )
        })?;
    }

    Ok(())
}

fn build_current_thread_runtime(db_name: &str) -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .with_context(|| {
            format!("failed to create {db_name} tokio runtime. Try: rerun the command; if the issue persists, verify the local Tokio runtime environment.")
        })
}

/// Drives `fut` to completion on `runtime`, isolating it on a dedicated
/// thread when the calling thread already has an active Tokio runtime
/// context.
///
/// `Runtime::block_on` panics ("Cannot start a runtime from within a
/// runtime") if invoked directly from a thread that is already driving
/// another runtime, which happens when async callers (for example, Agent
/// Trace sync's control-plane client) reach into a `TursoDb`/`EncryptedTursoDb`
/// synchronously. Tokio's "already in a runtime" check is thread-local, so
/// running `block_on` on a fresh scoped thread sidesteps it safely.
fn block_on_isolated<T, F>(runtime: &tokio::runtime::Runtime, fut: F) -> T
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|scope| scope.spawn(|| runtime.block_on(fut)).join())
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    } else {
        runtime.block_on(fut)
    }
}

fn run_embedded_migrations(
    conn: &turso::Connection,
    runtime: &tokio::runtime::Runtime,
    db_name: &str,
    migrations: &[(&str, &str)],
) -> Result<()> {
    ensure_migrations_table(conn, runtime, db_name)?;

    for (id, sql) in migrations {
        if is_migration_applied(conn, runtime, db_name, id)? {
            continue;
        }

        apply_migration(conn, runtime, db_name, id, sql)?;
    }

    Ok(())
}

fn ensure_migrations_table(
    conn: &turso::Connection,
    runtime: &tokio::runtime::Runtime,
    db_name: &str,
) -> Result<()> {
    block_on_isolated(runtime, async {
        conn.execute(MIGRATIONS_TABLE_SQL, ())
            .await
            .map_err(|e| anyhow::anyhow!("{db_name} migration metadata setup failed: {e}"))
    })?;

    Ok(())
}

fn is_migration_applied(
    conn: &turso::Connection,
    runtime: &tokio::runtime::Runtime,
    db_name: &str,
    id: &str,
) -> Result<bool> {
    block_on_isolated(runtime, async {
        let mut rows = conn.query(SELECT_MIGRATION_SQL, (id,)).await.map_err(|e| {
            anyhow::anyhow!("{db_name} migration metadata query failed for {id}: {e}")
        })?;

        rows.next().await.map(|row| row.is_some()).map_err(|e| {
            anyhow::anyhow!("{db_name} migration metadata row fetch failed for {id}: {e}")
        })
    })
}

fn apply_migration(
    conn: &turso::Connection,
    runtime: &tokio::runtime::Runtime,
    db_name: &str,
    id: &str,
    sql: &str,
) -> Result<()> {
    block_on_isolated(runtime, async {
        // Migration files may contain multiple statements (the repository
        // Agent Trace baseline is one multi-statement schema file), so batch
        // execution is required; `execute` would stop after the first
        // statement.
        conn.execute_batch(sql)
            .await
            .map_err(|e| anyhow::anyhow!("{db_name} migration {id} failed: {e}"))?;
        conn.execute(INSERT_MIGRATION_SQL, (id,))
            .await
            .map_err(|e| {
                anyhow::anyhow!("{db_name} migration metadata record failed for {id}: {e}")
            })?;

        Ok(())
    })
}

/// Body of [`TursoDb::execute_transactional_insert_pair_if_absent`], run
/// against an already-open transaction. Kept as a standalone `async fn` so
/// the caller can uniformly commit on `Ok` and roll back on `Err`.
#[allow(clippy::too_many_arguments)]
async fn execute_insert_pair_if_absent_body(
    tx: &turso::transaction::Transaction<'_>,
    db_name: &str,
    exists_sql: &str,
    exists_params: turso::params::Params,
    first_sql: &str,
    first_params: turso::params::Params,
    second_sql: &str,
    second_params: turso::params::Params,
    fail_before_second: bool,
) -> std::result::Result<bool, WriteAttemptFailure> {
    let mut rows = tx.query(exists_sql, exists_params).await.map_err(|e| {
        classify_turso_error(
            db_name,
            &format!("existence check failed: {exists_sql}"),
            &e,
        )
    })?;
    let already_exists = rows
        .next()
        .await
        .map_err(|e| {
            classify_turso_error(
                db_name,
                &format!("existence row fetch failed: {exists_sql}"),
                &e,
            )
        })?
        .is_some();

    if already_exists {
        return Ok(false);
    }

    tx.execute(first_sql, first_params)
        .await
        .map_err(|e| classify_turso_error(db_name, &format!("execute failed: {first_sql}"), &e))?;

    if fail_before_second {
        return Err(WriteAttemptFailure::Deterministic(anyhow::anyhow!(
            "{db_name} injected failure before second statement (test-only)"
        )));
    }

    tx.execute(second_sql, second_params)
        .await
        .map_err(|e| classify_turso_error(db_name, &format!("execute failed: {second_sql}"), &e))?;

    Ok(true)
}

#[allow(dead_code)]
pub struct TransactionStatement<'a> {
    sql: &'a str,
    params: turso::params::Params,
    expected_rows_affected: Option<u64>,
}

impl<'a> TransactionStatement<'a> {
    #[allow(dead_code)]
    pub fn new(sql: &'a str, params: impl turso::params::IntoParams) -> Result<Self> {
        let params = turso::params::IntoParams::into_params(params)
            .map_err(|e| anyhow::anyhow!("parameter conversion failed: {sql}: {e}"))?;

        Ok(Self {
            sql,
            params,
            expected_rows_affected: None,
        })
    }

    #[allow(dead_code)]
    pub fn expect_rows_affected(mut self, expected: u64) -> Self {
        self.expected_rows_affected = Some(expected);
        self
    }
}

fn is_retryable_turso_error(error: &turso::Error) -> bool {
    matches!(error, turso::Error::Busy(_) | turso::Error::BusySnapshot(_))
}

enum WriteAttemptFailure {
    Retryable(anyhow::Error),
    Deterministic(anyhow::Error),
}

impl WriteAttemptFailure {
    fn into_error(self) -> anyhow::Error {
        match self {
            Self::Retryable(err) | Self::Deterministic(err) => err,
        }
    }
}

fn classify_turso_error(db_name: &str, action: &str, error: &turso::Error) -> WriteAttemptFailure {
    let wrapped = anyhow::anyhow!("{db_name} {action}: {error}");

    if is_retryable_turso_error(error) {
        WriteAttemptFailure::Retryable(wrapped)
    } else {
        WriteAttemptFailure::Deterministic(wrapped)
    }
}

#[allow(dead_code)]
enum CasBatchAttemptOutcome {
    Settled(bool),
    Deterministic(anyhow::Error),
}

#[allow(dead_code)]
fn cas_batch_failure_into_attempt_result(
    failure: WriteAttemptFailure,
) -> Result<CasBatchAttemptOutcome> {
    match failure {
        WriteAttemptFailure::Retryable(err) => Err(err),
        WriteAttemptFailure::Deterministic(err) => Ok(CasBatchAttemptOutcome::Deterministic(err)),
    }
}

#[allow(dead_code)]
async fn execute_cas_batch_body(
    tx: &turso::transaction::Transaction<'_>,
    db_name: &str,
    guard: &TransactionStatement<'_>,
    statements: &[TransactionStatement<'_>],
) -> std::result::Result<bool, WriteAttemptFailure> {
    let guard_rows_affected = tx
        .execute(guard.sql, guard.params.clone())
        .await
        .map_err(|e| {
            classify_turso_error(db_name, &format!("execute failed: {}", guard.sql), &e)
        })?;

    match guard_rows_affected {
        0 => return Ok(false),
        1 => {}
        n => {
            return Err(WriteAttemptFailure::Deterministic(anyhow::anyhow!(
                "{db_name} CAS guard affected {n} rows; expected 0 or 1: {}",
                guard.sql
            )));
        }
    }

    for statement in statements {
        let rows_affected = tx
            .execute(statement.sql, statement.params.clone())
            .await
            .map_err(|e| {
                classify_turso_error(db_name, &format!("execute failed: {}", statement.sql), &e)
            })?;

        if let Some(expected) = statement.expected_rows_affected {
            if rows_affected != expected {
                return Err(WriteAttemptFailure::Deterministic(anyhow::anyhow!(
                    "{db_name} statement affected {rows_affected} rows; expected {expected}: {}",
                    statement.sql
                )));
            }
        }
    }

    Ok(true)
}

struct TursoConnectionCore<M: DbSpec> {
    conn: turso::Connection,
    runtime: tokio::runtime::Runtime,
    spec: PhantomData<fn() -> M>,
}

impl<M: DbSpec> TursoConnectionCore<M> {
    fn new(conn: turso::Connection, runtime: tokio::runtime::Runtime) -> Self {
        Self {
            conn,
            runtime,
            spec: PhantomData,
        }
    }

    fn run_migrations(&self) -> Result<()> {
        run_embedded_migrations(&self.conn, &self.runtime, M::db_name(), M::migrations())
    }
}

fn resolve_connection_open_retry_policy<M: DbSpec>() -> RetryPolicy {
    if let Some(config) = crate::services::config::get_database_retry_config() {
        let per_db = match M::db_config_key() {
            "local_db" => config.local_db.as_ref(),
            "agent_trace_db" => config.agent_trace_db.as_ref().map(|db| &db.retry),
            "auth_db" => config.auth_db.as_ref(),
            _ => None,
        };
        if let Some(per_db) = per_db {
            if let Some(policy) = per_db.connection_open {
                return policy;
            }
        }
    }
    CONNECTION_OPEN_RETRY_POLICY
}

fn resolve_query_retry_policy<M: DbSpec>() -> RetryPolicy {
    if let Some(config) = crate::services::config::get_database_retry_config() {
        let per_db = match M::db_config_key() {
            "local_db" => config.local_db.as_ref(),
            "agent_trace_db" => config.agent_trace_db.as_ref().map(|db| &db.retry),
            "auth_db" => config.auth_db.as_ref(),
            _ => None,
        };
        if let Some(per_db) = per_db {
            if let Some(policy) = per_db.query {
                return policy;
            }
        }
    }
    QUERY_RETRY_POLICY
}

/// Resolve the Turso busy timeout applied to every connection opened for `M`.
///
/// Multiprocess WAL provides cross-process correctness and locking; it does
/// not wait for a contended lock. The busy timeout is Turso's own wait policy
/// for a `Busy` result: while another process holds the write lock, Turso
/// sleeps in short phases until the lock clears or the timeout elapses, and
/// only then returns `Busy`. Because `Transaction::new_unchecked(.., Immediate)`
/// runs `BEGIN IMMEDIATE` through `Connection::execute` on the same connection,
/// the wait also covers writer-lock acquisition. The timeout is connection-wide.
///
/// Only the Agent Trace DB has a busy timeout, taken from
/// `policies.database_retry.agent_trace_db.busy_timeout_ms` when configured;
/// every other database resolves to zero, which leaves Turso's busy handler
/// unset.
fn resolve_busy_timeout<M: DbSpec>() -> std::time::Duration {
    busy_timeout_from_config::<M>(crate::services::config::get_database_retry_config())
}

fn busy_timeout_from_config<M: DbSpec>(
    config: Option<&DatabaseRetryConfig>,
) -> std::time::Duration {
    agent_trace_db_millis::<M>(
        config,
        |db| db.busy_timeout_ms,
        AGENT_TRACE_DB_BUSY_TIMEOUT_MS,
    )
}

/// Resolve the Agent Trace write-contention deadline for `M`.
///
/// The deadline decides whether another outer write-contention retry may
/// start; it does not interrupt a running Turso operation. It is taken from
/// `policies.database_retry.agent_trace_db.contention_deadline_ms` when
/// configured. Every other database resolves to zero.
fn resolve_contention_deadline<M: DbSpec>() -> std::time::Duration {
    contention_deadline_from_config::<M>(crate::services::config::get_database_retry_config())
}

fn contention_deadline_from_config<M: DbSpec>(
    config: Option<&DatabaseRetryConfig>,
) -> std::time::Duration {
    agent_trace_db_millis::<M>(
        config,
        |db| db.contention_deadline_ms,
        AGENT_TRACE_DB_CONTENTION_DEADLINE_MS,
    )
}

fn agent_trace_db_millis<M: DbSpec>(
    config: Option<&DatabaseRetryConfig>,
    select: impl Fn(&AgentTraceDbRetryConfig) -> Option<u64>,
    default_ms: u64,
) -> std::time::Duration {
    if M::db_config_key() != AGENT_TRACE_DB_CONFIG_KEY {
        return std::time::Duration::ZERO;
    }
    let configured = config
        .and_then(|config| config.agent_trace_db.as_ref())
        .and_then(select);
    std::time::Duration::from_millis(configured.unwrap_or(default_ms))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WriteContentionPolicy {
    db_name: &'static str,
    max_attempts: u32,
    backoff_cap: std::time::Duration,
    busy_timeout: std::time::Duration,
    contention_deadline: std::time::Duration,
}

fn write_contention_policy<M: DbSpec>() -> Option<WriteContentionPolicy> {
    if M::db_config_key() != AGENT_TRACE_DB_CONFIG_KEY {
        return None;
    }
    Some(WriteContentionPolicy {
        db_name: M::db_name(),
        max_attempts: AGENT_TRACE_DB_WRITE_CONTENTION_MAX_ATTEMPTS,
        backoff_cap: std::time::Duration::from_millis(
            AGENT_TRACE_DB_WRITE_CONTENTION_BACKOFF_CAP_MS,
        ),
        busy_timeout: resolve_busy_timeout::<M>(),
        contention_deadline: resolve_contention_deadline::<M>(),
    })
}

fn write_contention_backoff(
    jitter: &mut impl rand::Rng,
    cap: std::time::Duration,
) -> std::time::Duration {
    let cap_ms = u64::try_from(cap.as_millis()).unwrap_or(u64::MAX);
    std::time::Duration::from_millis(jitter.gen_range(0..=cap_ms))
}

fn write_contention_retry_may_sleep(
    policy: WriteContentionPolicy,
    elapsed: std::time::Duration,
    backoff: std::time::Duration,
) -> bool {
    let Some(remaining) = policy.contention_deadline.checked_sub(elapsed) else {
        return false;
    };
    !remaining.is_zero() && remaining >= backoff.saturating_add(policy.busy_timeout)
}

fn write_contention_retry_may_start_now(
    policy: WriteContentionPolicy,
    elapsed: std::time::Duration,
) -> bool {
    write_contention_retry_may_sleep(policy, elapsed, std::time::Duration::ZERO)
}

fn run_with_write_contention_retry<T>(
    policy: WriteContentionPolicy,
    operation_name: &str,
    retry_hint: &str,
    attempt: impl FnMut(u32) -> std::result::Result<T, WriteAttemptFailure>,
) -> Result<T> {
    let mut jitter = rand::thread_rng();
    let started_at = std::time::Instant::now();
    run_with_write_contention_retry_using(
        policy,
        &mut || write_contention_backoff(&mut jitter, policy.backoff_cap),
        &mut std::thread::sleep,
        &mut || started_at.elapsed(),
        operation_name,
        retry_hint,
        attempt,
    )
}

fn run_with_write_contention_retry_using<T>(
    policy: WriteContentionPolicy,
    draw_backoff: &mut impl FnMut() -> std::time::Duration,
    sleep: &mut impl FnMut(std::time::Duration),
    elapsed: &mut impl FnMut() -> std::time::Duration,
    operation_name: &str,
    retry_hint: &str,
    mut attempt: impl FnMut(u32) -> std::result::Result<T, WriteAttemptFailure>,
) -> Result<T> {
    let mut attempt_number = 0;

    loop {
        attempt_number += 1;
        #[cfg(test)]
        note_write_contention(|counts| counts.attempts += 1);

        let error = match attempt(attempt_number) {
            Ok(value) => return Ok(value),
            Err(WriteAttemptFailure::Deterministic(error)) => return Err(error),
            Err(WriteAttemptFailure::Retryable(error)) => error,
        };

        if attempt_number < policy.max_attempts {
            let backoff = draw_backoff();
            if write_contention_retry_may_sleep(policy, elapsed(), backoff) {
                sleep(backoff);
                if write_contention_retry_may_start_now(policy, elapsed()) {
                    #[cfg(test)]
                    note_write_contention(|counts| counts.outer_retries += 1);
                    continue;
                }
            }
        }

        return Err(contention_exhausted_error(
            policy,
            operation_name,
            attempt_number,
            elapsed(),
            &error,
            retry_hint,
        ));
    }
}

const CONTENTION_EXHAUSTED_EVENT_ID: &str = "sce.agent_trace_db.contention_exhausted";
const CONTENTION_EXHAUSTED_CAUSE: &str = "database busy (busy timeout exhausted)";

fn contention_exhausted_error(
    policy: WriteContentionPolicy,
    operation_name: &str,
    attempts: u32,
    elapsed: std::time::Duration,
    last_error: &anyhow::Error,
    retry_hint: &str,
) -> anyhow::Error {
    let busy_timeout_ms = u64::try_from(policy.busy_timeout.as_millis()).unwrap_or(u64::MAX);
    let contention_deadline_ms =
        u64::try_from(policy.contention_deadline.as_millis()).unwrap_or(u64::MAX);
    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);

    #[cfg(test)]
    note_write_contention(|counts| counts.exhaustions += 1);

    tracing::warn!(
        target: "sce",
        event_id = CONTENTION_EXHAUSTED_EVENT_ID,
        db_name = policy.db_name,
        operation = operation_name,
        attempts,
        busy_timeout_ms,
        contention_deadline_ms,
        elapsed_ms,
        cause = CONTENTION_EXHAUSTED_CAUSE,
        last_error = %last_error,
        "Agent Trace DB write contention retries exhausted"
    );

    anyhow::anyhow!(
        "Operation '{operation_name}' failed after {attempts} attempt(s) under write contention (db_name={}, operation={operation_name}, attempts={attempts}, busy_timeout_ms={busy_timeout_ms}, contention_deadline_ms={contention_deadline_ms} [no retry is scheduled past this cutoff], elapsed_ms={elapsed_ms}, cause={CONTENTION_EXHAUSTED_CAUSE}). Last error: {last_error}. Try: {retry_hint}",
        policy.db_name,
    )
}

/// Install `busy_timeout` on `conn`; a zero timeout leaves the handler unset.
fn apply_busy_timeout(
    conn: &turso::Connection,
    db_name: &str,
    busy_timeout: std::time::Duration,
) -> Result<()> {
    if busy_timeout.is_zero() {
        return Ok(());
    }
    conn.busy_timeout(busy_timeout)
        .map_err(|e| anyhow::anyhow!("failed to set {db_name} database busy timeout: {e}"))
}

#[cfg(test)]
thread_local! {
    static READ_STATEMENTS_ISSUED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Record that one [`TursoDb`] read statement was issued on this thread.
#[cfg(test)]
fn note_read_statement_issued() {
    READ_STATEMENTS_ISSUED.with(|count| count.set(count.get() + 1));
}

/// Run `body`, returning its result together with the number of [`TursoDb`]
/// read statements ([`TursoDb::query`], [`TursoDb::query_values`],
/// [`TursoDb::query_map`]) it issued on the current thread.
///
/// Each read method bumps the counter once in its synchronous prelude, before
/// the retry wrapper, so a transient retry never inflates the count and the
/// number reflects *logical* read statements, not connection round-trips.
/// Lets a deterministic single-threaded test assert that an operation which
/// must observe one coherent database snapshot — for example
/// `MutationTraceStore::load_all_tree_roots`, a single `UNION` statement —
/// issues exactly one, and fail if it is ever reimplemented as several
/// independent `SELECT`s unioned in Rust. Not shared across threads.
#[cfg(test)]
pub(crate) fn count_read_statements<T>(body: impl FnOnce() -> T) -> (T, usize) {
    READ_STATEMENTS_ISSUED.with(|count| count.set(0));
    let result = body();
    let issued = READ_STATEMENTS_ISSUED.with(std::cell::Cell::get);
    (result, issued)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WriteContentionCounts {
    pub(crate) attempts: u32,
    pub(crate) outer_retries: u32,
    pub(crate) exhaustions: u32,
}

#[cfg(test)]
thread_local! {
    static WRITE_CONTENTION_COUNTS: std::cell::Cell<WriteContentionCounts> =
        const { std::cell::Cell::new(WriteContentionCounts { attempts: 0, outer_retries: 0, exhaustions: 0 }) };
}

#[cfg(test)]
fn note_write_contention(update: impl FnOnce(&mut WriteContentionCounts)) {
    WRITE_CONTENTION_COUNTS.with(|cell| {
        let mut counts = cell.get();
        update(&mut counts);
        cell.set(counts);
    });
}

#[cfg(test)]
pub(crate) fn count_write_contention<T>(body: impl FnOnce() -> T) -> (T, WriteContentionCounts) {
    WRITE_CONTENTION_COUNTS.with(|cell| cell.set(WriteContentionCounts::default()));
    let result = body();
    let counts = WRITE_CONTENTION_COUNTS.with(std::cell::Cell::get);
    (result, counts)
}

/// Generic Turso database adapter.
///
/// Wraps a Turso connection with a tokio current-thread runtime so callers can
/// use synchronous `execute`/`query` methods while the underlying Turso API
/// remains async.
pub struct TursoDb<M: DbSpec> {
    core: TursoConnectionCore<M>,
}

/// Fully fetched SQL query result for deterministic rendering outside the
/// async Turso row iterator lifetime.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub struct QueryRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<TursoValue>>,
}

/// Generic encrypted Turso database adapter.
///
/// Mirrors the structural seams of [`TursoDb`] while reserving encrypted local
/// database initialization for services that require at-rest encryption.
pub struct EncryptedTursoDb<M: DbSpec> {
    core: TursoConnectionCore<M>,
}

impl<M: DbSpec> TursoDb<M> {
    /// Open or create the database at the spec-provided canonical path.
    ///
    /// Parent directories are created automatically. Migrations are run after
    /// the database connection is established.
    pub fn new() -> Result<Self> {
        let db = Self::open_without_migrations()?;

        db.run_migrations()
            .with_context(|| format!("failed to run {} migrations", M::db_name()))?;

        Ok(db)
    }

    /// Open or create the database at an explicit path.
    ///
    /// Parent directories are created automatically. Migrations are run after
    /// the database connection is established. The service-specific retry and
    /// migration configuration still comes from `M`.
    pub fn new_at(db_path: impl AsRef<Path>) -> Result<Self> {
        let db = Self::open_without_migrations_at(db_path)?;

        db.run_migrations()
            .with_context(|| format!("failed to run {} migrations", M::db_name()))?;

        Ok(db)
    }

    /// Open or create the database at the spec-provided canonical path without
    /// running embedded migrations.
    ///
    /// Parent directories are created automatically and the connection-open
    /// retry policy is preserved. Runtime callers that use this path are
    /// responsible for verifying schema readiness before query/write work.
    pub fn open_without_migrations() -> Result<Self> {
        let db_name = M::db_name();
        let db_path = M::db_path().with_context(|| format!("failed to resolve {db_name} path"))?;

        Self::open_without_migrations_at(db_path)
    }

    /// Open or create the database at an explicit path without running embedded
    /// migrations.
    ///
    /// Parent directories are created automatically and the connection-open
    /// retry policy is preserved. Runtime callers that use this path are
    /// responsible for verifying schema readiness before query/write work.
    pub fn open_without_migrations_at(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_name = M::db_name();
        let db_path = db_path.as_ref().to_path_buf();

        ensure_db_parent_dir(db_name, &db_path)?;

        let runtime = build_current_thread_runtime(db_name)?;
        let retry_policy = resolve_connection_open_retry_policy::<M>();
        let busy_timeout = resolve_busy_timeout::<M>();
        let operation_name = format!("open {db_name} database connection");

        let conn = run_with_retry_sync(
            retry_policy,
            &operation_name,
            CONNECTION_OPEN_RETRY_HINT,
            |_| {
                block_on_isolated(&runtime, async {
                    let path_str = db_path.to_str().ok_or_else(|| {
                        anyhow::anyhow!("invalid UTF-8 in database path: {}", db_path.display())
                    })?;
                    let db = turso::Builder::new_local(path_str)
                        .experimental_multiprocess_wal(true)
                        .build()
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "failed to open {db_name} database at {}: {e}",
                                db_path.display()
                            )
                        })?;
                    let conn = db.connect().map_err(|e| {
                        anyhow::anyhow!("failed to connect to {db_name} database: {e}")
                    })?;
                    apply_busy_timeout(&conn, db_name, busy_timeout)?;
                    Ok(conn)
                })
            },
        )?;

        Ok(Self {
            core: TursoConnectionCore::new(conn, runtime),
        })
    }

    /// Execute a SQL statement that does not return rows.
    ///
    /// # Arguments
    /// * `sql` - SQL statement, which may contain `?` placeholders.
    /// * `params` - Parameter values implementing `IntoParams`.
    ///
    /// # Returns
    /// Number of rows affected.
    pub fn execute(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<u64> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("execute {} database query", M::db_name());

        run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    self.core
                        .conn
                        .execute(sql, params.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("{} execute failed: {sql}: {e}", M::db_name()))
                })
            },
        )
    }

    pub fn execute_idempotent_write(
        &self,
        sql: &str,
        params: impl turso::params::IntoParams,
    ) -> Result<u64> {
        let Some(policy) = write_contention_policy::<M>() else {
            return self.execute(sql, params);
        };
        let db_name = M::db_name();
        let params = turso::params::IntoParams::into_params(params)
            .map_err(|e| anyhow::anyhow!("{db_name} parameter conversion failed: {sql}: {e}"))?;
        let operation_name = format!("execute {db_name} database query");

        run_with_write_contention_retry(policy, &operation_name, QUERY_RETRY_HINT, |_| {
            block_on_isolated(&self.core.runtime, async {
                self.core
                    .conn
                    .execute(sql, params.clone())
                    .await
                    .map_err(|e| {
                        classify_turso_error(db_name, &format!("execute failed: {sql}"), &e)
                    })
            })
        })
    }

    /// Execute a SQL query that returns rows.
    ///
    /// # Arguments
    /// * `sql` - SQL query, which may contain `?` placeholders.
    /// * `params` - Parameter values implementing `IntoParams`.
    ///
    /// # Returns
    /// A `turso::Rows` iterator over the result set.
    #[allow(dead_code)]
    pub fn query(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<turso::Rows> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("query {} database", M::db_name());

        #[cfg(test)]
        note_read_statement_issued();

        run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    self.core
                        .conn
                        .query(sql, params.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name()))
                })
            },
        )
    }

    /// Execute a SQL query and synchronously fetch column names plus raw values.
    #[allow(dead_code)]
    pub fn query_values(
        &self,
        sql: &str,
        params: impl turso::params::IntoParams,
    ) -> Result<QueryRows> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("query and fetch {} database values", M::db_name());

        #[cfg(test)]
        note_read_statement_issued();

        run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    let mut rows =
                        self.core
                            .conn
                            .query(sql, params.clone())
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name())
                            })?;
                    let columns = rows.column_names();
                    let column_count = rows.column_count();
                    let mut fetched_rows = Vec::new();

                    while let Some(row) = rows.next().await.map_err(|e| {
                        anyhow::anyhow!("{} row fetch failed: {sql}: {e}", M::db_name())
                    })? {
                        let mut values = Vec::with_capacity(column_count);
                        for column_index in 0..column_count {
                            values.push(row.get_value(column_index).map_err(|e| {
                                anyhow::anyhow!("{} value fetch failed: {sql}: {e}", M::db_name())
                            })?);
                        }
                        fetched_rows.push(values);
                    }

                    Ok(QueryRows {
                        columns,
                        rows: fetched_rows,
                    })
                })
            },
        )
    }

    /// Run an "insert row pair if absent" write transaction.
    ///
    /// If `exists_sql` (bound to `exists_params`) finds a matching row, no
    /// insert statements run, the no-write transaction commits, and this
    /// returns `false`. Otherwise `first_sql` then `second_sql` execute in order inside one
    /// `BEGIN IMMEDIATE` transaction and commit together, returning `true`.
    /// `BEGIN IMMEDIATE` serializes concurrent callers against the same
    /// database file, so the existence check and both inserts are never
    /// interleaved with another writer's attempt. The whole attempt is
    /// retried as one unit on transient failure.
    ///
    /// `fail_before_second` is a test-only hook: when `true`, an error is
    /// forced immediately after `first_sql` succeeds and before `second_sql`
    /// runs or the transaction commits, so callers can prove the whole
    /// transaction — including the already-executed `first_sql` — rolls
    /// back together.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_transactional_insert_pair_if_absent(
        &self,
        operation_name: &str,
        retry_hint: &str,
        exists_sql: &str,
        exists_params: impl turso::params::IntoParams,
        first_sql: &str,
        first_params: impl turso::params::IntoParams,
        second_sql: &str,
        second_params: impl turso::params::IntoParams,
        fail_before_second: bool,
    ) -> Result<bool> {
        let db_name = M::db_name();
        let exists_params = turso::params::IntoParams::into_params(exists_params).map_err(|e| {
            anyhow::anyhow!("{db_name} parameter conversion failed: {exists_sql}: {e}")
        })?;
        let first_params = turso::params::IntoParams::into_params(first_params).map_err(|e| {
            anyhow::anyhow!("{db_name} parameter conversion failed: {first_sql}: {e}")
        })?;
        let second_params = turso::params::IntoParams::into_params(second_params).map_err(|e| {
            anyhow::anyhow!("{db_name} parameter conversion failed: {second_sql}: {e}")
        })?;

        let run_attempt = || {
            block_on_isolated(&self.core.runtime, async {
                let tx = turso::transaction::Transaction::new_unchecked(
                    &self.core.conn,
                    turso::transaction::TransactionBehavior::Immediate,
                )
                .await
                .map_err(|e| classify_turso_error(db_name, "failed to begin transaction", &e))?;

                let outcome = execute_insert_pair_if_absent_body(
                    &tx,
                    db_name,
                    exists_sql,
                    exists_params.clone(),
                    first_sql,
                    first_params.clone(),
                    second_sql,
                    second_params.clone(),
                    fail_before_second,
                )
                .await;

                match outcome {
                    Ok(inserted) => {
                        tx.commit().await.map_err(|e| {
                            classify_turso_error(db_name, "failed to commit transaction", &e)
                        })?;
                        Ok(inserted)
                    }
                    Err(failure) => {
                        let _ = tx.rollback().await;
                        Err(failure)
                    }
                }
            })
        };

        if let Some(policy) = write_contention_policy::<M>() {
            return run_with_write_contention_retry(policy, operation_name, retry_hint, |_| {
                run_attempt()
            });
        }

        run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            operation_name,
            retry_hint,
            |_| run_attempt().map_err(WriteAttemptFailure::into_error),
        )
    }

    /// Execute a SQL query and synchronously map all returned rows.
    pub fn query_map<T, F>(
        &self,
        sql: &str,
        params: impl turso::params::IntoParams,
        mut map_row: F,
    ) -> Result<Vec<T>>
    where
        F: FnMut(&turso::Row) -> Result<T>,
    {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("query and fetch {} database rows", M::db_name());

        #[cfg(test)]
        note_read_statement_issued();

        let rows = run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    let mut rows =
                        self.core
                            .conn
                            .query(sql, params.clone())
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name())
                            })?;
                    let mut fetched_rows = Vec::new();

                    while let Some(row) = rows.next().await.map_err(|e| {
                        anyhow::anyhow!("{} row fetch failed: {sql}: {e}", M::db_name())
                    })? {
                        fetched_rows.push(row);
                    }

                    Ok(fetched_rows)
                })
            },
        )?;

        let mut results = Vec::new();

        for row in rows {
            results.push(
                map_row(&row)
                    .with_context(|| format!("{} row mapping failed: {sql}", M::db_name()))?,
            );
        }

        Ok(results)
    }

    #[allow(dead_code)]
    pub fn execute_transactional_cas_batch(
        &self,
        operation_name: &str,
        retry_hint: &str,
        guard: &TransactionStatement<'_>,
        statements: &[TransactionStatement<'_>],
    ) -> Result<bool> {
        let db_name = M::db_name();

        let run_attempt = || {
            block_on_isolated(&self.core.runtime, async {
                let tx = turso::transaction::Transaction::new_unchecked(
                    &self.core.conn,
                    turso::transaction::TransactionBehavior::Immediate,
                )
                .await
                .map_err(|e| classify_turso_error(db_name, "failed to begin transaction", &e))?;

                match execute_cas_batch_body(&tx, db_name, guard, statements).await {
                    Ok(applied) => {
                        tx.commit().await.map_err(|e| {
                            classify_turso_error(db_name, "failed to commit transaction", &e)
                        })?;
                        Ok(applied)
                    }
                    Err(failure) => {
                        let _ = tx.rollback().await;
                        Err(failure)
                    }
                }
            })
        };

        if let Some(policy) = write_contention_policy::<M>() {
            return run_with_write_contention_retry(policy, operation_name, retry_hint, |_| {
                run_attempt()
            });
        }

        let outcome = run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            operation_name,
            retry_hint,
            |_| match run_attempt() {
                Ok(applied) => Ok(CasBatchAttemptOutcome::Settled(applied)),
                Err(failure) => cas_batch_failure_into_attempt_result(failure),
            },
        )?;

        match outcome {
            CasBatchAttemptOutcome::Settled(applied) => Ok(applied),
            CasBatchAttemptOutcome::Deterministic(err) => Err(err),
        }
    }

    /// Run all embedded migrations in order.
    ///
    /// Applied migration IDs are recorded in `__sce_migrations` so later
    /// initializations apply only migrations that were not already recorded.
    /// Existing databases without migration metadata are brought forward by
    /// re-applying the current idempotent migration set and recording each ID.
    pub fn run_migrations(&self) -> Result<()> {
        self.core.run_migrations()
    }

    /// Run a passive WAL checkpoint (`PRAGMA wal_checkpoint(PASSIVE)`).
    ///
    /// PASSIVE checkpoints only what is currently safe to move from the WAL
    /// into the main database file and never blocks on active readers or
    /// writers, so it does not guarantee WAL truncation. Safe to call
    /// repeatedly. Routine maintenance only; not a durability boundary.
    pub fn passive_checkpoint(&self) -> Result<()> {
        let operation_name = format!("checkpoint {} database WAL", M::db_name());

        run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    let mut rows = self
                        .core
                        .conn
                        .query("PRAGMA wal_checkpoint(PASSIVE)", ())
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!("{} WAL checkpoint failed: {e}", M::db_name())
                        })?;

                    while rows
                        .next()
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!("{} WAL checkpoint row fetch failed: {e}", M::db_name())
                        })?
                        .is_some()
                    {}

                    Ok(())
                })
            },
        )
    }

    /// Check migration metadata for problems that would prevent safe hook
    /// runtime access.
    ///
    /// Returns a list of problems: missing migration metadata table,
    /// incomplete applied migrations, or unexpected extra migrations.
    /// An empty list means the schema is ready.
    pub fn migration_metadata_problems(&self) -> Result<Vec<String>> {
        let migration_table_exists = self.query_map(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '__sce_migrations' LIMIT 1",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )?;

        if migration_table_exists.is_empty() {
            return Ok(vec![String::from("missing migration metadata table")]);
        }

        let applied_ids = self.query_map(
            "SELECT id FROM __sce_migrations ORDER BY id ASC",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )?;
        let expected_ids = M::migrations()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut problems = Vec::new();

        if applied_ids.len() != expected_ids.len() {
            problems.push(format!(
                "expected {} applied migrations, found {}",
                expected_ids.len(),
                applied_ids.len()
            ));
        }

        let missing_ids = expected_ids
            .iter()
            .copied()
            .filter(|id| !applied_ids.iter().any(|applied_id| applied_id == id))
            .collect::<Vec<_>>();
        if !missing_ids.is_empty() {
            problems.push(format!("missing migrations {}", missing_ids.join(", ")));
        }

        let unexpected_ids = applied_ids
            .iter()
            .filter(|applied_id| !expected_ids.iter().any(|id| id == &applied_id.as_str()))
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !unexpected_ids.is_empty() {
            problems.push(format!(
                "unexpected migrations {}",
                unexpected_ids.join(", ")
            ));
        }

        Ok(problems)
    }

    /// Verify that the database schema needed by hook runtime readers and
    /// writers already exists.
    ///
    /// This check is intentionally non-mutating. Missing or incomplete schema
    /// is reported with the provided setup guidance instead of running
    /// migrations from a high-frequency hook path.
    pub fn ensure_schema_ready(&self, setup_guidance: &str) -> Result<()> {
        let problems = self.migration_metadata_problems()?;

        if problems.is_empty() {
            return Ok(());
        }

        anyhow::bail!(
            "{} schema is not initialized or is incomplete: {}. {setup_guidance}",
            M::db_name(),
            problems.join(", ")
        )
    }
}

impl<M: DbSpec> EncryptedTursoDb<M> {
    /// Open or create the encrypted database at the spec-provided canonical
    /// path.
    ///
    /// This constructor is the encrypted counterpart to [`TursoDb::new`] and
    /// uses a strict encrypted local-builder path.
    pub fn new() -> Result<Self> {
        let db_name = M::db_name();
        let db_path = M::db_path().with_context(|| format!("failed to resolve {db_name} path"))?;
        let encryption_key = encryption_key::get_or_create_encryption_key(&db_path, db_name)?;

        ensure_db_parent_dir(db_name, &db_path)?;

        let runtime = build_current_thread_runtime(db_name)?;
        let retry_policy = resolve_connection_open_retry_policy::<M>();
        let operation_name = format!("open encrypted {db_name} database connection");

        let conn = run_with_retry_sync(
            retry_policy,
            &operation_name,
            CONNECTION_OPEN_RETRY_HINT,
            |_| {
                block_on_isolated(&runtime, async {
                    let path_str = db_path.to_str().ok_or_else(|| {
                        anyhow::anyhow!("invalid UTF-8 in database path: {}", db_path.display())
                    })?;

                    let encryption_opts = turso::EncryptionOpts {
                        hexkey: encryption_key.clone(),
                        cipher: ENCRYPTION_CIPHER_AEGIS256.to_string(),
                    };

                    let db = turso::Builder::new_local(path_str)
                        .experimental_encryption(true)
                        .with_encryption(encryption_opts)
                        .build()
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "failed to open encrypted {db_name} database at {} with cipher {ENCRYPTION_CIPHER_AEGIS256}. Try: verify the credential store encryption key is valid and that local Turso encryption support is available: {e}",
                                db_path.display()
                            )
                        })?;

                    db.connect().map_err(|e| {
                        anyhow::anyhow!("failed to connect to encrypted {db_name} database: {e}")
                    })
                })
            },
        )?;

        let db = Self {
            core: TursoConnectionCore::new(conn, runtime),
        };

        db.run_migrations()
            .with_context(|| format!("failed to run {db_name} migrations"))?;

        Ok(db)
    }

    /// Execute a SQL statement that does not return rows.
    ///
    /// # Arguments
    /// * `sql` - SQL statement, which may contain `?` placeholders.
    /// * `params` - Parameter values implementing `IntoParams`.
    ///
    /// # Returns
    /// Number of rows affected.
    pub fn execute(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<u64> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("execute encrypted {} database query", M::db_name());

        run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    self.core
                        .conn
                        .execute(sql, params.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("{} execute failed: {sql}: {e}", M::db_name()))
                })
            },
        )
    }

    /// Execute a SQL query that returns rows.
    ///
    /// # Arguments
    /// * `sql` - SQL query, which may contain `?` placeholders.
    /// * `params` - Parameter values implementing `IntoParams`.
    ///
    /// # Returns
    /// A `turso::Rows` iterator over the result set.
    #[allow(dead_code)]
    pub fn query(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<turso::Rows> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("query encrypted {} database", M::db_name());

        run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    self.core
                        .conn
                        .query(sql, params.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name()))
                })
            },
        )
    }

    /// Execute a SQL query and synchronously map all returned rows.
    pub fn query_map<T, F>(
        &self,
        sql: &str,
        params: impl turso::params::IntoParams,
        mut map_row: F,
    ) -> Result<Vec<T>>
    where
        F: FnMut(&turso::Row) -> Result<T>,
    {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("query and fetch encrypted {} database rows", M::db_name());

        let rows = run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    let mut rows =
                        self.core
                            .conn
                            .query(sql, params.clone())
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name())
                            })?;
                    let mut fetched_rows = Vec::new();

                    while let Some(row) = rows.next().await.map_err(|e| {
                        anyhow::anyhow!("{} row fetch failed: {sql}: {e}", M::db_name())
                    })? {
                        fetched_rows.push(row);
                    }

                    Ok(fetched_rows)
                })
            },
        )?;

        let mut results = Vec::new();

        for row in rows {
            results.push(
                map_row(&row)
                    .with_context(|| format!("{} row mapping failed: {sql}", M::db_name()))?,
            );
        }

        Ok(results)
    }

    /// Run all embedded migrations in order.
    ///
    /// Applied migration IDs are recorded in `__sce_migrations` so later
    /// initializations apply only migrations that were not already recorded.
    /// Existing databases without migration metadata are brought forward by
    /// re-applying the current idempotent migration set and recording each ID.
    pub fn run_migrations(&self) -> Result<()> {
        self.core.run_migrations()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use rand::SeedableRng;

    use super::*;

    const QUERY_RETRY_FAILURE_BUDGET_MS: u64 = 2_000;

    struct TestDbSpec;

    impl DbSpec for TestDbSpec {
        fn db_name() -> &'static str {
            "test"
        }

        fn db_path() -> Result<PathBuf> {
            unreachable!("tests always open via TursoDb::new_at with an explicit path")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            &[]
        }

        fn db_config_key() -> &'static str {
            "test_db"
        }
    }

    struct AgentTraceTestDbSpec;

    impl DbSpec for AgentTraceTestDbSpec {
        fn db_name() -> &'static str {
            "agent trace test"
        }

        fn db_path() -> Result<PathBuf> {
            unreachable!("tests always open via TursoDb::new_at with an explicit path")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            &[]
        }

        fn db_config_key() -> &'static str {
            AGENT_TRACE_DB_CONFIG_KEY
        }
    }

    const BUSY_TIMEOUT_LOCK_HOLD_MS: u64 = 300;

    fn begin_immediate_while_write_lock_is_held<M: DbSpec>(
        db_path: &Path,
    ) -> (std::result::Result<u64, turso::Error>, Duration) {
        let hold = Duration::from_millis(BUSY_TIMEOUT_LOCK_HOLD_MS);
        drop(TursoDb::<TestDbSpec>::new_at(db_path).expect("test DB should be created up front"));
        let contender =
            TursoDb::<M>::open_without_migrations_at(db_path).expect("contender DB should open");
        let lock_acquired = std::sync::Arc::new(std::sync::Barrier::new(2));
        let holder = {
            let db_path = db_path.to_path_buf();
            let lock_acquired = std::sync::Arc::clone(&lock_acquired);
            thread::spawn(move || {
                let holder = TursoDb::<TestDbSpec>::open_without_migrations_at(&db_path)
                    .expect("holder DB should open");
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

        lock_acquired.wait();
        let started_at = Instant::now();
        let outcome = block_on_isolated(&contender.core.runtime, async {
            contender.core.conn.execute("BEGIN IMMEDIATE", ()).await
        });
        let elapsed = started_at.elapsed();
        if outcome.is_ok() {
            contender
                .execute("ROLLBACK", ())
                .expect("contender should release the write lock");
        }
        holder.join().expect("holder thread should not panic");
        drop(contender);

        (outcome, elapsed)
    }

    #[test]
    fn busy_timeout_resolves_default_for_agent_trace_db_and_zero_for_other_dbs() {
        assert_eq!(
            resolve_busy_timeout::<AgentTraceTestDbSpec>(),
            Duration::from_millis(AGENT_TRACE_DB_BUSY_TIMEOUT_MS)
        );
        assert_eq!(resolve_busy_timeout::<TestDbSpec>(), Duration::ZERO);
    }

    fn agent_trace_retry_config(
        busy_timeout_ms: Option<u64>,
        contention_deadline_ms: Option<u64>,
    ) -> DatabaseRetryConfig {
        DatabaseRetryConfig {
            local_db: None,
            agent_trace_db: Some(AgentTraceDbRetryConfig {
                retry: crate::services::config::PerDbRetryConfig {
                    connection_open: None,
                    query: None,
                },
                busy_timeout_ms,
                contention_deadline_ms,
            }),
            auth_db: None,
        }
    }

    #[test]
    fn database_retry_configured_busy_timeout_overrides_agent_trace_default() {
        let config = agent_trace_retry_config(Some(1_500), None);
        assert_eq!(
            busy_timeout_from_config::<AgentTraceTestDbSpec>(Some(&config)),
            Duration::from_millis(1_500)
        );
        assert_eq!(
            busy_timeout_from_config::<TestDbSpec>(Some(&config)),
            Duration::ZERO
        );

        let disabled = agent_trace_retry_config(Some(0), None);
        assert_eq!(
            busy_timeout_from_config::<AgentTraceTestDbSpec>(Some(&disabled)),
            Duration::ZERO
        );

        let unset = agent_trace_retry_config(None, None);
        assert_eq!(
            busy_timeout_from_config::<AgentTraceTestDbSpec>(Some(&unset)),
            Duration::from_millis(AGENT_TRACE_DB_BUSY_TIMEOUT_MS)
        );
    }

    #[test]
    fn database_retry_contention_deadline_resolves_default_and_configured_value() {
        assert_eq!(
            resolve_contention_deadline::<AgentTraceTestDbSpec>(),
            Duration::from_millis(AGENT_TRACE_DB_CONTENTION_DEADLINE_MS)
        );
        assert_eq!(
            AGENT_TRACE_DB_CONTENTION_DEADLINE_MS, 1_250,
            "contention deadline default"
        );
        assert_eq!(resolve_contention_deadline::<TestDbSpec>(), Duration::ZERO);

        let config = agent_trace_retry_config(None, Some(3_500));
        assert_eq!(
            contention_deadline_from_config::<AgentTraceTestDbSpec>(Some(&config)),
            Duration::from_millis(3_500)
        );
        assert_eq!(
            contention_deadline_from_config::<TestDbSpec>(Some(&config)),
            Duration::ZERO
        );

        let zero = agent_trace_retry_config(None, Some(0));
        assert_eq!(
            contention_deadline_from_config::<AgentTraceTestDbSpec>(Some(&zero)),
            Duration::ZERO
        );
    }

    #[test]
    fn busy_timeout_unset_connection_returns_busy_promptly_on_begin_immediate() {
        let db_path = unique_test_db_path();

        let (outcome, elapsed) = begin_immediate_while_write_lock_is_held::<TestDbSpec>(&db_path);

        assert!(
            matches!(outcome, Err(turso::Error::Busy(_))),
            "a connection without a busy handler should get Busy, got {outcome:?}"
        );
        assert!(
            elapsed < Duration::from_millis(BUSY_TIMEOUT_LOCK_HOLD_MS / 2),
            "Busy should be returned without waiting for the holder, took {elapsed:?}"
        );
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn busy_timeout_agent_trace_connection_waits_for_begin_immediate_holder() {
        let db_path = unique_test_db_path();

        let (outcome, elapsed) =
            begin_immediate_while_write_lock_is_held::<AgentTraceTestDbSpec>(&db_path);

        assert!(
            outcome.is_ok(),
            "Turso's busy handler should wait out the holder, got {outcome:?}"
        );
        assert!(
            elapsed >= Duration::from_millis(BUSY_TIMEOUT_LOCK_HOLD_MS / 2),
            "BEGIN IMMEDIATE should have waited for the holder, took {elapsed:?}"
        );
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn unique_test_db_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("sce-db-mod-test-{}-{nonce}", std::process::id()))
            .join("test.db")
    }

    fn open_test_db() -> (TursoDb<TestDbSpec>, PathBuf) {
        let db_path = unique_test_db_path();
        let db = TursoDb::<TestDbSpec>::new_at(&db_path).expect("test DB should open");
        db.execute(
            "CREATE TABLE IF NOT EXISTS checkpoint_probe (value TEXT NOT NULL)",
            (),
        )
        .expect("test table creation should succeed");

        (db, db_path)
    }

    fn cleanup_test_db(db: TursoDb<TestDbSpec>, db_path: &Path) {
        drop(db);
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn open_cas_test_db() -> (TursoDb<TestDbSpec>, PathBuf) {
        let db_path = unique_test_db_path();
        let db = TursoDb::<TestDbSpec>::new_at(&db_path).expect("test DB should open");
        db.execute(
            "CREATE TABLE IF NOT EXISTS cas_target (id INTEGER PRIMARY KEY, revision INTEGER NOT NULL)",
            (),
        )
        .expect("cas_target table creation should succeed");
        db.execute(
            "CREATE TABLE IF NOT EXISTS cas_effect (name TEXT PRIMARY KEY)",
            (),
        )
        .expect("cas_effect table creation should succeed");
        db.execute("INSERT INTO cas_target (id, revision) VALUES (1, 0)", ())
            .expect("cas_target seed row should insert");

        (db, db_path)
    }

    fn cas_target_revision(db: &TursoDb<TestDbSpec>, id: i64) -> i64 {
        db.query_map(
            "SELECT revision FROM cas_target WHERE id = ?1",
            (id,),
            |row| row.get::<i64>(0).map_err(Into::into),
        )
        .expect("cas_target revision read should succeed")
        .into_iter()
        .next()
        .expect("cas_target seed row should exist")
    }

    fn cas_effect_names(db: &TursoDb<TestDbSpec>) -> Vec<String> {
        db.query_map("SELECT name FROM cas_effect ORDER BY name", (), |row| {
            row.get::<String>(0).map_err(Into::into)
        })
        .expect("cas_effect read should succeed")
    }

    #[test]
    fn execute_transactional_cas_batch_returns_false_and_runs_nothing_when_guard_matches_no_rows() {
        let (db, db_path) = open_cas_test_db();
        let guard = TransactionStatement::new(
            "UPDATE cas_target SET revision = 1 WHERE id = 1 AND revision = 999",
            (),
        )
        .expect("guard statement should build");
        let statements =
            [
                TransactionStatement::new("INSERT INTO cas_effect (name) VALUES ('applied')", ())
                    .expect("effect statement should build"),
            ];

        let applied = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect("no-op CAS batch should succeed");

        assert!(!applied);
        assert_eq!(cas_target_revision(&db, 1), 0);
        assert!(cas_effect_names(&db).is_empty());

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn execute_transactional_cas_batch_returns_true_and_runs_every_statement_when_guard_matches_one_row(
    ) {
        let (db, db_path) = open_cas_test_db();
        let guard = TransactionStatement::new(
            "UPDATE cas_target SET revision = 1 WHERE id = 1 AND revision = 0",
            (),
        )
        .expect("guard statement should build");
        let statements =
            [
                TransactionStatement::new("INSERT INTO cas_effect (name) VALUES ('applied')", ())
                    .expect("effect statement should build"),
            ];

        let applied = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect("applied CAS batch should succeed");

        assert!(applied);
        assert_eq!(cas_target_revision(&db, 1), 1);
        assert_eq!(cas_effect_names(&db), vec![String::from("applied")]);

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn execute_transactional_cas_batch_rolls_back_and_fails_after_one_attempt_on_deterministic_failure(
    ) {
        let (db, db_path) = open_cas_test_db();
        db.execute("INSERT INTO cas_effect (name) VALUES ('applied')", ())
            .expect("pre-existing conflicting row should insert");
        let guard = TransactionStatement::new(
            "UPDATE cas_target SET revision = 1 WHERE id = 1 AND revision = 0",
            (),
        )
        .expect("guard statement should build");
        let statements =
            [
                TransactionStatement::new("INSERT INTO cas_effect (name) VALUES ('applied')", ())
                    .expect("effect statement should build"),
            ];

        let started_at = Instant::now();
        let error = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect_err("duplicate insert should fail deterministically");
        let elapsed = started_at.elapsed();

        assert!(
            elapsed < Duration::from_millis(150),
            "deterministic failure appears to have been retried instead of failing after one attempt: {elapsed:?}"
        );
        assert!(error.to_string().contains("execute failed"));
        assert_eq!(cas_target_revision(&db, 1), 0);
        assert_eq!(cas_effect_names(&db), vec![String::from("applied")]);

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn execute_transactional_cas_batch_rejects_a_guard_matching_more_than_one_row_without_retrying()
    {
        let (db, db_path) = open_cas_test_db();
        db.execute("INSERT INTO cas_target (id, revision) VALUES (2, 0)", ())
            .expect("second cas_target row should insert");
        let guard =
            TransactionStatement::new("UPDATE cas_target SET revision = 1 WHERE revision = 0", ())
                .expect("guard statement should build");
        let statements =
            [
                TransactionStatement::new("INSERT INTO cas_effect (name) VALUES ('applied')", ())
                    .expect("effect statement should build"),
            ];

        let started_at = Instant::now();
        let error = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect_err("a guard matching more than one row should fail deterministically");
        let elapsed = started_at.elapsed();

        assert!(
            elapsed < Duration::from_millis(150),
            "guard over-match appears to have been retried instead of failing after one attempt: {elapsed:?}"
        );
        assert!(error.to_string().contains("expected 0 or 1"));
        assert_eq!(cas_target_revision(&db, 1), 0);
        assert_eq!(cas_target_revision(&db, 2), 0);
        assert!(cas_effect_names(&db).is_empty());

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn execute_transactional_cas_batch_applies_a_statement_whose_expected_rows_affected_matches() {
        let (db, db_path) = open_cas_test_db();
        let guard = TransactionStatement::new(
            "UPDATE cas_target SET revision = 1 WHERE id = 1 AND revision = 0",
            (),
        )
        .expect("guard statement should build");
        let statements =
            [
                TransactionStatement::new("INSERT INTO cas_effect (name) VALUES ('applied')", ())
                    .expect("effect statement should build")
                    .expect_rows_affected(1),
            ];

        let applied = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect("a statement matching its row expectation should succeed");

        assert!(applied);
        assert_eq!(cas_target_revision(&db, 1), 1);
        assert_eq!(cas_effect_names(&db), vec![String::from("applied")]);

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn execute_transactional_cas_batch_rejects_a_statement_affecting_fewer_rows_than_expected() {
        let (db, db_path) = open_cas_test_db();
        let guard = TransactionStatement::new(
            "UPDATE cas_target SET revision = 1 WHERE id = 1 AND revision = 0",
            (),
        )
        .expect("guard statement should build");
        let statements = [TransactionStatement::new(
            "UPDATE cas_effect SET name = 'applied' WHERE name = 'missing'",
            (),
        )
        .expect("effect statement should build")
        .expect_rows_affected(1)];

        let error = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect_err("a statement affecting zero rows should fail its row expectation");

        assert!(error.to_string().contains("affected 0 rows"));
        assert!(error.to_string().contains("expected 1"));
        assert_eq!(cas_target_revision(&db, 1), 0);

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn execute_transactional_cas_batch_rejects_a_statement_affecting_more_rows_than_expected() {
        let (db, db_path) = open_cas_test_db();
        db.execute("INSERT INTO cas_effect (name) VALUES ('a')", ())
            .expect("first pre-existing effect row should insert");
        db.execute("INSERT INTO cas_effect (name) VALUES ('b')", ())
            .expect("second pre-existing effect row should insert");
        let guard = TransactionStatement::new(
            "UPDATE cas_target SET revision = 1 WHERE id = 1 AND revision = 0",
            (),
        )
        .expect("guard statement should build");
        let statements =
            [
                TransactionStatement::new("DELETE FROM cas_effect WHERE name IN ('a', 'b')", ())
                    .expect("effect statement should build")
                    .expect_rows_affected(1),
            ];

        let error = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect_err(
                "a statement affecting more rows than expected should fail its row expectation",
            );

        assert!(error.to_string().contains("affected 2 rows"));
        assert!(error.to_string().contains("expected 1"));
        assert_eq!(cas_target_revision(&db, 1), 0);
        assert_eq!(
            cas_effect_names(&db),
            vec![String::from("a"), String::from("b")]
        );

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn execute_transactional_cas_batch_allows_a_statement_with_no_row_expectation_to_affect_zero_rows(
    ) {
        let (db, db_path) = open_cas_test_db();
        let guard = TransactionStatement::new(
            "UPDATE cas_target SET revision = 1 WHERE id = 1 AND revision = 0",
            (),
        )
        .expect("guard statement should build");
        let statements = [TransactionStatement::new(
            "UPDATE cas_effect SET name = 'applied' WHERE name = 'missing'",
            (),
        )
        .expect("effect statement should build")];

        let applied = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect("a statement with no row expectation should not enforce a row count");

        assert!(applied);
        assert_eq!(cas_target_revision(&db, 1), 1);

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn is_retryable_turso_error_classifies_busy_and_busy_snapshot_as_retryable() {
        assert!(is_retryable_turso_error(&turso::Error::Busy(String::from(
            "database is locked"
        ))));
        assert!(is_retryable_turso_error(&turso::Error::BusySnapshot(
            String::from("snapshot is busy")
        )));
    }

    #[test]
    fn is_retryable_turso_error_classifies_every_other_variant_as_deterministic() {
        assert!(!is_retryable_turso_error(&turso::Error::Constraint(
            String::from("UNIQUE constraint failed")
        )));
        assert!(!is_retryable_turso_error(&turso::Error::Misuse(
            String::from("misuse")
        )));
        assert!(!is_retryable_turso_error(&turso::Error::Corrupt(
            String::from("corrupt")
        )));
        assert!(!is_retryable_turso_error(&turso::Error::NotAdb(
            String::from("not a database")
        )));
        assert!(!is_retryable_turso_error(&turso::Error::DatabaseFull(
            String::from("database full")
        )));
        assert!(!is_retryable_turso_error(&turso::Error::Readonly(
            String::from("readonly")
        )));
        assert!(!is_retryable_turso_error(&turso::Error::Error(
            String::from("generic error")
        )));
        assert!(!is_retryable_turso_error(&turso::Error::IoError(
            std::io::ErrorKind::Other,
            "io"
        )));
    }

    #[test]
    fn classify_turso_error_wraps_busy_as_retryable_with_the_supplied_action_context() {
        let failure = classify_turso_error(
            "test",
            "failed to begin transaction",
            &turso::Error::Busy(String::from("database is locked")),
        );

        match failure {
            WriteAttemptFailure::Retryable(err) => {
                let message = err.to_string();
                assert!(message.contains("failed to begin transaction"));
                assert!(message.contains("database is locked"));
            }
            WriteAttemptFailure::Deterministic(err) => {
                panic!("Busy should classify as retryable, got deterministic: {err}")
            }
        }
    }

    #[test]
    fn classify_turso_error_wraps_constraint_violations_as_deterministic_with_the_supplied_action_context(
    ) {
        let failure = classify_turso_error(
            "test",
            "failed to commit transaction",
            &turso::Error::Constraint(String::from("UNIQUE constraint failed")),
        );

        match failure {
            WriteAttemptFailure::Deterministic(err) => {
                let message = err.to_string();
                assert!(message.contains("failed to commit transaction"));
                assert!(message.contains("UNIQUE constraint failed"));
            }
            WriteAttemptFailure::Retryable(err) => {
                panic!("Constraint should classify as deterministic, got retryable: {err}")
            }
        }
    }

    #[test]
    fn execute_transactional_cas_batch_retries_a_begin_immediate_busy_error_and_then_succeeds() {
        const LOCK_HOLD_MS: u64 = 60;

        let (db, db_path) = open_cas_test_db();
        let lock_holder =
            TursoDb::<TestDbSpec>::new_at(&db_path).expect("second handle should open");
        lock_holder
            .execute("BEGIN IMMEDIATE", ())
            .expect("lock holder should acquire the write lock before any guard or statement runs");

        let hold_handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(LOCK_HOLD_MS));
            lock_holder
                .execute("COMMIT", ())
                .expect("lock holder should release the write lock");
        });

        let guard = TransactionStatement::new(
            "UPDATE cas_target SET revision = 1 WHERE id = 1 AND revision = 0",
            (),
        )
        .expect("guard statement should build");
        let statements =
            [
                TransactionStatement::new("INSERT INTO cas_effect (name) VALUES ('applied')", ())
                    .expect("effect statement should build"),
            ];

        let started_at = Instant::now();
        let applied = db
            .execute_transactional_cas_batch("cas test", "retry the operation", &guard, &statements)
            .expect(
                "CAS batch should retry BEGIN IMMEDIATE through the transient lock and succeed",
            );
        let elapsed = started_at.elapsed();

        hold_handle
            .join()
            .expect("lock holder thread should finish");

        assert!(
            elapsed >= Duration::from_millis(LOCK_HOLD_MS / 2),
            "success arrived before the lock holder could plausibly have released the write lock, meaning BEGIN IMMEDIATE contention was not actually retried: {elapsed:?}"
        );
        assert!(applied);
        assert_eq!(cas_target_revision(&db, 1), 1);
        assert_eq!(cas_effect_names(&db), vec![String::from("applied")]);

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn passive_checkpoint_keeps_previously_written_data_readable() {
        let (db, db_path) = open_test_db();

        db.execute(
            "INSERT INTO checkpoint_probe (value) VALUES (?1)",
            ("hello",),
        )
        .expect("insert should succeed");

        db.passive_checkpoint()
            .expect("passive checkpoint should succeed");

        let values = db
            .query_map("SELECT value FROM checkpoint_probe", (), |row| {
                row.get::<String>(0).map_err(Into::into)
            })
            .expect("post-checkpoint read should succeed");

        assert_eq!(values, vec![String::from("hello")]);

        cleanup_test_db(db, &db_path);
    }

    #[test]
    fn passive_checkpoint_is_safe_to_call_repeatedly() {
        let (db, db_path) = open_test_db();

        db.passive_checkpoint()
            .expect("first passive checkpoint should succeed");
        db.passive_checkpoint()
            .expect("second passive checkpoint should succeed");

        cleanup_test_db(db, &db_path);
    }

    fn worst_case_retry_failure_budget_ms(policy: RetryPolicy) -> u64 {
        let attempt_timeouts = policy
            .timeout_ms
            .saturating_mul(u64::from(policy.max_attempts));
        let retry_backoffs = (2..=policy.max_attempts)
            .map(|attempt| retry_backoff_ms(policy, attempt))
            .fold(0_u64, u64::saturating_add);

        attempt_timeouts.saturating_add(retry_backoffs)
    }

    fn retry_backoff_ms(policy: RetryPolicy, attempt: u32) -> u64 {
        if attempt <= 1 {
            return 0;
        }

        let exponent = (attempt - 2).min(20);
        let multiplier = 1_u64 << exponent;

        policy
            .initial_backoff_ms
            .saturating_mul(multiplier)
            .min(policy.max_backoff_ms)
    }

    #[test]
    fn default_query_retry_policy_stays_within_two_second_failure_budget() {
        let budget_ms = worst_case_retry_failure_budget_ms(QUERY_RETRY_POLICY);

        assert!(
            budget_ms <= QUERY_RETRY_FAILURE_BUDGET_MS,
            "default query retry failure budget was {budget_ms}ms; expected <= {QUERY_RETRY_FAILURE_BUDGET_MS}ms"
        );
    }

    fn write_contention_test_policy(
        busy_timeout_ms: u64,
        contention_deadline_ms: u64,
    ) -> WriteContentionPolicy {
        WriteContentionPolicy {
            db_name: "agent trace test",
            max_attempts: AGENT_TRACE_DB_WRITE_CONTENTION_MAX_ATTEMPTS,
            backoff_cap: Duration::from_millis(AGENT_TRACE_DB_WRITE_CONTENTION_BACKOFF_CAP_MS),
            busy_timeout: Duration::from_millis(busy_timeout_ms),
            contention_deadline: Duration::from_millis(contention_deadline_ms),
        }
    }

    fn busy_failure() -> WriteAttemptFailure {
        classify_turso_error(
            "agent trace test",
            "failed to begin transaction",
            &turso::Error::Busy(String::from("database is locked")),
        )
    }

    fn run_write_contention_test<T>(
        policy: WriteContentionPolicy,
        attempt: impl FnMut(u32) -> std::result::Result<T, WriteAttemptFailure>,
    ) -> (Result<T>, Vec<Duration>, WriteContentionCounts) {
        let mut jitter = rand::rngs::StdRng::seed_from_u64(7);
        let clock = std::cell::Cell::new(Duration::ZERO);
        let mut sleeps = Vec::new();
        let (result, counts) = count_write_contention(|| {
            run_with_write_contention_retry_using(
                policy,
                &mut || write_contention_backoff(&mut jitter, policy.backoff_cap),
                &mut |backoff| {
                    sleeps.push(backoff);
                    clock.set(clock.get() + backoff);
                },
                &mut || clock.get(),
                "write contention test",
                "retry the operation",
                attempt,
            )
        });
        (result, sleeps, counts)
    }

    struct OversleepOutcome {
        result: Result<&'static str>,
        attempt_calls: u32,
        counts: WriteContentionCounts,
    }

    fn run_oversleep_scenario(
        busy_at: Duration,
        backoff: Duration,
        oversleep: Duration,
    ) -> OversleepOutcome {
        let policy = write_contention_test_policy(500, 1_250);
        let clock = std::cell::Cell::new(Duration::ZERO);
        let mut attempt_calls = 0;
        let (result, counts) = count_write_contention(|| {
            run_with_write_contention_retry_using(
                policy,
                &mut || backoff,
                &mut |requested| clock.set(clock.get() + requested + oversleep),
                &mut || clock.get(),
                "write contention test",
                "retry the operation",
                |_| {
                    attempt_calls += 1;
                    if attempt_calls == 1 {
                        clock.set(busy_at);
                        Err(busy_failure())
                    } else {
                        Ok("written")
                    }
                },
            )
        });
        OversleepOutcome {
            result,
            attempt_calls,
            counts,
        }
    }

    #[test]
    fn agent_trace_db_write_contention_retry_seeded_jitter_is_reproducible_and_bounded() {
        let cap = Duration::from_millis(AGENT_TRACE_DB_WRITE_CONTENTION_BACKOFF_CAP_MS);
        let draw = |seed| {
            let mut jitter = rand::rngs::StdRng::seed_from_u64(seed);
            (0..64)
                .map(|_| write_contention_backoff(&mut jitter, cap))
                .collect::<Vec<_>>()
        };

        let first = draw(42);
        assert_eq!(
            first,
            draw(42),
            "a seeded jitter source must be reproducible"
        );
        assert!(first.iter().all(|backoff| *backoff <= cap));
        assert!(
            first.iter().any(|backoff| *backoff != first[0]),
            "full jitter should vary across draws"
        );
        assert_eq!(
            write_contention_backoff(&mut rand::rngs::StdRng::seed_from_u64(1), Duration::ZERO),
            Duration::ZERO
        );
    }

    #[test]
    fn agent_trace_db_write_contention_retry_start_rule_boundaries() {
        let policy = write_contention_test_policy(500, 1_250);
        let backoff = Duration::from_millis(50);

        assert!(write_contention_retry_may_sleep(
            policy,
            Duration::from_millis(699),
            backoff
        ));
        assert!(write_contention_retry_may_sleep(
            policy,
            Duration::from_millis(700),
            backoff
        ));
        assert!(!write_contention_retry_may_sleep(
            policy,
            Duration::from_millis(701),
            backoff
        ));
        assert!(!write_contention_retry_may_sleep(
            policy,
            Duration::from_millis(1_250),
            backoff
        ));
        assert!(!write_contention_retry_may_sleep(
            policy,
            Duration::from_secs(5),
            backoff
        ));

        let no_wait = write_contention_test_policy(0, 1_250);
        assert!(write_contention_retry_may_sleep(
            no_wait,
            Duration::from_millis(1_249),
            Duration::ZERO
        ));
        assert!(
            !write_contention_retry_may_sleep(
                no_wait,
                Duration::from_millis(1_250),
                Duration::ZERO
            ),
            "nothing starts once the deadline has expired, even with zero backoff and busy timeout"
        );
        assert!(
            !write_contention_retry_may_sleep(
                write_contention_test_policy(0, 0),
                Duration::ZERO,
                Duration::ZERO
            ),
            "a zero contention deadline never starts an outer retry"
        );
    }

    #[test]
    fn agent_trace_db_write_contention_retry_post_sleep_admission_requires_a_full_busy_timeout() {
        let policy = write_contention_test_policy(500, 1_250);

        assert!(write_contention_retry_may_start_now(
            policy,
            Duration::from_millis(749)
        ));
        assert!(
            write_contention_retry_may_start_now(policy, Duration::from_millis(750)),
            "remaining == busy_timeout must still admit the next attempt"
        );
        assert!(!write_contention_retry_may_start_now(
            policy,
            Duration::from_millis(751)
        ));
        assert!(!write_contention_retry_may_start_now(
            policy,
            Duration::from_millis(1_250)
        ));
        assert!(!write_contention_retry_may_start_now(
            write_contention_test_policy(0, 1_250),
            Duration::from_millis(1_250)
        ));
    }

    #[test]
    fn agent_trace_db_write_contention_retry_rejects_a_retry_after_the_backoff_sleep_oversleeps() {
        let outcome = run_oversleep_scenario(
            Duration::from_millis(690),
            Duration::from_millis(50),
            Duration::from_millis(70),
        );

        let message = outcome
            .result
            .expect_err("an oversleep past the admission budget must exhaust")
            .to_string();
        assert!(
            message.contains("failed after 1 attempt(s) under write contention"),
            "{message}"
        );
        assert!(message.contains("elapsed_ms=810"), "{message}");
        assert!(message.contains("database is locked"), "{message}");
        assert_eq!(outcome.attempt_calls, 1, "attempt 2 must not run");
        assert_eq!(
            outcome.counts,
            WriteContentionCounts {
                attempts: 1,
                outer_retries: 0,
                exhaustions: 1
            }
        );
    }

    #[derive(Clone, Debug, Default)]
    struct CapturedEvent {
        target: String,
        level: String,
        fields: BTreeMap<String, String>,
    }

    struct CapturedEventVisitor<'a>(&'a mut BTreeMap<String, String>);

    impl tracing::field::Visit for CapturedEventVisitor<'_> {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }

    #[derive(Clone, Default)]
    struct CapturingSubscriber {
        events: std::sync::Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
    }

    impl tracing::Subscriber for CapturingSubscriber {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            let mut fields = BTreeMap::new();
            event.record(&mut CapturedEventVisitor(&mut fields));
            self.events
                .lock()
                .expect("captured events mutex should not be poisoned")
                .push(CapturedEvent {
                    target: event.metadata().target().to_string(),
                    level: event.metadata().level().to_string(),
                    fields,
                });
        }

        fn enter(&self, _span: &tracing::span::Id) {}

        fn exit(&self, _span: &tracing::span::Id) {}
    }

    fn capture_tracing_events<T>(body: impl FnOnce() -> T) -> (T, Vec<CapturedEvent>) {
        let subscriber = CapturingSubscriber::default();
        let events = std::sync::Arc::clone(&subscriber.events);
        let result = tracing::subscriber::with_default(subscriber, body);
        let events = events
            .lock()
            .expect("captured events mutex should not be poisoned")
            .clone();
        (result, events)
    }

    fn contention_exhausted_events(events: &[CapturedEvent]) -> Vec<&CapturedEvent> {
        events
            .iter()
            .filter(|event| {
                event.fields.get("event_id").map(String::as_str)
                    == Some(CONTENTION_EXHAUSTED_EVENT_ID)
            })
            .collect()
    }

    #[test]
    fn agent_trace_db_contention_exhausted_error_and_event_carry_every_field() {
        let (outcome, events) = capture_tracing_events(|| {
            run_oversleep_scenario(
                Duration::from_millis(690),
                Duration::from_millis(50),
                Duration::from_millis(70),
            )
        });

        let message = outcome
            .result
            .expect_err("an oversleep past the admission budget must exhaust")
            .to_string();
        assert_eq!(
            message,
            "Operation 'write contention test' failed after 1 attempt(s) under write contention \
             (db_name=agent trace test, operation=write contention test, attempts=1, \
             busy_timeout_ms=500, contention_deadline_ms=1250 [no retry is scheduled past this cutoff], \
             elapsed_ms=810, cause=database busy (busy timeout exhausted)). \
             Last error: agent trace test failed to begin transaction: database is locked. \
             Try: retry the operation"
        );
        assert_eq!(outcome.counts.exhaustions, 1);

        let exhausted = contention_exhausted_events(&events);
        assert_eq!(
            exhausted.len(),
            1,
            "exactly one exhaustion event: {events:?}"
        );
        let event = exhausted[0];
        assert_eq!(event.target, "sce");
        assert_eq!(event.level, "WARN");
        let expected = [
            ("db_name", "agent trace test"),
            ("operation", "write contention test"),
            ("attempts", "1"),
            ("busy_timeout_ms", "500"),
            ("contention_deadline_ms", "1250"),
            ("elapsed_ms", "810"),
            ("cause", "database busy (busy timeout exhausted)"),
            (
                "last_error",
                "agent trace test failed to begin transaction: database is locked",
            ),
        ];
        for (key, value) in expected {
            assert_eq!(
                event.fields.get(key).map(String::as_str),
                Some(value),
                "field {key}: {event:?}"
            );
        }
    }

    #[test]
    fn agent_trace_db_contention_exhausted_event_is_emitted_once_after_the_attempt_cap() {
        let (outcome, events) = capture_tracing_events(|| {
            run_write_contention_test(write_contention_test_policy(0, 30_000), |_| {
                Err::<(), _>(busy_failure())
            })
        });
        let (result, _sleeps, counts) = outcome;

        let message = result
            .expect_err("persistent Busy must exhaust")
            .to_string();
        assert!(message.contains("attempts=2"), "{message}");
        assert_eq!(counts.exhaustions, 1);

        let exhausted = contention_exhausted_events(&events);
        assert_eq!(exhausted.len(), 1, "{events:?}");
        assert_eq!(
            exhausted[0].fields.get("attempts").map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn agent_trace_db_contention_exhausted_event_is_not_emitted_for_success_or_deterministic_errors(
    ) {
        let ((), events) = capture_tracing_events(|| {
            let mut calls = 0;
            let _ = run_write_contention_test(write_contention_test_policy(0, 30_000), |_| {
                calls += 1;
                if calls == 1 {
                    Err(busy_failure())
                } else {
                    Ok(())
                }
            });
            let _ = run_write_contention_test(write_contention_test_policy(0, 30_000), |_| {
                Err::<(), _>(WriteAttemptFailure::Deterministic(anyhow::anyhow!(
                    "constraint failed"
                )))
            });
        });

        assert!(
            contention_exhausted_events(&events).is_empty(),
            "{events:?}"
        );
    }

    #[test]
    fn agent_trace_db_write_contention_retry_admits_a_retry_when_post_sleep_remaining_equals_busy_timeout(
    ) {
        let outcome = run_oversleep_scenario(
            Duration::from_millis(690),
            Duration::from_millis(50),
            Duration::from_millis(10),
        );

        assert_eq!(
            outcome
                .result
                .expect("remaining == busy_timeout must admit attempt 2"),
            "written"
        );
        assert_eq!(outcome.attempt_calls, 2);
        assert_eq!(
            outcome.counts,
            WriteContentionCounts {
                attempts: 2,
                outer_retries: 1,
                exhaustions: 0
            }
        );
    }

    #[test]
    fn agent_trace_db_write_contention_retry_retries_busy_and_busy_snapshot_once() {
        for failure in [
            turso::Error::Busy(String::from("database is locked")),
            turso::Error::BusySnapshot(String::from("snapshot is stale")),
        ] {
            let mut failure = Some(failure);
            let (result, sleeps, counts) = run_write_contention_test(
                write_contention_test_policy(0, 30_000),
                |_| match failure.take() {
                    Some(error) => Err(classify_turso_error("agent trace test", "write", &error)),
                    None => Ok("written"),
                },
            );

            assert_eq!(result.expect("the retry should succeed"), "written");
            assert_eq!(sleeps.len(), 1);
            assert!(
                sleeps[0] <= Duration::from_millis(AGENT_TRACE_DB_WRITE_CONTENTION_BACKOFF_CAP_MS)
            );
            assert_eq!(
                counts,
                WriteContentionCounts {
                    attempts: 2,
                    outer_retries: 1,
                    exhaustions: 0
                }
            );
        }
    }

    #[test]
    fn agent_trace_db_write_contention_retry_fails_deterministic_errors_once_without_sleeping() {
        for error in [
            turso::Error::Constraint(String::from("UNIQUE constraint failed")),
            turso::Error::Misuse(String::from("misuse")),
            turso::Error::Readonly(String::from("readonly")),
        ] {
            let mut error = Some(error);
            let (result, sleeps, counts) =
                run_write_contention_test(write_contention_test_policy(0, 30_000), |_| {
                    Err::<(), _>(classify_turso_error(
                        "agent trace test",
                        "write",
                        &error
                            .take()
                            .expect("a deterministic error must not be retried"),
                    ))
                });

            assert!(result.is_err());
            assert!(sleeps.is_empty(), "a deterministic error must not sleep");
            assert_eq!(
                counts,
                WriteContentionCounts {
                    attempts: 1,
                    outer_retries: 0,
                    exhaustions: 0
                }
            );
        }
    }

    #[test]
    fn agent_trace_db_write_contention_retry_caps_outer_attempts_at_two() {
        let (result, sleeps, counts) =
            run_write_contention_test(write_contention_test_policy(0, 30_000), |_| {
                Err::<(), _>(busy_failure())
            });

        let message = result
            .expect_err("persistent Busy must exhaust")
            .to_string();
        assert!(message.contains("failed after 2 attempt(s)"), "{message}");
        assert!(message.contains("database is locked"), "{message}");
        assert!(message.contains("Try: retry the operation"), "{message}");
        assert_eq!(sleeps.len(), 1);
        assert_eq!(
            counts,
            WriteContentionCounts {
                attempts: 2,
                outer_retries: 1,
                exhaustions: 1
            }
        );
    }

    #[test]
    fn agent_trace_db_write_contention_retry_starts_no_retry_the_rule_disallows() {
        for policy in [
            write_contention_test_policy(0, 0),
            write_contention_test_policy(1_251, 1_250),
        ] {
            let (result, sleeps, counts) =
                run_write_contention_test(policy, |_| Err::<(), _>(busy_failure()));

            let message = result.expect_err("Busy must fail").to_string();
            assert!(message.contains("failed after 1 attempt(s)"), "{message}");
            assert!(sleeps.is_empty());
            assert_eq!(
                counts,
                WriteContentionCounts {
                    attempts: 1,
                    outer_retries: 0,
                    exhaustions: 1
                }
            );
        }
    }

    #[test]
    fn agent_trace_db_write_contention_retry_policy_applies_only_to_agent_trace_db() {
        assert_eq!(
            write_contention_policy::<AgentTraceTestDbSpec>(),
            Some(write_contention_test_policy(
                AGENT_TRACE_DB_BUSY_TIMEOUT_MS,
                AGENT_TRACE_DB_CONTENTION_DEADLINE_MS
            ))
        );
        assert_eq!(write_contention_policy::<TestDbSpec>(), None);
    }

    #[test]
    fn agent_trace_db_write_contention_retry_leaves_reads_and_other_writes_on_the_generic_policy() {
        assert_eq!(
            resolve_query_retry_policy::<AgentTraceTestDbSpec>(),
            QUERY_RETRY_POLICY
        );

        let db_path = unique_test_db_path();
        let db = TursoDb::<AgentTraceTestDbSpec>::new_at(&db_path).expect("test DB should open");
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", ())
            .expect("table should be created");

        let ((), generic) = count_write_contention(|| {
            db.execute("INSERT INTO t (id, v) VALUES (1, 'a')", ())
                .expect("generic execute should succeed");
            db.query("SELECT v FROM t", ())
                .expect("query should succeed");
            db.query_values("SELECT v FROM t", ())
                .expect("query_values should succeed");
            db.query_map("SELECT v FROM t", (), |row| {
                row.get::<String>(0).map_err(Into::into)
            })
            .expect("query_map should succeed");
            db.passive_checkpoint()
                .expect("passive checkpoint should succeed");
        });
        assert_eq!(
            generic,
            WriteContentionCounts::default(),
            "reads, passive_checkpoint, and generic execute must not use the write-contention policy"
        );

        let (affected, opted_in) = count_write_contention(|| {
            db.execute_idempotent_write(
                "INSERT INTO t (id, v) VALUES (1, 'b') ON CONFLICT (id) DO NOTHING",
                (),
            )
            .expect("idempotent write should succeed")
        });
        assert_eq!(affected, 0);
        assert_eq!(opted_in.attempts, 1);

        drop(db);
        if let Some(parent) = db_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }
}
