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
        name: services::setup::NAME,
        status: ImplementationStatus::Placeholder,
        purpose: "Prepare local repository/workspace prerequisites",
    },
    CommandContract {
        name: services::mcp::NAME,
        status: ImplementationStatus::Placeholder,
        purpose: "Host MCP file-cache tooling commands",
    },
    CommandContract {
        name: services::hooks::NAME,
        status: ImplementationStatus::Placeholder,
        purpose: "Manage git-hook listener and generated-region awareness",
    },
    CommandContract {
        name: services::sync::NAME,
        status: ImplementationStatus::Placeholder,
        purpose: "Coordinate future cloud sync workflows",
    },
];

pub fn is_known_command(name: &str) -> bool {
    COMMANDS.iter().any(|command| command.name == name)
}

pub fn help_text() -> String {
    let mut out = String::new();
    out.push_str("sce - Shared Context Engineering CLI (placeholder foundation)\n\n");
    out.push_str("Usage:\n  sce [command]\n\n");
    out.push_str("Setup usage:\n  sce setup [--opencode|--claude|--both]\n\n");
    out.push_str("Commands:\n");

    for command in COMMANDS {
        let status = match command.status {
            ImplementationStatus::Implemented => "implemented",
            ImplementationStatus::Placeholder => "placeholder",
        };

        out.push_str(&format!(
            "  {:<8} {:<12} {}\n",
            command.name, status, command.purpose
        ));
    }

    out.push_str(
        "\nSetup defaults to interactive target selection when no setup target flag is passed.\n",
    );
    out.push_str("Only command-surface scaffolding is implemented in this task slice.\n");
    out
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
        assert!(help.contains("sce setup [--opencode|--claude|--both]"));
    }
}
