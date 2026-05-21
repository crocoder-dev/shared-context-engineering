pub mod command;

use anyhow::{Context, Result};

use crate::services::agent_trace_db::AgentTraceDb;

pub const NAME: &str = "sync";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncSubcommand {
    Push,
    Pull,
}

/// Perform a push or pull sync operation on the Agent Trace database.
///
/// Opens the Agent Trace DB (which auto-detects sync mode from env vars).
/// If the database is in sync mode, executes the requested operation.
/// If in local mode, returns an error indicating sync is not configured.
pub fn run_sync(subcommand: SyncSubcommand) -> Result<String> {
    let db = AgentTraceDb::new().context("failed to open Agent Trace DB for sync")?;

    if !db.is_sync_mode() {
        return Err(anyhow::anyhow!(
            "Sync is not configured. Set SCE_SYNC_URL and SCE_SYNC_TOKEN to enable sync mode."
        ));
    }

    match subcommand {
        SyncSubcommand::Push => {
            db.push().with_context(|| "sync push failed")?;
            Ok("Pushed local changes to remote.".to_string())
        }
        SyncSubcommand::Pull => {
            let has_changes = db.pull().with_context(|| "sync pull failed")?;
            if has_changes {
                Ok("Pulled remote changes.".to_string())
            } else {
                Ok("No remote changes to pull.".to_string())
            }
        }
    }
}
