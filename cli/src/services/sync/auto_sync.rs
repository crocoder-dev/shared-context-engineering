//! Best-effort launcher for one-shot automatic Agent Trace synchronization.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SYNC_ARGS: &[&str] = &["sync", "--format", "json"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StdioMode {
    Null,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AutoSyncCommand {
    executable: PathBuf,
    args: Vec<String>,
    current_dir: PathBuf,
    stdin: StdioMode,
    stdout: StdioMode,
    stderr: StdioMode,
}

impl AutoSyncCommand {
    fn new(executable: PathBuf, repository_root: &Path) -> Self {
        Self {
            executable,
            args: SYNC_ARGS.iter().map(|arg| (*arg).to_string()).collect(),
            current_dir: repository_root.to_path_buf(),
            stdin: StdioMode::Null,
            stdout: StdioMode::Null,
            stderr: StdioMode::Null,
        }
    }
}

/// Launches the current executable to synchronize the repository in the
/// background. Launcher failures are intentionally ignored by the caller.
pub fn launch(repository_root: &Path) {
    let _ = launch_with(repository_root, std::env::current_exe, spawn_command);
}

fn launch_with<FCurrentExe, FSpawn>(
    repository_root: &Path,
    current_exe: FCurrentExe,
    spawn: FSpawn,
) -> bool
where
    FCurrentExe: FnOnce() -> io::Result<PathBuf>,
    FSpawn: FnOnce(AutoSyncCommand) -> io::Result<()>,
{
    let Ok(executable) = current_exe() else {
        return false;
    };

    spawn(AutoSyncCommand::new(executable, repository_root)).is_ok()
}

fn spawn_command(spec: AutoSyncCommand) -> io::Result<()> {
    let mut command = Command::new(spec.executable);
    command
        .args(spec.args)
        .current_dir(spec.current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Dropping Child does not wait for it; the spawned sync continues
    // independently of the post-commit caller.
    let _child = command.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use super::{launch_with, AutoSyncCommand, StdioMode, SYNC_ARGS};

    #[test]
    fn launch_builds_the_expected_detached_command() {
        let captured = Rc::new(RefCell::new(None));
        let captured_by_spawn = Rc::clone(&captured);

        let launched = launch_with(
            Path::new("/repo/root"),
            || Ok(PathBuf::from("/usr/local/bin/sce")),
            move |command: AutoSyncCommand| {
                *captured_by_spawn.borrow_mut() = Some(command);
                Ok(())
            },
        );

        assert!(launched);
        assert_eq!(
            captured.borrow().clone(),
            Some(AutoSyncCommand {
                executable: PathBuf::from("/usr/local/bin/sce"),
                args: SYNC_ARGS.iter().map(|arg| (*arg).to_string()).collect(),
                current_dir: PathBuf::from("/repo/root"),
                stdin: StdioMode::Null,
                stdout: StdioMode::Null,
                stderr: StdioMode::Null,
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
            move |_| {
                *spawn_called_by_spawn.borrow_mut() = true;
                Ok(())
            },
        );

        assert!(!launched);
        assert!(!*spawn_called.borrow());
    }

    #[test]
    fn spawn_failure_is_fail_open() {
        let launched = launch_with(
            Path::new("/repo/root"),
            || Ok(PathBuf::from("/usr/local/bin/sce")),
            |_| Err(io::Error::other("spawn unavailable")),
        );

        assert!(!launched);
    }
}
