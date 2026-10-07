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
use rand::SeedableRng;
use turso::Value as TursoValue;

use crate::services::config::{AgentTraceDbRetryConfig, DatabaseRetryConfig};
use crate::services::lifecycle::{
    HealthCategory, HealthFixability, HealthProblem, HealthProblemKind, HealthSeverity,
};
use crate::services::resilience::{run_with_retry_elapsed, RetryPolicy};

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
const AGENT_TRACE_DB_BUSY_TIMEOUT_MS: u64 = 1_000;
const AGENT_TRACE_DB_CONTENTION_DEADLINE_MS: u64 = 2_250;
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

async fn run_embedded_migrations(
    conn: &turso::Connection,
    db_name: &str,
    migrations: &[(&str, &str)],
) -> Result<()> {
    ensure_migrations_table(conn, db_name).await?;

    for (id, sql) in migrations {
        if is_migration_applied(conn, db_name, id).await? {
            continue;
        }

        apply_migration(conn, db_name, id, sql).await?;
    }

    Ok(())
}

async fn ensure_migrations_table(conn: &turso::Connection, db_name: &str) -> Result<()> {
    async {
        conn.execute(MIGRATIONS_TABLE_SQL, ())
            .await
            .map_err(|e| anyhow::anyhow!("{db_name} migration metadata setup failed: {e}"))
    }
    .await?;

    Ok(())
}

async fn is_migration_applied(conn: &turso::Connection, db_name: &str, id: &str) -> Result<bool> {
    async {
        let mut rows = conn.query(SELECT_MIGRATION_SQL, (id,)).await.map_err(|e| {
            anyhow::anyhow!("{db_name} migration metadata query failed for {id}: {e}")
        })?;

        rows.next().await.map(|row| row.is_some()).map_err(|e| {
            anyhow::anyhow!("{db_name} migration metadata row fetch failed for {id}: {e}")
        })
    }
    .await
}

