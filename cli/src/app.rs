use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;

use crate::services;
use services::app_support::{self, RunOutcome};
use services::error::ClassifiedError;
use services::observability::traits::{Logger as LoggerTrait, Telemetry};

struct StartupContext {
    observability_config: services::config::ResolvedObservabilityRuntimeConfig,
    startup_diagnostic: Option<String>,
}

struct AppRuntime {
    context: AppContext,
    registry: services::command_registry::CommandRegistry,
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

    pub(crate) fn logger(&self) -> &dyn LoggerTrait {
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
    app_support::render_run_outcome(
        try_run_with_dependency_check(args, dependency_check),
        stdout,
        stderr,
    )
}

fn try_run_with_dependency_check<I, F>(args: I, dependency_check: F) -> RunOutcome
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    let result = perform_dependency_check(dependency_check)
        .and_then(|()| build_startup_context())
        .and_then(initialize_runtime)
        .map(|runtime| RunOutcome {
            logger: Some(Arc::clone(&runtime.context.logger)),
            startup_diagnostic: runtime.startup_diagnostic.clone(),
            result: run_command_lifecycle(args, &runtime),
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

fn perform_dependency_check<F: FnOnce() -> anyhow::Result<()>>(
    dependency_check: F,
) -> Result<(), ClassifiedError> {
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
        .map_err(|error| app_support::classify_observability_configuration_error(&error))?;
    let startup_diagnostic = app_support::invalid_discovered_config_guidance(&observability_config);
    Ok(StartupContext {
        observability_config,
        startup_diagnostic,
    })
}

fn initialize_runtime(startup: StartupContext) -> Result<AppRuntime, ClassifiedError> {
    let logger =
        services::observability::Logger::from_resolved_config(&startup.observability_config)
            .map_err(|error| app_support::classify_observability_configuration_error(&error))?;
    app_support::log_startup_configuration(&logger, &startup.observability_config);
    let telemetry = services::observability::TelemetryRuntime::from_resolved_config(
        &startup.observability_config,
    )
    .map_err(|error| app_support::classify_observability_configuration_error(&error))?;
    let context = AppContext::new(
        Arc::new(logger),
        Arc::new(telemetry),
        Arc::new(services::capabilities::StdFsOps),
        Arc::new(services::capabilities::ProcessGitOps),
    );
    Ok(AppRuntime {
        context,
        registry: services::command_registry::build_default_registry(),
        startup_diagnostic: startup.startup_diagnostic,
    })
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
            &runtime.registry,
            context.logger(),
        )?;
        app_support::execute_command_phase(command.as_ref(), context)
    })
}

fn parse_command_phase<I>(
    args: I,
    registry: &services::command_registry::CommandRegistry,
    logger: &dyn LoggerTrait,
) -> Result<services::command_registry::RuntimeCommandHandle, ClassifiedError>
where
    I: IntoIterator<Item = String>,
{
    let command =
        services::parse::command_runtime::parse_runtime_command(args, registry, Some(logger))?;
    logger.info(
        "sce.command.parsed",
        "Command parsed",
        &[("command", command.name().as_ref())],
    );
    Ok(command)
}
