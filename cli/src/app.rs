use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::services;
use services::app_support::{self, RunOutcome};
use services::error::ClassifiedError;
use services::observability::traits::{
    Logger as LoggerTrait, NoopTelemetry, Telemetry as TelemetryTrait,
};

const REPEATED_COMMAND_DISPATCH_ERROR: &str =
    "Command lifecycle telemetry attempted to execute command dispatch more than once";

struct StartupContext {
    observability_config: services::config::ResolvedObservabilityRuntimeConfig,
    startup_diagnostic: Option<String>,
}

struct AppRuntime {
    logger: services::observability::Logger,
    telemetry: NoopTelemetry,
    fs: services::capabilities::StdFsOps,
    git: services::capabilities::ProcessGitOps,
    registry: services::command_registry::CommandRegistry,
    startup_diagnostic: Option<String>,
}

pub struct AppContext<
    'a,
    L: LoggerTrait = services::observability::Logger,
    T: TelemetryTrait = NoopTelemetry,
    F: services::capabilities::FsOps = services::capabilities::StdFsOps,
    G: services::capabilities::GitOps = services::capabilities::ProcessGitOps,
> {
    logger: &'a L,
    telemetry: &'a T,
    fs: &'a F,
    git: &'a G,
    repo_root: Option<PathBuf>,
}

type ProductionAppContext<'a> = AppContext<
    'a,
    services::observability::Logger,
    NoopTelemetry,
    services::capabilities::StdFsOps,
    services::capabilities::ProcessGitOps,
>;

pub(crate) trait HasLogger {
    type Logger: LoggerTrait;

    fn logger(&self) -> &Self::Logger;
}

#[allow(dead_code)]
pub(crate) trait HasTelemetry {
    type Telemetry: TelemetryTrait;

    fn telemetry(&self) -> &Self::Telemetry;
}

#[allow(dead_code)]
pub(crate) trait HasFs {
    type Fs: services::capabilities::FsOps;

    fn fs(&self) -> &Self::Fs;
}

#[allow(dead_code)]
pub(crate) trait HasGit {
    type Git: services::capabilities::GitOps;

    fn git(&self) -> &Self::Git;
}

pub(crate) trait HasRepoRoot {
    fn repo_root(&self) -> Option<&Path>;
}

pub(crate) trait ContextWithRepoRoot: HasRepoRoot {
    fn with_repo_root(&self, repo_root: impl Into<PathBuf>) -> Self;
}

impl<'a, L, T, F, G> AppContext<'a, L, T, F, G>
where
    L: LoggerTrait,
    T: TelemetryTrait,
    F: services::capabilities::FsOps,
    G: services::capabilities::GitOps,
{
    pub(crate) fn new(
        logger: &'a L,
        telemetry: &'a T,
        fs: &'a F,
        git: &'a G,
        repo_root: Option<PathBuf>,
    ) -> Self {
        Self {
            logger,
            telemetry,
            fs,
            git,
            repo_root,
        }
    }

    pub(crate) fn logger(&self) -> &L {
        HasLogger::logger(self)
    }

    #[allow(dead_code)]
    pub(crate) fn fs(&self) -> &F {
        HasFs::fs(self)
    }

    #[allow(dead_code)]
    pub(crate) fn git(&self) -> &G {
        HasGit::git(self)
    }

    fn telemetry(&self) -> &T {
        HasTelemetry::telemetry(self)
    }

    /// Returns a context for a command-scoped repository root while preserving
    /// the runtime logger, telemetry, and capability dependencies.
    #[allow(dead_code)]
    pub(crate) fn with_repo_root(&self, repo_root: impl Into<PathBuf>) -> Self {
        Self {
            logger: self.logger,
            telemetry: self.telemetry,
            fs: self.fs,
            git: self.git,
            repo_root: Some(repo_root.into()),
        }
    }

    /// Returns the resolved repository root path when available.
    ///
    /// Lifecycle providers use this during setup to avoid re-resolving
    /// the repository root independently.
    #[allow(dead_code)]
    pub fn repo_root(&self) -> Option<&Path> {
        HasRepoRoot::repo_root(self)
    }
}

impl<L, T, F, G> HasLogger for AppContext<'_, L, T, F, G>
where
    L: LoggerTrait,
    T: TelemetryTrait,
    F: services::capabilities::FsOps,
    G: services::capabilities::GitOps,
{
    type Logger = L;

    fn logger(&self) -> &Self::Logger {
        self.logger
    }
}

