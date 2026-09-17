#[cfg(not(unix))]
use std::path::Path;
#[cfg(not(unix))]
use std::sync::mpsc;

#[cfg(not(unix))]
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

use super::coordinator::CoordinateError;
use super::protected_worktree::ProtectedWorktreeError;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GuardRequest {
    pub command: String,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GuardEvent {
    Armed,
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GuardOutcome {
    pub exit_code: Option<i32>,
    pub marker_clear_failed: bool,
}

#[derive(Debug)]
pub(crate) enum GuardError {
    Acquire(ProtectedWorktreeError),
    Spawn(std::io::Error),
    Wait(std::io::Error),
    Finish(CoordinateError),
    UnsupportedPlatform,
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardError::Acquire(source) => write!(f, "{source}"),
            GuardError::Spawn(source) => write!(f, "failed to spawn the guarded shell: {source}"),
            GuardError::Wait(source) => {
                write!(f, "failed to wait for the guarded shell: {source}")
            }
            GuardError::Finish(source) => write!(f, "{source}"),
            GuardError::UnsupportedPlatform => {
                write!(f, "the external-mutation guard is Unix-only")
            }
        }
    }
}

impl std::error::Error for GuardError {}

#[cfg(not(unix))]
pub(crate) fn run_external_mutation_guard<P, E>(
    _repository_root: &Path,
    _request: &GuardRequest,
    _open_db: P,
    _on_event: E,
    _cancel_rx: &mpsc::Receiver<()>,
) -> Result<GuardOutcome, GuardError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    E: FnMut(GuardEvent),
{
    Err(GuardError::UnsupportedPlatform)
}

#[cfg(unix)]
mod unix_impl {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    use std::os::unix::io::RawFd;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

    use super::super::coordinator::coordinate_on_held_worktree;
    use super::super::protected_worktree::ProtectedWorktree;
    use super::super::RuntimeBoundary;
    use super::{GuardError, GuardEvent, GuardOutcome, GuardRequest};

    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const SIGTERM: i32 = 15;

    mod raw {
        unsafe extern "C" {
            pub(super) fn dup(fd: i32) -> i32;
            pub(super) fn kill(pid: i32, sig: i32) -> i32;
        }
    }

    pub(super) fn spawn_guarded_shell(
        repository_root: &Path,
        request: &GuardRequest,
        lock_fd: RawFd,
    ) -> std::io::Result<Child> {
        let duplicated_lock_fd = unsafe { raw::dup(lock_fd) };
        if duplicated_lock_fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let duplicated_lock_fd_guard =
            unsafe { std::fs::File::from_raw_fd(duplicated_lock_fd) };

        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(&request.command)
            .current_dir(repository_root)
            .envs(request.env.iter().cloned())
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let result = command.spawn();
        drop(duplicated_lock_fd_guard);
        result
    }

    fn spawn_pipe_reader<R>(
        mut pipe: R,
        wrap: fn(Vec<u8>) -> GuardEvent,
        tx: mpsc::Sender<GuardEvent>,
    ) -> thread::JoinHandle<()>
    where
        R: Read + Send + 'static,
    {
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if tx.send(wrap(buffer[..count].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        })
    }

    pub(crate) fn run_external_mutation_guard<P, E>(
        repository_root: &Path,
        request: &GuardRequest,
        open_db: P,
        mut on_event: E,
        cancel_rx: &mpsc::Receiver<()>,
    ) -> Result<GuardOutcome, GuardError>
    where
        P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
        E: FnMut(GuardEvent),
    {
        let protected = ProtectedWorktree::acquire(repository_root).map_err(GuardError::Acquire)?;
        on_event(GuardEvent::Armed);

        let lock_fd = protected.lock_raw_fd();
        let mut child =
            spawn_guarded_shell(repository_root, request, lock_fd).map_err(GuardError::Spawn)?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (stream_tx, stream_rx) = mpsc::channel::<GuardEvent>();
        let stdout_reader =
            stdout.map(|pipe| spawn_pipe_reader(pipe, GuardEvent::Stdout, stream_tx.clone()));
        let stderr_reader = stderr.map(|pipe| spawn_pipe_reader(pipe, GuardEvent::Stderr, stream_tx));

        let pid = child.id();
        let (wait_tx, wait_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let status = child.wait();
            let _ = wait_tx.send(status);
        });

        let mut cancel_signaled = false;
        let wait_result = loop {
            while let Ok(event) = stream_rx.try_recv() {
                on_event(event);
            }

            match wait_rx.try_recv() {
                Ok(status) => break status,
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    unreachable!("the waiter thread always sends exactly once before exiting")
                }
            }

