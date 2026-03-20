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
        purpose: "Inspect SCE operator health and explicit repair readiness",
    },
    CommandContract {
        name: services::auth_command::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Authenticate with WorkOS and inspect local auth state",
    },
    CommandContract {
        name: services::hooks::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Run git-hook runtime entrypoints for local Agent Trace flows",
    },
    CommandContract {
        name: services::trace::NAME,
        status: ImplementationStatus::Implemented,
        purpose: "Inspect persisted Agent Trace records and captured prompts",
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
Config usage:\n  sce config <show|validate> [--format <text|json>] [options]\n\n\
Setup usage:\n  sce setup [--opencode|--claude|--both] [--non-interactive] [--hooks] [--repo <path>]\n\n\
Doctor usage:\n  sce doctor [--fix] [--all-databases] [--format <text|json>]\n\n\
Auth usage:\n  sce auth <login|logout|status> [--format <text|json>]\n\n\
Completion usage:\n  sce completion --shell <bash|zsh|fish>\n\n\
Trace usage:\n  sce trace prompts <commit-sha> [--format <text|json>|--json]\n\n\
Output format contract:\n  Supported commands accept --format <text|json>\n\n\
Examples:\n  sce setup\n  sce setup --opencode --non-interactive --hooks\n  sce setup --hooks --repo ../demo-repo\n  sce auth status\n  sce auth login --format json\n  sce trace prompts abc1234\n  sce trace prompts abc1234 --json\n  sce doctor --format json\n  sce doctor --all-databases --format json\n  sce doctor --fix\n  sce version --format json\n\n\
Commands:\n{command_rows}\n\n\
Setup defaults to interactive target selection when no setup target flag is passed, and installs hooks in the same run.\n\
Use '--hooks' to install required git hooks for the current repository or '--repo <path>' for a specific repository.\n\
`setup`, `doctor`, `auth`, `hooks`, `trace`, `version`, and `completion` are implemented; `sync` remains placeholder-oriented.\n"
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
    fn help_text_mentions_auth_usage_examples() {
        let help = help_text();
        assert!(help.contains("sce auth <login|logout|status> [--format <text|json>]"));
        assert!(help.contains("sce auth status"));
        assert!(help.contains("sce auth login --format json"));
    }

    #[test]
    fn help_text_mentions_completion_command() {
        let help = help_text();
        assert!(help.contains("completion"));
        assert!(help.contains("sce completion --shell <bash|zsh|fish>"));
    }

    #[test]
    fn help_text_mentions_trace_command() {
        let help = help_text();
        assert!(help.contains("trace"));
        assert!(help.contains("sce trace prompts <commit-sha>"));
        assert!(help.contains("sce trace prompts abc1234 --json"));
    }

    #[test]
    fn help_text_mentions_shared_output_format_contract() {
        let help = help_text();
        assert!(help.contains("Output format contract:"));
        assert!(help.contains("--format <text|json>"));
        assert!(help.contains("sce doctor --format json"));
        assert!(help.contains("sce doctor --all-databases --format json"));
        assert!(help.contains("sce doctor --fix"));
        assert!(help.contains("sce version --format json"));
    }
}
