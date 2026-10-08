use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::services;
use services::app_support::{self, RunOutcome};
use services::error::CliError;
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

/// Lightweight borrowed view of the CLI runtime dependencies.
///
/// `AppContext` does **not** own its dependencies; it borrows them from `AppRuntime`.
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

pub async fn run<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    run_with_dependency_check(args, || Ok(())).await
}

async fn run_with_dependency_check<I, F>(args: I, dependency_check: F) -> ExitCode
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
{
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    run_with_dependency_check_and_streams(args, dependency_check, &mut stdout, &mut stderr).await
}

async fn run_with_dependency_check_and_streams<I, F, StdoutW, StderrW>(
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
        try_run_with_dependency_check(args, dependency_check, stderr).await,
        stdout,
        stderr,
    )
}

async fn try_run_with_dependency_check<I, F, StderrW>(
    args: I,
    dependency_check: F,
    stderr: &mut StderrW,
) -> RunOutcome<services::observability::Logger>
where
    I: IntoIterator<Item = String>,
    F: FnOnce() -> anyhow::Result<()>,
    StderrW: Write,
{
    let result = perform_dependency_check(dependency_check)
        .and_then(|()| build_startup_context())
        .and_then(initialize_runtime);

    match result {
        Ok(runtime) => {
            let startup_diagnostic = runtime.startup_diagnostic.clone();
            let result = run_command_lifecycle(args, &runtime, stderr).await;
            RunOutcome {
                logger: Some(runtime.logger),
                startup_diagnostic,
                result,
            }
        }
        Err(error) => RunOutcome {
            result: Err(error),
            logger: None,
            startup_diagnostic: None,
        },
    }
}

fn perform_dependency_check<F: FnOnce() -> anyhow::Result<()>>(
    dependency_check: F,
) -> Result<(), CliError> {
    dependency_check().map_err(|error| {
        CliError::dependency(anyhow::Error::msg(format!(
            "Failed to initialize dependency checks: {error}"
        )))
    })
}

