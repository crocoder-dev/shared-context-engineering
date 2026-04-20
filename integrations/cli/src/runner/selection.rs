use super::catalog::{CommandSuite, COMMAND_SUITES};
use crate::error::HarnessError;

pub(super) fn select_suites(
    command: Option<&str>,
) -> Result<Vec<&'static CommandSuite>, HarnessError> {
    match command {
        Some(name) => {
            let suite = COMMAND_SUITES
                .iter()
                .find(|suite| suite.name == name)
                .ok_or_else(|| HarnessError::UnknownCommandSelector {
                    selected: name.to_string(),
                    available: render_available_command_suites(),
                })?;
            Ok(vec![suite])
        }
        None => Ok(COMMAND_SUITES.iter().collect()),
    }
}

fn render_available_command_suites() -> String {
    let mut rendered = String::new();
    for (index, suite) in COMMAND_SUITES.iter().enumerate() {
        if index > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(suite.name);
    }
    rendered
}
