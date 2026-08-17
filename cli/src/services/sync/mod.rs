//! Top-level `sce sync` command and Agent Trace synchronization service.

pub mod command;
pub mod render_sync;
pub mod sync;

pub const NAME: &str = "sync";

use crate::services::output_format::OutputFormat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncRequest {
    pub format: OutputFormat,
}
