use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;

use crate::services;
use services::error::ClassifiedError;
use services::observability::traits::{Logger as LoggerTrait, Telemetry};

const INVALID_CONFIG_WARNING_EVENT_ID: &str = "sce.config.invalid_config";

struct RunOutcome {
    result: Result<String, ClassifiedError>,
    logger: Option<Arc<dyn LoggerTrait>>,
    startup_diagnostic: Option<String>,
}

struct StartupContext {
    observability_config: services::config::ResolvedObservabilityRuntimeConfig,
    startup_diagnostic: Option<String>,
}

struct AppRuntime {
    context: AppContext,
    startup_diagnostic: Option<String>,
}

pub struct AppContext {
    logger: Arc<dyn LoggerTrait>,
    telemetry: Arc<dyn Telemetry>,
    #[allow(dead_code)]
    fs: Arc<dyn services::capabilities::FsOps>,
    #[allow(dead_code)]
    git: Arc<dyn services::capabilities::GitOps>,
}

impl AppContext {
    fn new(
        logger: Arc<dyn LoggerTrait>,
        telemetry: Arc<dyn Telemetry>,
        fs: Arc<dyn services::capabilities::FsOps>,
        git: Arc<dyn services::capabilities::GitOps>,
    ) -> Self {
        Self {
            logger,
            telemetry,
            fs,
            git,
        }
    }

    fn logger(&self) -> &dyn LoggerTrait {
        self.logger.as_ref()
    }

    fn telemetry(&self) -> &dyn Telemetry {
        self.telemetry.as_ref()
    }
}

pub fn run<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    run_with_dependency_check(args, || Ok(()))
}

fn run_with_dependency_check<I, F>(args: I, dependency_check: F) -> ExitCode
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    run_with_dependency_check_and_streams(args, dependency_check, &mut stdout, &mut stderr)
}

fn run_with_dependency_check_and_streams<I, F, StdoutW, StderrW>(
    args: I,
    dependency_check: F,
    stdout: &mut StdoutW,
    stderr: &mut StderrW,
) -> ExitCode
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
    StdoutW: Write,
    StderrW: Write,
{
    let outcome = try_run_with_dependency_check(args, dependency_check);
    render_run_outcome(outcome, stdout, stderr)
}

fn render_run_outcome<StdoutW, StderrW>(
    outcome: RunOutcome,
    stdout: &mut StdoutW,
    stderr: &mut StderrW,
) -> ExitCode
where
    StdoutW: Write,
    StderrW: Write,
{
    match outcome.result {
        Ok(payload) => {
            if let Some(diagnostic) = outcome.startup_diagnostic {
                write_startup_diagnostic(stderr, &diagnostic);
            }

            if let Err(error) = write_stdout_payload(stdout, &payload) {
                if let Some(ref log) = outcome.logger {
                    log.log_classified_error(&error);
                }
                write_error_diagnostic(stderr, &error);
                ExitCode::from(error.class().exit_code())
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            if let Some(ref log) = outcome.logger {
                log.log_classified_error(&error);
            }
            write_error_diagnostic(stderr, &error);
            ExitCode::from(error.class().exit_code())
        }
    }
}

fn write_stdout_payload<W>(writer: &mut W, payload: &str) -> Result<(), ClassifiedError>
where
    W: Write,
{
    if payload.is_empty() {
        return Ok(());
    }

    writeln!(writer, "{payload}").map_err(|error| {
        ClassifiedError::runtime(format!("Failed to write command output to stdout: {error}"))
    })
}

fn write_error_diagnostic<W>(writer: &mut W, error: &ClassifiedError)
where
    W: Write,
{
    let rendered = if error.message().contains("Try:") {
        error.message().to_string()
    } else {
        format!(
            "{} Try: {}",
            error.message(),
            error.class().default_try_guidance()
        )
    };

    let styled_code = services::style::error_code(error.code());
    let styled_heading = services::style::heading("Error");
    let styled_message =
        services::style::error_text(&services::security::redact_sensitive_text(&rendered));

    writeln!(writer, "{styled_heading} [{styled_code}]: {styled_message}")
        .expect("writing error diagnostic to writer should not fail");
}

fn write_startup_diagnostic<W>(writer: &mut W, diagnostic: &str)
where
    W: Write,
{
    writeln!(writer, "{}", services::style::error_code(diagnostic))
        .expect("writing startup diagnostic to writer should not fail");
}

fn invalid_discovered_config_guidance(
    observability_config: &services::config::ResolvedObservabilityRuntimeConfig,
) -> Option<String> {
    if observability_config.validation_errors.is_empty() {
        return None;
    }

    let has_invalid_local_config = observability_config
        .loaded_config_paths
        .iter()
        .filter(|loaded_path| {
            loaded_path.source == services::config::ConfigPathSource::DefaultDiscoveredLocal
        })
        .any(|loaded_path| {
            let rendered_path = loaded_path.path.to_string_lossy();
            observability_config
                .validation_errors
                .iter()
                .any(|error| error.contains(rendered_path.as_ref()))
        });

    Some(if has_invalid_local_config {
        String::from("Local `.sce` config is invalid. Fix `.sce` and run `sce config validate`.")
    } else {
        String::from("A discovered config file is invalid. Fix it and run `sce config validate`.")
    })
}

fn try_run_with_dependency_check<I, F>(args: I, dependency_check: F) -> RunOutcome
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    let result = perform_dependency_check(dependency_check)
        .and_then(|()| build_startup_context())
        .and_then(initialize_runtime)
        .map(|runtime| {
            let logger = Arc::clone(&runtime.context.logger);
            let startup_diagnostic = runtime.startup_diagnostic.clone();
            let result = run_command_lifecycle(args, &runtime);

            RunOutcome {
                result,
                logger: Some(logger),
                startup_diagnostic,
            }
        });

    match result {
        Ok(outcome) => outcome,
        Err(error) => RunOutcome {
            result: Err(error),
            logger: None,
            startup_diagnostic: None,
        },
    }
}

