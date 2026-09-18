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
    pub cwd: Option<String>,
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
    Cwd(anyhow::Error),
    Exec(anyhow::Error),
    CancelledBeforeExec,
    Spawn(std::io::Error),
    ArmedDelivery(std::io::Error),
    Wait(std::io::Error),
    Finish(CoordinateError),
    UnsupportedPlatform,
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardError::Acquire(source) => write!(f, "{source}"),
            GuardError::Cwd(source) | GuardError::Exec(source) => write!(f, "{source}"),
            GuardError::CancelledBeforeExec => {
                write!(f, "external-mutation guard was cancelled before exec")
            }
            GuardError::Spawn(source) => write!(f, "failed to spawn the guarded shell: {source}"),
            GuardError::ArmedDelivery(source) => write!(
                f,
                "failed to deliver the external-mutation guard admission acknowledgement: {source}"
            ),
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
pub(crate) struct ArmedExternalMutationGuard<P>(std::marker::PhantomData<P>);

#[cfg(not(unix))]
impl<P> ArmedExternalMutationGuard<P> {
    pub(crate) fn exec<E>(
        self,
        _request: &GuardRequest,
        _on_event: E,
    ) -> Result<GuardOutcome, GuardError>
    where
        E: FnMut(GuardEvent),
    {
        Err(GuardError::UnsupportedPlatform)
    }
}

#[cfg(not(unix))]
pub(crate) fn arm_external_mutation_guard<P, A>(
    _repository_root: &Path,
    _open_db: P,
    _on_armed: A,
    _cancel_rx: mpsc::Receiver<()>,
) -> Result<ArmedExternalMutationGuard<P>, GuardError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    A: FnOnce() -> std::io::Result<()>,
{
    Err(GuardError::UnsupportedPlatform)
}

#[cfg(not(unix))]
pub(crate) fn run_external_mutation_guard<P, E>(
    _repository_root: &Path,
    _request: &GuardRequest,
    _open_db: P,
    _on_event: E,
    _cancel_rx: mpsc::Receiver<()>,
) -> Result<GuardOutcome, GuardError>
where
    P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    E: FnMut(GuardEvent),
{
    Err(GuardError::UnsupportedPlatform)
}

#[cfg(unix)]
mod unix_impl {
    use std::fs::File;
    use std::io::{self, Read};
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use anyhow::anyhow;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

    use super::super::coordinator::coordinate_on_held_worktree;
    use super::super::git_snapshot::resolve_worktree_root;
    use super::super::protected_worktree::ProtectedWorktree;
    use super::super::RuntimeBoundary;
    use super::{GuardError, GuardEvent, GuardOutcome, GuardRequest};

    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const STDIO_IDLE_GRACE: Duration = Duration::from_millis(100);
    const STDIO_FINALIZATION_LIMIT: Duration = Duration::from_secs(1);
    const SIGTERM: i32 = 15;

    const F_GETFD: i32 = 1;
    const F_SETFD: i32 = 2;
    const FD_CLOEXEC: i32 = 1;
    const POLLIN: i16 = 0x001;
    const POLLERR: i16 = 0x008;
    const POLLHUP: i16 = 0x010;
    const POLLNVAL: i16 = 0x020;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    fn resolve_shell_executable() -> PathBuf {
        const PREFERRED_BASH_PATH: &str = "/bin/bash";
        if Path::new(PREFERRED_BASH_PATH).exists() {
            return PathBuf::from(PREFERRED_BASH_PATH);
        }
        if let Some(bash_on_path) = locate_bash_on_path() {
            return bash_on_path;
        }
        PathBuf::from("sh")
    }

    fn locate_bash_on_path() -> Option<PathBuf> {
        let output = Command::new("which").arg("bash").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8(output.stdout).ok()?;
        let first_line = stdout.lines().next()?.trim();
        if first_line.is_empty() {
            return None;
        }
        Some(PathBuf::from(first_line))
    }

    fn resolve_execution_cwd(
        repository_root: &Path,
        requested_cwd: Option<&str>,
    ) -> Result<PathBuf, GuardError> {
        let worktree_root = resolve_worktree_root(repository_root).map_err(GuardError::Cwd)?;

        let Some(raw_cwd) = requested_cwd else {
            return Ok(worktree_root);
        };

        if raw_cwd.trim().is_empty() {
            return Err(GuardError::Cwd(anyhow!(
                "external-mutation guard request field 'cwd' must not be blank"
            )));
        }

        let candidate = Path::new(raw_cwd);
        if !candidate.is_absolute() {
            return Err(GuardError::Cwd(anyhow!(
                "external-mutation guard request field 'cwd' must be an absolute path, got '{raw_cwd}'"
            )));
        }

        let canonical_cwd = std::fs::canonicalize(candidate).map_err(|source| {
            GuardError::Cwd(anyhow!(
                "external-mutation guard request field 'cwd' '{raw_cwd}' could not be resolved: {source}"
            ))
        })?;

        if !canonical_cwd.is_dir() {
            return Err(GuardError::Cwd(anyhow!(
                "external-mutation guard request field 'cwd' '{raw_cwd}' is not a directory"
            )));
        }

        if !canonical_cwd.starts_with(&worktree_root) {
            return Err(GuardError::Cwd(anyhow!(
                "external-mutation guard request field 'cwd' '{raw_cwd}' resolves outside the guarded checkout '{}'",
                worktree_root.display()
            )));
        }

        Ok(canonical_cwd)
    }

    mod raw {
        unsafe extern "C" {
            pub(super) fn dup(fd: i32) -> i32;
            pub(super) fn fcntl(fd: i32, command: i32, ...) -> i32;
            pub(super) fn kill(pid: i32, sig: i32) -> i32;
            pub(super) fn pipe(fds: *mut i32) -> i32;
            pub(super) fn poll(fds: *mut super::PollFd, count: usize, timeout: i32) -> i32;
        }
    }