impl<L, T, F, G> HasTelemetry for AppContext<'_, L, T, F, G>
where
    L: LoggerTrait,
    T: TelemetryTrait,
    F: services::capabilities::FsOps,
    G: services::capabilities::GitOps,
{
    type Telemetry = T;

    fn telemetry(&self) -> &Self::Telemetry {
        self.telemetry
    }
}

impl<L, T, F, G> HasFs for AppContext<'_, L, T, F, G>
where
    L: LoggerTrait,
    T: TelemetryTrait,
    F: services::capabilities::FsOps,
    G: services::capabilities::GitOps,
{
    type Fs = F;

    fn fs(&self) -> &Self::Fs {
        self.fs
    }
}

impl<L, T, F, G> HasGit for AppContext<'_, L, T, F, G>
where
    L: LoggerTrait,
    T: TelemetryTrait,
    F: services::capabilities::FsOps,
    G: services::capabilities::GitOps,
{
    type Git = G;

    fn git(&self) -> &Self::Git {
        self.git
    }
}

impl<L, T, F, G> HasRepoRoot for AppContext<'_, L, T, F, G>
where
    L: LoggerTrait,
    T: TelemetryTrait,
    F: services::capabilities::FsOps,
    G: services::capabilities::GitOps,
{
    fn repo_root(&self) -> Option<&Path> {
        self.repo_root.as_deref()
    }
}

impl<L, T, F, G> ContextWithRepoRoot for AppContext<'_, L, T, F, G>
where
    L: LoggerTrait,
    T: TelemetryTrait,
    F: services::capabilities::FsOps,
    G: services::capabilities::GitOps,
{
    fn with_repo_root(&self, repo_root: impl Into<PathBuf>) -> Self {
        AppContext::with_repo_root(self, repo_root)
    }
}

impl AppRuntime {
    fn context(&self) -> ProductionAppContext<'_> {
        AppContext::new(&self.logger, &self.telemetry, &self.fs, &self.git, None)
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

fn try_run_with_dependency_check<I, F>(
    args: I,
    dependency_check: F,
) -> RunOutcome<services::observability::Logger>
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    let result = perform_dependency_check(dependency_check)
        .and_then(|()| build_startup_context())
        .and_then(initialize_runtime)
        .map(|runtime| {
            let startup_diagnostic = runtime.startup_diagnostic.clone();
            let result = run_command_lifecycle(args, &runtime);
            RunOutcome {
                logger: Some(runtime.logger),
                startup_diagnostic,
                result,
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
    services::config::init_database_retry_config_from_environment(&cwd);
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
    Ok(AppRuntime {
        logger,
        telemetry: NoopTelemetry,
        fs: services::capabilities::StdFsOps,
        git: services::capabilities::ProcessGitOps,
        registry: services::command_registry::build_default_registry(),
        startup_diagnostic: startup.startup_diagnostic,
    })
}

fn run_command_lifecycle<I>(args: I, runtime: &AppRuntime) -> Result<String, ClassifiedError>
where
    I: IntoIterator<Item = String>,
{
    let context = runtime.context();
    let mut args = Some(args.into_iter().collect::<Vec<_>>());
    context.telemetry().with_default_subscriber(&mut || {
        context.logger().info(
            "sce.app.start",
            "Starting command dispatch",
            &[("component", services::observability::NAME)],
        );
        let Some(command_args) = args.take() else {
            return Err(ClassifiedError::runtime(REPEATED_COMMAND_DISPATCH_ERROR));
        };
        let command = parse_command_phase(command_args, &runtime.registry, &context)?;
        app_support::execute_command_phase(&command, &context)
    })
}

fn parse_command_phase<I>(
    args: I,
    registry: &services::command_registry::CommandRegistry,
    context: &impl HasLogger,
) -> Result<services::command_registry::RuntimeCommand, ClassifiedError>
where
    I: IntoIterator<Item = String>,
{
    let logger = context.logger();
    let command =
        services::parse::command_runtime::parse_runtime_command(args, registry, Some(logger))?;
    logger.info(
        "sce.command.parsed",
        "Command parsed",
        &[("command", command.name().as_ref())],
    );
    Ok(command)
}
