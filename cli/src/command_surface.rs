use crate::services;

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
}

pub const COMMANDS: &[CommandContract] = &[
    CommandContract {
        name: "help",
        status: ImplementationStatus::Implemented,
        purpose: "Print the current placeholder command surface",
    },
    CommandContract {
        name: services::config::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Inspect and validate resolved CLI configuration",
    },
    CommandContract {
        name: services::setup::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Prepare local repository/workspace prerequisites",
    },
    CommandContract {
        name: services::doctor::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Validate local git-hook installation readiness",
    },
    CommandContract {
        name: services::mcp::NAME,
        status: ImplementationStatus::Placeholder,
        purpose: "Host MCP file-cache tooling commands",
    },
    CommandContract {
        name: services::hooks::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Run git-hook runtime entrypoints for local Agent Trace flows",
    },
    CommandContract {
        name: services::sync::NAME,
        status: ImplementationStatus::Placeholder,
        purpose: "Coordinate future cloud sync workflows",
    },
    CommandContract {
        name: services::version::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Print deterministic runtime version metadata",
    },
    CommandContract {
        name: services::completion::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Generate deterministic shell completion scripts",
    },
];

pub fn is_known_command(name: &str) -> bool {
    COMMANDS.iter().any(|command| command.name == name)
}

pub fn help_text() -> String {
    let command_rows = COMMANDS
        .iter()
        .map(|command| {
            let status = match command.status {
                ImplementationStatus::Implemented => "implemented",
                ImplementationStatus::Placeholder => "placeholder",
            };

            format!("  {:<8} {:<12} {}", command.name, status, command.purpose)
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "sce - Shared Context Engineering CLI (placeholder foundation)\n\n\
Usage:\n  sce [command]\n\n\
Config usage:\n  sce config <show|validate> [options]\n\n\
Setup usage:\n  sce setup [--opencode|--claude|--both] [--non-interactive] [--hooks] [--repo <path>]\n\n\
Completion usage:\n  sce completion --shell <bash|zsh|fish>\n\n\
Output format contract:\n  Supported commands accept --format <text|json>\n\n\
Commands:\n{}\n\n\
Setup defaults to interactive target selection when no setup target flag is passed, and installs hooks in the same run.\n\
Use '--hooks' to install required git hooks for the current repository or '--repo <path>' for a specific repository.\n\
`setup`, `doctor`, and `hooks` are implemented; `mcp` and `sync` remain placeholder-oriented.\n",
        command_rows
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
        assert!(help.contains("installs hooks in the same run"));
    }

    #[test]
    fn help_text_mentions_version_command() {
        let help = help_text();
        assert!(help.contains("version"));
    }

    #[test]
    fn help_text_mentions_completion_command() {
        let help = help_text();
        assert!(help.contains("completion"));
        assert!(help.contains("sce completion --shell <bash|zsh|fish>"));
    }

    #[test]
    fn help_text_mentions_shared_output_format_contract() {
        let help = help_text();
        assert!(help.contains("Output format contract:"));
        assert!(help.contains("--format <text|json>"));
    }
}
