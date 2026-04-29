//! Local Turso database adapter for agent traces.
//!
//! Provides a `LocalDb` struct that wraps a Turso connection with a tokio
//! runtime for blocking operations. Migrations are embedded at compile time
//! via `include_str!` from `cli/migrations/`.

use anyhow::{Context, Result};

/// Embedded migration SQL files.
///
/// Migrations are loaded at compile time from `cli/migrations/`.
/// The numeric prefix determines execution order.
#[allow(dead_code)]
const MIGRATION_001: &str = include_str!("../../../migrations/001_create_agent_traces.sql");

/// Ordered list of embedded migrations (id, sql).
#[allow(dead_code)]
const MIGRATIONS: &[(&str, &str)] = &[
    ("001", MIGRATION_001),
    // Add new migrations here with sequential IDs
];

/// Local Turso database adapter.
///
/// Wraps a Turso connection with a lazily-initialized tokio current-thread
/// runtime so that callers can use synchronous `execute`/`query` methods.
#[allow(dead_code)]
pub struct LocalDb {
    conn: turso::Connection,
    runtime: tokio::runtime::Runtime,
}

#[allow(dead_code)]
impl LocalDb {
    /// Open or create a local Turso database at the canonical path.
    ///
    /// The path is resolved from the shared default-path catalog
    /// (`cli/src/services/default_paths.rs`). Parent directories are
    /// created automatically.
    ///
    /// Migrations are run automatically after the database is opened.
    pub fn new() -> Result<Self> {
        let db_path =
            super::default_paths::local_db_path().context("failed to resolve local DB path")?;

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create local DB parent directory: {}",
                    parent.display()
                )
            })?;
        }

        // Build a current-thread tokio runtime for async turso operations
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .context("failed to create local DB tokio runtime. Try: rerun the command; if the issue persists, verify the local Tokio runtime environment.")?;

        // Open or create the database, then connect
        let conn = runtime.block_on(async {
            let path_str = db_path.to_str().ok_or_else(|| {
                anyhow::anyhow!("invalid UTF-8 in database path: {}", db_path.display())
            })?;
            let db = turso::Builder::new_local(path_str)
                .build()
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to open local database at {}: {e}",
                        db_path.display()
                    )
                })?;
            db.connect()
                .map_err(|e| anyhow::anyhow!("failed to connect to local database: {e}"))
        })?;

        let db = Self { conn, runtime };

        // Run migrations after connection is established
        db.run_migrations()
            .context("failed to run local DB migrations")?;

        Ok(db)
    }

    /// Execute a SQL statement that does not return rows.
    ///
    /// # Arguments
    /// * `sql` - SQL statement (may contain `?` placeholders)
    /// * `params` - Parameter values implementing `IntoParams`
    ///
    /// # Returns
    /// Number of rows affected.
    pub fn execute(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<u64> {
        self.runtime.block_on(async {
            self.conn
                .execute(sql, params)
                .await
                .map_err(|e| anyhow::anyhow!("local DB execute failed: {sql}: {e}"))
        })
    }

    /// Execute a SQL query that returns rows.
    ///
    /// # Arguments
    /// * `sql` - SQL query (may contain `?` placeholders)
    /// * `params` - Parameter values implementing `IntoParams`
    ///
    /// # Returns
    /// A `turso::Rows` iterator over the result set.
    pub fn query(&self, sql: &str, params: impl turso::params::IntoParams) -> Result<turso::Rows> {
        self.runtime.block_on(async {
            self.conn
                .query(sql, params)
                .await
                .map_err(|e| anyhow::anyhow!("local DB query failed: {sql}: {e}"))
        })
    }

    /// Run all embedded migrations in order.
    ///
    /// Each migration is executed. Migrations that
    /// use `CREATE TABLE IF NOT EXISTS` are idempotent and safe to re-run.
    fn run_migrations(&self) -> Result<()> {
        for (id, sql) in MIGRATIONS {
            self.runtime.block_on(async {
                self.conn
                    .execute(sql, ())
                    .await
                    .map_err(|e| anyhow::anyhow!("migration {id} failed: {e}"))
            })?;
        }
        Ok(())
    }
}
