use std::path::{Path, PathBuf};

use anyhow::{anyhow, ensure, Context, Result};
use turso::Builder;

use crate::services::resilience::{run_with_retry, RetryPolicy};

const CORE_SCHEMA_STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS repositories (\
        id INTEGER PRIMARY KEY,\
        vcs_provider TEXT NOT NULL DEFAULT 'git',\
        canonical_root TEXT NOT NULL UNIQUE,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
    )",
    "CREATE TABLE IF NOT EXISTS commits (\
        id INTEGER PRIMARY KEY,\
        repository_id INTEGER NOT NULL,\
        commit_sha TEXT NOT NULL,\
        parent_sha TEXT,\
        committed_at TEXT,\
        author_name TEXT,\
        author_email TEXT,\
        idempotency_key TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, commit_sha),\
        UNIQUE(repository_id, idempotency_key)\
    )",
    "CREATE TABLE IF NOT EXISTS trace_records (\
        id INTEGER PRIMARY KEY,\
        repository_id INTEGER NOT NULL,\
        commit_id INTEGER NOT NULL,\
        trace_id TEXT NOT NULL UNIQUE,\
        version TEXT NOT NULL,\
        content_type TEXT NOT NULL,\
        notes_ref TEXT NOT NULL,\
        payload_json TEXT NOT NULL,\
        quality_status TEXT NOT NULL,\
        idempotency_key TEXT NOT NULL,\
        recorded_at TEXT NOT NULL,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        FOREIGN KEY(commit_id) REFERENCES commits(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, idempotency_key),\
        UNIQUE(commit_id)\
    )",
    "CREATE TABLE IF NOT EXISTS trace_ranges (\
        id INTEGER PRIMARY KEY,\
        trace_record_id INTEGER NOT NULL,\
        file_path TEXT NOT NULL,\
        conversation_url TEXT NOT NULL,\
        start_line INTEGER NOT NULL,\
        end_line INTEGER NOT NULL,\
        contributor_type TEXT NOT NULL,\
        contributor_model_id TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(trace_record_id) REFERENCES trace_records(id) ON DELETE CASCADE\
    )",
    "CREATE TABLE IF NOT EXISTS reconciliation_runs (\
        id INTEGER PRIMARY KEY,\
        repository_id INTEGER NOT NULL,\
        provider TEXT NOT NULL,\
        idempotency_key TEXT NOT NULL,\
        status TEXT NOT NULL,\
        initiated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        completed_at TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, idempotency_key)\
    )",
    "CREATE TABLE IF NOT EXISTS rewrite_mappings (\
        id INTEGER PRIMARY KEY,\
        reconciliation_run_id INTEGER NOT NULL,\
        repository_id INTEGER NOT NULL,\
        old_commit_sha TEXT NOT NULL,\
        new_commit_sha TEXT,\
        mapping_status TEXT NOT NULL,\
        confidence REAL,\
        idempotency_key TEXT NOT NULL,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(reconciliation_run_id) REFERENCES reconciliation_runs(id) ON DELETE CASCADE,\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, idempotency_key)\
    )",
    "CREATE TABLE IF NOT EXISTS conversations (\
        id INTEGER PRIMARY KEY,\
        repository_id INTEGER NOT NULL,\
        url TEXT NOT NULL,\
        source TEXT NOT NULL,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, url)\
    )",
    "CREATE TABLE IF NOT EXISTS trace_retry_queue (\
        id INTEGER PRIMARY KEY,\
        trace_id TEXT NOT NULL UNIQUE,\
        commit_sha TEXT NOT NULL,\
        failed_targets TEXT NOT NULL,\
        content_type TEXT NOT NULL,\
        notes_ref TEXT NOT NULL,\
        payload_json TEXT NOT NULL,\
        attempts INTEGER NOT NULL DEFAULT 0,\
        last_error_class TEXT,\
        last_error_message TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
    )",
    "CREATE TABLE IF NOT EXISTS reconciliation_metrics (\
        id INTEGER PRIMARY KEY,\
        run_id INTEGER,\
        mapped_count INTEGER NOT NULL,\
        unmapped_count INTEGER NOT NULL,\
        histogram_high INTEGER NOT NULL,\
        histogram_medium INTEGER NOT NULL,\
        histogram_low INTEGER NOT NULL,\
        runtime_ms INTEGER NOT NULL,\
        error_class TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
    )",
    "CREATE INDEX IF NOT EXISTS idx_commits_repository_commit_sha ON commits(repository_id, commit_sha)",
    "CREATE INDEX IF NOT EXISTS idx_trace_records_repository_commit ON trace_records(repository_id, commit_id)",
    "CREATE INDEX IF NOT EXISTS idx_trace_ranges_record_file ON trace_ranges(trace_record_id, file_path)",
    "CREATE INDEX IF NOT EXISTS idx_reconciliation_runs_repository_status ON reconciliation_runs(repository_id, status)",
    "CREATE INDEX IF NOT EXISTS idx_rewrite_mappings_run_old_sha ON rewrite_mappings(reconciliation_run_id, old_commit_sha)",
    "CREATE INDEX IF NOT EXISTS idx_rewrite_mappings_repository_old_sha ON rewrite_mappings(repository_id, old_commit_sha)",
    "CREATE INDEX IF NOT EXISTS idx_conversations_repository_source ON conversations(repository_id, source)",
    "CREATE INDEX IF NOT EXISTS idx_trace_retry_queue_created_at ON trace_retry_queue(created_at)",
    "CREATE INDEX IF NOT EXISTS idx_reconciliation_metrics_created_at ON reconciliation_metrics(created_at)",
];

