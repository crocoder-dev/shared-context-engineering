use anyhow::{Context, Result};

use crate::services::local_db::{run_smoke_check, LocalDatabaseTarget};

pub const NAME: &str = "sync";

pub fn run_placeholder_sync() -> Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .context("failed to create tokio runtime for sync placeholder")?;

    let outcome = runtime
        .block_on(run_smoke_check(LocalDatabaseTarget::InMemory))
        .context("local Turso smoke check failed")?;

    Ok(format!(
        "TODO: '{NAME}' cloud workflows are planned and not implemented yet. Local Turso smoke check succeeded ({}) row inserted.",
        outcome.inserted_rows
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::run_placeholder_sync;

    #[test]
    fn sync_placeholder_runs_local_smoke_check() -> Result<()> {
        let message = run_placeholder_sync()?;
        assert!(message.contains("Local Turso smoke check succeeded"));
        Ok(())
    }
}
