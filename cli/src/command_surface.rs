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
        .unwrap();
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

#[cfg(test)]
mod tests {
    use super::{ImplementationStatus, COMMANDS};
    use crate::command_surface::help_text;

    #[test]
    fn command_surface_marks_placeholder_boundaries() {
        assert!(COMMANDS
            .iter()
            .any(|command| command.status == ImplementationStatus::Implemented));
        assert!(COMMANDS
            .iter()
            .any(|command| command.status == ImplementationStatus::Placeholder));
    }

    #[test]
    fn help_text_mentions_setup_target_flags() {
        let help = help_text();
        assert!(help.contains(
            "sce setup [--opencode|--claude|--both] [--non-interactive] [--hooks] [--repo <path>]"
        ));
        assert!(help.contains("sce setup --opencode --non-interactive --hooks"));
    }

    #[test]
    fn help_text_mentions_version_command() {
        let help = help_text();
        assert!(help.contains("version"));
    }

    #[test]
    fn command_surface_includes_auth_as_known_implemented_command() {
        let auth = COMMANDS
            .iter()
            .find(|command| command.name == crate::services::auth_command::NAME)
            .expect("auth command should be listed");

        assert_eq!(auth.status, ImplementationStatus::Implemented);
        assert!(crate::command_surface::is_known_command("auth"));
    }

    #[test]
    fn hidden_commands_remain_known_even_when_not_shown_in_top_level_help() {
        for hidden_command in [
            crate::services::auth_command::NAME,
            crate::services::hooks::NAME,
            crate::services::trace::NAME,
            crate::services::sync::NAME,
        ] {
            assert!(crate::command_surface::is_known_command(hidden_command));
        }
    }

    #[test]
    fn help_text_hides_selected_commands_from_top_level_help() {
        let help = help_text();

        assert!(!help.contains("auth"));
        assert!(!help.contains("hooks"));
        assert!(!help.contains("trace"));
        assert!(!help.contains("sync"));
    }

    #[test]
    fn help_text_mentions_completion_command() {
        let help = help_text();
        assert!(help.contains("completion"));
        assert!(help.contains("sce completion --shell <bash|zsh|fish>"));
    }

    #[test]
    fn help_text_drops_placeholder_and_status_copy() {
        let help = help_text();

        assert!(!help.contains("placeholder foundation"));
        assert!(!help.contains("implemented"));
        assert!(!help.contains("placeholder-oriented"));
        assert!(!help.contains("Setup defaults to interactive target selection"));
        assert!(!help.contains("Use '--hooks' to install required git hooks"));
    }

    #[test]
    fn help_text_mentions_shared_output_format_contract() {
        let help = help_text();
        assert!(help.contains("Output format contract:"));
        assert!(help.contains("--format <text|json>"));
        assert!(help.contains("sce doctor --format json"));
        assert!(help.contains("sce doctor --fix"));
        assert!(help.contains("sce version --format json"));
    }
}
