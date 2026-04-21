use std::fmt::Write;

use crate::{cli_schema, services};
use services::style::{banner_with_gradient, command_name, heading};

const SCE_BANNER_LINES: &[&str] = &[
    r"  ______     ______  ________  ",
    r".' ____ \  .' ___  ||_   __  | ",
    r"| (___ \_|/ .'   \_|  | |_ \_| ",
    r" _.____`. | |         |  _| _  ",
    r"| \____) |\ `.___.'\ _| |__/ | ",
    r" \______.' `.____ .'|________| ",
];

const HELP_COMMAND_NAME: &str = "help";
const HELP_COMMAND_PURPOSE: &str = "Show help for the current CLI surface";

pub fn is_known_command(name: &str) -> bool {
    name == HELP_COMMAND_NAME
        || cli_schema::TOP_LEVEL_COMMANDS
            .iter()
            .any(|command| command.name == name)
}
enum HelpSectionBodyLine {
    Text(&'static str),
    Command {
        cmd: &'static str,
        suffix: &'static str,
    },
}

struct HelpSection {
    title: &'static str,
    body: &'static [HelpSectionBodyLine],
}

const HELP_SECTIONS: &[HelpSection] = &[
    HelpSection {
        title: "Usage:",
        body: &[HelpSectionBodyLine::Text("  sce [command]")],
    },
    HelpSection {
        title: "Config Usage:",
        body: &[HelpSectionBodyLine::Command {
            cmd: "  sce config",
            suffix: " <show|validate> [--format <text|json>] [options]",
        }],
    },
    HelpSection {
        title: "Setup Usage:",
        body: &[HelpSectionBodyLine::Command {
            cmd: "  sce setup",
            suffix: " [--opencode|--claude|--both] [--non-interactive] [--hooks] [--repo <path>]",
        }],
    },
    HelpSection {
        title: "Doctor Usage:",
        body: &[HelpSectionBodyLine::Command {
            cmd: "  sce doctor",
            suffix: " [--fix] [--format <text|json>]",
        }],
    },
    HelpSection {
        title: "Version Usage:",
        body: &[HelpSectionBodyLine::Command {
            cmd: "  sce version",
            suffix: " [--format <text|json>]",
        }],
    },
    HelpSection {
        title: "Completion Usage:",
        body: &[HelpSectionBodyLine::Command {
            cmd: "  sce completion",
            suffix: " --shell <bash|zsh|fish>",
        }],
    },
    HelpSection {
        title: "Output format contract:",
        body: &[HelpSectionBodyLine::Text(
            "  Supported commands accept --format <text|json>",
        )],
    },
    HelpSection {
        title: "Examples:",
        body: &[
            HelpSectionBodyLine::Text("  sce config"),
            HelpSectionBodyLine::Text("  sce config show --format json"),
            HelpSectionBodyLine::Text("  sce setup"),
            HelpSectionBodyLine::Text("  sce setup --opencode --non-interactive --hooks"),
            HelpSectionBodyLine::Text("  sce setup --hooks --repo ../demo-repo"),
            HelpSectionBodyLine::Text("  sce doctor --format json"),
            HelpSectionBodyLine::Text("  sce doctor --fix"),
            HelpSectionBodyLine::Text("  sce version --format json"),
        ],
    },
];

fn commands_section() -> String {
    let mut out = String::new();
    writeln!(out, "{}", heading("Commands")).expect("writing to String should not fail");
    writeln!(
        out,
        "  {:<10} {}",
        command_name(HELP_COMMAND_NAME),
        HELP_COMMAND_PURPOSE
    )
    .expect("writing to String should never fail");
    for command in cli_schema::TOP_LEVEL_COMMANDS {
        if command.show_in_top_level_help {
            writeln!(
                out,
                "  {:<10} {}",
                command_name(command.name),
                command.purpose
            )
            .expect("writing to String should never fail");
        }
    }
    out
}

fn push_blank_line(out: &mut String) {
    out.push('\n');
}

fn push_section(out: &mut String, section: &str) {
    out.push_str(section);
}

pub fn help_text() -> String {
    let mut output = String::new();

    push_section(&mut output, &banner_with_gradient(SCE_BANNER_LINES));
    push_blank_line(&mut output);
    push_blank_line(&mut output);

    push_section(
        &mut output,
        &heading("sce - Shared Context Engineering CLI"),
    );
    push_blank_line(&mut output);
    push_blank_line(&mut output);

    for section in HELP_SECTIONS {
        push_section(&mut output, &heading(section.title));
        push_blank_line(&mut output);

        for line in section.body {
            match line {
                HelpSectionBodyLine::Text(text) => {
                    push_section(&mut output, text);
                    push_blank_line(&mut output);
                }
                HelpSectionBodyLine::Command { cmd, suffix } => {
                    writeln!(output, "{}{}", command_name(cmd), suffix)
                        .expect("writing to String should never fail");
                }
            }
        }

        push_blank_line(&mut output);
    }

    writeln!(output, "{}", commands_section()).expect("writing to String should not fail");

    output
}