fn perform_dependency_check<F>(dependency_check: F) -> Result<(), ClassifiedError>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    dependency_check().map_err(|error| {
        ClassifiedError::dependency(format!("Failed to initialize dependency checks: {error}"))
    })
}

fn build_startup_context() -> Result<StartupContext, ClassifiedError> {
    let cwd = std::env::current_dir().map_err(|error| {
        ClassifiedError::runtime(format!(
            "Failed to determine current directory for observability config resolution: {error}"
        ))
    })?;

    let observability_config = services::config::resolve_observability_runtime_config(&cwd)
        .map_err(|error| classify_observability_configuration_error(&error))?;

    Ok(StartupContext {
        startup_diagnostic: invalid_discovered_config_guidance(&observability_config),
        observability_config,
    })
}

fn initialize_runtime(startup: StartupContext) -> Result<AppRuntime, ClassifiedError> {
    let logger =
        services::observability::Logger::from_resolved_config(&startup.observability_config)
            .map_err(|error| classify_observability_configuration_error(&error))?;

    log_startup_configuration(&logger, &startup.observability_config);

    let telemetry = services::observability::TelemetryRuntime::from_resolved_config(
        &startup.observability_config,
    )
    .map_err(|error| classify_observability_configuration_error(&error))?;

    let context = AppContext::new(
        Arc::new(logger),
        Arc::new(telemetry),
        Arc::new(services::capabilities::StdFsOps),
        Arc::new(services::capabilities::ProcessGitOps),
    );

    Ok(AppRuntime {
        context,
        startup_diagnostic: startup.startup_diagnostic,
    })
}

fn classify_observability_configuration_error(error: &anyhow::Error) -> ClassifiedError {
    ClassifiedError::validation(format!("Invalid observability configuration: {error}"))
}

fn log_startup_configuration(
    logger: &services::observability::Logger,
    observability_config: &services::config::ResolvedObservabilityRuntimeConfig,
) {
    for loaded_path in &observability_config.loaded_config_paths {
        logger.debug(
            "sce.config.file_discovered",
            "Config file discovered",
            &[
                ("path", loaded_path.path.to_string_lossy().as_ref()),
                ("source", loaded_path.source.as_str()),
            ],
        );
    }

    for validation_error in &observability_config.validation_errors {
        logger.warn(
            INVALID_CONFIG_WARNING_EVENT_ID,
            "Invalid discovered config skipped; using degraded defaults",
            &[("error", validation_error.as_str())],
        );
    }
}

