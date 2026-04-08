use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use turso::Builder;

use crate::services::default_paths::resolve_sce_default_locations;
use crate::services::resilience::{run_with_retry, RetryPolicy};

const LOCAL_DB_OPEN_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 3,
    timeout_ms: 5_000,
    initial_backoff_ms: 150,
    max_backoff_ms: 600,
};

#[derive(Clone, Copy, Debug)]
pub enum LocalDatabaseTarget<'a> {
    Path(&'a Path),
}

pub fn resolve_agent_trace_local_db_path() -> Result<PathBuf> {
    Ok(resolve_sce_default_locations()?.agent_trace_local_db())
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
        LOCAL_DB_OPEN_RETRY_POLICY,
        "local_db.open_local_database",
        "retry the command; if it persists, verify state-directory permissions and available disk space.",
        |_| open_local_database(LocalDatabaseTarget::Path(&db_path)),
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
        LocalDatabaseTarget::Path(path) => path
            .to_str()
            .ok_or_else(|| anyhow!("Local DB path must be valid UTF-8: {}", path.display())),
    }
}

pub(crate) fn resolve_state_data_root() -> Result<PathBuf> {
    Ok(resolve_sce_default_locations()?
        .roots()
        .state_root()
        .to_path_buf())
}

async fn open_local_database(target: LocalDatabaseTarget<'_>) -> Result<()> {
    let _ = connect_local(target).await?;
    Ok(())
}
