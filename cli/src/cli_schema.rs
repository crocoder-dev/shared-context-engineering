use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::services::style;

pub struct TopLevelCommandMetadata {
    pub name: &'static str,
    pub purpose: &'static str,
    pub show_in_top_level_help: bool,
}

pub const AUTH_CLAP_ABOUT: &str = "Authenticate with `WorkOS` device authorization flow";
pub const AUTH_TOP_LEVEL_PURPOSE: &str = "Authenticate with WorkOS and inspect local auth state";
pub const AUTH_SHOW_IN_TOP_LEVEL_HELP: bool = false;

pub const CONFIG_CLAP_ABOUT: &str =
    "Inspect or validate runtime config and observability resolution";
pub const CONFIG_TOP_LEVEL_PURPOSE: &str = "Inspect and validate resolved CLI configuration";
pub const CONFIG_SHOW_IN_TOP_LEVEL_HELP: bool = true;

pub const SETUP_CLAP_ABOUT: &str = "Prepare local repository/workspace prerequisites";
pub const SETUP_TOP_LEVEL_PURPOSE: &str = "Prepare local repository/workspace prerequisites";
pub const SETUP_SHOW_IN_TOP_LEVEL_HELP: bool = true;

pub const DOCTOR_CLAP_ABOUT: &str = "Inspect and repair SCE operator environment health";
pub const DOCTOR_TOP_LEVEL_PURPOSE: &str =
    "Inspect SCE operator health and explicit repair readiness";
pub const DOCTOR_SHOW_IN_TOP_LEVEL_HELP: bool = true;

pub const HOOKS_CLAP_ABOUT: &str = "Run attribution-only git hooks (disabled by default)";
pub const HOOKS_TOP_LEVEL_PURPOSE: &str = "Run attribution-only git hooks (disabled by default)";
pub const HOOKS_SHOW_IN_TOP_LEVEL_HELP: bool = false;

pub const VERSION_CLAP_ABOUT: &str = "Print deterministic runtime version metadata";
pub const VERSION_TOP_LEVEL_PURPOSE: &str = "Print deterministic runtime version metadata";
pub const VERSION_SHOW_IN_TOP_LEVEL_HELP: bool = true;

pub const COMPLETION_CLAP_ABOUT: &str = "Generate deterministic shell completion scripts";
pub const COMPLETION_TOP_LEVEL_PURPOSE: &str = "Generate deterministic shell completion scripts";
pub const COMPLETION_SHOW_IN_TOP_LEVEL_HELP: bool = true;

pub const TOP_LEVEL_COMMANDS: &[TopLevelCommandMetadata] = &[
    TopLevelCommandMetadata {
        name: crate::services::auth_command::NAME,
        purpose: AUTH_TOP_LEVEL_PURPOSE,
        show_in_top_level_help: AUTH_SHOW_IN_TOP_LEVEL_HELP,
    },
    TopLevelCommandMetadata {
        name: crate::services::config::NAME,
        purpose: CONFIG_TOP_LEVEL_PURPOSE,
        show_in_top_level_help: CONFIG_SHOW_IN_TOP_LEVEL_HELP,
    },
    TopLevelCommandMetadata {
        name: crate::services::setup::NAME,
        purpose: SETUP_TOP_LEVEL_PURPOSE,
        show_in_top_level_help: SETUP_SHOW_IN_TOP_LEVEL_HELP,
    },
    TopLevelCommandMetadata {
        name: crate::services::doctor::NAME,
        purpose: DOCTOR_TOP_LEVEL_PURPOSE,
        show_in_top_level_help: DOCTOR_SHOW_IN_TOP_LEVEL_HELP,
    },
    TopLevelCommandMetadata {
        name: crate::services::hooks::NAME,
        purpose: HOOKS_TOP_LEVEL_PURPOSE,
        show_in_top_level_help: HOOKS_SHOW_IN_TOP_LEVEL_HELP,
    },
    TopLevelCommandMetadata {
        name: crate::services::version::NAME,
        purpose: VERSION_TOP_LEVEL_PURPOSE,
        show_in_top_level_help: VERSION_SHOW_IN_TOP_LEVEL_HELP,
    },
    TopLevelCommandMetadata {
        name: crate::services::completion::NAME,
        purpose: COMPLETION_TOP_LEVEL_PURPOSE,
        show_in_top_level_help: COMPLETION_SHOW_IN_TOP_LEVEL_HELP,
    },
];

#[derive(Parser, Debug)]
#[command(name = "sce", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