fn run_command_lifecycle<I>(args: I, runtime: &AppRuntime) -> Result<String, ClassifiedError>
where
    I: IntoIterator<Item = String>,
{
    let context = &runtime.context;
    let mut args = Some(args.into_iter().collect::<Vec<_>>());
    context.telemetry().with_default_subscriber(&mut || {
        context.logger().info(
            "sce.app.start",
            "Starting command dispatch",
            &[("component", services::observability::NAME)],
        );

        let command = parse_command_phase(
            args.take()
                .expect("command lifecycle should execute exactly once"),
            context.logger(),
        )?;
        execute_command_phase(command.as_ref(), context)
    })
}

fn parse_command_phase<I>(
    args: I,
    logger: &dyn LoggerTrait,
) -> Result<services::command_registry::RuntimeCommandHandle, ClassifiedError>
where
    I: IntoIterator<Item = String>,
{
    let command = command_runtime::parse_runtime_command(args, Some(logger))?;
    let command_name = command.name();

    logger.info(
        "sce.command.parsed",
        "Command parsed",
        &[("command", command_name.as_ref())],
    );

    Ok(command)
}

fn execute_command_phase(
    command: &dyn services::command_registry::RuntimeCommand,
    context: &AppContext,
) -> Result<String, ClassifiedError> {
    let command_name = command.name();
    let logger = context.logger();

    logger.debug(
        "sce.command.dispatch_start",
        "Dispatching command",
        &[("command", command_name.as_ref())],
    );

    let dispatch_result = command.execute(context);

    if dispatch_result.is_ok() {
        logger.debug(
            "sce.command.dispatch_end",
            "Command dispatch completed",
            &[("command", command_name.as_ref())],
        );
    }

    match dispatch_result {
        Ok(payload) => {
            logger.info(
                "sce.command.completed",
                "Command completed",
                &[("command", command_name.as_ref())],
            );
            Ok(payload)
        }
        Err(error) => Err(error),
    }
}

mod command_runtime {
    use std::path::PathBuf;

    use anyhow::Context;

    use crate::app::AppContext;
    use crate::{cli_schema, command_surface, services};
    use services::command_registry::RuntimeCommandHandle;
    use services::error::{ClassifiedError, FailureClass};
    use services::observability::traits::Logger as LoggerTrait;

    pub fn parse_runtime_command<I>(
        args: I,
        logger: Option<&dyn LoggerTrait>,
    ) -> Result<RuntimeCommandHandle, ClassifiedError>
    where
        I: IntoIterator<Item = String>,
    {
        let args_vec: Vec<String> = args.into_iter().collect();

        if let Some(log) = logger {
            let args_summary = if args_vec.len() <= 1 {
                args_vec.join(" ")
            } else {
                format!("{} ...", args_vec[0])
            };
            log.debug(
                "sce.command.raw_args",
                "Parsing command arguments",
                &[("args_summary", &args_summary)],
            );
        }

        if args_vec.len() <= 1 {
            return Ok(services::help::command::make_help_command());
        }

        let cli = match cli_schema::Cli::try_parse_from(&args_vec) {
            Ok(cli) => cli,
            Err(error) => {
                if error.kind() == clap::error::ErrorKind::DisplayHelp {
                    if let Some((name, text)) = render_subcommand_help_from_args(&args_vec) {
                        return Ok(Box::new(services::help::command::HelpTextCommand {
                            name,
                            text,
                        }));
                    }

                    return Ok(services::help::command::make_help_command());
                }
                if error.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                {
                    if let Some(help_text) = render_missing_subcommand_help(&args_vec) {
                        return Ok(help_text);
                    }

                    return Err(ClassifiedError::parse(
                        "Missing required subcommand. Try: run 'sce --help' to see valid commands.",
                    ));
                }
                if error.kind() == clap::error::ErrorKind::DisplayVersion {
                    return Ok(Box::new(services::version::command::VersionCommand {
                        request: services::version::VersionRequest {
                            format: services::version::VersionFormat::Text,
                        },
                    }));
                }
                return Err(classify_clap_error(&error));
            }
        };

        let Some(command) = cli.command else {
            return Ok(services::help::command::make_help_command());
        };

        convert_clap_command(command)
    }

    struct SetupCommand {
        request: services::setup::SetupRequest,
    }

    impl services::command_registry::RuntimeCommand for SetupCommand {
        fn name(&self) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Borrowed(services::setup::NAME)
        }

        fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
            let current_dir = std::env::current_dir()
                .context("Failed to determine current directory")
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            let repository_root = services::setup::ensure_git_repository(&current_dir)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            services::setup::bootstrap_repo_local_config(&repository_root)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            services::setup::bootstrap_local_db()
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