const CORE_SCHEMA_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 3,
    timeout_ms: 5_000,
    initial_backoff_ms: 150,
    max_backoff_ms: 600,
};

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum LocalDatabaseTarget<'a> {
    InMemory,
    Path(&'a Path),
}

#[derive(Clone, Copy, Debug)]
pub struct SmokeCheckOutcome {
    pub inserted_rows: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreSchemaMigrationOutcome {
    pub executed_statements: usize,
}

pub fn resolve_agent_trace_local_db_path() -> Result<PathBuf> {
    let state_root = resolve_state_data_root()?;
    Ok(state_root.join("sce").join("agent-trace").join("local.db"))
}

pub fn ensure_agent_trace_local_db_ready_blocking() -> Result<PathBuf> {
    let db_path = resolve_agent_trace_local_db_path()?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Agent Trace local DB directory '{}'.",
                parent.display()
            )
        })?;
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    runtime.block_on(run_with_retry(
        CORE_SCHEMA_RETRY_POLICY,
        "local_db.apply_core_schema_migrations",
        "retry the command; if it persists, verify state-directory permissions and available disk space.",
        |_| apply_core_schema_migrations(LocalDatabaseTarget::Path(&db_path)),
    ))?;
    Ok(db_path)
}

pub(crate) fn check_agent_trace_local_db_health_blocking(path: &Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    runtime.block_on(check_agent_trace_local_db_health(path))
}

async fn check_agent_trace_local_db_health(path: &Path) -> Result<()> {
    let conn = connect_local(LocalDatabaseTarget::Path(path)).await?;
    let mut rows = conn.query("PRAGMA schema_version", ()).await?;
    let _ = rows.next().await?;
    Ok(())
}

async fn connect_local(target: LocalDatabaseTarget<'_>) -> Result<turso::Connection> {
    let location = target_location(target)?;
    let db = Builder::new_local(location).build().await?;
    let conn = db.connect()?;
    conn.execute("PRAGMA foreign_keys = ON", ()).await?;
    Ok(conn)
}

fn target_location(target: LocalDatabaseTarget<'_>) -> Result<&str> {
    match target {
        LocalDatabaseTarget::InMemory => Ok(":memory:"),
        LocalDatabaseTarget::Path(path) => path
            .to_str()
            .ok_or_else(|| anyhow!("Local DB path must be valid UTF-8: {}", path.display())),
    }
}

pub(crate) fn resolve_state_data_root() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        if let Some(state_dir) = dirs::state_dir() {
            return Ok(state_dir);
        }
        if let Some(home_dir) = dirs::home_dir() {
            return Ok(home_dir.join(".local").join("state"));
        }
        Err(anyhow!(
            "Unable to resolve state directory: neither XDG_STATE_HOME nor HOME is set"
        ))
    }

    #[cfg(target_os = "macos")]
    {
        dirs::data_dir().ok_or_else(|| anyhow!("Unable to resolve data directory for macOS"))
    }

    #[cfg(target_os = "windows")]
    {
        dirs::data_local_dir()
            .or_else(|| dirs::data_dir())
            .ok_or_else(|| anyhow!("Unable to resolve local data directory for Windows"))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        if let Some(state_dir) = dirs::state_dir() {
            return Ok(state_dir);
        }
        if let Some(data_dir) = dirs::data_dir() {
            return Ok(data_dir);
        }
        if let Some(home_dir) = dirs::home_dir() {
            return Ok(home_dir.join(".local").join("state"));
        }
        Err(anyhow!("Unable to resolve state or data directory"))
    }
}

pub async fn apply_core_schema_migrations(
    target: LocalDatabaseTarget<'_>,
) -> Result<CoreSchemaMigrationOutcome> {
    let conn = connect_local(target).await?;
    for statement in CORE_SCHEMA_STATEMENTS {
        conn.execute(statement, ()).await?;
    }

    Ok(CoreSchemaMigrationOutcome {
        executed_statements: CORE_SCHEMA_STATEMENTS.len(),
    })
}

pub async fn run_smoke_check(target: LocalDatabaseTarget<'_>) -> Result<SmokeCheckOutcome> {
    let conn = connect_local(target).await?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS sce_smoke (id INTEGER PRIMARY KEY, label TEXT NOT NULL)",
        (),
    )
    .await?;

    let inserted_rows = conn
        .execute("INSERT INTO sce_smoke (label) VALUES (?1)", ["connected"])
        .await?;

    let mut rows = conn
        .query("SELECT label FROM sce_smoke ORDER BY id DESC LIMIT 1", ())
        .await?;

    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("Turso smoke query returned no rows"))?;
    let label = row.get_value(0)?;
    ensure!(
        label.as_text().is_some(),
        "Turso smoke query returned a non-text label"
    );

    Ok(SmokeCheckOutcome { inserted_rows })
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{run_smoke_check, LocalDatabaseTarget};

    #[test]
    fn in_memory_smoke_check_succeeds() -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let outcome = runtime.block_on(run_smoke_check(LocalDatabaseTarget::InMemory))?;
        assert_eq!(outcome.inserted_rows, 1);
        Ok(())
    }
}
