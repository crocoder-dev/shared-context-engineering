//! Best-effort launcher for one-shot automatic Agent Trace synchronization.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

use crate::services::app_support;
use crate::services::error::{AutomaticSyncFailureKind, CliError, UserError};
use crate::services::sync::{AUTOMATIC_SYNC_INVOCATION_ENV, AUTOMATIC_SYNC_INVOCATION_VALUE};

const SYNC_ARGS: &[&str] = &["sync", "--format", "json"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StdioMode {
    Null,
    Inherit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AutoSyncCommand {
    executable: PathBuf,
    args: Vec<String>,
    current_dir: PathBuf,
    stdin: StdioMode,
    stdout: StdioMode,
    stderr: StdioMode,
    environment: Vec<(String, String)>,
}

impl AutoSyncCommand {
    fn new(executable: PathBuf, repository_root: &Path) -> Self {
        Self {
            executable,
            args: SYNC_ARGS.iter().map(|arg| (*arg).to_string()).collect(),
            current_dir: repository_root.to_path_buf(),
            stdin: StdioMode::Null,
            stdout: StdioMode::Null,
            stderr: StdioMode::Inherit,
            environment: vec![(
                AUTOMATIC_SYNC_INVOCATION_ENV.to_string(),
                AUTOMATIC_SYNC_INVOCATION_VALUE.to_string(),
            )],
        }
    }
}

#[derive(Debug)]
enum AutoSyncLaunchError {
    CurrentExecutable(io::Error),
    Spawn(io::Error),
    Wait(io::Error),
}

impl std::fmt::Display for AutoSyncLaunchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CurrentExecutable(error) => {
                write!(formatter, "failed to resolve current executable: {error}")
            }
            Self::Spawn(error) => write!(formatter, "failed to spawn automatic sync: {error}"),
            Self::Wait(error) => write!(formatter, "failed to wait for automatic sync: {error}"),
        }
    }
}

impl std::error::Error for AutoSyncLaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CurrentExecutable(error) | Self::Spawn(error) | Self::Wait(error) => Some(error),
        }
    }
}

struct AutoSyncChild {
    wait: Option<Box<dyn FnOnce() -> io::Result<ExitStatus>>>,
}

impl AutoSyncChild {
    fn from_child(mut child: Child) -> Self {
        Self {
            wait: Some(Box::new(move || child.wait())),
        }
    }

    fn wait(mut self) -> io::Result<ExitStatus> {
        (self
            .wait
            .take()
            .expect("automatic sync child wait callback must be present"))()
    }
}

fn launcher_failure_diagnostic(error: AutoSyncLaunchError) -> CliError {
    let reason = error.to_string();
    CliError::user_with_source(
        UserError::AutomaticSyncFailed {
            failure_kind: AutomaticSyncFailureKind::Runtime,
            reason,
        },
        error,
    )
}

/// Launches the current executable to synchronize the repository and waits for
/// the one-shot child to reach terminal completion. Launcher failures are
/// reported on stderr but remain fail-open to the post-commit caller.
pub fn launch(repository_root: &Path) {
    if let Err(error) = launch_with(repository_root, std::env::current_exe, spawn_command) {
        let diagnostic = launcher_failure_diagnostic(error);
        let mut stderr = io::stderr();
        app_support::write_error_diagnostic(&mut stderr, &diagnostic);
    }
}

fn launch_with<FCurrentExe, FSpawn>(
    repository_root: &Path,
    current_exe: FCurrentExe,
    spawn: FSpawn,
) -> Result<(), AutoSyncLaunchError>
where
    FCurrentExe: FnOnce() -> io::Result<PathBuf>,
    FSpawn: FnOnce(AutoSyncCommand) -> io::Result<AutoSyncChild>,
{
    let executable = current_exe().map_err(AutoSyncLaunchError::CurrentExecutable)?;

    let child = spawn(AutoSyncCommand::new(executable, repository_root))
        .map_err(AutoSyncLaunchError::Spawn)?;
    child.wait().map(|_| ()).map_err(AutoSyncLaunchError::Wait)
}

fn spawn_command(spec: AutoSyncCommand) -> io::Result<AutoSyncChild> {
    let mut command = Command::new(spec.executable);
    command
        .args(spec.args)
        .current_dir(spec.current_dir)
        .envs(spec.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());

    Ok(AutoSyncChild::from_child(command.spawn()?))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use super::{
        launch_with, AutoSyncChild, AutoSyncCommand, StdioMode, AUTOMATIC_SYNC_INVOCATION_ENV,
        AUTOMATIC_SYNC_INVOCATION_VALUE, SYNC_ARGS,
    };

    fn child_with_wait<F>(wait: F) -> AutoSyncChild
    where
        F: FnOnce() -> io::Result<std::process::ExitStatus> + 'static,
    {
        AutoSyncChild {
            wait: Some(Box::new(wait)),
        }
    }

    fn exit_status(success: bool) -> std::process::ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;

            std::process::ExitStatus::from_raw(i32::from(!success))
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;

            std::process::ExitStatus::from_raw(u32::from(!success))
        }
    }

    #[test]
    fn launch_builds_the_expected_command() {
        let captured = Rc::new(RefCell::new(None));
        let captured_by_spawn = Rc::clone(&captured);

        let launched = launch_with(
            Path::new("/repo/root"),
            || Ok(PathBuf::from("/usr/local/bin/sce")),
            move |command: AutoSyncCommand| {
                *captured_by_spawn.borrow_mut() = Some(command);
                Ok(child_with_wait(|| Ok(exit_status(true))))
            },
        );

        assert!(launched.is_ok());
        assert_eq!(
            captured.borrow().clone(),
            Some(AutoSyncCommand {
                executable: PathBuf::from("/usr/local/bin/sce"),
                args: SYNC_ARGS.iter().map(|arg| (*arg).to_string()).collect(),
                current_dir: PathBuf::from("/repo/root"),
                stdin: StdioMode::Null,
                stdout: StdioMode::Null,
                stderr: StdioMode::Inherit,
                environment: vec![(
                    AUTOMATIC_SYNC_INVOCATION_ENV.to_string(),
                    AUTOMATIC_SYNC_INVOCATION_VALUE.to_string(),
                )],
            })
        );
    }

    #[test]
    fn current_executable_failure_is_fail_open() {
        let spawn_called = Rc::new(RefCell::new(false));
        let spawn_called_by_spawn = Rc::clone(&spawn_called);

        let launched = launch_with(
            Path::new("/repo/root"),
            || Err(io::Error::other("current executable unavailable")),
            move |_| -> io::Result<AutoSyncChild> {
                *spawn_called_by_spawn.borrow_mut() = true;
                Ok(child_with_wait(|| Ok(exit_status(true))))
            },
        );

        assert!(launched.is_err());
        assert!(!*spawn_called.borrow());
    }

    #[test]
    fn spawn_failure_is_fail_open() {
        let launched = launch_with(
            Path::new("/repo/root"),
            || Ok(PathBuf::from("/usr/local/bin/sce")),
            |_| Err(io::Error::other("spawn unavailable")),
        );

        assert!(launched.is_err());
    }
}
