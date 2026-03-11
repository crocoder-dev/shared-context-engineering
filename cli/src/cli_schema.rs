//! Clap-based CLI schema for the Shared Context Engineering CLI.
//!
//! This module defines the complete command-line interface using clap derive macros.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Shared Context Engineering CLI
#[derive(Parser, Debug)]
#[command(name = "sce", version, about, long_about = None)]
pub struct Cli {
    /// The subcommand to run
    #[command(subcommand)]
    pub command: Option<Commands>,
}

impl Cli {
    /// Parse arguments from an iterator of strings
    #[allow(dead_code)]
    pub fn parse_from<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::parse_from(args)
    }

    /// Try to parse arguments, returning an error on failure
    pub fn try_parse_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::try_parse_from(args)
    }
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    /// Authenticate with `WorkOS` device authorization flow
    Auth {
        #[command(subcommand)]
        subcommand: AuthSubcommand,
    },

    /// Inspect and validate resolved CLI configuration
    Config {
        #[command(subcommand)]
        subcommand: ConfigSubcommand,
    },

    /// Prepare local repository/workspace prerequisites
    #[command(about = "Prepare local repository/workspace prerequisites")]
    Setup {
        /// Install `OpenCode` configuration
        #[arg(long, conflicts_with_all = ["claude", "both"])]
        opencode: bool,

        /// Install Claude configuration
        #[arg(long, conflicts_with_all = ["opencode", "both"])]
        claude: bool,

        /// Install both `OpenCode` and Claude configuration
        #[arg(long, conflicts_with_all = ["opencode", "claude"])]
        both: bool,

        /// Run without interactive prompts (requires a target flag when not using --hooks)
        #[arg(long)]
        non_interactive: bool,

        /// Install required git hooks
        #[arg(long)]
        hooks: bool,

        /// Repository path for hook installation (requires --hooks)
        #[arg(long, requires = "hooks")]
        repo: Option<PathBuf>,
    },

    /// Validate local git-hook installation readiness
    #[command(about = "Validate local git-hook installation readiness")]
    Doctor {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    /// Host MCP file-cache tooling commands (placeholder)
    #[command(about = "Host MCP file-cache tooling commands (placeholder)")]
    Mcp {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    /// Run git-hook runtime entrypoints for local Agent Trace flows
    #[command(about = "Run git-hook runtime entrypoints for local Agent Trace flows")]
    Hooks {
        #[command(subcommand)]
        subcommand: HooksSubcommand,
    },

    /// Coordinate future cloud sync workflows (placeholder)
    #[command(about = "Coordinate future cloud sync workflows (placeholder)")]
    Sync {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    /// Print deterministic runtime version metadata
    #[command(about = "Print deterministic runtime version metadata")]
    Version {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    /// Generate deterministic shell completion scripts
    #[command(about = "Generate deterministic shell completion scripts")]
    Completion {
        /// Shell type for completion script
        #[arg(long, value_enum)]
        shell: CompletionShell,
    },
}

/// Config subcommands
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AuthSubcommand {
    /// Start login flow and store credentials
    Login {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    /// Clear stored credentials
    Logout {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    /// Show current authentication status
    Status {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

/// Config subcommands
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConfigSubcommand {
    /// Show resolved configuration
    Show {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,

        /// Path to configuration file
        #[arg(long)]
        config: Option<PathBuf>,

        /// Override log level
        #[arg(long, value_enum)]
        log_level: Option<LogLevel>,

        /// Override timeout in milliseconds
        #[arg(long)]
        timeout_ms: Option<u64>,
    },

    /// Validate configuration file
    Validate {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,

        /// Path to configuration file
        #[arg(long)]
        config: Option<PathBuf>,

        /// Override log level
        #[arg(long, value_enum)]
        log_level: Option<LogLevel>,

        /// Override timeout in milliseconds
        #[arg(long)]
        timeout_ms: Option<u64>,
    },
}

/// Hooks subcommands
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum HooksSubcommand {
    /// Run pre-commit hook
    #[command(about = "Run pre-commit hook")]
    PreCommit,

    /// Run commit-msg hook
    #[command(about = "Run commit-msg hook")]
    CommitMsg {
        /// Path to the commit message file
        message_file: PathBuf,
    },

    /// Run post-commit hook
    #[command(about = "Run post-commit hook")]
    PostCommit,

    /// Run post-rewrite hook
    #[command(about = "Run post-rewrite hook (reads pairs from STDIN)")]
    PostRewrite {
        /// Rewrite method (amend, rebase, or other)
        rewrite_method: String,
    },
}

/// Output format for commands that support multiple formats
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Plain text output
    #[default]
    Text,
    /// JSON output
    Json,
}

/// Shell types for completion generation
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionShell {
    /// Bash shell completion
    Bash,
    /// Zsh shell completion
    Zsh,
    /// Fish shell completion
    Fish,
}

/// Log level configuration
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// Error level only
    Error,
    /// Warning and above
    Warn,
    /// Info and above
    Info,
    /// Debug and above
    Debug,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_auth_login() {
        let cli = Cli::try_parse_from(["sce", "auth", "login"]).expect("auth login should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Login { format } => {
                    assert_eq!(format, OutputFormat::Text);
                }
                _ => panic!("Expected Login subcommand"),
            },
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn parse_auth_login_json() {
        let cli = Cli::try_parse_from(["sce", "auth", "login", "--format", "json"])
            .expect("auth login --format json should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Login { format } => {
                    assert_eq!(format, OutputFormat::Json);
                }
                _ => panic!("Expected Login subcommand"),
            },
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn parse_auth_logout() {
        let cli = Cli::try_parse_from(["sce", "auth", "logout"]).expect("auth logout should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Logout { format } => {
                    assert_eq!(format, OutputFormat::Text);
                }
                _ => panic!("Expected Logout subcommand"),
            },
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn parse_auth_logout_json() {
        let cli = Cli::try_parse_from(["sce", "auth", "logout", "--format", "json"])
            .expect("auth logout --format json should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Logout { format } => {
                    assert_eq!(format, OutputFormat::Json);
                }
                _ => panic!("Expected Logout subcommand"),
            },
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn parse_auth_status() {
        let cli = Cli::try_parse_from(["sce", "auth", "status"]).expect("auth status should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Status { format } => {
                    assert_eq!(format, OutputFormat::Text);
                }
                _ => panic!("Expected Status subcommand"),
            },
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn parse_auth_status_json() {
        let cli = Cli::try_parse_from(["sce", "auth", "status", "--format", "json"])
            .expect("auth status --format json should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Status { format } => {
                    assert_eq!(format, OutputFormat::Json);
                }
                _ => panic!("Expected Status subcommand"),
            },
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn parse_version_command() {
        let cli = Cli::try_parse_from(["sce", "version"]).expect("version should parse");
        match cli.command {
            Some(Commands::Version { format }) => {
                assert_eq!(format, OutputFormat::Text);
            }
            _ => panic!("Expected Version command"),
        }
    }

    #[test]
    fn parse_version_json() {
        let cli = Cli::try_parse_from(["sce", "version", "--format", "json"])
            .expect("version --format json should parse");
        match cli.command {
            Some(Commands::Version { format }) => {
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("Expected Version command"),
        }
    }

    #[test]
    fn parse_config_show() {
        let cli = Cli::try_parse_from(["sce", "config", "show"]).expect("config show should parse");
        match cli.command {
            Some(Commands::Config { subcommand }) => match subcommand {
                ConfigSubcommand::Show { format, .. } => {
                    assert_eq!(format, OutputFormat::Text);
                }
                ConfigSubcommand::Validate { .. } => panic!("Expected Show subcommand"),
            },
            _ => panic!("Expected Config command"),
        }
    }

    #[test]
    fn parse_config_validate_json() {
        let cli = Cli::try_parse_from(["sce", "config", "validate", "--format", "json"])
            .expect("config validate --format json should parse");
        match cli.command {
            Some(Commands::Config { subcommand }) => match subcommand {
                ConfigSubcommand::Validate { format, .. } => {
                    assert_eq!(format, OutputFormat::Json);
                }
                ConfigSubcommand::Show { .. } => panic!("Expected Validate subcommand"),
            },
            _ => panic!("Expected Config command"),
        }
    }

    #[test]
    fn parse_config_with_options() {
        let cli = Cli::try_parse_from([
            "sce",
            "config",
            "show",
            "--config",
            "/path/to/config.json",
            "--log-level",
            "debug",
            "--timeout-ms",
            "60000",
        ])
        .expect("config show with options should parse");
        match cli.command {
            Some(Commands::Config { subcommand }) => match subcommand {
                ConfigSubcommand::Show {
                    config,
                    log_level,
                    timeout_ms,
                    ..
                } => {
                    assert_eq!(config, Some(PathBuf::from("/path/to/config.json")));
                    assert_eq!(log_level, Some(LogLevel::Debug));
                    assert_eq!(timeout_ms, Some(60000));
                }
                ConfigSubcommand::Validate { .. } => panic!("Expected Show subcommand"),
            },
            _ => panic!("Expected Config command"),
        }
    }

    #[test]
    fn parse_setup_opencode() {
        let cli = Cli::try_parse_from(["sce", "setup", "--opencode"])
            .expect("setup --opencode should parse");
        match cli.command {
            Some(Commands::Setup {
                opencode,
                claude,
                both,
                ..
            }) => {
                assert!(opencode);
                assert!(!claude);
                assert!(!both);
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn parse_setup_claude() {
        let cli =
            Cli::try_parse_from(["sce", "setup", "--claude"]).expect("setup --claude should parse");
        match cli.command {
            Some(Commands::Setup {
                opencode,
                claude,
                both,
                ..
            }) => {
                assert!(!opencode);
                assert!(claude);
                assert!(!both);
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn parse_setup_both() {
        let cli =
            Cli::try_parse_from(["sce", "setup", "--both"]).expect("setup --both should parse");
        match cli.command {
            Some(Commands::Setup {
                opencode,
                claude,
                both,
                ..
            }) => {
                assert!(!opencode);
                assert!(!claude);
                assert!(both);
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn parse_setup_hooks() {
        let cli =
            Cli::try_parse_from(["sce", "setup", "--hooks"]).expect("setup --hooks should parse");
        match cli.command {
            Some(Commands::Setup { hooks, repo, .. }) => {
                assert!(hooks);
                assert!(repo.is_none());
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn parse_setup_hooks_with_repo() {
        let cli = Cli::try_parse_from(["sce", "setup", "--hooks", "--repo", "../demo-repo"])
            .expect("setup --hooks --repo should parse");
        match cli.command {
            Some(Commands::Setup { hooks, repo, .. }) => {
                assert!(hooks);
                assert_eq!(repo, Some(PathBuf::from("../demo-repo")));
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn parse_setup_opencode_with_hooks() {
        let cli = Cli::try_parse_from(["sce", "setup", "--opencode", "--hooks"])
            .expect("setup --opencode --hooks should parse");
        match cli.command {
            Some(Commands::Setup {
                opencode, hooks, ..
            }) => {
                assert!(opencode);
                assert!(hooks);
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn parse_setup_non_interactive_requires_target() {
        // Note: This validation is now handled at runtime in resolve_setup_request,
        // not at the clap parsing level. The parsing succeeds but runtime would fail.
        let cli = Cli::try_parse_from(["sce", "setup", "--non-interactive"])
            .expect("parsing should succeed (runtime validation handles this)");
        match cli.command {
            Some(Commands::Setup {
                non_interactive, ..
            }) => {
                assert!(non_interactive);
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn parse_setup_non_interactive_with_target() {
        let cli = Cli::try_parse_from(["sce", "setup", "--opencode", "--non-interactive"])
            .expect("setup --opencode --non-interactive should parse");
        match cli.command {
            Some(Commands::Setup {
                opencode,
                non_interactive,
                ..
            }) => {
                assert!(opencode);
                assert!(non_interactive);
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn parse_setup_mutually_exclusive_targets() {
        // opencode and claude are mutually exclusive
        let result = Cli::try_parse_from(["sce", "setup", "--opencode", "--claude"]);
        assert!(
            result.is_err(),
            "mutually exclusive targets should fail to parse"
        );
    }

    #[test]
    fn parse_setup_repo_requires_hooks() {
        // --repo requires --hooks
        let result = Cli::try_parse_from(["sce", "setup", "--repo", "../demo-repo"]);
        assert!(result.is_err(), "--repo without --hooks should fail");
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::try_parse_from(["sce", "doctor"]).expect("doctor should parse");
        match cli.command {
            Some(Commands::Doctor { format }) => {
                assert_eq!(format, OutputFormat::Text);
            }
            _ => panic!("Expected Doctor command"),
        }
    }

    #[test]
    fn parse_doctor_json() {
        let cli = Cli::try_parse_from(["sce", "doctor", "--format", "json"])
            .expect("doctor json should parse");
        match cli.command {
            Some(Commands::Doctor { format }) => {
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("Expected Doctor command"),
        }
    }

    #[test]
    fn parse_hooks_pre_commit() {
        let cli = Cli::try_parse_from(["sce", "hooks", "pre-commit"])
            .expect("hooks pre-commit should parse");
        match cli.command {
            Some(Commands::Hooks { subcommand }) => {
                assert_eq!(subcommand, HooksSubcommand::PreCommit);
            }
            _ => panic!("Expected Hooks command"),
        }
    }

    #[test]
    fn parse_hooks_commit_msg() {
        let cli = Cli::try_parse_from(["sce", "hooks", "commit-msg", ".git/COMMIT_EDITMSG"])
            .expect("hooks commit-msg should parse");
        match cli.command {
            Some(Commands::Hooks { subcommand }) => match subcommand {
                HooksSubcommand::CommitMsg { message_file } => {
                    assert_eq!(message_file, PathBuf::from(".git/COMMIT_EDITMSG"));
                }
                _ => panic!("Expected CommitMsg subcommand"),
            },
            _ => panic!("Expected Hooks command"),
        }
    }

    #[test]
    fn parse_hooks_post_commit() {
        let cli = Cli::try_parse_from(["sce", "hooks", "post-commit"])
            .expect("hooks post-commit should parse");
        match cli.command {
            Some(Commands::Hooks { subcommand }) => {
                assert_eq!(subcommand, HooksSubcommand::PostCommit);
            }
            _ => panic!("Expected Hooks command"),
        }
    }

    #[test]
    fn parse_hooks_post_rewrite() {
        let cli = Cli::try_parse_from(["sce", "hooks", "post-rewrite", "amend"])
            .expect("hooks post-rewrite should parse");
        match cli.command {
            Some(Commands::Hooks { subcommand }) => match subcommand {
                HooksSubcommand::PostRewrite { rewrite_method } => {
                    assert_eq!(rewrite_method, "amend");
                }
                _ => panic!("Expected PostRewrite subcommand"),
            },
            _ => panic!("Expected Hooks command"),
        }
    }

    #[test]
    fn parse_sync() {
        let cli = Cli::try_parse_from(["sce", "sync"]).expect("sync should parse");
        match cli.command {
            Some(Commands::Sync { format }) => {
                assert_eq!(format, OutputFormat::Text);
            }
            _ => panic!("Expected Sync command"),
        }
    }

    #[test]
    fn parse_mcp() {
        let cli = Cli::try_parse_from(["sce", "mcp"]).expect("mcp should parse");
        match cli.command {
            Some(Commands::Mcp { format }) => {
                assert_eq!(format, OutputFormat::Text);
            }
            _ => panic!("Expected Mcp command"),
        }
    }

    #[test]
    fn parse_completion_bash() {
        let cli = Cli::try_parse_from(["sce", "completion", "--shell", "bash"])
            .expect("completion bash should parse");
        match cli.command {
            Some(Commands::Completion { shell }) => {
                assert_eq!(shell, CompletionShell::Bash);
            }
            _ => panic!("Expected Completion command"),
        }
    }

    #[test]
    fn parse_completion_zsh() {
        let cli = Cli::try_parse_from(["sce", "completion", "--shell", "zsh"])
            .expect("completion zsh should parse");
        match cli.command {
            Some(Commands::Completion { shell }) => {
                assert_eq!(shell, CompletionShell::Zsh);
            }
            _ => panic!("Expected Completion command"),
        }
    }

    #[test]
    fn parse_completion_fish() {
        let cli = Cli::try_parse_from(["sce", "completion", "--shell", "fish"])
            .expect("completion fish should parse");
        match cli.command {
            Some(Commands::Completion { shell }) => {
                assert_eq!(shell, CompletionShell::Fish);
            }
            _ => panic!("Expected Completion command"),
        }
    }

    #[test]
    fn completion_requires_shell() {
        let result = Cli::try_parse_from(["sce", "completion"]);
        assert!(result.is_err(), "completion without --shell should fail");
    }

    #[test]
    fn no_command_defaults_to_none() {
        let cli = Cli::try_parse_from(["sce"]).expect("no command should parse");
        assert_eq!(cli.command, None);
    }
}
