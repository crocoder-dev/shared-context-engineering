use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crate::services;
use services::app_support::{self, RunOutcome};
use services::error::CliError;
use services::managed_telemetry::trace_export::ManagedSpanKind;
use services::managed_telemetry::RuntimeTelemetry;
use services::observability::traits::{
    Logger as LoggerTrait, NoopTelemetry, Telemetry as TelemetryTrait,
};

const MIN_WORKER_THREADS: usize = 2;
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
const REPEATED_COMMAND_DISPATCH_ERROR: &str =
    "Command lifecycle telemetry attempted to execute command dispatch more than once";
const CLAUDE_ONLY_HOOK_SUBCOMMANDS: [&str; 2] = ["claude-mutation-scope", "claude-model-state"];
const SHARED_CLAUDE_HOOK_SUBCOMMANDS: [&str; 2] = ["diff-trace", "conversation-trace"];
const CLAUDE_HOOK_CALLER_MARKERS: [&str; 2] = ["CLAUDECODE", "CLAUDE_PROJECT_DIR"];

struct StartupContext {
    observability_config: services::config::ResolvedObservabilityRuntimeConfig,
    startup_diagnostic: Option<String>,
}

struct AppRuntime {
    logger: services::observability::Logger,
    telemetry: RuntimeTelemetry,
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
    RuntimeTelemetry,
    services::capabilities::StdFsOps,
    services::capabilities::ProcessGitOps,
>;

pub(crate) trait HasLogger {
    type Logger: LoggerTrait;

    fn logger(&self) -> &Self::Logger;
}

pub(crate) trait HasTelemetry {
    type Telemetry: TelemetryTrait;

    fn telemetry(&self) -> &Self::Telemetry;
}

pub(crate) trait HasFs {
    type Fs: services::capabilities::FsOps;
}

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

    fn telemetry(&self) -> &T {
        HasTelemetry::telemetry(self)
    }

    pub(crate) fn with_repo_root(&self, repo_root: impl Into<PathBuf>) -> Self {
        Self {
            logger: self.logger,
            telemetry: self.telemetry,
            fs: self.fs,
            git: self.git,
            repo_root: Some(repo_root.into()),
        }
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

pub fn run_process<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    let runtime = match build_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            return app_support::render_run_outcome(
                RunOutcome::<services::observability::Logger> {
                    result: Err(CliError::runtime(anyhow::Error::msg(format!(
                        "Failed to start async runtime: {error}"
                    )))),
                    logger: None,
                    startup_diagnostic: None,
                },
                &mut io::stdout(),
                &mut io::stderr(),
            );
        }
    };
    let exit_code = runtime.block_on(run(args));
    finish_runtime(runtime);
    exit_code
}

pub(crate) fn build_runtime() -> io::Result<tokio::runtime::Runtime> {
    let worker_threads = std::thread::available_parallelism().map_or(MIN_WORKER_THREADS, |count| {
        count.get().max(MIN_WORKER_THREADS)
    });
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
}

pub(crate) fn finish_runtime(runtime: tokio::runtime::Runtime) {
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_GRACE);
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
    let args = args.into_iter().collect::<Vec<_>>();
    let span_kind = managed_span_kind(&args, is_claude_hook_caller());
    let startup = perform_dependency_check(dependency_check)
        .and_then(|()| build_startup_context())
        .and_then(initialize_runtime);
    let mut runtime = match startup {
        Ok(runtime) => runtime,
        Err(error) => {
            return app_support::render_run_outcome(
                RunOutcome::<services::observability::Logger> {
                    result: Err(error),
                    logger: None,
                    startup_diagnostic: None,
                },
                stdout,
                stderr,
            );
        }
    };
    runtime.telemetry = RuntimeTelemetry::resolve(span_kind, &runtime.logger).await;
    let rendered = run_command_lifecycle(args, &runtime, stdout, stderr).await;
    runtime.telemetry.shutdown().await;
    app_support::exit_code_for(&rendered)
}

