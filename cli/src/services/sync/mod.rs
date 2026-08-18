//! Top-level `sce sync` command and Agent Trace synchronization service.

pub mod command;
pub mod progress;
pub mod render_sync;
#[allow(clippy::module_inception)]
pub mod sync;

pub const NAME: &str = "sync";

use crate::services::output_format::OutputFormat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncRequest {
    pub format: OutputFormat,
}
