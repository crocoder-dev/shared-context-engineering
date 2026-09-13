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
) -> Result<bool> {
    let mut rows = tx
        .query(exists_sql, exists_params)
        .await
        .map_err(|e| anyhow::anyhow!("{db_name} existence check failed: {exists_sql}: {e}"))?;
    let already_exists = rows
        .next()
        .await
        .map_err(|e| anyhow::anyhow!("{db_name} existence row fetch failed: {exists_sql}: {e}"))?
        .is_some();

    if already_exists {
        return Ok(false);
    }

    tx.execute(first_sql, first_params)
        .await
        .map_err(|e| anyhow::anyhow!("{db_name} execute failed: {first_sql}: {e}"))?;

    if fail_before_second {
        anyhow::bail!("{db_name} injected failure before second statement (test-only)");
    }

    tx.execute(second_sql, second_params)
        .await
        .map_err(|e| anyhow::anyhow!("{db_name} execute failed: {second_sql}: {e}"))?;

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

#[allow(dead_code)]
fn is_retryable_turso_error(error: &turso::Error) -> bool {
    matches!(error, turso::Error::Busy(_) | turso::Error::BusySnapshot(_))
}

#[allow(dead_code)]
enum CasBatchFailure {
    Retryable(anyhow::Error),
    Deterministic(anyhow::Error),
}

#[allow(dead_code)]
fn classify_turso_error(db_name: &str, action: &str, error: &turso::Error) -> CasBatchFailure {
    let wrapped = anyhow::anyhow!("{db_name} {action}: {error}");

    if is_retryable_turso_error(error) {
        CasBatchFailure::Retryable(wrapped)
    } else {
        CasBatchFailure::Deterministic(wrapped)
    }
}

#[allow(dead_code)]
enum CasBatchAttemptOutcome {
    Settled(bool),
    Deterministic(anyhow::Error),
}

#[allow(dead_code)]
fn cas_batch_failure_into_attempt_result(
    failure: CasBatchFailure,
) -> Result<CasBatchAttemptOutcome> {
    match failure {
        CasBatchFailure::Retryable(err) => Err(err),
        CasBatchFailure::Deterministic(err) => Ok(CasBatchAttemptOutcome::Deterministic(err)),
    }
}

#[allow(dead_code)]
async fn execute_cas_batch_body(
    tx: &turso::transaction::Transaction<'_>,
    db_name: &str,
    guard: &TransactionStatement<'_>,
    statements: &[TransactionStatement<'_>],
) -> std::result::Result<bool, CasBatchFailure> {
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
            return Err(CasBatchFailure::Deterministic(anyhow::anyhow!(
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
                return Err(CasBatchFailure::Deterministic(anyhow::anyhow!(
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
            "agent_trace_db" => config.agent_trace_db.as_ref(),
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
            "agent_trace_db" => config.agent_trace_db.as_ref(),
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
                    db.connect().map_err(|e| {
                        anyhow::anyhow!("failed to connect to {db_name} database: {e}")
                    })
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

        run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            operation_name,
            retry_hint,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    let tx = turso::transaction::Transaction::new_unchecked(
                        &self.core.conn,
                        turso::transaction::TransactionBehavior::Immediate,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{db_name} failed to begin transaction: {e}"))?;

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
                                anyhow::anyhow!("{db_name} failed to commit transaction: {e}")
                            })?;
                            Ok(inserted)
                        }
                        Err(err) => {
                            let _ = tx.rollback().await;
                            Err(err)
                        }
                    }
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
        let operation_name = format!("query and fetch {} database rows", M::db_name());

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

        let outcome = run_with_retry_sync(
            resolve_query_retry_policy::<M>(),
            operation_name,
            retry_hint,
            |_| {
                block_on_isolated(&self.core.runtime, async {
                    let tx = match turso::transaction::Transaction::new_unchecked(
                        &self.core.conn,
                        turso::transaction::TransactionBehavior::Immediate,
                    )
                    .await
                    {
                        Ok(tx) => tx,
                        Err(e) => {
                            return cas_batch_failure_into_attempt_result(classify_turso_error(
                                db_name,
                                "failed to begin transaction",
                                &e,
                            ));
                        }
                    };

                    match execute_cas_batch_body(&tx, db_name, guard, statements).await {
                        Ok(applied) => match tx.commit().await {
                            Ok(()) => Ok(CasBatchAttemptOutcome::Settled(applied)),
                            Err(e) => cas_batch_failure_into_attempt_result(classify_turso_error(
                                db_name,
                                "failed to commit transaction",
                                &e,
                            )),
                        },
                        Err(failure) => {
                            let _ = tx.rollback().await;
                            cas_batch_failure_into_attempt_result(failure)
                        }
                    }
                })
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
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
            CasBatchFailure::Retryable(err) => {
                let message = err.to_string();
                assert!(message.contains("failed to begin transaction"));
                assert!(message.contains("database is locked"));
            }
            CasBatchFailure::Deterministic(err) => {
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
            CasBatchFailure::Deterministic(err) => {
                let message = err.to_string();
                assert!(message.contains("failed to commit transaction"));
                assert!(message.contains("UNIQUE constraint failed"));
            }
            CasBatchFailure::Retryable(err) => {
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
}