fn build_startup_context() -> Result<StartupContext, CliError> {
    let cwd = std::env::current_dir().map_err(|error| {
        CliError::runtime(anyhow::Error::msg(format!(
            "Failed to determine current directory for observability config resolution: {error}"
        )))
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

fn initialize_runtime(startup: StartupContext) -> Result<AppRuntime, CliError> {
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

async fn run_command_lifecycle<I, StderrW>(
    args: I,
    runtime: &AppRuntime,
    stderr: &mut StderrW,
) -> Result<String, CliError>
where
    I: IntoIterator<Item = String>,
    StderrW: Write,
{
    let context = runtime.context();
    run_command_lifecycle_with_context(args, &runtime.registry, &context, stderr).await
}

async fn run_command_lifecycle_with_context<I, L, T, StderrW>(
    args: I,
    registry: &services::command_registry::CommandRegistry,
    context: &AppContext<'_, L, T>,
    stderr: &mut StderrW,
) -> Result<String, CliError>
where
    I: IntoIterator<Item = String>,
    L: LoggerTrait,
    T: TelemetryTrait,
    StderrW: Write,
{
    let mut args = Some(args.into_iter().collect::<Vec<_>>());
    let mut stderr = Some(stderr);
    context
        .telemetry()
        .with_default_subscriber(&mut || {
            let command_inputs = args.take().zip(stderr.take());
            async move {
                context.logger().info(
                    "sce.app.start",
                    "Starting command dispatch",
                    &[("component", services::observability::NAME)],
                    None,
                );
                let Some((command_args, stderr)) = command_inputs else {
                    return Err(CliError::runtime(anyhow::Error::msg(
                        REPEATED_COMMAND_DISPATCH_ERROR,
                    )));
                };
                let command = parse_command_phase(command_args, registry, context)?;
                app_support::execute_command_phase(&command, context, stderr).await
            }
        })
        .await
}

fn parse_command_phase<I>(
    args: I,
    registry: &services::command_registry::CommandRegistry,
    context: &impl HasLogger,
) -> Result<services::command_registry::RuntimeCommand, CliError>
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
        None,
    );
    Ok(command)
}

#[cfg(test)]
mod tests {
    use std::future::{poll_fn, Future};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct RecordingScope {
        active: bool,
        polls: usize,
        events: Vec<String>,
    }

    struct RecordingLogger(Arc<Mutex<RecordingScope>>);

    impl LoggerTrait for RecordingLogger {
        fn info(&self, event: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {
            let mut scope = self.0.lock().unwrap();
            assert!(
                scope.active,
                "dispatch logging must occur inside a telemetry poll"
            );
            scope.events.push(event.to_string());
        }
        fn debug(
            &self,
            event: &str,
            message: &str,
            fields: &[(&str, &str)],
            session: Option<&str>,
        ) {
            self.info(event, message, fields, session);
        }
        fn warn(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn error(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn log_cli_error(&self, _: &CliError, _: Option<&str>) {}
    }

    struct RecordingTelemetry {
        scope: Arc<Mutex<RecordingScope>>,
        repeat: bool,
    }

    impl RecordingTelemetry {
        async fn poll_scoped<F: Future>(&self, future: F) -> F::Output {
            let mut future = std::pin::pin!(future);
            poll_fn(|cx| {
                {
                    let mut scope = self.scope.lock().unwrap();
                    assert!(!scope.active);
                    scope.active = true;
                    scope.polls += 1;
                    scope.events.push("telemetry.poll.start".into());
                }
                let result = future.as_mut().poll(cx);
                let mut scope = self.scope.lock().unwrap();
                scope.events.push("telemetry.poll.end".into());
                scope.active = false;
                result
            })
            .await
        }
    }

    impl TelemetryTrait for RecordingTelemetry {
        async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
        where
            F: FnMut() -> Fut,
            Fut: Future<Output = Result<String, CliError>>,
        {
            let result = self.poll_scoped(action()).await;
            if self.repeat {
                assert!(result.is_ok());
                self.poll_scoped(action()).await
            } else {
                result
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn telemetry_scopes_every_poll_of_yielding_work() {
        let scope = Arc::new(Mutex::new(RecordingScope::default()));
        let telemetry = RecordingTelemetry {
            scope: scope.clone(),
            repeat: false,
        };
        let result = telemetry
            .with_default_subscriber(&mut || async {
                NoopTelemetry
                    .with_default_subscriber(&mut || async {
                        assert!(scope.lock().unwrap().active);
                        tokio::task::yield_now().await;
                        assert!(scope.lock().unwrap().active);
                        assert_eq!(
                            tokio::runtime::Handle::current().runtime_flavor(),
                            tokio::runtime::RuntimeFlavor::MultiThread
                        );
                        Ok("finished".into())
                    })
                    .await
            })
            .await
            .unwrap();
        assert_eq!(result, "finished");
        let scope = scope.lock().unwrap();
        assert!(scope.polls >= 2);
        assert!(!scope.active);
        assert_eq!(scope.events.len(), scope.polls * 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lifecycle_telemetry_preserves_events_errors_and_repeat_guard() {
        for (args, repeat, expected_events, expected_error) in [
            (
                vec!["sce", "help"],
                false,
                vec![
                    "sce.app.start",
                    "sce.command.raw_args",
                    "sce.command.parsed",
                    "sce.command.dispatch_start",
                    "sce.command.dispatch_end",
                    "sce.command.completed",
                ],
                None,
            ),
            (
                vec!["sce", "--invalid-boundary-option"],
                false,
                vec!["sce.app.start", "sce.command.raw_args"],
                Some(services::error::FailureClass::Parse),
            ),
            (
                vec![
                    "sce",
                    "config",
                    "validate",
                    "--config",
                    "/nonexistent-sce-boundary-config.json",
                ],
                false,
                vec![
                    "sce.app.start",
                    "sce.command.raw_args",
                    "sce.command.parsed",
                    "sce.command.dispatch_start",
                ],
                Some(services::error::FailureClass::Runtime),
            ),
            (
                vec!["sce", "help"],
                true,
                vec![
                    "sce.app.start",
                    "sce.command.raw_args",
                    "sce.command.parsed",
                    "sce.command.dispatch_start",
                    "sce.command.dispatch_end",
                    "sce.command.completed",
                    "sce.app.start",
                ],
                Some(services::error::FailureClass::Runtime),
            ),
        ] {
            let scope = Arc::new(Mutex::new(RecordingScope::default()));
            let logger = RecordingLogger(scope.clone());
            let telemetry = RecordingTelemetry {
                scope: scope.clone(),
                repeat,
            };
            let fs = services::capabilities::StdFsOps;
            let git = services::capabilities::ProcessGitOps;
            let context = AppContext::new(&logger, &telemetry, &fs, &git, None);
            let result = Box::pin(run_command_lifecycle_with_context(
                args.into_iter().map(String::from),
                &services::command_registry::CommandRegistry::default(),
                &context,
                &mut Vec::new(),
            ))
            .await;
            if let Some(class) = expected_error {
                let error = result.unwrap_err();
                assert_eq!(error.class(), class);
                if repeat {
                    assert_eq!(error.to_string(), REPEATED_COMMAND_DISPATCH_ERROR);
                }
            } else {
                assert_eq!(result.unwrap(), services::help::help_text());
            }
            let scope = scope.lock().unwrap();
            assert!(!scope.active);
            let events = scope
                .events
                .iter()
                .filter(|event| event.starts_with("sce."))
                .map(String::as_str)
                .collect::<Vec<_>>();
            assert_eq!(events, expected_events);
        }
    }

    #[test]
    fn app_output_boundary_uses_isolated_process() {
        let sandbox = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::tests::isolated_app_output_boundary",
                "--nocapture",
            ])
            .current_dir(sandbox.path())
            .env("SCE_APP_BOUNDARY_CHILD", "1")
            .env("HOME", sandbox.path())
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_STATE_HOME", sandbox.path().join("state"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("NO_COLOR", "1")
            .env_remove("SCE_CONFIG_FILE")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn isolated_app_output_boundary() {
        if std::env::var_os("SCE_APP_BOUNDARY_CHILD").is_none() {
            return;
        }
        for command in ["help", "unknown-boundary-command", "version"] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = run_with_dependency_check_and_streams(
                ["sce", command].map(String::from),
                || {
                    assert_eq!(
                        tokio::runtime::Handle::current().runtime_flavor(),
                        tokio::runtime::RuntimeFlavor::MultiThread
                    );
                    Ok(())
                },
                &mut stdout,
                &mut stderr,
            )
            .await;
            assert_eq!(code, ExitCode::SUCCESS);
            let expected = if command == "version" {
                format!(
                    "shared-context-engineering {} ({})\n",
                    services::version::PACKAGE_VERSION,
                    option_env!("SCE_GIT_COMMIT").unwrap_or("unknown")
                )
            } else {
                format!("{}\n", services::help::help_text())
            };
            assert_eq!(stdout, expected.as_bytes());
            assert_eq!(stderr, b"");
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_dependency_check_and_streams(
            ["sce", "help"].map(String::from),
            || anyhow::bail!("boundary dependency unavailable"),
            &mut stdout,
            &mut stderr,
        )
        .await;
        assert_eq!(code, ExitCode::from(5));
        assert_eq!(stdout, b"");
        assert_eq!(stderr, b"Error [SCE-ERR-DEPENDENCY]: Failed to initialize dependency checks: boundary dependency unavailable Try: verify required runtime dependencies and environment setup, then retry.\n");

        stdout.clear();
        stderr.clear();
        let missing_config = std::env::current_dir().unwrap().join("missing-config.json");
        let code = Box::pin(run_with_dependency_check_and_streams(
            [
                "sce",
                "config",
                "validate",
                "--config",
                missing_config.to_str().unwrap(),
            ]
            .map(String::from),
            || Ok(()),
            &mut stdout,
            &mut stderr,
        ))
        .await;
        assert_eq!(code, ExitCode::from(4));
        assert_eq!(stdout, b"");
        assert_eq!(
            stderr,
            b"An unexpected error occurred. Check the log files for more details.\n"
        );
    }
}