            let preflight_hooks_repository = if self.request.install_hooks {
                let hooks_repository = self
                    .request
                    .hooks_repo_path
                    .as_deref()
                    .unwrap_or(repository_root.as_path());
                Some(
                    services::setup::prepare_setup_hooks_repository(hooks_repository)
                        .map_err(|error| ClassifiedError::runtime(error.to_string()))?,
                )
            } else {
                None
            };

            let mut sections = Vec::new();

            if let Some(mode) = self.request.config_mode {
                let dispatch = services::setup::resolve_setup_dispatch(
                    mode,
                    &services::setup::InquireSetupTargetPrompter,
                )
                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;

                match dispatch {
                    services::setup::SetupDispatch::Proceed(resolved_mode) => {
                        let setup_message =
                            services::setup::run_setup_for_mode(&repository_root, resolved_mode)
                                .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
                        sections.push(setup_message);
                    }
                    services::setup::SetupDispatch::Cancelled => {
                        return Ok(services::setup::setup_cancelled_text());
                    }
                }
            }

            if self.request.install_hooks {
                let repository_root = preflight_hooks_repository
                    .as_deref()
                    .expect("hook repository preflight should exist when install_hooks is true");
                let hooks_message = services::setup::run_setup_hooks(repository_root)
                    .map_err(|error| ClassifiedError::runtime(error.to_string()))?;
                sections.push(hooks_message);
            }

