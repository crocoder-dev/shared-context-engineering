//! Local Turso database adapter for agent traces.
//!
//! Provides a `LocalDb` struct that wraps a Turso connection with a tokio
//! runtime for blocking operations. Migrations are embedded at compile time
//! via `include_str!` from `cli/migrations/`.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

use super::agent_trace::{AgentTrace, HunkContributor};

/// Embedded migration SQL files.
///
/// Migrations are loaded at compile time from `cli/migrations/`.
/// The numeric prefix determines execution order.
#[allow(dead_code)]
const MIGRATION_001: &str = include_str!("../../migrations/001_create_agent_traces.sql");

/// Ordered list of embedded migrations (id, sql).
#[allow(dead_code)]
const MIGRATIONS: &[(&str, &str)] = &[
    ("001", MIGRATION_001),
    // Add new migrations here with sequential IDs
];

const PLACEHOLDER_AGENT_TRACES_COLUMNS: &[&str] = &["created_at", "id", "trace_json"];
const NORMALIZED_AGENT_TRACES_COLUMNS: &[&str] = &[
    "created_at",
    "timestamp",
    "trace_id",
    "trace_json",
    "version",
];

const AGENT_TRACE_TABLES_DROP_ORDER: &[&str] = &[
    "agent_trace_ranges",
    "agent_trace_conversations",
    "agent_trace_files",
    "agent_traces",
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

        Self::open_at_path(&db_path)
    }

    fn open_at_path(db_path: &Path) -> Result<Self> {
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

    /// Insert an Agent Trace payload and its normalized query rows.
    ///
    /// The complete trace is serialized into `agent_traces.trace_json`, while
    /// files, conversations, and ranges are inserted into the normalized child
    /// tables in their existing vector order. The insert runs inside an explicit
    /// transaction so duplicate trace IDs or child-row failures do not leave
    /// partial trace data behind.
    pub fn insert_agent_trace(&self, trace: &AgentTrace) -> Result<()> {
        let trace_json = serde_json::to_string(trace)
            .context("failed to serialize Agent Trace for local DB persistence")?;

        self.runtime.block_on(async {
            self.conn
                .execute("BEGIN IMMEDIATE", ())
                .await
                .map_err(|e| anyhow::anyhow!("failed to begin local DB Agent Trace insert transaction: {e}"))?;

            let insert_result = self.insert_agent_trace_rows(trace, &trace_json).await;

            match insert_result {
                Ok(()) => self
                    .conn
                    .execute("COMMIT", ())
                    .await
                    .map(|_| ())
                    .map_err(|e| anyhow::anyhow!("failed to commit local DB Agent Trace insert transaction: {e}")),
                Err(error) => {
                    let rollback_result = self.conn.execute("ROLLBACK", ()).await;
                    if let Err(rollback_error) = rollback_result {
                        return Err(error).with_context(|| {
                            format!(
                                "local DB Agent Trace insert failed, then rollback failed: {rollback_error}"
                            )
                        });
                    }

                    Err(error.context("failed to insert Agent Trace into local DB; transaction rolled back"))
                }
            }
        })
    }

    async fn insert_agent_trace_rows(&self, trace: &AgentTrace, trace_json: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO agent_traces (trace_id, version, timestamp, trace_json) \
                 VALUES (?1, ?2, ?3, ?4)",
                (
                    trace.id.as_str(),
                    trace.version.as_str(),
                    trace.timestamp.as_str(),
                    trace_json,
                ),
            )
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to insert Agent Trace parent row for trace '{}': {e}",
                    trace.id
                )
            })?;

        for (file_index, file) in trace.files.iter().enumerate() {
            let file_index = usize_to_db_i64(file_index, "file_index")?;
            self.conn
                .execute(
                    "INSERT INTO agent_trace_files (trace_id, file_index, path) \
                     VALUES (?1, ?2, ?3)",
                    (trace.id.as_str(), file_index, file.path.as_str()),
                )
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to insert Agent Trace file row for trace '{}' at file_index {}: {e}",
                        trace.id,
                        file_index
                    )
                })?;
            let file_id = self.conn.last_insert_rowid();

            for (conversation_index, conversation) in file.conversations.iter().enumerate() {
                let conversation_index = usize_to_db_i64(conversation_index, "conversation_index")?;
                self.conn
                    .execute(
                        "INSERT INTO agent_trace_conversations \
                         (file_id, conversation_index, contributor_type) \
                         VALUES (?1, ?2, ?3)",
                        (
                            file_id,
                            conversation_index,
                            contributor_type(conversation.contributor.kind),
                        ),
                    )
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "failed to insert Agent Trace conversation row for trace '{}' at file_index {} conversation_index {}: {e}",
                            trace.id,
                            file_index,
                            conversation_index
                        )
                    })?;
                let conversation_id = self.conn.last_insert_rowid();

                for (range_index, range) in conversation.ranges.iter().enumerate() {
                    let range_index = usize_to_db_i64(range_index, "range_index")?;
                    let start_line = u64_to_db_i64(range.start_line, "start_line")?;
                    let end_line = u64_to_db_i64(range.end_line, "end_line")?;
                    self.conn
                        .execute(
                            "INSERT INTO agent_trace_ranges \
                             (conversation_id, range_index, start_line, end_line) \
                             VALUES (?1, ?2, ?3, ?4)",
                            (conversation_id, range_index, start_line, end_line),
                        )
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "failed to insert Agent Trace range row for trace '{}' at file_index {} conversation_index {} range_index {}: {e}",
                                trace.id,
                                file_index,
                                conversation_index,
                                range_index
                            )
                        })?;
                }
            }
        }

        Ok(())
    }

    /// Run all embedded migrations in order.
    ///
    /// Each migration is executed. Migrations that
    /// use `CREATE TABLE IF NOT EXISTS` are idempotent and safe to re-run.
    fn run_migrations(&self) -> Result<()> {
        self.prepare_agent_trace_schema()?;

        for (id, sql) in MIGRATIONS {
            self.runtime.block_on(async {
                self.conn
                    .execute_batch(sql)
                    .await
                    .map_err(|e| anyhow::anyhow!("migration {id} failed: {e}"))
            })?;
        }
        Ok(())
    }

    fn prepare_agent_trace_schema(&self) -> Result<()> {
        let columns = self.table_columns("agent_traces")?;

        if columns.is_empty() || columns_equal(&columns, NORMALIZED_AGENT_TRACES_COLUMNS) {
            return Ok(());
        }

        if columns_equal(&columns, PLACEHOLDER_AGENT_TRACES_COLUMNS) {
            return self.reset_placeholder_agent_trace_schema();
        }

        bail!(
            "incompatible local DB schema for agent_traces: found columns [{}]. \
             Try: move or remove the local SCE database file and rerun setup/doctor repair so \
             the normalized Agent Trace schema can be bootstrapped.",
            columns.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }

    fn reset_placeholder_agent_trace_schema(&self) -> Result<()> {
        let mut sql = String::from("BEGIN;");
        for table_name in AGENT_TRACE_TABLES_DROP_ORDER {
            sql.push_str("DROP TABLE IF EXISTS ");
            sql.push_str(table_name);
            sql.push(';');
        }
        sql.push_str("COMMIT;");

        self.runtime.block_on(async {
            self.conn
                .execute_batch(&sql)
                .await
                .map_err(|e| anyhow::anyhow!("failed to reset placeholder local DB schema: {e}"))
        })
    }

    fn table_columns(&self, table_name: &str) -> Result<BTreeSet<String>> {
        let sql = format!("PRAGMA table_info({table_name})");
        self.runtime.block_on(async {
            let mut rows = self.conn.query(sql.as_str(), ()).await.map_err(|e| {
                anyhow::anyhow!("failed to inspect local DB table {table_name}: {e}")
            })?;
            let mut columns = BTreeSet::new();
            while let Some(row) = rows.next().await.map_err(|e| {
                anyhow::anyhow!("failed to inspect local DB table {table_name}: {e}")
            })? {
                let column_name = row.get::<String>(1).map_err(|e| {
                    anyhow::anyhow!("failed to read local DB table {table_name} column name: {e}")
                })?;
                columns.insert(column_name);
            }
            Ok(columns)
        })
    }
}

fn columns_equal(columns: &BTreeSet<String>, expected: &[&str]) -> bool {
    columns.len() == expected.len() && expected.iter().all(|column| columns.contains(*column))
}

fn contributor_type(contributor: HunkContributor) -> &'static str {
    match contributor {
        HunkContributor::Ai => "ai",
        HunkContributor::Mixed => "mixed",
        HunkContributor::Unknown => "unknown",
    }
}

fn usize_to_db_i64(value: usize, label: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| {
        format!("Agent Trace {label} value {value} exceeds local DB integer range")
    })
}

fn u64_to_db_i64(value: u64, label: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| {
        format!("Agent Trace {label} value {value} exceeds local DB integer range")
    })
}
