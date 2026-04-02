use std::fmt::Write;

use crate::services;
use services::style::{command_name, heading};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImplementationStatus {
    Implemented,
    Placeholder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandContract {
    pub name: &'static str,
    pub status: ImplementationStatus,
    pub purpose: &'static str,
    pub show_in_top_level_help: bool,
}

pub const COMMANDS: &[CommandContract] = &[
    CommandContract {
        name: "help",
        status: ImplementationStatus::Implemented,
        purpose: "Show help for the current CLI surface",
        show_in_top_level_help: true,
    },
    CommandContract {
        name: services::config::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Inspect and validate resolved CLI configuration",
        show_in_top_level_help: true,
    },
    CommandContract {
        name: services::setup::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Prepare local repository/workspace prerequisites",
        show_in_top_level_help: true,
    },
    CommandContract {
        name: services::doctor::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Inspect SCE operator health and explicit repair readiness",
        show_in_top_level_help: true,
    },
    CommandContract {
        name: services::auth_command::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Authenticate with WorkOS and inspect local auth state",
        show_in_top_level_help: false,
    },
    CommandContract {
        name: services::hooks::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Run git-hook runtime entrypoints for local Agent Trace flows",
        show_in_top_level_help: false,
    },
    CommandContract {
        name: services::trace::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Inspect persisted Agent Trace records and captured prompts",
        show_in_top_level_help: false,
    },
    CommandContract {
        name: services::sync::NAME,
        status: ImplementationStatus::Placeholder,
        purpose: "Coordinate future cloud sync workflows",
        show_in_top_level_help: false,
    },
    CommandContract {
        name: services::version::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Print deterministic runtime version metadata",
        show_in_top_level_help: true,
    },
    CommandContract {
        name: services::completion::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Generate deterministic shell completion scripts",
        show_in_top_level_help: true,
    },
];

pub fn is_known_command(name: &str) -> bool {
    COMMANDS.iter().any(|command| command.name == name)
}

pub fn help_text() -> String {
    let mut command_rows = String::new();
    for command in COMMANDS {
        if !command.show_in_top_level_help {
            continue;
        }

        writeln!(
            command_rows,
            "  {:<10} {}",
            command_name(command.name),
            command.purpose
        )
        .expect("writing to String should never fail");
    }

    format!(
        "{}\n\n\
{}:\n  sce [command]\n\n\
{}:\n  {} <show|validate> [--format <text|json>] [options]\n\n\
{}:\n  {} [--opencode|--claude|--both] [--non-interactive] [--hooks] [--repo <path>]\n\n\
{}:\n  {} [--fix] [--format <text|json>]\n\n\
{}:\n  {} --shell <bash|zsh|fish>\n\n\
{}:\n  Supported commands accept --format <text|json>\n\n\
{}:\n  sce setup\n  sce setup --opencode --non-interactive --hooks\n  sce setup --hooks --repo ../demo-repo\n  sce doctor --format json\n  sce doctor --fix\n  sce version --format json\n\n\
{}:\n{command_rows}",
        heading("sce - Shared Context Engineering CLI"),
        heading("Usage"),
        heading("Config usage"),
        command_name("sce config"),
        heading("Setup usage"),
        command_name("sce setup"),
        heading("Doctor usage"),
        command_name("sce doctor"),
        heading("Completion usage"),
        command_name("sce completion"),
        heading("Output format contract"),
        heading("Examples"),
        heading("Commands"),
    )
}