    fn set_cloexec(fd: RawFd, enabled: bool) -> io::Result<()> {
        let flags = unsafe { raw::fcntl(fd, F_GETFD, 0) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let updated = if enabled {
            flags | FD_CLOEXEC
        } else {
            flags & !FD_CLOEXEC
        };
        if unsafe { raw::fcntl(fd, F_SETFD, updated) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[derive(Debug)]
    pub(super) struct LifetimeToken {
        reader: File,
        writer: Option<File>,
    }

    impl LifetimeToken {
        pub(super) fn new() -> io::Result<Self> {
            let mut fds = [-1_i32; 2];
            if unsafe { raw::pipe(fds.as_mut_ptr()) } < 0 {
                return Err(io::Error::last_os_error());
            }

            let reader = unsafe { File::from_raw_fd(fds[0]) };
            let writer = unsafe { File::from_raw_fd(fds[1]) };
            set_cloexec(reader.as_raw_fd(), true)?;
            set_cloexec(writer.as_raw_fd(), false)?;

            Ok(Self {
                reader,
                writer: Some(writer),
            })
        }

        pub(super) fn writer_fd(&self) -> RawFd {
            self.writer
                .as_ref()
                .expect("the supervisor writer must exist before spawn")
                .as_raw_fd()
        }

        fn close_writer(&mut self) {
            self.writer.take();
        }

        fn observe_eof(&mut self) -> io::Result<bool> {
            let mut byte = [0_u8; 1];
            match self.reader.read(&mut byte)? {
                0 => Ok(true),
                _ => Ok(false),
            }
        }
    }

    pub(super) fn spawn_guarded_shell(
        execution_cwd: &Path,
        request: &GuardRequest,
        lock_fd: RawFd,
        lifetime_fd: RawFd,
    ) -> std::io::Result<Child> {
        let duplicated_lock_fd = unsafe { raw::dup(lock_fd) };
        if duplicated_lock_fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let duplicated_lock_fd_guard = unsafe { File::from_raw_fd(duplicated_lock_fd) };
        set_cloexec(duplicated_lock_fd_guard.as_raw_fd(), false)?;
        set_cloexec(lifetime_fd, false)?;

        let mut command = Command::new(resolve_shell_executable());
        command
            .arg("-c")
            .arg(&request.command)
            .current_dir(execution_cwd)
            .envs(request.env.iter().cloned())
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let result = command.spawn();
        drop(duplicated_lock_fd_guard);
        result
    }

    fn poll_fds(fds: &mut [PollFd], timeout: Duration) -> io::Result<()> {
        let milliseconds = i32::try_from(timeout.as_millis().clamp(1, i32::MAX as u128))
            .expect("poll timeout was clamped to i32::MAX");
        loop {
            if unsafe { raw::poll(fds.as_mut_ptr(), fds.len(), milliseconds) } >= 0 {
                return Ok(());
            }
            let source = io::Error::last_os_error();
            if source.kind() != io::ErrorKind::Interrupted {
                return Err(source);
            }
        }
    }

    enum StreamRead {
        Data,
        Closed,
        NoData,
    }

    fn consume_stream<R: Read>(
        pipe: &mut R,
        wrap: fn(Vec<u8>) -> GuardEvent,
        on_event: &mut impl FnMut(GuardEvent),
    ) -> io::Result<StreamRead> {
        let mut buffer = [0_u8; 8192];
        match pipe.read(&mut buffer) {
            Ok(0) => Ok(StreamRead::Closed),
            Ok(count) => {
                on_event(wrap(buffer[..count].to_vec()));
                Ok(StreamRead::Data)
            }
            Err(source) if source.kind() == io::ErrorKind::Interrupted => Ok(StreamRead::NoData),
            Err(source) => Err(source),
        }
    }

    fn stream_is_open(stdout: Option<&ChildStdout>, stderr: Option<&ChildStderr>) -> bool {
        stdout.is_some() || stderr.is_some()
    }

    fn poll_and_consume_streams(
        lifetime: &mut LifetimeToken,
        stdout: &mut Option<ChildStdout>,
        stderr: &mut Option<ChildStderr>,
        on_event: &mut impl FnMut(GuardEvent),
        timeout: Duration,
        lifetime_complete: &mut bool,
        last_stream_activity: &mut Instant,
    ) -> io::Result<()> {
        let mut descriptors = Vec::with_capacity(3);
        let lifetime_index = if *lifetime_complete {
            None
        } else {
            let index = descriptors.len();
            descriptors.push(PollFd {
                fd: lifetime.reader.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            });
            Some(index)
        };
        let stdout_index = stdout.as_ref().map(|pipe| {
            let index = descriptors.len();
            descriptors.push(PollFd {
                fd: pipe.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            });
            index
        });
        let stderr_index = stderr.as_ref().map(|pipe| {
            let index = descriptors.len();
            descriptors.push(PollFd {
                fd: pipe.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            });
            index
        });

        poll_fds(&mut descriptors, timeout)?;
        if let Some(index) = lifetime_index {
            if descriptors[index].revents & (POLLIN | POLLERR | POLLHUP | POLLNVAL) != 0 {
                *lifetime_complete = lifetime.observe_eof()?;
            }
        }
        if let Some(index) = stdout_index {
            if descriptors[index].revents & (POLLIN | POLLERR | POLLHUP | POLLNVAL) != 0 {
                if let Some(pipe) = stdout.as_mut() {
                    match consume_stream(pipe, GuardEvent::Stdout, on_event)? {
                        StreamRead::Data => *last_stream_activity = Instant::now(),
                        StreamRead::Closed => *stdout = None,
                        StreamRead::NoData => {}
                    }
                }
            }
        }
        if let Some(index) = stderr_index {
            if descriptors[index].revents & (POLLIN | POLLERR | POLLHUP | POLLNVAL) != 0 {
                if let Some(pipe) = stderr.as_mut() {
                    match consume_stream(pipe, GuardEvent::Stderr, on_event)? {
                        StreamRead::Data => *last_stream_activity = Instant::now(),
                        StreamRead::Closed => *stderr = None,
                        StreamRead::NoData => {}
                    }
                }
            }
        }
        Ok(())
    }

    struct GuardOwnership {
        protected: Option<ProtectedWorktree>,
        spawned: bool,
        lifetime_complete: bool,
    }

    impl GuardOwnership {
        fn new(protected: ProtectedWorktree) -> Self {
            Self {
                protected: Some(protected),
                spawned: false,
                lifetime_complete: false,
            }
        }

        fn mark_spawned(&mut self) {
            self.spawned = true;
        }

        fn mark_lifetime_complete(&mut self) {
            self.lifetime_complete = true;
        }

        fn protected(&self) -> &ProtectedWorktree {
            self.protected
                .as_ref()
                .expect("guard ownership must contain the protected worktree")
        }

        fn abandon_after_spawn_without_unlock(&mut self) {
            if let Some(protected) = self.protected.take() {
                protected.abandon_after_spawn_without_unlock();
            }
        }

        fn take_protected(&mut self) -> ProtectedWorktree {
            self.protected
                .take()
                .expect("guard ownership must contain the protected worktree")
        }
    }

    impl Drop for GuardOwnership {
        fn drop(&mut self) {
            if self.spawned && !self.lifetime_complete {
                self.abandon_after_spawn_without_unlock();
            }
        }
    }

    fn supervision_poll_timeout(
        finalization_started: Option<Instant>,
        last_stream_activity: Instant,
    ) -> Duration {
        finalization_started.map_or(POLL_INTERVAL, |started| {
            let idle_remaining = STDIO_IDLE_GRACE.saturating_sub(last_stream_activity.elapsed());
            let hard_remaining = STDIO_FINALIZATION_LIMIT.saturating_sub(started.elapsed());
            idle_remaining.min(hard_remaining)
        })
    }

    fn should_finish_finalization(
        status: Option<&ExitStatus>,
        lifetime_complete: bool,
        stdout: Option<&ChildStdout>,
        stderr: Option<&ChildStderr>,
        finalization_started: &mut Option<Instant>,
        last_stream_activity: &mut Instant,
    ) -> bool {
        if status.is_none() || !lifetime_complete {
            return false;
        }
        if finalization_started.is_none() {
            let now = Instant::now();
            *finalization_started = Some(now);
            *last_stream_activity = now;
        }
        let started = finalization_started.expect("finalization start must be set");
        let idle_expired = last_stream_activity.elapsed() >= STDIO_IDLE_GRACE;
        let hard_limit_expired = started.elapsed() >= STDIO_FINALIZATION_LIMIT;
        (!stream_is_open(stdout, stderr)) || idle_expired || hard_limit_expired
    }

    #[derive(Clone, Copy, Debug, Default)]
    pub(super) struct GuardTestHooks {
        pub(super) fail_lifetime_token: bool,
        pub(super) fail_after_first_poll: bool,
    }

    pub(crate) struct ArmedExternalMutationGuard<P> {
        repository_root: PathBuf,
        ownership: GuardOwnership,
        lifetime: LifetimeToken,
        open_db: Option<P>,
        cancel_rx: mpsc::Receiver<()>,
        hooks: GuardTestHooks,
    }

    impl<P> ArmedExternalMutationGuard<P>
    where
        P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    {
        #[allow(clippy::too_many_lines)]
        pub(crate) fn exec<E>(
            mut self,
            request: &GuardRequest,
            mut on_event: E,
        ) -> Result<GuardOutcome, GuardError>
        where
            E: FnMut(GuardEvent),
        {
            if self.cancel_rx.try_recv().is_ok() {
                return Err(GuardError::CancelledBeforeExec);
            }
            if request.command.trim().is_empty() {
                return Err(GuardError::Exec(anyhow!(
                    "external-mutation guard exec command must not be blank"
                )));
            }
            let execution_cwd =
                resolve_execution_cwd(&self.repository_root, request.cwd.as_deref())?;
            let lock_fd = self.ownership.protected().lock_raw_fd();
            let mut child = match spawn_guarded_shell(
                &execution_cwd,
                request,
                lock_fd,
                self.lifetime.writer_fd(),
            ) {
                Ok(child) => child,
                Err(source) => return Err(GuardError::Spawn(source)),
            };
            self.ownership.mark_spawned();
            self.lifetime.close_writer();

            let mut stdout = child.stdout.take();
            let mut stderr = child.stderr.take();
            let pid = child.id();
            let mut status: Option<ExitStatus> = None;
            let mut lifetime_complete = false;
            let mut cancel_signaled = false;
            let mut finalization_started = None;
            let mut last_stream_activity = Instant::now();
            loop {
                if status.is_none() {
                    status = match child.try_wait() {
                        Ok(status) => status,
                        Err(source) => {
                            self.ownership.abandon_after_spawn_without_unlock();
                            return Err(GuardError::Wait(source));
                        }
                    };
                }

                if should_finish_finalization(
                    status.as_ref(),
                    lifetime_complete,
                    stdout.as_ref(),
                    stderr.as_ref(),
                    &mut finalization_started,
                    &mut last_stream_activity,
                ) {
                    break;
                }

                if !cancel_signaled && self.cancel_rx.try_recv().is_ok() {
                    cancel_signaled = true;
                    #[allow(clippy::cast_possible_wrap)]
                    let group = -(pid as i32);
                    unsafe {
                        raw::kill(group, SIGTERM);
                    }
                }

                let poll_timeout =
                    supervision_poll_timeout(finalization_started, last_stream_activity);
                if let Err(source) = poll_and_consume_streams(
                    &mut self.lifetime,
                    &mut stdout,
                    &mut stderr,
                    &mut on_event,
                    poll_timeout,
                    &mut lifetime_complete,
                    &mut last_stream_activity,
                ) {
                    self.ownership.abandon_after_spawn_without_unlock();
                    return Err(GuardError::Wait(source));
                }
                if lifetime_complete {
                    self.ownership.mark_lifetime_complete();
                }
                if self.hooks.fail_after_first_poll {
                    self.ownership.abandon_after_spawn_without_unlock();
                    return Err(GuardError::Wait(io::Error::other(
                        "injected post-spawn supervision failure",
                    )));
                }
            }

            self.ownership.mark_lifetime_complete();
            let status = status.expect("the guard loop only finishes after shell exit");
            let worktree_id = self.ownership.protected().worktree_id().clone();
            if let Err(source) = coordinate_on_held_worktree(
                &self.repository_root,
                &worktree_id,
                &RuntimeBoundary::Flush,
                self.open_db
                    .take()
                    .expect("the guard database opener must be available before exec"),
                true,
            ) {
                return Err(GuardError::Finish(source));
            }

            let protected = self.ownership.take_protected();
            let marker_clear_failed = protected.complete().is_err();

            Ok(GuardOutcome {
                exit_code: status.code(),
                marker_clear_failed,
            })
        }
    }

    fn arm_external_mutation_guard_inner<P, A>(
        repository_root: &Path,
        open_db: P,
        on_armed: A,
        cancel_rx: mpsc::Receiver<()>,
        hooks: GuardTestHooks,
    ) -> Result<ArmedExternalMutationGuard<P>, GuardError>
    where
        P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
        A: FnOnce() -> io::Result<()>,
    {
        let protected = ProtectedWorktree::acquire(repository_root).map_err(GuardError::Acquire)?;
        let ownership = GuardOwnership::new(protected);
        let lifetime = if hooks.fail_lifetime_token {
            Err(io::Error::other(
                "injected lifetime-token establishment failure",
            ))
        } else {
            LifetimeToken::new()
        }
        .map_err(GuardError::Spawn)?;
        on_armed().map_err(GuardError::ArmedDelivery)?;

        Ok(ArmedExternalMutationGuard {
            repository_root: repository_root.to_path_buf(),
            ownership,
            lifetime,
            open_db: Some(open_db),
            cancel_rx,
            hooks,
        })
    }

    pub(crate) fn arm_external_mutation_guard<P, A>(
        repository_root: &Path,
        open_db: P,
        on_armed: A,
        cancel_rx: mpsc::Receiver<()>,
    ) -> Result<ArmedExternalMutationGuard<P>, GuardError>
    where
        P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
        A: FnOnce() -> io::Result<()>,
    {
        arm_external_mutation_guard_inner(
            repository_root,
            open_db,
            on_armed,
            cancel_rx,
            GuardTestHooks::default(),
        )
    }

    pub(crate) fn run_external_mutation_guard<P, E>(
        repository_root: &Path,
        request: &GuardRequest,
        open_db: P,
        mut on_event: E,
        cancel_rx: mpsc::Receiver<()>,
    ) -> Result<GuardOutcome, GuardError>
    where
        P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
        E: FnMut(GuardEvent),
    {
        let guard = arm_external_mutation_guard(
            repository_root,
            open_db,
            || {
                on_event(GuardEvent::Armed);
                Ok(())
            },
            cancel_rx,
        )?;
        guard.exec(request, on_event)
    }

    #[cfg(test)]
    pub(super) fn run_external_mutation_guard_with_hooks<P, E>(
        repository_root: &Path,
        request: &GuardRequest,
        open_db: P,
        mut on_event: E,
        cancel_rx: mpsc::Receiver<()>,
        hooks: GuardTestHooks,
    ) -> Result<GuardOutcome, GuardError>
    where
        P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
        E: FnMut(GuardEvent),
    {
        let guard = arm_external_mutation_guard_inner(
            repository_root,
            open_db,
            || {
                on_event(GuardEvent::Armed);
                Ok(())
            },
            cancel_rx,
            hooks,
        )?;
        guard.exec(request, on_event)
    }

    #[cfg(test)]
    pub(super) fn arm_external_mutation_guard_with_hooks<P, A>(
        repository_root: &Path,
        open_db: P,
        on_armed: A,
        cancel_rx: mpsc::Receiver<()>,
        hooks: GuardTestHooks,
    ) -> Result<ArmedExternalMutationGuard<P>, GuardError>
    where
        P: FnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
        A: FnOnce() -> io::Result<()>,
    {
        arm_external_mutation_guard_inner(repository_root, open_db, on_armed, cancel_rx, hooks)
    }
}

#[cfg(unix)]
pub(crate) use unix_impl::{
    arm_external_mutation_guard, run_external_mutation_guard, ArmedExternalMutationGuard,
};

#[cfg(all(unix, test))]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
    };
    use crate::services::mutation_trace::store::MutationTraceStore;

    use super::super::coordinator::{coordinate, RuntimeBoundary};
    use super::super::git_snapshot::GitSnapshotService;
    use super::super::protected_worktree::ProtectedWorktree;
    use super::super::worktree_lock::WorktreeLock;
    use super::super::{resolve_git_dir, resolve_worktree_id};
    use super::unix_impl::{
        arm_external_mutation_guard_with_hooks, run_external_mutation_guard_with_hooks,
        spawn_guarded_shell, GuardTestHooks, LifetimeToken,
    };
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

        fn nested_dir(&self, relative: &str) -> PathBuf {
            let dir = self.root.join(relative);
            fs::create_dir_all(&dir).expect("nested test directory should be created");
            dir.canonicalize()
                .expect("nested test directory should canonicalize")
        }

        fn open_db(&self) -> anyhow::Result<RepositoryAgentTraceDb> {
            crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                &self.root,
                &self.state_root,
                "external-mutation-guard test assertions",
            )
        }
    }

    #[test]
    fn lifetime_token_establishment_failure_cannot_emit_armed() {
        let repo = TestRepo::new("lifetime-token-establishment-failure");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);

        let result = run_external_mutation_guard_with_hooks(
            &repo.root,
            &request("true"),
            || repo.open_db(),
            move |event| captured_events.lock().expect("events mutex").push(event),
            cancel_rx,
            GuardTestHooks {
                fail_lifetime_token: true,
                fail_after_first_poll: false,
            },
        );

        assert!(matches!(result, Err(GuardError::Spawn(_))));
        assert!(
            events.lock().expect("events mutex").is_empty(),
            "Armed must not be emitted before lifetime-token establishment"
        );
        WorktreeLock::acquire(&repo.git_dir(), Duration::from_millis(200))
            .expect("pre-spawn lifetime failure must release the ordinary lock");
        assert!(
            repo.git_dir()
                .join("sce")
                .join("mutation-cursor-tainted")
                .exists(),
            "the existing write-ahead marker remains armed after establishment failure"
        );
    }

    #[test]
    fn post_spawn_supervision_failure_abandons_without_unlocking_inherited_lock() {
        let repo = TestRepo::new("post-spawn-supervision-failure");
        let release = repo.root.join("release");
        let descendant_done = repo.root.join("descendant-done");
        let command = format!(
            "(while [ ! -f '{}' ]; do sleep 0.01; done; printf descendant > '{}'; touch '{}') & printf ready",
            release.display(),
            repo.root.join("file.txt").display(),
            descendant_done.display(),
        );
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let result = run_external_mutation_guard_with_hooks(
            &repo.root,
            &request(&command),
            || repo.open_db(),
            move |event| {
                if let GuardEvent::Stdout(chunk) = event {
                    if String::from_utf8_lossy(&chunk).contains("ready") {
                        ready_tx.send(()).expect("ready channel should be open");
                    }
                }
            },
            cancel_rx,
            GuardTestHooks {
                fail_lifetime_token: false,
                fail_after_first_poll: true,
            },
        );

        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the shell must have spawned before the injected failure");
        assert!(matches!(result, Err(GuardError::Wait(_))));

        assert!(
            WorktreeLock::acquire(&repo.git_dir(), Duration::from_millis(200)).is_err(),
            "post-spawn abandonment must not execute flock(LOCK_UN) while the descendant lives"
        );
        let marker = repo.git_dir().join("sce").join("mutation-cursor-tainted");
        assert!(
            marker.exists(),
            "the marker must remain armed on abandonment"
        );

        fs::write(&release, "release\n").expect("descendant release handshake should write");
        wait_for_path(&descendant_done);
        WorktreeLock::acquire(&repo.git_dir(), Duration::from_secs(1))
            .expect("the inherited descriptor should release the flock naturally");

        coordinate(&repo.root, &RuntimeBoundary::Flush, || repo.open_db())
            .expect("the next boundary should recover inherited external taint");
        assert!(
            !marker.exists(),
            "inherited-taint recovery should clear the marker"
        );
    }

    #[test]
    fn post_spawn_supervision_failure_without_descendants_uses_the_same_abandonment_path() {
        let repo = TestRepo::new("post-spawn-no-descendant-failure");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let result = run_external_mutation_guard_with_hooks(
            &repo.root,
            &request("printf ready"),
            || repo.open_db(),
            move |event| {
                if let GuardEvent::Stdout(chunk) = event {
                    if String::from_utf8_lossy(&chunk).contains("ready") {
                        ready_tx.send(()).expect("ready channel should be open");
                    }
                }
            },
            cancel_rx,
            GuardTestHooks {
                fail_lifetime_token: false,
                fail_after_first_poll: true,
            },
        );

        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the shell must have spawned before the injected failure");
        assert!(matches!(result, Err(GuardError::Wait(_))));
        let marker = repo.git_dir().join("sce").join("mutation-cursor-tainted");
        assert!(
            marker.exists(),
            "the marker must remain armed on abandonment"
        );
        WorktreeLock::acquire(&repo.git_dir(), Duration::from_secs(1))
            .expect("without descendants the shell's inherited fd closes naturally");

        coordinate(&repo.root, &RuntimeBoundary::Flush, || repo.open_db())
            .expect("the next boundary should recover the still-armed marker");
        assert!(
            !marker.exists(),
            "inherited-taint recovery should clear the marker"
        );
    }

    #[test]
    fn lost_armed_acknowledgement_cannot_spawn_or_mutate() {
        let repo = TestRepo::new("lost-armed-ack");
        let marker = repo.git_dir().join("sce").join("mutation-cursor-tainted");
        let target = repo.root.join("lost-armed-command-ran");
        let (_cancel_tx, cancel_rx) = mpsc::channel();

        let result = arm_external_mutation_guard_with_hooks(
            &repo.root,
            || repo.open_db(),
            || {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "injected lost Armed acknowledgement",
                ))
            },
            cancel_rx,
            GuardTestHooks::default(),
        );

        assert!(matches!(result, Err(GuardError::ArmedDelivery(_))));
        assert!(
            !target.exists(),
            "the command must not run without Armed delivery"
        );
        WorktreeLock::acquire(&repo.git_dir(), Duration::from_secs(1))
            .expect("pre-spawn acknowledgement failure must release the ordinary lock");
        assert!(
            marker.exists(),
            "ambiguous establishment remains conservatively tainted"
        );
    }

    #[test]
    fn armed_guard_waits_for_exec_and_drops_without_spawning_on_eof() {
        let repo = TestRepo::new("armed-without-exec");
        let marker = repo.git_dir().join("sce").join("mutation-cursor-tainted");
        let target = repo.root.join("never-ran");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let armed = arm_external_mutation_guard_with_hooks(
            &repo.root,
            || repo.open_db(),
            || Ok(()),
            cancel_rx,
            GuardTestHooks::default(),
        )
        .expect("arm should succeed");

        drop(armed);
        assert!(!target.exists(), "EOF before exec must not create a shell");
        WorktreeLock::acquire(&repo.git_dir(), Duration::from_secs(1))
            .expect("ordinary pre-spawn cleanup must release the lock");
        assert!(
            marker.exists(),
            "pre-spawn EOF remains conservatively tainted"
        );

        coordinate(&repo.root, &RuntimeBoundary::Flush, || repo.open_db())
            .expect("the next boundary must self-heal the conservative marker");
        assert!(!marker.exists());
    }

    #[test]
    fn cancellation_before_exec_cannot_spawn() {
        let repo = TestRepo::new("cancel-before-exec");
        let target = repo.root.join("cancelled-command-ran");
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let armed = arm_external_mutation_guard_with_hooks(
            &repo.root,
            || repo.open_db(),
            || Ok(()),
            cancel_rx,
            GuardTestHooks::default(),
        )
        .expect("arm should succeed");
        cancel_tx.send(()).expect("cancel should be received");

        let result = armed.exec(
            &request(&format!("touch '{}'", target.display())),
            |_event| {},
        );
        assert!(matches!(result, Err(GuardError::CancelledBeforeExec)));
        assert!(!target.exists());
    }

    #[test]
    fn armed_guard_has_no_side_effect_before_exec_and_exec_runs_once() {
        let repo = TestRepo::new("two-phase-happy-path");
        let target = repo.root.join("exec-ran");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let armed = arm_external_mutation_guard_with_hooks(
            &repo.root,
            || repo.open_db(),
            || Ok(()),
            cancel_rx,
            GuardTestHooks::default(),
        )
        .expect("arm should succeed");
        assert!(!target.exists(), "Armed must not execute the later command");

        let request = request(&format!("touch '{}'", target.display()));
        let outcome = armed
            .exec(&request, |_event| {})
            .expect("the explicit exec request should run");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(target.exists(), "the command must run after explicit exec");
    }

    fn request(command: &str) -> GuardRequest {
        GuardRequest {
            command: command.to_string(),
            cwd: None,
            env: Vec::new(),
        }
    }

    fn request_with_cwd(command: &str, cwd: &Path) -> GuardRequest {
        GuardRequest {
            command: command.to_string(),
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env: Vec::new(),
        }
    }

    fn wait_for_path(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for '{}'",
                path.display()
            );
            std::thread::yield_now();
        }
    }

    fn wait_for_output(output: &Mutex<Vec<u8>>, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !String::from_utf8_lossy(&output.lock().expect("output mutex")).contains(expected) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for output '{expected}'"
            );
            std::thread::yield_now();
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
                cancel_rx,
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
    fn graceful_completion_waits_for_an_inherited_background_descendant() {
        let repo = TestRepo::new("graceful-background-descendant");
        let release = repo.root.join("release");
        let foreground_exited = repo.root.join("foreground-exited");
        let descendant_done = repo.root.join("descendant-done");
        let command = format!(
            "(while [ ! -f '{}' ]; do sleep 0.01; done; printf descendant > '{}'; touch '{}') >/dev/null 2>&1 & touch '{}'",
            release.display(),
            repo.root.join("file.txt").display(),
            descendant_done.display(),
            foreground_exited.display(),
        );
        let (cancel_tx, cancel_rx) = mpsc::channel();
        drop(cancel_tx);
        let (armed_tx, armed_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let root = repo.root.clone();
        let state_root = repo.state_root.clone();
        let handle = std::thread::spawn(move || {
            let result = run_external_mutation_guard(
                &root,
                &request(&command),
                || {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        &root,
                        &state_root,
                        "external-mutation-guard graceful-descendant test",
                    )
                },
                |event| {
                    if matches!(event, GuardEvent::Armed) {
                        armed_tx.send(()).expect("armed channel should be open");
                    }
                },
                cancel_rx,
            );
            result_tx
                .send(result)
                .expect("result channel should be open");
        });

        armed_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("guard should arm before spawning the shell");
        wait_for_path(&foreground_exited);
        assert!(
            result_rx.try_recv().is_err(),
            "foreground shell exit must not complete the guard while its descendant is alive"
        );
        let marker = repo.git_dir().join("sce").join("mutation-cursor-tainted");
        assert!(
            marker.exists(),
            "the external-taint marker must remain armed"
        );
        assert!(
            WorktreeLock::acquire(&repo.git_dir(), Duration::from_millis(200)).is_err(),
            "the real worktree lock must exclude foreign acquirers until descendant completion"
        );

        fs::write(&release, "release\n").expect("descendant release handshake should write");
        wait_for_path(&descendant_done);
        let outcome = result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("guard should finish after the descendant releases the token")
            .expect("guard should recover successfully");
        assert_eq!(outcome.exit_code, Some(0));
        handle.join().expect("guard thread should not panic");

        assert_eq!(
            fs::read_to_string(repo.root.join("file.txt")).expect("mutated file should read"),
            "descendant"
        );
        let final_tree = GitSnapshotService::new(&repo.root)
            .expect("snapshot service should construct")
            .capture_tree()
            .expect("final tree should capture");
        let worktree_id = resolve_worktree_id(&repo.root).expect("worktree id should resolve");
        let db = repo
            .open_db()
            .expect("database should reopen for assertions");
        let projection = MutationTraceStore::new(&db)
            .load_worktree(&worktree_id, None, None)
            .expect("worktree state should load")
            .expect("guard recovery should initialize worktree state");
        assert_eq!(
            projection.worktree_state.cursor_tree, final_tree,
            "final recovery must rebaseline to the descendant mutation"
        );
        assert!(!marker.exists(), "marker clears only after final recovery");
        WorktreeLock::acquire(&repo.git_dir(), Duration::from_secs(1))
            .expect("worktree lock should be released after final recovery");
    }

    #[test]
    fn output_is_consumed_while_a_background_descendant_holds_the_lifetime_token() {
        let repo = TestRepo::new("background-output");
        let release = repo.root.join("release");
        let foreground_exited = repo.root.join("foreground-exited");
        let start_output = repo.root.join("start-output");
        let output_ready = repo.root.join("output-ready");
        let descendant_done = repo.root.join("descendant-done");
        let command = format!(
            "(while [ ! -f '{}' ]; do sleep 0.01; done; i=0; while [ $i -lt 20 ]; do printf 'stdout-%s\\n' $i; printf 'stderr-%s\\n' $i >&2; i=$((i+1)); done; touch '{}'; while [ ! -f '{}' ]; do sleep 0.01; done; i=20; while [ $i -lt 40 ]; do printf 'stdout-%s\\n' $i; printf 'stderr-%s\\n' $i >&2; i=$((i+1)); done; touch '{}') & touch '{}'",
            start_output.display(),
            output_ready.display(),
            release.display(),
            descendant_done.display(),
            foreground_exited.display(),
        );
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let (armed_tx, armed_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let captured_stdout = Arc::clone(&stdout);
        let captured_stderr = Arc::clone(&stderr);
        let root = repo.root.clone();
        let state_root = repo.state_root.clone();
        let handle = std::thread::spawn(move || {
            let result = run_external_mutation_guard(
                &root,
                &request(&command),
                || {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        &root,
                        &state_root,
                        "external-mutation-guard background-output test",
                    )
                },
                move |event| match event {
                    GuardEvent::Armed => armed_tx.send(()).expect("armed channel should be open"),
                    GuardEvent::Stdout(chunk) => {
                        captured_stdout.lock().expect("stdout mutex").extend(chunk);
                    }
                    GuardEvent::Stderr(chunk) => {
                        captured_stderr.lock().expect("stderr mutex").extend(chunk);
                    }
                },
                cancel_rx,
            );
            result_tx
                .send(result)
                .expect("result channel should be open");
        });

        armed_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("guard should arm before spawning the shell");
        wait_for_path(&foreground_exited);
        assert!(
            result_rx.try_recv().is_err(),
            "guard completion must wait for the lifetime token, not stream timing"
        );
        fs::write(&start_output, "start\n").expect("output handshake should write");
        wait_for_path(&output_ready);
        wait_for_output(&stdout, "stdout-19");
        wait_for_output(&stderr, "stderr-19");
        assert!(
            result_rx.try_recv().is_err(),
            "guard completion must wait for the lifetime token, not stream timing"
        );

        fs::write(&release, "release\n").expect("descendant release handshake should write");
        wait_for_path(&descendant_done);
        result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("guard should finish after descendant output and exit")
            .expect("guard should recover successfully");
        handle.join().expect("guard thread should not panic");

        let stdout_text = String::from_utf8(stdout.lock().expect("stdout mutex").clone())
            .expect("stdout should be valid UTF-8");
        let stderr_text = String::from_utf8(stderr.lock().expect("stderr mutex").clone())
            .expect("stderr should be valid UTF-8");
        assert!(stdout_text.contains("stdout-39"));
        assert!(stderr_text.contains("stderr-39"));
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
            cancel_rx,
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
        let lifetime = LifetimeToken::new().expect("lifetime token should be created");
        let mut child = spawn_guarded_shell(
            &repo.root,
            &request("sleep 1"),
            lock_fd,
            lifetime.writer_fd(),
        )
        .expect("the guarded shell should spawn");
        drop(lifetime);

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
            cancel_rx,
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
            cancel_rx,
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
                cancel_rx,
            )
        });

        std::thread::sleep(Duration::from_millis(200));
        cancel_tx
            .send(())
            .expect("cancel channel should still be open");

        let outcome = handle
            .join()
            .expect("guard thread should not panic")
            .expect("the guard should reach its finish step after the signaled shell exits");
        assert_eq!(outcome.exit_code, Some(9));
    }

    fn run_and_capture_stdout(repo: &TestRepo, req: &GuardRequest) -> (GuardOutcome, String) {
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let captured_stdout: Mutex<Vec<u8>> = Mutex::new(Vec::new());
        let outcome = run_external_mutation_guard(
            &repo.root,
            req,
            || repo.open_db(),
            |event| {
                if let GuardEvent::Stdout(chunk) = event {
                    captured_stdout.lock().expect("stdout mutex").extend(chunk);
                }
            },
            cancel_rx,
        )
        .expect("guard should succeed");
        let output = String::from_utf8(captured_stdout.into_inner().expect("stdout mutex"))
            .expect("stdout should be valid UTF-8");
        (outcome, output)
    }

    #[test]
    fn a_root_cwd_request_is_preserved_exactly() {
        let repo = TestRepo::new("cwd-root");
        let canonical_root = repo
            .root
            .canonicalize()
            .expect("repo root should canonicalize");
        let (outcome, output) =
            run_and_capture_stdout(&repo, &request_with_cwd("pwd", &canonical_root));
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(output.trim(), canonical_root.to_string_lossy());
    }

    #[test]
    fn an_absent_cwd_defaults_to_the_worktree_root() {
        let repo = TestRepo::new("cwd-default");
        let canonical_root = repo
            .root
            .canonicalize()
            .expect("repo root should canonicalize");
        let (outcome, output) = run_and_capture_stdout(&repo, &request("pwd"));
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(output.trim(), canonical_root.to_string_lossy());
    }

    #[test]
    fn a_nested_cwd_request_is_preserved_exactly() {
        let repo = TestRepo::new("cwd-nested");
        let nested = repo.nested_dir("crates/foo");
        let (outcome, output) = run_and_capture_stdout(&repo, &request_with_cwd("pwd", &nested));
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(output.trim(), nested.to_string_lossy());
    }

    #[test]
    fn a_relative_exec_cwd_is_rejected_fail_closed() {
        let repo = TestRepo::new("cwd-relative");
        let (_cancel_tx, cancel_rx) = mpsc::channel();

        let result = run_external_mutation_guard(
            &repo.root,
            &request_with_cwd("true", Path::new("relative")),
            || repo.open_db(),
            |_event| {},
            cancel_rx,
        );
        assert!(matches!(result, Err(GuardError::Cwd(_))));
    }

    #[test]
    fn a_parent_traversal_cwd_cannot_escape_the_checkout() {
        let repo = TestRepo::new("cwd-traversal");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let escaping_cwd = repo.root.join("..").to_string_lossy().into_owned();

        let result = run_external_mutation_guard(
            &repo.root,
            &GuardRequest {
                command: "true".to_string(),
                cwd: Some(escaping_cwd),
                env: Vec::new(),
            },
            || repo.open_db(),
            |_event| {},
            cancel_rx,
        );
        assert!(matches!(result, Err(GuardError::Cwd(_))));
    }

    #[test]
    fn an_absolute_cwd_outside_the_checkout_is_rejected() {
        let repo = TestRepo::new("cwd-outside");
        let (_cancel_tx, cancel_rx) = mpsc::channel();

        let result = run_external_mutation_guard(
            &repo.root,
            &request_with_cwd("true", &repo.state_root),
            || repo.open_db(),
            |_event| {},
            cancel_rx,
        );
        assert!(matches!(result, Err(GuardError::Cwd(_))));
    }

    #[test]
    fn a_nonexistent_cwd_is_rejected_fail_closed() {
        let repo = TestRepo::new("cwd-missing");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let missing = repo.root.join("does-not-exist");

        let result = run_external_mutation_guard(
            &repo.root,
            &request_with_cwd("true", &missing),
            || repo.open_db(),
            |_event| {},
            cancel_rx,
        );
        assert!(matches!(result, Err(GuardError::Cwd(_))));
    }

    #[test]
    fn a_non_directory_cwd_is_rejected_fail_closed() {
        let repo = TestRepo::new("cwd-file");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let file_path = repo.root.join("file.txt");

        let result = run_external_mutation_guard(
            &repo.root,
            &request_with_cwd("true", &file_path),
            || repo.open_db(),
            |_event| {},
            cancel_rx,
        );
        assert!(matches!(result, Err(GuardError::Cwd(_))));
    }

    #[test]
    fn a_rejected_exec_cwd_never_spawns_and_keeps_the_marker_conservative() {
        let repo = TestRepo::new("cwd-rejected-no-lock");
        let (_cancel_tx, cancel_rx) = mpsc::channel();

        let result = run_external_mutation_guard(
            &repo.root,
            &request_with_cwd("true", &repo.state_root),
            || repo.open_db(),
            |_event| {},
            cancel_rx,
        );
        assert!(matches!(result, Err(GuardError::Cwd(_))));

        WorktreeLock::acquire(&repo.git_dir(), Duration::from_millis(200))
            .expect("a rejected pre-spawn exec must release the ordinary worktree lock");

        let marker = repo.git_dir().join("sce").join("mutation-cursor-tainted");
        assert!(
            marker.exists(),
            "an armed guard that rejects its exec request must keep the marker conservative"
        );
    }

    #[test]
    fn multiple_stdout_chunks_immediately_before_exit_are_not_truncated() {
        let repo = TestRepo::new("stdout-chunks");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let captured_stdout: Mutex<Vec<u8>> = Mutex::new(Vec::new());

        let outcome = run_external_mutation_guard(
            &repo.root,
            &request(
                "i=0; while [ $i -lt 4000 ]; do echo \"line-$i-0123456789ABCDEF\"; i=$((i+1)); done",
            ),
            || repo.open_db(),
            |event| {
                if let GuardEvent::Stdout(chunk) = event {
                    captured_stdout.lock().expect("stdout mutex").extend(chunk);
                }
            },
            cancel_rx,
        )
        .expect("guard should succeed");
        assert_eq!(outcome.exit_code, Some(0));

        let output = String::from_utf8(captured_stdout.into_inner().expect("stdout mutex"))
            .expect("stdout should be valid UTF-8");
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines.len(),
            4000,
            "every line must be delivered, none dropped at process exit"
        );
        assert_eq!(lines[0], "line-0-0123456789ABCDEF");
        assert_eq!(lines[3999], "line-3999-0123456789ABCDEF");
    }

    #[test]
    fn stderr_output_immediately_before_exit_is_not_truncated() {
        let repo = TestRepo::new("stderr-chunks");
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        let captured_stderr: Mutex<Vec<u8>> = Mutex::new(Vec::new());

        let outcome = run_external_mutation_guard(
            &repo.root,
            &request(
                "i=0; while [ $i -lt 4000 ]; do echo \"err-$i-0123456789ABCDEF\" >&2; i=$((i+1)); done",
            ),
            || repo.open_db(),
            |event| {
                if let GuardEvent::Stderr(chunk) = event {
                    captured_stderr.lock().expect("stderr mutex").extend(chunk);
                }
            },
            cancel_rx,
        )
        .expect("guard should succeed");
        assert_eq!(outcome.exit_code, Some(0));

        let output = String::from_utf8(captured_stderr.into_inner().expect("stderr mutex"))
            .expect("stderr should be valid UTF-8");
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 4000);
        assert_eq!(lines[3999], "err-3999-0123456789ABCDEF");
    }
}
