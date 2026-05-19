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

use crate::services::lifecycle::{
    HealthCategory, HealthFixability, HealthProblem, HealthProblemKind, HealthSeverity,
};

const MIGRATIONS_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS __sce_migrations (
    id TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)";
const SELECT_MIGRATION_SQL: &str = "SELECT id FROM __sce_migrations WHERE id = ?1 LIMIT 1";
const INSERT_MIGRATION_SQL: &str = "INSERT INTO __sce_migrations (id) VALUES (?1)";

/// Service-specific Turso database configuration.
#[allow(dead_code)]
pub trait DbSpec {
    /// Human-readable database name used in diagnostics.
    fn db_name() -> &'static str;

    /// Canonical database file path.
    fn db_path() -> Result<PathBuf>;

    /// Ordered embedded migration SQL files as `(id, sql)` pairs.
    fn migrations() -> &'static [(&'static str, &'static str)];
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

/// Generic Turso database adapter.
///
/// Wraps a Turso connection with a tokio current-thread runtime so callers can
/// use synchronous `execute`/`query` methods while the underlying Turso API
/// remains async.
///
/// Supports two modes:
/// - **Local mode** (default): opens a plain Turso file database.
/// - **Sync mode**: opens a Turso Cloud synced database when both
///   `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` environment variables are set.
///   Sync operations (`push`, `pull`, `checkpoint`, `stats`) are available
///   through explicit methods or the `sce sync` CLI command.
#[allow(dead_code)]
pub struct TursoDb<M: DbSpec> {
    conn: turso::Connection,
    runtime: tokio::runtime::Runtime,
    /// Optional sync database handle for Turso Cloud operations (push, pull,
    /// checkpoint, stats). `None` when the database is in local-only mode.
    sync_db: Option<turso::sync::Database>,
    spec: PhantomData<fn() -> M>,
}

#[allow(dead_code)]
impl<M: DbSpec> TursoDb<M> {
    /// Open or create the database at the spec-provided canonical path.
    ///
    /// Parent directories are created automatically. Migrations are run after
    /// the database connection is established.
    ///
    /// When both `SCE_SYNC_URL` and `SCE_SYNC_TOKEN` are set, the database opens
    /// in sync (Turso Cloud) mode. Otherwise, local-only mode is used.
    pub fn new() -> Result<Self> {
        let db_name = M::db_name();
        let db_path = M::db_path().with_context(|| format!("failed to resolve {db_name} path"))?;

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create {db_name} parent directory: {}",
                    parent.display()
                )
            })?;
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .with_context(|| {
                format!("failed to create {db_name} tokio runtime. Try: rerun the command; if the issue persists, verify the local Tokio runtime environment.")
            })?;

        let sync_url = std::env::var(crate::services::config::SYNC_URL_ENV_KEY).ok();
        let sync_token = std::env::var(crate::services::config::SYNC_TOKEN_ENV_KEY).ok();

        let path_str = db_path.to_str().ok_or_else(|| {
            anyhow::anyhow!("invalid UTF-8 in database path: {}", db_path.display())
        })?;

        if let (Some(url), Some(token)) = (sync_url, sync_token) {
            let (conn, sync_db) = runtime.block_on(async {
                let sync_db = turso::sync::Builder::new_remote(path_str)
                    .with_remote_url(url)
                    .with_auth_token(token)
                    .build()
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "failed to open {db_name} synced database at {}: {e}",
                            db_path.display()
                        )
                    })?;
                let conn = sync_db.connect().await.map_err(|e| {
                    anyhow::anyhow!("failed to connect to {db_name} synced database: {e}")
                })?;
                Ok::<_, anyhow::Error>((conn, sync_db))
            })?;

            let db = Self {
                conn,
                runtime,
                sync_db: Some(sync_db),
                spec: PhantomData,
            };

            db.run_migrations()
                .with_context(|| format!("failed to run {db_name} migrations"))?;

            Ok(db)
        } else {
            let conn = runtime.block_on(async {
                let db = turso::Builder::new_local(path_str)
                    .build()
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "failed to open {db_name} database at {}: {e}",
                            db_path.display()
                        )
                    })?;
                db.connect()
                    .map_err(|e| anyhow::anyhow!("failed to connect to {db_name} database: {e}"))
            })?;

            let db = Self {
                conn,
                runtime,
                sync_db: None,
                spec: PhantomData,
            };

            db.run_migrations()
                .with_context(|| format!("failed to run {db_name} migrations"))?;

            Ok(db)
        }
    }

    /// Execute a SQL statement that does not return rows.
    ///
    /// # Arguments
    /// * `sql` - SQL statement, which may contain `?` placeholders.
    /// * `params` - Parameter values implementing `IntoParams`.
    ///
    /// # Returns
    /// Number of rows affected.
    ///
    /// Sync is not triggered automatically — callers use the explicit `push()`
    /// or `pull()` methods (or the `sce sync` CLI command) to sync with the
    /// remote.
    pub fn execute(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<u64> {
        self.runtime.block_on(async {
            self.conn
                .execute(sql, params)
                .await
                .map_err(|e| anyhow::anyhow!("{} execute failed: {sql}: {e}", M::db_name()))
        })
    }

    /// Push local changes to the remote (sync mode only).
    ///
    /// In local mode this is a no-op that returns `Ok(())`.
    pub fn push(&self) -> Result<()> {
        match self.sync_db {
            Some(ref sync_db) => self
                .runtime
                .block_on(sync_db.push())
                .map_err(|e| anyhow::anyhow!("{} sync push failed: {e}", M::db_name())),
            None => Ok(()),
        }
    }

    /// Pull remote changes (sync mode only).
    ///
    /// Returns `true` if any changes were applied to the local database.
    /// In local mode this is a no-op that returns `Ok(false)`.
    pub fn pull(&self) -> Result<bool> {
        match self.sync_db {
            Some(ref sync_db) => self
                .runtime
                .block_on(sync_db.pull())
                .map_err(|e| anyhow::anyhow!("{} sync pull failed: {e}", M::db_name())),
            None => Ok(false),
        }
    }

    /// Force a WAL checkpoint (sync mode only).
    ///
    /// In local mode this is a no-op that returns `Ok(())`.
    pub fn checkpoint(&self) -> Result<()> {
        match self.sync_db {
            Some(ref sync_db) => self
                .runtime
                .block_on(sync_db.checkpoint())
                .map_err(|e| anyhow::anyhow!("{} sync checkpoint failed: {e}", M::db_name())),
            None => Ok(()),
        }
    }

    /// Retrieve sync statistics (sync mode only).
    ///
    /// Returns `None` in local mode.
    pub fn stats(&self) -> Result<Option<turso::sync::DatabaseSyncStats>> {
        match self.sync_db {
            Some(ref sync_db) => self
                .runtime
                .block_on(sync_db.stats())
                .map(Some)
                .map_err(|e| anyhow::anyhow!("{} sync stats failed: {e}", M::db_name())),
            None => Ok(None),
        }
    }

    /// Returns `true` if the database is in sync (Turso Cloud) mode.
    pub fn is_sync_mode(&self) -> bool {
        self.sync_db.is_some()
    }

    /// Execute a SQL query that returns rows.
    ///
    /// # Arguments
    /// * `sql` - SQL query, which may contain `?` placeholders.
    /// * `params` - Parameter values implementing `IntoParams`.
    ///
    /// # Returns
    /// A `turso::Rows` iterator over the result set.
    pub fn query(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<turso::Rows> {
        self.runtime.block_on(async {
            self.conn
                .query(sql, params)
                .await
                .map_err(|e| anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name()))
        })
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
        self.runtime.block_on(async {
            let mut rows = self
                .conn
                .query(sql, params)
                .await
                .map_err(|e| anyhow::anyhow!("{} query failed: {sql}: {e}", M::db_name()))?;
            let mut results = Vec::new();

            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| anyhow::anyhow!("{} row fetch failed: {sql}: {e}", M::db_name()))?
            {
                results.push(
                    map_row(&row)
                        .with_context(|| format!("{} row mapping failed: {sql}", M::db_name()))?,
                );
            }

            Ok(results)
        })
    }

    /// Run all embedded migrations in order.
    ///
    /// Applied migration IDs are recorded in `__sce_migrations` so later
    /// initializations apply only migrations that were not already recorded.
    /// Existing databases without migration metadata are brought forward by
    /// re-applying the current idempotent migration set and recording each ID.
    pub fn run_migrations(&self) -> Result<()> {
        self.ensure_migrations_table()?;

        for (id, sql) in M::migrations() {
            if self.is_migration_applied(id)? {
                continue;
            }

            self.apply_migration(id, sql)?;
        }

        Ok(())
    }

    fn ensure_migrations_table(&self) -> Result<()> {
        self.runtime.block_on(async {
            self.conn
                .execute(MIGRATIONS_TABLE_SQL, ())
                .await
                .map_err(|e| {
                    anyhow::anyhow!("{} migration metadata setup failed: {e}", M::db_name())
                })
        })?;

        Ok(())
    }

    fn is_migration_applied(&self, id: &str) -> Result<bool> {
        self.runtime.block_on(async {
            let mut rows = self
                .conn
                .query(SELECT_MIGRATION_SQL, (id,))
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "{} migration metadata query failed for {id}: {e}",
                        M::db_name()
                    )
                })?;

            rows.next().await.map(|row| row.is_some()).map_err(|e| {
                anyhow::anyhow!(
                    "{} migration metadata row fetch failed for {id}: {e}",
                    M::db_name()
                )
            })
        })
    }

    fn apply_migration(&self, id: &str, sql: &str) -> Result<()> {
        self.runtime.block_on(async {
            self.conn
                .execute(sql, ())
                .await
                .map_err(|e| anyhow::anyhow!("{} migration {id} failed: {e}", M::db_name()))?;
            self.conn
                .execute(INSERT_MIGRATION_SQL, (id,))
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "{} migration metadata record failed for {id}: {e}",
                        M::db_name()
                    )
                })?;

            Ok(())
        })
    }
}
