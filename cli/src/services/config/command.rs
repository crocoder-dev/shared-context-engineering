use std::borrow::Cow;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::config;
use crate::services::error::ClassifiedError;

pub struct ConfigCommand {
    pub subcommand: config::ConfigSubcommand,
}

impl RuntimeCommand for ConfigCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(config::NAME)
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        config::run_config_subcommand(self.subcommand.clone())
            .map_err(|error| ClassifiedError::runtime(error.to_string()))
    }
}

/// Construct a `ConfigCommand` with a default show-text request (used by the registry).
///
/// This default constructor is available for registry-based dispatch.
/// The parse layer constructs `ConfigCommand` with the user's chosen subcommand and options.
#[allow(dead_code)]
pub fn make_config_command() -> RuntimeCommandHandle {
    Box::new(ConfigCommand {
        subcommand: config::ConfigSubcommand::Show(config::ConfigRequest {
            report_format: config::ReportFormat::Text,
            config_path: None,
            log_level: None,
            timeout_ms: None,
        }),
    })
}
