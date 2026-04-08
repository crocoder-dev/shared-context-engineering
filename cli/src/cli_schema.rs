use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::services::style;

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
    #[command(about = "Authenticate with `WorkOS` device authorization flow")]
    Auth {
        #[command(subcommand)]
        subcommand: AuthSubcommand,
    },

    #[command(about = "Inspect or validate runtime config and observability resolution")]
    Config {
        #[command(subcommand)]
        subcommand: ConfigSubcommand,
    },

    #[command(about = "Prepare local repository/workspace prerequisites")]
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

    #[command(about = "Inspect and repair SCE operator environment health")]
    Doctor {
        #[arg(long)]
        fix: bool,

        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    #[command(about = "Run attribution-only git hooks (disabled by default)")]
    Hooks {
        #[command(subcommand)]
        subcommand: HooksSubcommand,
    },

    #[command(about = "Print deterministic runtime version metadata")]
    Version {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    #[command(about = "Generate deterministic shell completion scripts")]
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
