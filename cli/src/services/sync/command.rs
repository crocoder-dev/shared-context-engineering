use std::borrow::Cow;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::error::ClassifiedError;
use crate::services::sync;

pub struct SyncCommand {
    pub subcommand: sync::SyncSubcommand,
}

impl RuntimeCommand for SyncCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(sync::NAME)
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        sync::run_sync(self.subcommand).map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}

/// Construct a `SyncCommand` with the Push subcommand (used by the registry).
#[allow(dead_code)]
pub fn make_sync_command() -> RuntimeCommandHandle {
    Box::new(SyncCommand {
        subcommand: sync::SyncSubcommand::Push,
    })
}
