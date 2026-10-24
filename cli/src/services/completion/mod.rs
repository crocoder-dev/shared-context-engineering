pub mod command;

use clap::{Command, CommandFactory};
use clap_complete::Shell;

pub const NAME: &str = "completion";

const PLAN_COMMAND_NAME: &str = "plan";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionRequest {
    pub shell: CompletionShell,
}

pub fn render_completion(request: CompletionRequest) -> String {
    let shell = match request.shell {
        CompletionShell::Bash => Shell::Bash,
        CompletionShell::Zsh => Shell::Zsh,
        CompletionShell::Fish => Shell::Fish,
    };

    let mut command = crate::cli_schema::Cli::command().mut_subcommands(|subcommand| {
        if subcommand.get_name() == PLAN_COMMAND_NAME {
            plan_without_hidden_subcommands(&subcommand)
        } else {
            subcommand
        }
    });

    let mut buffer = Vec::new();
    clap_complete::generate(shell, &mut command, "sce", &mut buffer);

    String::from_utf8(buffer).expect("Generated completion script should be valid UTF-8")
}

fn plan_without_hidden_subcommands(command: &Command) -> Command {
    let mut rebuilt = Command::new(PLAN_COMMAND_NAME)
        .hide(command.is_hide_set())
        .args(command.get_arguments().cloned())
        .subcommands(
            command
                .get_subcommands()
                .filter(|subcommand| !subcommand.is_hide_set())
                .cloned(),
        );
    if let Some(about) = command.get_about() {
        rebuilt = rebuilt.about(about.clone());
    }
    if let Some(long_about) = command.get_long_about() {
        rebuilt = rebuilt.long_about(long_about.clone());
    }
    rebuilt
}
