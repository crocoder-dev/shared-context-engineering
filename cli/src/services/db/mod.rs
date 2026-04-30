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
#[allow(dead_code)]
pub struct TursoDb<M: DbSpec> {
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

        let conn = runtime.block_on(async {
            let path_str = db_path.to_str().ok_or_else(|| {
                anyhow::anyhow!("invalid UTF-8 in database path: {}", db_path.display())
            })?;
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

    /// Run all embedded migrations in order.
    ///
    /// Migrations that use idempotent SQL such as `CREATE TABLE IF NOT EXISTS`
    /// are safe to re-run.
    pub fn run_migrations(&self) -> Result<()> {
        for (id, sql) in M::migrations() {
            self.runtime.block_on(async {
                self.conn
                    .execute(sql, ())
                    .await
                    .map_err(|e| anyhow::anyhow!("{} migration {id} failed: {e}", M::db_name()))
            })?;
        }

        Ok(())
    }
}
