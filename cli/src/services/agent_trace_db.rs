//! Agent trace local Turso database adapter.
//!
//! Provides an `AgentTraceDb` struct that wraps a Turso connection with a tokio
//! runtime for blocking operations plus focused persistence helpers for agent
//! trace diff-trace payloads. Migrations are embedded at compile time via
//! `include_str!` from `cli/migrations/agent-trace/`.

use anyhow::{Context, Result};

/// Embedded migration SQL files.
///
/// Migrations are loaded at compile time from `cli/migrations/agent-trace/`.
/// The numeric prefix determines execution order.
#[allow(dead_code)]
const MIGRATION_001: &str = include_str!("../../migrations/agent-trace/001_create_diff_traces.sql");

/// Ordered list of embedded migrations (id, sql).
#[allow(dead_code)]
const MIGRATIONS: &[(&str, &str)] = &[
    ("001", MIGRATION_001),
    // Add new migrations here with sequential IDs
];

const INSERT_DIFF_TRACE_SQL: &str =
    "INSERT INTO diff_traces (time_ms, session_id, patch) VALUES (?1, ?2, ?3)";

/// Validated diff-trace payload fields ready for agent-trace DB insertion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffTraceInsert<'a> {
    /// Incoming `time` value as Unix epoch milliseconds (signed for `SQLite` `INTEGER` compatibility).
    pub time_ms: i64,
    /// Incoming `sessionID` value.
    pub session_id: &'a str,
    /// Incoming `diff` payload body stored as a patch.
    pub patch: &'a str,
}

/// Agent trace local Turso database adapter.
///
/// Wraps a Turso connection with a lazily-initialized tokio current-thread
/// runtime so that callers can use synchronous local DB methods.
#[allow(dead_code)]
pub struct AgentTraceDb {
    conn: turso::Connection,
    runtime: tokio::runtime::Runtime,
}

#[allow(dead_code)]
impl AgentTraceDb {
    /// Open or create an agent-trace database at the canonical path.
    ///
    /// The path is resolved from the shared default-path catalog
    /// (`cli/src/services/default_paths.rs`). Parent directories are
    /// created automatically.
    ///
    /// Migrations are run automatically after the database is opened.
    pub fn new() -> Result<Self> {
        let db_path = super::default_paths::agent_trace_db_path()
            .context("failed to resolve agent-trace DB path")?;

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create agent-trace DB parent directory: {}",
                    parent.display()
                )
            })?;
        }

        // Build a current-thread tokio runtime for async turso operations
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .context("failed to create agent-trace DB tokio runtime. Try: rerun the command; if the issue persists, verify the local Tokio runtime environment.")?;

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
                        "failed to open agent-trace database at {}: {e}",
                        db_path.display()
                    )
                })?;
            db.connect()
                .map_err(|e| anyhow::anyhow!("failed to connect to agent-trace database: {e}"))
        })?;

        let db = Self { conn, runtime };

        // Run migrations after connection is established
        db.run_migrations()
            .context("failed to run agent-trace DB migrations")?;

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
                .map_err(|e| anyhow::anyhow!("agent-trace DB execute failed: {sql}: {e}"))
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
                .map_err(|e| anyhow::anyhow!("agent-trace DB query failed: {sql}: {e}"))
        })
    }

    /// Insert one validated diff-trace payload into the agent-trace `diff_traces` table.
    ///
    /// # Errors
    ///
    /// Returns an error if the incoming millisecond timestamp exceeds `SQLite`'s
    /// signed integer range or if the agent-trace DB insert fails.
    pub fn insert_diff_trace(&self, input: DiffTraceInsert<'_>) -> Result<u64> {
        self.execute(
            INSERT_DIFF_TRACE_SQL,
            (input.time_ms, input.session_id, input.patch),
        )
        .context(
            "failed to insert diff-trace payload into agent-trace DB. Try: run 'sce doctor --fix' to verify agent-trace DB health.",
        )
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
