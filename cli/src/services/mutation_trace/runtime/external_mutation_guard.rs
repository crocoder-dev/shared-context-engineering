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
pub(crate) async fn arm_external_mutation_guard<P, A>(
    _repository_root: &Path,
    _open_db: P,
    _on_armed: A,
    _cancel_rx: mpsc::Receiver<()>,
) -> Result<ArmedExternalMutationGuard<P>, GuardError>
where
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
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
    P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
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
        pub(super) fail_after_first_output: bool,
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
        P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
    {
        #[allow(clippy::too_many_lines)]
        pub(crate) async fn exec<E>(
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
                let stream_activity_before_poll = last_stream_activity;
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
                if self.hooks.fail_after_first_output
                    && last_stream_activity != stream_activity_before_poll
                {
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
            )
            .await
            {
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

    async fn arm_external_mutation_guard_inner<P, A>(
        repository_root: &Path,
        open_db: P,
        on_armed: A,
        cancel_rx: mpsc::Receiver<()>,
        hooks: GuardTestHooks,
    ) -> Result<ArmedExternalMutationGuard<P>, GuardError>
    where
        P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
        A: FnOnce() -> io::Result<()>,
    {
        let protected = ProtectedWorktree::acquire(repository_root)
            .await
            .map_err(GuardError::Acquire)?;
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

    pub(crate) async fn arm_external_mutation_guard<P, A>(
        repository_root: &Path,
        open_db: P,
        on_armed: A,
        cancel_rx: mpsc::Receiver<()>,
    ) -> Result<ArmedExternalMutationGuard<P>, GuardError>
    where
        P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
        A: FnOnce() -> io::Result<()>,
    {
        arm_external_mutation_guard_inner(
            repository_root,
            open_db,
            on_armed,
            cancel_rx,
            GuardTestHooks::default(),
        )
        .await
    }

    pub(crate) async fn run_external_mutation_guard<P, E>(
        repository_root: &Path,
        request: &GuardRequest,
        open_db: P,
        mut on_event: E,
        cancel_rx: mpsc::Receiver<()>,
    ) -> Result<GuardOutcome, GuardError>
    where
        P: std::ops::AsyncFnOnce() -> anyhow::Result<RepositoryAgentTraceDb>,
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
        )
        .await?;
        guard.exec(request, on_event).await
    }
}

#[cfg(unix)]
pub(crate) use unix_impl::{
    arm_external_mutation_guard, run_external_mutation_guard, ArmedExternalMutationGuard,
};