            if !cancel_signaled && cancel_rx.try_recv().is_ok() {
                cancel_signaled = true;
                #[allow(clippy::cast_possible_wrap)]
                let group = -(pid as i32);
                unsafe {
                    raw::kill(group, SIGTERM);
                }
            }

            thread::sleep(POLL_INTERVAL);
        };

        while let Ok(event) = stream_rx.try_recv() {
            on_event(event);
        }
        if let Some(reader) = stdout_reader {
            let _ = reader.join();
        }
        if let Some(reader) = stderr_reader {
            let _ = reader.join();
        }
        let _ = waiter.join();

        let status = wait_result.map_err(GuardError::Wait)?;

        let worktree_id = protected.worktree_id().clone();
        coordinate_on_held_worktree(
            repository_root,
            &worktree_id,
            &RuntimeBoundary::Flush,
            open_db,
            true,
        )
        .map_err(GuardError::Finish)?;

        let marker_clear_failed = protected.complete().is_err();

        Ok(GuardOutcome {
            exit_code: status.code(),
            marker_clear_failed,
        })
    }
}

#[cfg(unix)]
pub(crate) use unix_impl::run_external_mutation_guard;

#[cfg(all(unix, test))]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::mpsc;
    use std::sync::Mutex;
    use std::time::Duration;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
    };

    use super::super::protected_worktree::ProtectedWorktree;
    use super::super::resolve_git_dir;
    use super::super::worktree_lock::WorktreeLock;
    use super::unix_impl::spawn_guarded_shell;
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    struct TestRepo {
        _temp: tempfile::TempDir,
        root: PathBuf,
        state_root: PathBuf,
    }

    impl TestRepo {
        fn new(label: &str) -> Self {
            let temp = tempfile::Builder::new()
                .prefix(&format!("sce-external-mutation-guard-{label}-"))
                .tempdir()
                .expect("temp dir should be created");
            let root = temp.path().join("repo");
            fs::create_dir_all(&root).expect("repo dir should be created");
            git(&root, &["init", "-q"]);
            git(&root, &["config", "user.email", "test@example.invalid"]);
            git(&root, &["config", "user.name", "SCE Test"]);
            git(
                &root,
                &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
            );
            fs::write(root.join("file.txt"), "one\n").expect("seed file should write");
            git(&root, &["add", "-A"]);
            git(&root, &["commit", "-qm", "base"]);

            let state_root = temp.path().join("state");
            fs::create_dir_all(&state_root).expect("state root should be created");
            resolve_agent_trace_storage_at_state_root(
                &AgentTraceStorageContext {
                    repository_root: &root,
                    explicit_repository_id: None,
                    repository_remote: "origin",
                },
                &state_root,
            )
            .expect("state-root storage should initialize the repository DB");

            Self {
                _temp: temp,
                root,
                state_root,
            }
        }

        fn git_dir(&self) -> PathBuf {
            resolve_git_dir(&self.root).expect("git dir should resolve")
        }

        fn open_db(&self) -> anyhow::Result<RepositoryAgentTraceDb> {
            crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                &self.root,
                &self.state_root,
                "external-mutation-guard test assertions",
            )
        }
    }

    fn request(command: &str) -> GuardRequest {
        GuardRequest {
            command: command.to_string(),
            env: Vec::new(),
        }
    }

    #[test]
    fn a_concurrent_foreign_lock_attempt_times_out_while_the_guard_is_active() {
        let repo = TestRepo::new("foreign-contention");
        let (_cancel_tx, cancel_rx) = mpsc::channel();

        let root = repo.root.clone();
        let state_root = repo.state_root.clone();
        let handle = std::thread::spawn(move || {
            run_external_mutation_guard(
                &root,
                &request("sleep 1"),
                || {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        &root,
                        &state_root,
                        "external-mutation-guard test assertions",
                    )
                },
                |_event| {},
                &cancel_rx,
            )
        });

        std::thread::sleep(Duration::from_millis(200));

        let foreign = WorktreeLock::acquire(&repo.git_dir(), Duration::from_millis(200));
        assert!(
            foreign.is_err(),
            "a foreign lock attempt must fail closed while the guard holds the lock"
        );

        let outcome = handle
            .join()
            .expect("guard thread should not panic")
            .expect("the guard should reach its finish step");
        assert_eq!(outcome.exit_code, Some(0));

        WorktreeLock::acquire(&repo.git_dir(), Duration::from_millis(500))
            .expect("the lock must free once the guard's own finish step completes");
    }

    #[test]
    fn the_spawned_shells_parent_is_the_calling_process() {
        let repo = TestRepo::new("parent-pid");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let captured_stdout: Mutex<Vec<u8>> = Mutex::new(Vec::new());

        let outcome = run_external_mutation_guard(
            &repo.root,
            &request("echo $PPID"),
            || repo.open_db(),
            |event| {
                if let GuardEvent::Stdout(chunk) = event {
                    captured_stdout.lock().expect("stdout mutex").extend(chunk);
                }
            },
            &cancel_rx,
        )
        .expect("guard should succeed");
        assert_eq!(outcome.exit_code, Some(0));

        let reported = String::from_utf8(captured_stdout.into_inner().expect("stdout mutex"))
            .expect("$PPID output should be valid UTF-8");
        let ppid: u32 = reported
            .trim()
            .parse()
            .expect("$PPID should parse as an integer");
        assert_eq!(ppid, std::process::id());
    }

    unsafe extern "C" {
        fn close(fd: i32) -> i32;
    }

    #[test]
    fn a_supervisor_killed_without_unlocking_leaves_the_flock_held_by_the_spawned_shell() {
        let repo = TestRepo::new("kill-9-simulated");

        let protected = ProtectedWorktree::acquire(&repo.root).expect("acquire should succeed");
        let lock_fd = protected.lock_raw_fd();
        let mut child = spawn_guarded_shell(&repo.root, &request("sleep 1"), lock_fd)
            .expect("the guarded shell should spawn");

        unsafe {
            close(lock_fd);
        }
        std::mem::forget(protected);

        let foreign_while_shell_alive =
            WorktreeLock::acquire(&repo.git_dir(), Duration::from_millis(200));
        assert!(
            foreign_while_shell_alive.is_err(),
            "an implicit close (as a killed process's fd table teardown performs, never an \
             explicit flock unlock) must not release the flock while the spawned shell still \
             holds its own inherited descriptor"
        );

        child.wait().expect("the shell should terminate");
        std::thread::sleep(Duration::from_millis(100));

        WorktreeLock::acquire(&repo.git_dir(), Duration::from_millis(500))
            .expect("the lock must free once the spawned shell itself exits");
    }

    #[test]
    fn closing_the_control_channel_does_not_trigger_finish_or_signal_the_shell() {
        let repo = TestRepo::new("control-channel-death");
        let (cancel_tx, cancel_rx) = mpsc::channel();
        drop(cancel_tx);

        let outcome = run_external_mutation_guard(
            &repo.root,
            &request("exit 7"),
            || repo.open_db(),
            |_event| {},
            &cancel_rx,
        )
        .expect("a disconnected cancel channel must not itself trigger anything abnormal");
        assert_eq!(outcome.exit_code, Some(7));
    }

    #[test]
    fn a_failed_finish_commit_leaves_the_marker_armed_and_reports_failure() {
        let repo = TestRepo::new("finish-failure");
        let (_cancel_tx, cancel_rx) = mpsc::channel();

        let result = run_external_mutation_guard(
            &repo.root,
            &request("true"),
            || Err(anyhow::anyhow!("injected DB-unavailable failure")),
            |_event| {},
            &cancel_rx,
        );
        assert!(matches!(result, Err(GuardError::Finish(_))));

        let marker = repo.git_dir().join("sce").join("mutation-cursor-tainted");
        assert!(
            marker.exists(),
            "a failed finish commit must leave the external-taint marker armed"
        );
    }

    #[test]
    fn a_cancel_request_signals_the_shells_process_group_and_finish_still_waits_for_real_exit() {
        let repo = TestRepo::new("cancel-request");
        let (cancel_tx, cancel_rx) = mpsc::channel();

        let root = repo.root.clone();
        let state_root = repo.state_root.clone();
        let handle = std::thread::spawn(move || {
            run_external_mutation_guard(
                &root,
                &request("trap 'exit 9' TERM; sleep 30"),
                || {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        &root,
                        &state_root,
                        "external-mutation-guard test assertions",
                    )
                },
                |_event| {},
                &cancel_rx,
            )
        });

        std::thread::sleep(Duration::from_millis(200));
        cancel_tx.send(()).expect("cancel channel should still be open");

        let outcome = handle
            .join()
            .expect("guard thread should not panic")
            .expect("the guard should reach its finish step after the signaled shell exits");
        assert_eq!(outcome.exit_code, Some(9));
    }
}
