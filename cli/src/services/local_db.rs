use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use turso::Builder;

use crate::services::default_paths::resolve_sce_default_locations;


#[derive(Clone, Copy, Debug)]
pub enum LocalDatabaseTarget<'a> {
    Path(&'a Path),
}

pub fn resolve_local_db_path() -> Result<PathBuf> {
    Ok(resolve_sce_default_locations()?.local_db())
}

pub(crate) fn check_local_db_health_blocking(path: &Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    runtime.block_on(check_local_db_health(path))
}

async fn check_local_db_health(path: &Path) -> Result<()> {
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

