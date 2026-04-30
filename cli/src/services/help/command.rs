use std::borrow::Cow;

use crate::app::AppContext;
use crate::services::command_registry::{RuntimeCommand, RuntimeCommandHandle};
use crate::services::error::ClassifiedError;
use crate::services::help;

pub struct HelpCommand;

impl RuntimeCommand for HelpCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(help::NAME)
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        Ok(help::help_text())
    }
}

pub struct HelpTextCommand {
    pub name: String,
    pub text: String,
}

impl RuntimeCommand for HelpTextCommand {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.name.as_str())
    }

    fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
        Ok(self.text.clone())
    }
}

/// Construct a `HelpCommand` (used by the registry).
pub fn make_help_command() -> RuntimeCommandHandle {
    Box::new(HelpCommand)
}