impl Cli {
    pub fn try_parse_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::try_parse_from(args)
    }
}

pub fn render_help_for_path(path: &[&str]) -> Option<String> {
    let mut command = Cli::command();

    for segment in path {
        // Clone required: find_subcommand_mut returns a mutable reference that cannot
        // be kept alive across loop iterations, so we must clone to get an owned value
        command = command.find_subcommand_mut(segment)?.clone();
    }

    let mut buffer = Vec::new();
    command
        .write_long_help(&mut buffer)
        .expect("help rendering should write to memory");

    let help = String::from_utf8(buffer).expect("help output should be valid UTF-8");

    Some(style::clap_help(&help))
}

pub fn auth_help_text() -> String {
    use crate::services::style::{command_name, heading};

    let base = render_help_for_path(&["auth"]).expect("auth help should be renderable");

    format!(
        "{}\n{}:\n  {}\n  {}\n  {}\n  {}\n",
        base,
        heading("Examples"),
        command_name("sce auth login"),
        command_name("sce auth renew"),
        command_name("sce auth status"),
        command_name("sce auth logout")
    )
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    #[command(about = AUTH_CLAP_ABOUT, hide = !AUTH_SHOW_IN_TOP_LEVEL_HELP)]
    Auth {
        #[command(subcommand)]
        subcommand: AuthSubcommand,
    },

    #[command(about = CONFIG_CLAP_ABOUT, hide = !CONFIG_SHOW_IN_TOP_LEVEL_HELP)]
    Config {
        #[command(subcommand)]
        subcommand: ConfigSubcommand,
    },

    #[command(about = SETUP_CLAP_ABOUT, hide = !SETUP_SHOW_IN_TOP_LEVEL_HELP)]
    Setup {
        #[arg(long, conflicts_with_all = ["claude", "both"])]
        opencode: bool,

        #[arg(long, conflicts_with_all = ["opencode", "both"])]
        claude: bool,

        #[arg(long, conflicts_with_all = ["opencode", "claude"])]
        both: bool,

        #[arg(long)]
        non_interactive: bool,

        #[arg(long)]
        hooks: bool,

        #[arg(long, requires = "hooks")]
        repo: Option<PathBuf>,
    },

    #[command(about = DOCTOR_CLAP_ABOUT, hide = !DOCTOR_SHOW_IN_TOP_LEVEL_HELP)]
    Doctor {
        #[arg(long)]
        fix: bool,

        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    #[command(about = HOOKS_CLAP_ABOUT, hide = !HOOKS_SHOW_IN_TOP_LEVEL_HELP)]
    Hooks {
        #[command(subcommand)]
        subcommand: HooksSubcommand,
    },

    #[command(about = VERSION_CLAP_ABOUT, hide = !VERSION_SHOW_IN_TOP_LEVEL_HELP)]
    Version {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    #[command(about = COMPLETION_CLAP_ABOUT, hide = !COMPLETION_SHOW_IN_TOP_LEVEL_HELP)]
    Completion {
        #[arg(long, value_enum)]
        shell: CompletionShell,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AuthSubcommand {
    #[command(about = "Start login flow and store credentials")]
    Login {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    #[command(about = "Renew stored credentials when they are expired or near expiry")]
    Renew {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,

        #[arg(long)]
        force: bool,
    },

    #[command(about = "Remove stored credentials from the local machine")]
    Logout {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    #[command(about = "Show current authentication status from stored credentials")]
    Status {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConfigSubcommand {
    #[command(about = "Show resolved runtime config, including observability sources")]
    Show {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,

        #[arg(long)]
        config: Option<PathBuf>,

        #[arg(long, value_enum)]
        log_level: Option<LogLevel>,

        #[arg(long)]
        timeout_ms: Option<u64>,
    },

    #[command(about = "Validate config files and report pass/fail with errors or warnings")]
    Validate {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,

        #[arg(long)]
        config: Option<PathBuf>,

        #[arg(long, value_enum)]
        log_level: Option<LogLevel>,

        #[arg(long)]
        timeout_ms: Option<u64>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum HooksSubcommand {
    #[command(about = "Run pre-commit hook")]
    PreCommit,

    #[command(about = "Run commit-msg hook")]
    CommitMsg { message_file: PathBuf },

    #[command(about = "Run post-commit hook")]
    PostCommit,

    #[command(about = "Run post-rewrite hook (reads pairs from STDIN)")]
    PostRewrite { rewrite_method: String },
}

#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}