fn managed_span_kind(args: &[String], claude_hook_caller: bool) -> Option<ManagedSpanKind> {
    let mut words = args
        .iter()
        .skip(1)
        .map(String::as_str)
        .filter(|arg| !arg.starts_with('-'));
    let claude_hook = match (words.next(), words.next()) {
        (Some(first), Some(subcommand)) if first == services::hooks::NAME => {
            CLAUDE_ONLY_HOOK_SUBCOMMANDS.contains(&subcommand)
                || (claude_hook_caller && SHARED_CLAUDE_HOOK_SUBCOMMANDS.contains(&subcommand))
        }
        (Some(first), _) if first == services::hooks::NAME => false,
        (Some(first), Some("bash")) if first == services::bash_policy::NAME => claude_hook_caller,
        (Some(first), Some("telemetry-headers")) if first == services::auth_command::NAME => false,
        _ => return Some(ManagedSpanKind::Command),
    };
    claude_hook.then_some(ManagedSpanKind::ClaudeHook)
}

fn is_claude_hook_caller() -> bool {
    CLAUDE_HOOK_CALLER_MARKERS
        .iter()
        .all(|marker| std::env::var_os(marker).is_some())
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
        telemetry: RuntimeTelemetry::Noop(NoopTelemetry),
        fs: services::capabilities::StdFsOps,
        git: services::capabilities::ProcessGitOps,
        registry: services::command_registry::build_default_registry(),
        startup_diagnostic: startup.startup_diagnostic,
    })
}

async fn run_command_lifecycle<I, StdoutW, StderrW>(
    args: I,
    runtime: &AppRuntime,
    stdout: &mut StdoutW,
    stderr: &mut StderrW,
) -> Result<String, CliError>
where
    I: IntoIterator<Item = String>,
    StdoutW: Write,
    StderrW: Write,
{
    let context = runtime.context();
    run_command_lifecycle_with_context(
        args,
        &runtime.registry,
        &context,
        runtime.startup_diagnostic.as_deref(),
        stdout,
        stderr,
    )
    .await
}

async fn run_command_lifecycle_with_context<I, L, T, StdoutW, StderrW>(
    args: I,
    registry: &services::command_registry::CommandRegistry,
    context: &AppContext<'_, L, T>,
    startup_diagnostic: Option<&str>,
    stdout: &mut StdoutW,
    stderr: &mut StderrW,
) -> Result<String, CliError>
where
    I: IntoIterator<Item = String>,
    L: LoggerTrait,
    T: TelemetryTrait,
    StdoutW: Write,
    StderrW: Write,
{
    let mut args = Some(args.into_iter().collect::<Vec<_>>());
    let mut streams = Some((stdout, stderr));
    context
        .telemetry()
        .with_default_subscriber(&mut || {
            let command_inputs = args.take().zip(streams.take());
            async move {
                context.logger().info(
                    "sce.app.start",
                    "Starting command dispatch",
                    &[("component", services::observability::NAME)],
                    None,
                );
                let Some((command_args, (stdout, stderr))) = command_inputs else {
                    return Err(repeated_dispatch_error());
                };
                let result = dispatch_command(command_args, registry, context, stderr).await;
                app_support::render_command_result(
                    result,
                    Some(context.logger()),
                    startup_diagnostic,
                    stdout,
                    stderr,
                )
                .map(|()| String::new())
            }
        })
        .await
}

fn repeated_dispatch_error() -> CliError {
    CliError::runtime(anyhow::Error::msg(REPEATED_COMMAND_DISPATCH_ERROR))
}

