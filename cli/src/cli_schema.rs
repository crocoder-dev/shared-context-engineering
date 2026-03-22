use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "sce", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

impl Cli {
    #[allow(dead_code)]
    pub fn parse_from<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::parse_from(args)
    }

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
        command = command.find_subcommand_mut(segment)?.clone();
    }

    let mut buffer = Vec::new();
    command
        .write_long_help(&mut buffer)
        .expect("help rendering should write to memory");

    Some(String::from_utf8(buffer).expect("help output should be valid UTF-8"))
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

        #[arg(long)]
        all_databases: bool,

        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },

    #[command(about = "Run git-hook runtime entrypoints for local Agent Trace flows")]
    Hooks {
        #[command(subcommand)]
        subcommand: HooksSubcommand,
    },

    #[command(about = "Inspect persisted Agent Trace records and prompt captures")]
    Trace {
        #[command(subcommand)]
        subcommand: TraceSubcommand,
    },

    #[command(about = "Coordinate future cloud sync workflows (placeholder)")]
    Sync {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
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

    #[command(about = "Validate config files and report resolved observability values")]
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

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TraceSubcommand {
    #[command(about = "Show captured prompts for a persisted commit trace")]
    Prompts {
        commit_sha: String,

        #[arg(long, value_enum, default_value_t = OutputFormat::Text, conflicts_with = "json")]
        format: OutputFormat,

        #[arg(long, conflicts_with = "format")]
        json: bool,
    },
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
    fn parse_auth_renew() {
        let cli = Cli::try_parse_from(["sce", "auth", "renew"]).expect("auth renew should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Renew { format, force } => {
                    assert_eq!(format, OutputFormat::Text);
                    assert!(!force);
                }
                _ => panic!("Expected Renew subcommand"),
            },
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn parse_auth_renew_json() {
        let cli = Cli::try_parse_from(["sce", "auth", "renew", "--format", "json"])
            .expect("auth renew --format json should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Renew { format, force } => {
                    assert_eq!(format, OutputFormat::Json);
                    assert!(!force);
                }
                _ => panic!("Expected Renew subcommand"),
            },
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn parse_auth_renew_force() {
        let cli = Cli::try_parse_from(["sce", "auth", "renew", "--force"])
            .expect("auth renew --force should parse");
        match cli.command {
            Some(Commands::Auth { subcommand }) => match subcommand {
                AuthSubcommand::Renew { format, force } => {
                    assert_eq!(format, OutputFormat::Text);
                    assert!(force);
                }
                _ => panic!("Expected Renew subcommand"),
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
    fn auth_help_text_lists_supported_subcommands() {
        let help = auth_help_text();

        assert!(help.contains("Usage: auth <COMMAND>"));
        assert!(help.contains("login"));
        assert!(help.contains("renew"));
        assert!(help.contains("logout"));
        assert!(help.contains("status"));
        assert!(help.contains("sce auth renew"));
        assert!(help.contains("sce auth status"));
    }

    #[test]
    fn render_help_for_auth_login_path_is_specific_to_login() {
        let help = render_help_for_path(&["auth", "login"]).expect("auth login help should render");

        assert!(help.contains("Start login flow and store credentials"));
        assert!(help.contains("--format <FORMAT>"));
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
    fn render_help_for_config_show_mentions_observability() {
        let help =
            render_help_for_path(&["config", "show"]).expect("config show help should render");

        assert!(help.contains("Show resolved runtime config, including observability sources"));
        assert!(help.contains("--config <CONFIG>"));
        assert!(help.contains("--format <FORMAT>"));
    }

    #[test]
    fn render_help_for_config_validate_mentions_observability() {
        let help = render_help_for_path(&["config", "validate"])
            .expect("config validate help should render");

        assert!(help.contains("Validate config files and report resolved observability values"));
        assert!(help.contains("--config <CONFIG>"));
        assert!(help.contains("--format <FORMAT>"));
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
        let result = Cli::try_parse_from(["sce", "setup", "--opencode", "--claude"]);
        assert!(
            result.is_err(),
            "mutually exclusive targets should fail to parse"
        );
    }

    #[test]
    fn parse_setup_repo_requires_hooks() {
        let result = Cli::try_parse_from(["sce", "setup", "--repo", "../demo-repo"]);
        assert!(result.is_err(), "--repo without --hooks should fail");
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::try_parse_from(["sce", "doctor"]).expect("doctor should parse");
        match cli.command {
            Some(Commands::Doctor {
                fix,
                all_databases,
                format,
            }) => {
                assert!(!fix);
                assert!(!all_databases);
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
            Some(Commands::Doctor {
                fix,
                all_databases,
                format,
            }) => {
                assert!(!fix);
                assert!(!all_databases);
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("Expected Doctor command"),
        }
    }

    #[test]
    fn parse_doctor_fix() {
        let cli =
            Cli::try_parse_from(["sce", "doctor", "--fix"]).expect("doctor --fix should parse");
        match cli.command {
            Some(Commands::Doctor {
                fix,
                all_databases,
                format,
            }) => {
                assert!(fix);
                assert!(!all_databases);
                assert_eq!(format, OutputFormat::Text);
            }
            _ => panic!("Expected Doctor command"),
        }
    }

    #[test]
    fn parse_doctor_all_databases() {
        let cli = Cli::try_parse_from(["sce", "doctor", "--all-databases"])
            .expect("doctor --all-databases should parse");
        match cli.command {
            Some(Commands::Doctor {
                fix,
                all_databases,
                format,
            }) => {
                assert!(!fix);
                assert!(all_databases);
                assert_eq!(format, OutputFormat::Text);
            }
            _ => panic!("Expected Doctor command"),
        }
    }

    #[test]
    fn render_help_for_doctor_mentions_fix_mode() {
        let help = render_help_for_path(&["doctor"]).expect("doctor help should render");

        assert!(help.contains("Inspect and repair SCE operator environment health"));
        assert!(help.contains("--fix"));
        assert!(help.contains("--all-databases"));
        assert!(help.contains("--format <FORMAT>"));
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
    fn parse_trace_prompts() {
        let cli = Cli::try_parse_from(["sce", "trace", "prompts", "abc1234"])
            .expect("trace prompts should parse");
        match cli.command {
            Some(Commands::Trace { subcommand }) => match subcommand {
                TraceSubcommand::Prompts {
                    commit_sha,
                    format,
                    json,
                } => {
                    assert_eq!(commit_sha, "abc1234");
                    assert_eq!(format, OutputFormat::Text);
                    assert!(!json);
                }
            },
            _ => panic!("Expected Trace command"),
        }
    }

    #[test]
    fn parse_trace_prompts_json_flag() {
        let cli = Cli::try_parse_from(["sce", "trace", "prompts", "abc1234", "--json"])
            .expect("trace prompts --json should parse");
        match cli.command {
            Some(Commands::Trace { subcommand }) => match subcommand {
                TraceSubcommand::Prompts { json, .. } => {
                    assert!(json);
                }
            },
            _ => panic!("Expected Trace command"),
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