async fn apply_migration(
    conn: &turso::Connection,
    db_name: &str,
    id: &str,
    sql: &str,
) -> Result<()> {
    async {
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
    }
    .await
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
    pub async fn new(sql: &'a str, params: impl turso::params::IntoParams) -> Result<Self> {
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
    spec: PhantomData<fn() -> M>,
}

impl<M: DbSpec> TursoConnectionCore<M> {
    fn new(conn: turso::Connection) -> Self {
        Self {
            conn,
            spec: PhantomData,
        }
    }

    async fn run_migrations(&self) -> Result<()> {
        run_embedded_migrations(&self.conn, M::db_name(), M::migrations()).await
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

async fn run_with_write_contention_retry<
    T,
    Fut: std::future::Future<Output = std::result::Result<T, WriteAttemptFailure>>,
>(
    policy: WriteContentionPolicy,
    operation_name: &str,
    retry_hint: &str,
    attempt: impl FnMut(u32) -> Fut,
) -> Result<T> {
    let mut jitter = rand::rngs::StdRng::from_entropy();
    let started_at = std::time::Instant::now();
    run_with_write_contention_retry_using(
        policy,
        &mut || write_contention_backoff(&mut jitter, policy.backoff_cap),
        &mut tokio::time::sleep,
        &mut || started_at.elapsed(),
        operation_name,
        retry_hint,
        attempt,
    )
    .await
}

async fn run_with_write_contention_retry_using<
    T,
    Fut: std::future::Future<Output = std::result::Result<T, WriteAttemptFailure>>,
    Sleep: std::future::Future<Output = ()>,
>(
    policy: WriteContentionPolicy,
    draw_backoff: &mut impl FnMut() -> std::time::Duration,
    sleep: &mut impl FnMut(std::time::Duration) -> Sleep,
    elapsed: &mut impl FnMut() -> std::time::Duration,
    operation_name: &str,
    retry_hint: &str,
    mut attempt: impl FnMut(u32) -> Fut,
) -> Result<T> {
    let mut attempt_number = 0;

    loop {
        attempt_number += 1;

        let outcome = attempt(attempt_number).await;

        let error = match outcome {
            Ok(value) => return Ok(value),
            Err(WriteAttemptFailure::Deterministic(error)) => return Err(error),
            Err(WriteAttemptFailure::Retryable(error)) => error,
        };

        if attempt_number < policy.max_attempts {
            let backoff = draw_backoff();
            if write_contention_retry_may_sleep(policy, elapsed(), backoff) {
                sleep(backoff).await;

                if write_contention_retry_may_start_now(policy, elapsed()) {
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

/// Record that one [`TursoDb`] read statement was issued on this thread.

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
    pub async fn new() -> Result<Self> {
        let db = Self::open_without_migrations().await?;

        db.run_migrations()
            .await
            .with_context(|| format!("failed to run {} migrations", M::db_name()))?;

        Ok(db)
    }

    /// Open or create the database at an explicit path.
    ///
    /// Parent directories are created automatically. Migrations are run after
    /// the database connection is established. The service-specific retry and
    /// migration configuration still comes from `M`.
    pub async fn new_at(db_path: impl AsRef<Path>) -> Result<Self> {
        let db = Self::open_without_migrations_at(db_path).await?;

        db.run_migrations()
            .await
            .with_context(|| format!("failed to run {} migrations", M::db_name()))?;

        Ok(db)
    }

    /// Open or create the database at the spec-provided canonical path without
    /// running embedded migrations.
    ///
    /// Parent directories are created automatically and the connection-open
    /// retry policy is preserved. Runtime callers that use this path are
    /// responsible for verifying schema readiness before query/write work.
    pub async fn open_without_migrations() -> Result<Self> {
        let db_name = M::db_name();
        let db_path = M::db_path().with_context(|| format!("failed to resolve {db_name} path"))?;

        Self::open_without_migrations_at(db_path).await
    }

    /// Open or create the database at an explicit path without running embedded
    /// migrations.
    ///
    /// Parent directories are created automatically and the connection-open
    /// retry policy is preserved. Runtime callers that use this path are
    /// responsible for verifying schema readiness before query/write work.
    pub async fn open_without_migrations_at(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_name = M::db_name();
        let db_path = db_path.as_ref().to_path_buf();

        ensure_db_parent_dir(db_name, &db_path)?;

        let retry_policy = resolve_connection_open_retry_policy::<M>();
        let busy_timeout = resolve_busy_timeout::<M>();
        let operation_name = format!("open {db_name} database connection");

        let conn = run_with_retry_elapsed(
            retry_policy,
            &operation_name,
            CONNECTION_OPEN_RETRY_HINT,
            async |_| {
                async {
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
                }
                .await
            },
        )
        .await?;

        Ok(Self {
            core: TursoConnectionCore::new(conn),
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
    pub async fn execute(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<u64> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("execute {} database query", M::db_name());

        run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            async |_| {
                async {
                    self.core
                        .conn
                        .execute(sql, params.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("{} execute failed: {sql}: {e}", M::db_name()))
                }
                .await
            },
        )
        .await
    }

    pub async fn execute_idempotent_write(
        &self,
        sql: &str,
        params: impl turso::params::IntoParams,
    ) -> Result<u64> {
        let Some(policy) = write_contention_policy::<M>() else {
            return self.execute(sql, params).await;
        };
        let db_name = M::db_name();
        let params = turso::params::IntoParams::into_params(params)
            .map_err(|e| anyhow::anyhow!("{db_name} parameter conversion failed: {sql}: {e}"))?;
        let operation_name = format!("execute {db_name} database query");

        run_with_write_contention_retry(policy, &operation_name, QUERY_RETRY_HINT, async |_| {
            async {
                self.core
                    .conn
                    .execute(sql, params.clone())
                    .await
                    .map_err(|e| {
                        classify_turso_error(db_name, &format!("execute failed: {sql}"), &e)
                    })
            }
            .await
        })
        .await
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
    pub async fn query(
        &self,
        sql: &str,
        params: impl turso::params::IntoParams,
    ) -> Result<turso::Rows> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("query {} database", M::db_name());

        run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            async |_| {
                async {
                    self.core
                        .conn
                        .query(sql, params.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name()))
                }
                .await
            },
        )
        .await
    }

    /// Execute a SQL query and synchronously fetch column names plus raw values.
    #[allow(dead_code)]
    pub async fn query_values(
        &self,
        sql: &str,
        params: impl turso::params::IntoParams,
    ) -> Result<QueryRows> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("query and fetch {} database values", M::db_name());

        run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            async |_| {
                async {
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
                }
                .await
            },
        )
        .await
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
    pub async fn execute_transactional_insert_pair_if_absent(
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

        let run_attempt = async || {
            async {
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
            }
            .await
        };

        if let Some(policy) = write_contention_policy::<M>() {
            return run_with_write_contention_retry(policy, operation_name, retry_hint, |_| {
                run_attempt()
            })
            .await;
        }

        run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            operation_name,
            retry_hint,
            async |_| run_attempt().await.map_err(WriteAttemptFailure::into_error),
        )
        .await
    }

    /// Execute a SQL query and synchronously map all returned rows.
    pub async fn query_map<T, F>(
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

        let rows = run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            async |_| {
                async {
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
                }
                .await
            },
        )
        .await?;

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
    pub async fn execute_transactional_cas_batch(
        &self,
        operation_name: &str,
        retry_hint: &str,
        guard: &TransactionStatement<'_>,
        statements: &[TransactionStatement<'_>],
    ) -> Result<bool> {
        let db_name = M::db_name();

        let run_attempt = async || {
            async {
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
            }
            .await
        };

        if let Some(policy) = write_contention_policy::<M>() {
            return run_with_write_contention_retry(policy, operation_name, retry_hint, |_| {
                run_attempt()
            })
            .await;
        }

        let outcome = run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            operation_name,
            retry_hint,
            async |_| match run_attempt().await {
                Ok(applied) => Ok(CasBatchAttemptOutcome::Settled(applied)),
                Err(failure) => cas_batch_failure_into_attempt_result(failure),
            },
        )
        .await?;

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
    pub async fn run_migrations(&self) -> Result<()> {
        self.core.run_migrations().await
    }

    /// Run a passive WAL checkpoint (`PRAGMA wal_checkpoint(PASSIVE)`).
    ///
    /// PASSIVE checkpoints only what is currently safe to move from the WAL
    /// into the main database file and never blocks on active readers or
    /// writers, so it does not guarantee WAL truncation. Safe to call
    /// repeatedly. Routine maintenance only; not a durability boundary.
    pub async fn passive_checkpoint(&self) -> Result<()> {
        let operation_name = format!("checkpoint {} database WAL", M::db_name());

        run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            async |_| {
                async {
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
                }
                .await
            },
        )
        .await
    }

    /// Check migration metadata for problems that would prevent safe hook
    /// runtime access.
    ///
    /// Returns a list of problems: missing migration metadata table,
    /// incomplete applied migrations, or unexpected extra migrations.
    /// An empty list means the schema is ready.
    pub async fn migration_metadata_problems(&self) -> Result<Vec<String>> {
        let migration_table_exists = self.query_map(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '__sce_migrations' LIMIT 1",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        ).await?;

        if migration_table_exists.is_empty() {
            return Ok(vec![String::from("missing migration metadata table")]);
        }

        let applied_ids = self
            .query_map(
                "SELECT id FROM __sce_migrations ORDER BY id ASC",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .await?;
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
    pub async fn ensure_schema_ready(&self, setup_guidance: &str) -> Result<()> {
        let problems = self.migration_metadata_problems().await?;

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
    pub async fn new() -> Result<Self> {
        let db_name = M::db_name();
        let db_path = M::db_path().with_context(|| format!("failed to resolve {db_name} path"))?;
        let encryption_key = encryption_key::get_or_create_encryption_key(&db_path, db_name)?;

        ensure_db_parent_dir(db_name, &db_path)?;

        let retry_policy = resolve_connection_open_retry_policy::<M>();
        let operation_name = format!("open encrypted {db_name} database connection");

        let conn = run_with_retry_elapsed(
            retry_policy,
            &operation_name,
            CONNECTION_OPEN_RETRY_HINT,
            async |_| {
                async {
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
                }.await
            },
        ).await?;

        let db = Self {
            core: TursoConnectionCore::new(conn),
        };

        db.run_migrations()
            .await
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
    pub async fn execute(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<u64> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("execute encrypted {} database query", M::db_name());

        run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            async |_| {
                async {
                    self.core
                        .conn
                        .execute(sql, params.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("{} execute failed: {sql}: {e}", M::db_name()))
                }
                .await
            },
        )
        .await
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
    pub async fn query(
        &self,
        sql: &str,
        params: impl turso::params::IntoParams,
    ) -> Result<turso::Rows> {
        let params = turso::params::IntoParams::into_params(params).map_err(|e| {
            anyhow::anyhow!("{} parameter conversion failed: {sql}: {e}", M::db_name())
        })?;
        let operation_name = format!("query encrypted {} database", M::db_name());

        run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            async |_| {
                async {
                    self.core
                        .conn
                        .query(sql, params.clone())
                        .await
                        .map_err(|e| anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name()))
                }
                .await
            },
        )
        .await
    }

    /// Execute a SQL query and synchronously map all returned rows.
    pub async fn query_map<T, F>(
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

        let rows = run_with_retry_elapsed(
            resolve_query_retry_policy::<M>(),
            &operation_name,
            QUERY_RETRY_HINT,
            async |_| {
                async {
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
                }
                .await
            },
        )
        .await?;

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
    pub async fn run_migrations(&self) -> Result<()> {
        self.core.run_migrations().await
    }
}