async fn dispatch_command<L, T, StderrW>(
    args: Vec<String>,
    registry: &services::command_registry::CommandRegistry,
    context: &AppContext<'_, L, T>,
    stderr: &mut StderrW,
) -> Result<String, CliError>
where
    L: LoggerTrait,
    T: TelemetryTrait,
    StderrW: Write,
{
    let command = parse_command_phase(args, registry, context)?;
    services::managed_telemetry::trace_export::record_command_name(&command.name());
    app_support::execute_command_phase(&command, context, stderr).await
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
mod doctor_reconciliation_cli_tests;

#[cfg(test)]
pub(crate) mod tests {
    use std::future::{poll_fn, Future};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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
        fn log_cli_error(&self, error: &CliError, _: Option<&str>) {
            let mut scope = self.0.lock().unwrap();
            assert!(
                scope.active,
                "final error logging must occur inside a telemetry poll"
            );
            scope.events.push(format!("final_error:{}", error.code()));
        }
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
            let mut stdout = Vec::new();
            let result = Box::pin(run_command_lifecycle_with_context(
                args.into_iter().map(String::from),
                &services::command_registry::CommandRegistry::default(),
                &context,
                None,
                &mut stdout,
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
                result.unwrap();
                assert_eq!(
                    stdout,
                    format!("{}\n", services::help::help_text()).as_bytes()
                );
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

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
    }

    async fn run_recorded_lifecycle<StdoutW: Write, StderrW: Write>(
        args: &[&str],
        stdout: &mut StdoutW,
        stderr: &mut StderrW,
    ) -> (Result<String, CliError>, Vec<String>) {
        let scope = Arc::new(Mutex::new(RecordingScope::default()));
        let logger = RecordingLogger(scope.clone());
        let telemetry = RecordingTelemetry {
            scope: scope.clone(),
            repeat: false,
        };
        let fs = services::capabilities::StdFsOps;
        let git = services::capabilities::ProcessGitOps;
        let context = AppContext::new(&logger, &telemetry, &fs, &git, None);
        let result = Box::pin(run_command_lifecycle_with_context(
            args.iter().map(|arg| (*arg).to_string()),
            &services::command_registry::CommandRegistry::default(),
            &context,
            None,
            stdout,
            stderr,
        ))
        .await;
        let events = scope
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| event.starts_with("final_error:"))
            .cloned()
            .collect();
        (result, events)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn final_command_outcome_and_error_log_are_produced_inside_telemetry_scope() {
        let missing_config = [
            "sce",
            "config",
            "validate",
            "--config",
            "/nonexistent-sce-outcome-config.json",
        ];

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (result, events) =
            run_recorded_lifecycle(&["sce", "help"], &mut stdout, &mut stderr).await;
        assert!(result.is_ok());
        assert!(events.is_empty());
        assert_eq!(app_support::exit_code_for(&result), ExitCode::SUCCESS);
        assert_eq!(
            stdout,
            format!("{}\n", services::help::help_text()).as_bytes()
        );
        assert!(stderr.is_empty());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (result, events) = run_recorded_lifecycle(
            &["sce", "--invalid-outcome-option"],
            &mut stdout,
            &mut stderr,
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(error.class(), services::error::FailureClass::Parse);
        assert_eq!(events, [format!("final_error:{}", error.code())]);
        assert!(stdout.is_empty());
        assert!(!stderr.is_empty());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (result, events) =
            run_recorded_lifecycle(&missing_config, &mut stdout, &mut stderr).await;
        let error = result.unwrap_err();
        assert_eq!(error.class(), services::error::FailureClass::Runtime);
        assert_eq!(events, [format!("final_error:{}", error.code())]);
        assert_eq!(
            app_support::exit_code_for(&Err::<(), _>(error)),
            ExitCode::from(4)
        );
        assert!(stdout.is_empty());
        assert!(!stderr.is_empty());

        let mut stderr = Vec::new();
        let (result, events) =
            run_recorded_lifecycle(&["sce", "help"], &mut FailingWriter, &mut stderr).await;
        let error = result.unwrap_err();
        assert_eq!(error.class(), services::error::FailureClass::Runtime);
        assert_eq!(events, [format!("final_error:{}", error.code())]);
        assert!(
            String::from_utf8_lossy(&stderr).contains("Failed to write command output to stdout")
        );

        let mut stdout = Vec::new();
        let (result, events) =
            run_recorded_lifecycle(&missing_config, &mut stdout, &mut FailingWriter).await;
        let error = result.unwrap_err();
        assert_eq!(error.class(), services::error::FailureClass::Runtime);
        assert_eq!(events, [format!("final_error:{}", error.code())]);
        assert_eq!(
            app_support::exit_code_for(&Err::<(), _>(error)),
            ExitCode::from(4)
        );
        assert!(stdout.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn managed_export_marks_failed_output_and_exports_final_error_log() {
        use services::managed_telemetry::trace_export::tests::{
            capture_requests, loopback_telemetry,
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = tokio::spawn(capture_requests(listener, "HTTP/1.1 200 OK", 2));
        let telemetry = loopback_telemetry(address, "bao-token-value");
        let logger = services::observability::Logger::without_log_file();
        let fs = services::capabilities::StdFsOps;
        let git = services::capabilities::ProcessGitOps;
        let context = AppContext::new(&logger, &telemetry, &fs, &git, None);
        let mut stderr = Vec::new();

        let result = Box::pin(run_command_lifecycle_with_context(
            ["sce", "help"].map(String::from),
            &services::command_registry::CommandRegistry::default(),
            &context,
            None,
            &mut FailingWriter,
            &mut stderr,
        ))
        .await;
        let error = result.unwrap_err();
        assert_eq!(error.class(), services::error::FailureClass::Runtime);
        telemetry.shutdown().await;

        let requests = receiver.await.unwrap();
        let body_for = |path: &str| {
            requests
                .iter()
                .find(|request| request.head.to_ascii_lowercase().starts_with(path))
                .map(|request| String::from_utf8_lossy(&request.body).to_string())
                .unwrap_or_default()
        };
        let trace = body_for("post /v1/traces ");
        let log = body_for("post /v1/logs ");
        assert!(trace.contains("failure"), "trace outcome must be failure");
        assert!(!trace.contains("success"));
        assert!(
            log.contains(&format!("sce.error.{}", error.code())),
            "{:?} {log:?}",
            requests
                .iter()
                .map(|r| r.head.lines().next().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        );
        assert!(!log.contains("bao-token-value"));
    }

    const FAULT_CHILD_ENV: &str = "SCE_TELEMETRY_FAULT_CHILD";
    const CHILD_KILL_AFTER: Duration = Duration::from_secs(30);
    const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(50);

    pub(crate) fn run_fault_child(test_name: &str) -> (Duration, std::process::Output) {
        let started = std::time::Instant::now();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
            .env(FAULT_CHILD_ENV, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > CHILD_KILL_AFTER {
                let _ = child.kill();
                panic!("{test_name} child process did not exit within {CHILD_KILL_AFTER:?}");
            }
            std::thread::sleep(CHILD_POLL_INTERVAL);
        }
        let elapsed = started.elapsed();
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        (elapsed, output)
    }

    pub(crate) fn is_fault_child() -> bool {
        std::env::var_os(FAULT_CHILD_ENV).is_some()
    }

    async fn stalled_keychain_lookup() -> Result<
        Option<services::telemetry_credential_storage::TelemetryCredential>,
        services::token_storage::TokenStorageError,
    > {
        let _ = tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_hours(1))).await;
        Ok(None)
    }

    async fn stalled_blocking_database_lookup() -> Result<
        Option<services::telemetry_credential_storage::TelemetryCredential>,
        services::token_storage::TokenStorageError,
    > {
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_hours(1));
        Ok(None)
    }

    #[test]
    fn stalled_startup_credential_lookup_cannot_extend_process_lifetime() {
        let (elapsed, _) =
            run_fault_child("app::tests::isolated_stalled_startup_credential_lookup");
        let bound = services::managed_telemetry::CREDENTIAL_LOOKUP_DEADLINE * 2
            + RUNTIME_SHUTDOWN_GRACE
            + Duration::from_secs(3);
        assert!(elapsed < bound, "child exited after {elapsed:?}");
    }

    #[test]
    fn isolated_stalled_startup_credential_lookup() {
        if !is_fault_child() {
            return;
        }
        let runtime = build_runtime().unwrap();
        runtime.block_on(async {
            let (keychain, fallback) =
                RuntimeTelemetry::resolve_with(stalled_keychain_lookup(), ManagedSpanKind::Command)
                    .await;
            assert!(matches!(keychain, RuntimeTelemetry::Noop(_)));
            assert_eq!(
                fallback.map(|fallback| fallback.reason),
                Some(services::managed_telemetry::TelemetryFallbackReason::LookupTimeout)
            );
            let (database, fallback) = RuntimeTelemetry::resolve_with(
                stalled_blocking_database_lookup(),
                ManagedSpanKind::ClaudeHook,
            )
            .await;
            assert!(matches!(database, RuntimeTelemetry::Noop(_)));
            assert_eq!(
                fallback.map(|fallback| fallback.reason),
                Some(services::managed_telemetry::TelemetryFallbackReason::LookupTimeout)
            );
        });
        finish_runtime(runtime);
    }

    #[test]
    fn stalled_telemetry_headers_lookup_fails_closed_within_deadline() {
        let (elapsed, _) = run_fault_child("app::tests::isolated_stalled_telemetry_headers_lookup");
        let bound = services::managed_telemetry::CREDENTIAL_LOOKUP_DEADLINE
            + RUNTIME_SHUTDOWN_GRACE
            + Duration::from_secs(3);
        assert!(elapsed < bound, "child exited after {elapsed:?}");
    }

    #[test]
    fn isolated_stalled_telemetry_headers_lookup() {
        if !is_fault_child() {
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        let runtime = build_runtime().unwrap();
        let result = runtime.block_on(services::auth_command::telemetry_headers_with(
            Some(repo.path().to_path_buf()),
            false,
            |_: &std::path::Path| Some("repository-a".to_string()),
            |_: String| stalled_keychain_lookup(),
        ));
        finish_runtime(runtime);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = app_support::render_run_outcome(
            RunOutcome::<services::observability::traits::NoopLogger> {
                result,
                logger: None,
                startup_diagnostic: None,
            },
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(code, ExitCode::from(4));
        assert!(stdout.is_empty());
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("Telemetry credential is unavailable."));
        assert!(!stderr.contains("Bearer"));
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

    #[test]
    fn only_claude_hook_invocations_are_admitted_to_managed_hook_telemetry() {
        let kind = |args: &[&str], claude_caller: bool| {
            let args = args.iter().map(ToString::to_string).collect::<Vec<_>>();
            managed_span_kind(&args, claude_caller)
        };
        let claude_hook = Some(ManagedSpanKind::ClaudeHook);
        let command = Some(ManagedSpanKind::Command);
        for caller in [false, true] {
            assert_eq!(
                kind(&["sce", "hooks", "claude-mutation-scope"], caller),
                claude_hook
            );
            assert_eq!(
                kind(&["sce", "hooks", "claude-model-state"], caller),
                claude_hook
            );
            assert_eq!(kind(&["sce", "hooks", "pre-commit"], caller), None);
            assert_eq!(kind(&["sce", "--verbose", "hooks", "codex"], caller), None);
            assert_eq!(
                kind(&["sce", "hooks", "codex-mutation-scope"], caller),
                None
            );
            assert_eq!(
                kind(&["sce", "hooks", "opencode-mutation-scope"], caller),
                None
            );
            assert_eq!(kind(&["sce", "hooks", "pi-mutation-scope"], caller), None);
            assert_eq!(kind(&["sce", "hooks"], caller), None);
            assert_eq!(kind(&["sce", "auth", "telemetry-headers"], caller), None);
            assert_eq!(kind(&["sce", "auth", "login"], caller), command);
            assert_eq!(kind(&["sce", "config", "show"], caller), command);
            assert_eq!(kind(&["sce", "doctor"], caller), command);
        }
        for shared in [
            &["sce", "hooks", "diff-trace"][..],
            &["sce", "hooks", "conversation-trace"],
            &["sce", "policy", "bash"],
        ] {
            assert_eq!(kind(shared, true), claude_hook);
            assert_eq!(kind(shared, false), None);
        }
    }

    #[tokio::test]
    async fn disallowed_managed_telemetry_resolves_to_noop_without_credential_lookup() {
        let telemetry =
            RuntimeTelemetry::resolve(None, &services::observability::traits::NoopLogger).await;
        assert!(matches!(telemetry, RuntimeTelemetry::Noop(_)));
    }
}