            Ok(sections.join("\n\n"))
        }
    }

    struct DoctorCommand {
        request: services::doctor::DoctorRequest,
    }

    impl services::command_registry::RuntimeCommand for DoctorCommand {
        fn name(&self) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Borrowed(services::doctor::NAME)
        }

        fn execute(&self, _context: &AppContext) -> Result<String, ClassifiedError> {
            services::doctor::run_doctor(self.request)
                .map_err(|error| ClassifiedError::runtime(error.to_string()))
        }
    }

    struct HooksCommand {
        subcommand: services::hooks::HookSubcommand,
    }

    impl services::command_registry::RuntimeCommand for HooksCommand {
        fn name(&self) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Borrowed(services::hooks::NAME)
        }

        fn execute(&self, context: &AppContext) -> Result<String, ClassifiedError> {
            services::hooks::run_hooks_subcommand(&self.subcommand, Some(context.logger()))
                .map_err(|error| ClassifiedError::runtime(error.to_string()))
        }
    }

    fn classify_clap_error(error: &clap::Error) -> ClassifiedError {
        use clap::error::ErrorKind;

        let message = error.to_string();

        let class = match error.kind() {
            ErrorKind::MissingRequiredArgument
            | ErrorKind::ArgumentConflict
            | ErrorKind::NoEquals => FailureClass::Validation,
            _ => FailureClass::Parse,
        };

        let cleaned_message = clean_clap_error_message(&message, error.kind());

        match class {
            FailureClass::Validation => ClassifiedError::validation(cleaned_message),
            _ => ClassifiedError::parse(cleaned_message),
        }
    }

    fn render_subcommand_help_from_args(args: &[String]) -> Option<(String, String)> {
        let command_name = args.get(1)?.to_owned();
        let command_path = args[1..]
            .iter()
            .take_while(|arg| !arg.starts_with('-'))
            .map(String::as_str)
            .collect::<Vec<_>>();

        if command_path.is_empty() {
            return None;
        }

        if command_path.as_slice() == [services::auth_command::NAME] {
            return Some((command_name, cli_schema::auth_help_text()));
        }

        cli_schema::render_help_for_path(&command_path).map(|text| (command_name, text))
    }

    fn render_missing_subcommand_help(args: &[String]) -> Option<RuntimeCommandHandle> {
        let command_name = args.get(1)?.as_str();

        match command_name {
            services::auth_command::NAME => {
                Some(Box::new(services::help::command::HelpTextCommand {
                    name: services::auth_command::NAME.to_string(),
                    text: cli_schema::auth_help_text(),
                }))
            }
            services::config::NAME => Some(Box::new(services::help::command::HelpTextCommand {
                name: services::config::NAME.to_string(),
                text: cli_schema::render_help_for_path(&[services::config::NAME])?,
            })),
            _ => None,
        }
    }

    fn clean_clap_error_message(message: &str, kind: clap::error::ErrorKind) -> String {
        use clap::error::ErrorKind;

        let message = message.strip_prefix("error: ").unwrap_or(message);

        match kind {
            ErrorKind::InvalidSubcommand => {
                if let Some(subcommand) = extract_quoted_value(message) {
                    if command_surface::is_known_command(&subcommand) {
                        format!(
                            "Command '{subcommand}' is currently unavailable in this build. Try: run 'sce --help' to see available commands in this build."
                        )
                    } else {
                        format!(
                            "Unknown command '{subcommand}'. Try: run 'sce --help' to list valid commands, then rerun with a valid command such as 'sce version' or 'sce setup --help'."
                        )
                    }
                } else {
                    format!("{message}. Try: run 'sce --help' to see valid usage.")
                }
            }
            ErrorKind::UnknownArgument => {
                if let Some(arg) = extract_quoted_value(message) {
                    format!(
                        "Unknown option '{arg}'. Try: run 'sce --help' to see top-level usage, or use 'sce <command> --help' for command-specific options."
                    )
                } else {
                    format!("{message}. Try: run 'sce --help' to see valid usage.")
                }
            }
            ErrorKind::MissingRequiredArgument => {
                if message.contains("required") {
                    format!("{message}. Try: run 'sce --help' to see required arguments.")
                } else {
                    format!("{message}. Try: run 'sce --help' to see valid usage.")
                }
            }
            ErrorKind::ArgumentConflict => {
                if message.contains("cannot be used with") || message.contains("conflicts with") {
                    format!("{message}. Try: use only one of the conflicting options.")
                } else {
                    format!("{message}. Try: run 'sce --help' to see valid usage.")
                }
            }
            _ => {
                if message.contains("Try:") {
                    message.to_string()
                } else {
                    format!("{message}. Try: run 'sce --help' to see valid usage.")
                }
            }
        }
    }

    fn extract_quoted_value(message: &str) -> Option<String> {
        let start = message.find('\'')?;
        let end = message[start + 1..].find('\'')?;
        Some(message[start + 1..start + 1 + end].to_string())
    }

    fn convert_clap_command(
        command: cli_schema::Commands,
    ) -> Result<RuntimeCommandHandle, ClassifiedError> {
        match command {
            cli_schema::Commands::Config { subcommand } => convert_config_subcommand(subcommand),
            cli_schema::Commands::Auth { subcommand } => convert_auth_subcommand(subcommand),
            cli_schema::Commands::Setup {
                opencode,
                claude,
                both,
                non_interactive,
                hooks,
                repo,
            } => convert_setup_command(opencode, claude, both, non_interactive, hooks, repo),
            cli_schema::Commands::Doctor { fix, format } => Ok(Box::new(DoctorCommand {
                request: services::doctor::DoctorRequest {
                    mode: if fix {
                        services::doctor::DoctorMode::Fix
                    } else {
                        services::doctor::DoctorMode::Diagnose
                    },
                    format: convert_output_format(format),
                },
            })),
            cli_schema::Commands::Hooks { subcommand } => convert_hooks_subcommand(subcommand),
            cli_schema::Commands::Version { format } => {
                Ok(Box::new(services::version::command::VersionCommand {
                    request: services::version::VersionRequest {
                        format: convert_output_format(format),
                    },
                }))
            }
            cli_schema::Commands::Completion { shell } => {
                Ok(Box::new(services::completion::command::CompletionCommand {
                    request: services::completion::CompletionRequest {
                        shell: convert_completion_shell(shell),
                    },
                }))
            }
        }
    }

    #[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
    fn convert_auth_subcommand(
        subcommand: cli_schema::AuthSubcommand,
    ) -> Result<RuntimeCommandHandle, ClassifiedError> {
        let subcommand = match subcommand {
            cli_schema::AuthSubcommand::Login { format } => {
                services::auth_command::AuthSubcommand::Login {
                    format: convert_output_format(format),
                }
            }
            cli_schema::AuthSubcommand::Renew { format, force } => {
                services::auth_command::AuthSubcommand::Renew {
                    format: convert_output_format(format),
                    force,
                }
            }
            cli_schema::AuthSubcommand::Logout { format } => {
                services::auth_command::AuthSubcommand::Logout {
                    format: convert_output_format(format),
                }
            }
            cli_schema::AuthSubcommand::Status { format } => {
                services::auth_command::AuthSubcommand::Status {
                    format: convert_output_format(format),
                }
            }
        };

        Ok(Box::new(services::auth_command::command::AuthCommand {
            request: services::auth_command::AuthRequest { subcommand },
        }))
    }

    fn convert_output_format(
        format: cli_schema::OutputFormat,
    ) -> services::output_format::OutputFormat {
        match format {
            cli_schema::OutputFormat::Text => services::output_format::OutputFormat::Text,
            cli_schema::OutputFormat::Json => services::output_format::OutputFormat::Json,
        }
    }

    fn convert_completion_shell(
        shell: cli_schema::CompletionShell,
    ) -> services::completion::CompletionShell {
        match shell {
            cli_schema::CompletionShell::Bash => services::completion::CompletionShell::Bash,
            cli_schema::CompletionShell::Zsh => services::completion::CompletionShell::Zsh,
            cli_schema::CompletionShell::Fish => services::completion::CompletionShell::Fish,
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn convert_config_subcommand(
        subcommand: cli_schema::ConfigSubcommand,
    ) -> Result<RuntimeCommandHandle, ClassifiedError> {
        match subcommand {
            cli_schema::ConfigSubcommand::Show {
                format,
                config,
                log_level,
                timeout_ms,
            } => Ok(Box::new(services::config::command::ConfigCommand {
                subcommand: services::config::ConfigSubcommand::Show(
                    services::config::ConfigRequest {
                        report_format: convert_output_format(format),
                        config_path: config,
                        log_level: log_level.map(convert_log_level),
                        timeout_ms,
                    },
                ),
            })),
            cli_schema::ConfigSubcommand::Validate {
                format,
                config,
                log_level,
                timeout_ms,
            } => Ok(Box::new(services::config::command::ConfigCommand {
                subcommand: services::config::ConfigSubcommand::Validate(
                    services::config::ConfigRequest {
                        report_format: convert_output_format(format),
                        config_path: config,
                        log_level: log_level.map(convert_log_level),
                        timeout_ms,
                    },
                ),
            })),
        }
    }

    fn convert_log_level(level: cli_schema::LogLevel) -> services::config::LogLevel {
        match level {
            cli_schema::LogLevel::Error => services::config::LogLevel::Error,
            cli_schema::LogLevel::Warn => services::config::LogLevel::Warn,
            cli_schema::LogLevel::Info => services::config::LogLevel::Info,
            cli_schema::LogLevel::Debug => services::config::LogLevel::Debug,
        }
    }

    #[allow(clippy::fn_params_excessive_bools)]
    fn convert_setup_command(
        opencode: bool,
        claude: bool,
        both: bool,
        non_interactive: bool,
        hooks: bool,
        repo: Option<PathBuf>,
    ) -> Result<RuntimeCommandHandle, ClassifiedError> {
        let options = services::setup::SetupCliOptions {
            help: false,
            non_interactive,
            opencode,
            claude,
            both,
            hooks,
            repo_path: repo,
        };

        let request = services::setup::resolve_setup_request(options)
            .map_err(|error| ClassifiedError::validation(error.to_string()))?;

        Ok(Box::new(SetupCommand { request }))
    }

    #[allow(clippy::unnecessary_wraps)]
    fn convert_hooks_subcommand(
        subcommand: cli_schema::HooksSubcommand,
    ) -> Result<RuntimeCommandHandle, ClassifiedError> {
        match subcommand {
            cli_schema::HooksSubcommand::PreCommit => Ok(Box::new(HooksCommand {
                subcommand: services::hooks::HookSubcommand::PreCommit,
            })),
            cli_schema::HooksSubcommand::CommitMsg { message_file } => Ok(Box::new(HooksCommand {
                subcommand: services::hooks::HookSubcommand::CommitMsg { message_file },
            })),
            cli_schema::HooksSubcommand::PostCommit => Ok(Box::new(HooksCommand {
                subcommand: services::hooks::HookSubcommand::PostCommit,
            })),
            cli_schema::HooksSubcommand::PostRewrite { rewrite_method } => {
                Ok(Box::new(HooksCommand {
                    subcommand: services::hooks::HookSubcommand::PostRewrite { rewrite_method },
                }))
            }
            cli_schema::HooksSubcommand::DiffTrace => Ok(Box::new(HooksCommand {
                subcommand: services::hooks::HookSubcommand::DiffTrace,
            })),
        }
    }
}
