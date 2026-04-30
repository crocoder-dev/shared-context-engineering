pub mod command;

use crate::command_surface;

pub const NAME: &str = "help";

pub fn help_text() -> String {
    command_surface::help_text()
}
