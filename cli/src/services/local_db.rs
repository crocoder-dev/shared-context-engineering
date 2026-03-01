use std::path::Path;

use anyhow::{anyhow, ensure, Result};
use turso::Builder;

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

pub async fn run_smoke_check(target: LocalDatabaseTarget<'_>) -> Result<SmokeCheckOutcome> {
    let location = match target {
        LocalDatabaseTarget::InMemory => ":memory:".to_string(),
        LocalDatabaseTarget::Path(path) => path.to_string_lossy().into_owned(),
    };

    let db = Builder::new_local(&location).build().await?;
    let conn = db.connect()?;

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
    use std::{
        fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;

    use super::{run_smoke_check, LocalDatabaseTarget};

    #[test]
    fn in_memory_smoke_check_succeeds() -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let outcome = runtime.block_on(run_smoke_check(LocalDatabaseTarget::InMemory))?;
        assert_eq!(outcome.inserted_rows, 1);
        Ok(())
    }

    #[test]
    fn file_backed_smoke_check_succeeds() -> Result<()> {
        let path = temporary_db_path();
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let outcome = runtime.block_on(run_smoke_check(LocalDatabaseTarget::Path(&path)))?;
        assert_eq!(outcome.inserted_rows, 1);
        assert!(path.exists());
        let _ = fs::remove_file(path);
        Ok(())
    }

    fn temporary_db_path() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("sce-smoke-{}-{timestamp}.db", process::id()))
    }
}
