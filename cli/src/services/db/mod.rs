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
const ENCRYPTION_CIPHER_AEGIS256: &str = "aegis256";

pub mod encryption_key;

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
    runtime.block_on(async {
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
    runtime.block_on(async {
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
    runtime.block_on(async {
        conn.execute(sql, ())
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

/// Generic Turso database adapter.
///
/// Wraps a Turso connection with a tokio current-thread runtime so callers can
/// use synchronous `execute`/`query` methods while the underlying Turso API
/// remains async.
#[allow(dead_code)]
pub struct TursoDb<M: DbSpec> {
    conn: turso::Connection,
    runtime: tokio::runtime::Runtime,
    spec: PhantomData<fn() -> M>,
}

/// Generic encrypted Turso database adapter.
///
/// Mirrors the structural seams of [`TursoDb`] while reserving encrypted local
/// database initialization for services that require at-rest encryption.
pub struct EncryptedTursoDb<M: DbSpec> {
    conn: turso::Connection,
    runtime: tokio::runtime::Runtime,
    spec: PhantomData<fn() -> M>,
}

#[allow(dead_code)]
impl<M: DbSpec> TursoDb<M> {
    /// Open or create the database at the spec-provided canonical path.
    ///
    /// Parent directories are created automatically. Migrations are run after
    /// the database connection is established.
    pub fn new() -> Result<Self> {
        let db_name = M::db_name();
        let db_path = M::db_path().with_context(|| format!("failed to resolve {db_name} path"))?;

        ensure_db_parent_dir(db_name, &db_path)?;

        let runtime = build_current_thread_runtime(db_name)?;

        let conn = runtime.block_on(async {
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
            db.connect()
                .map_err(|e| anyhow::anyhow!("failed to connect to {db_name} database: {e}"))
        })?;

        let db = Self {
            conn,
            runtime,
            spec: PhantomData,
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
        self.runtime.block_on(async {
            self.conn
                .execute(sql, params)
                .await
                .map_err(|e| anyhow::anyhow!("{} execute failed: {sql}: {e}", M::db_name()))
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
        run_embedded_migrations(&self.conn, &self.runtime, M::db_name(), M::migrations())
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

        let conn = runtime.block_on(async {
            let path_str = db_path.to_str().ok_or_else(|| {
                anyhow::anyhow!("invalid UTF-8 in database path: {}", db_path.display())
            })?;

            let encryption_opts = turso::EncryptionOpts {
                hexkey: encryption_key,
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
        })?;

        let db = Self {
            conn,
            runtime,
            spec: PhantomData,
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
        self.runtime.block_on(async {
            self.conn
                .execute(sql, params)
                .await
                .map_err(|e| anyhow::anyhow!("{} execute failed: {sql}: {e}", M::db_name()))
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
        run_embedded_migrations(&self.conn, &self.runtime, M::db_name(), M::migrations())
    }
}
