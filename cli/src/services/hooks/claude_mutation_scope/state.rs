use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::{format_claude_scope_id, AttemptKey};

const SCE_STATE_DIR: &str = "sce";
const ADAPTER_STATE_FILE: &str = "claude-mutation-scope-state.json";
const ADAPTER_STATE_LOCK_FILE: &str = "claude-mutation-scope-state.lock";

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(20);
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

const ADAPTER_STATE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttemptPhase {
    PendingStart,
    Active,
    PendingAbandon,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AdapterAttempt {
    pub attempt_seq: u64,
    pub scope_id: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub tool_use_id: String,
    pub tool_name: String,
    pub phase: AttemptPhase,
}

impl AdapterAttempt {
    fn matches_key(&self, key: &AttemptKey) -> bool {
        self.session_id == key.session_id
            && self.agent_id == key.agent_id
            && self.tool_use_id == key.tool_use_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AdapterState {
    pub version: u32,
    pub next_attempt_seq: u64,
    pub recovery_pending: bool,
    pub attempts: Vec<AdapterAttempt>,
}

impl Default for AdapterState {
    fn default() -> Self {
        AdapterState {
            version: ADAPTER_STATE_VERSION,
            next_attempt_seq: 1,
            recovery_pending: false,
            attempts: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AllocatedAttempt {
    pub attempt: AdapterAttempt,
    pub reused: bool,
}

fn state_dir(git_dir: &Path) -> PathBuf {
    git_dir.join(SCE_STATE_DIR)
}

pub(crate) fn state_path(git_dir: &Path) -> PathBuf {
    state_dir(git_dir).join(ADAPTER_STATE_FILE)
}

fn lock_path(git_dir: &Path) -> PathBuf {
    state_dir(git_dir).join(ADAPTER_STATE_LOCK_FILE)
}

struct AdapterStateLock {
    file: File,
}

#[derive(Debug)]
pub(crate) enum AdapterStateLockError {
    TimedOut { path: PathBuf, timeout: Duration },
    Io(anyhow::Error),
}

impl std::fmt::Display for AdapterStateLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterStateLockError::TimedOut { path, timeout } => write!(
                f,
                "Timed out after {timeout:?} waiting for adapter-state lock '{}'",
                path.display()
            ),
            AdapterStateLockError::Io(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for AdapterStateLockError {}

impl AdapterStateLock {
    fn acquire(
        git_dir: &Path,
        timeout: Duration,
    ) -> Result<AdapterStateLock, AdapterStateLockError> {
        let dir = state_dir(git_dir);
        std::fs::create_dir_all(&dir)
            .with_context(|| {
                format!(
                    "Failed to create adapter state directory '{}'",
                    dir.display()
                )
            })
            .map_err(AdapterStateLockError::Io)?;

        let path = lock_path(git_dir);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| {
                format!(
                    "Failed to open adapter-state lock file '{}'",
                    path.display()
                )
            })
            .map_err(AdapterStateLockError::Io)?;

        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(AdapterStateLock { file }),
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(AdapterStateLockError::TimedOut { path, timeout });
                    }
                    std::thread::sleep(LOCK_POLL_INTERVAL.min(deadline - now));
                }
                Err(TryLockError::Error(source)) => {
                    return Err(AdapterStateLockError::Io(
                        anyhow::Error::new(source).context(format!(
                            "Failed to acquire adapter-state lock '{}'",
                            path.display()
                        )),
                    ));
                }
            }
        }
    }
}

impl Drop for AdapterStateLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) fn read_state(git_dir: &Path) -> Result<AdapterState> {
    let path = state_path(git_dir);
    if !path.exists() {
        return Ok(AdapterState::default());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read adapter state '{}'", path.display()))?;
    parse_adapter_state(&content, &path)
}

fn parse_adapter_state(content: &str, path: &Path) -> Result<AdapterState> {
    let state: AdapterState = serde_json::from_str(content)
        .with_context(|| format!("Adapter state file '{}' is malformed", path.display()))?;
    if state.version != ADAPTER_STATE_VERSION {
        return Err(anyhow!(
            "Adapter state file '{}' has unsupported version {} (expected {})",
            path.display(),
            state.version,
            ADAPTER_STATE_VERSION
        ));
    }
    Ok(state)
}

fn write_state_durably(git_dir: &Path, state: &AdapterState) -> Result<()> {
    write_state_durably_inner(git_dir, state, |_, _| Ok(()))
}

fn write_state_durably_inner<F>(
    git_dir: &Path,
    state: &AdapterState,
    before_rename: F,
) -> Result<()>
where
    F: FnOnce(&Path, &Path) -> Result<()>,
{
    let dir = state_dir(git_dir);
    std::fs::create_dir_all(&dir).with_context(|| {
        format!(
            "Failed to create adapter state directory '{}'",
            dir.display()
        )
    })?;

    let path = dir.join(ADAPTER_STATE_FILE);
    let tmp_path = dir.join(format!("{ADAPTER_STATE_FILE}.tmp"));

    let serialized =
        serde_json::to_vec_pretty(state).context("Failed to serialize adapter state")?;

    let mut tmp_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .with_context(|| {
            format!(
                "Failed to open temporary adapter state file '{}'",
                tmp_path.display()
            )
        })?;
    tmp_file.write_all(&serialized).with_context(|| {
        format!(
            "Failed to write temporary adapter state file '{}'",
            tmp_path.display()
        )
    })?;
    tmp_file.sync_data().with_context(|| {
        format!(
            "Failed to sync temporary adapter state file '{}'",
            tmp_path.display()
        )
    })?;
    drop(tmp_file);

    before_rename(&tmp_path, &path)?;

    std::fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "Failed to rename '{}' to '{}'",
            tmp_path.display(),
            path.display()
        )
    })?;

    #[cfg(unix)]
    {
        if let Ok(dir_handle) = std::fs::File::open(&dir) {
            let _ = dir_handle.sync_all();
        }
    }

    Ok(())
}

pub(crate) fn allocate_attempt(
    git_dir: &Path,
    key: &AttemptKey,
    tool_name: &str,
) -> Result<AllocatedAttempt> {
    let _lock = AdapterStateLock::acquire(git_dir, DEFAULT_LOCK_TIMEOUT)
        .map_err(|err| anyhow!("Failed to acquire adapter-state lock: {err}"))?;

    let mut state = read_state(git_dir)?;

    if let Some(existing) = state
        .attempts
        .iter()
        .find(|attempt| attempt.matches_key(key))
    {
        return Ok(AllocatedAttempt {
            attempt: existing.clone(),
            reused: true,
        });
    }

    let attempt_seq = state.next_attempt_seq;
    let scope_id = format_claude_scope_id(attempt_seq, key);
    let attempt = AdapterAttempt {
        attempt_seq,
        scope_id,
        session_id: key.session_id.clone(),
        agent_id: key.agent_id.clone(),
        tool_use_id: key.tool_use_id.clone(),
        tool_name: tool_name.to_string(),
        phase: AttemptPhase::PendingStart,
    };

    state.attempts.push(attempt.clone());
    state.next_attempt_seq += 1;
    write_state_durably(git_dir, &state)?;

    Ok(AllocatedAttempt {
        attempt,
        reused: false,
    })
}

pub(crate) fn mark_active(git_dir: &Path, scope_id: &str) -> Result<()> {
    let _lock = AdapterStateLock::acquire(git_dir, DEFAULT_LOCK_TIMEOUT)
        .map_err(|err| anyhow!("Failed to acquire adapter-state lock: {err}"))?;

    let mut state = read_state(git_dir)?;
    let attempt = state
        .attempts
        .iter_mut()
        .find(|attempt| attempt.scope_id == scope_id)
        .ok_or_else(|| anyhow!("No adapter-state attempt found for scope_id '{scope_id}'"))?;

    match attempt.phase {
        AttemptPhase::PendingStart => {
            attempt.phase = AttemptPhase::Active;
        }
        AttemptPhase::Active => return Ok(()),
        AttemptPhase::PendingAbandon => {
            return Err(anyhow!(
                "Cannot mark mutation-scope attempt '{scope_id}' active after abandonment was established"
            ));
        }
    }

    write_state_durably(git_dir, &state)
}

fn transition_to_pending_abandon(state: &mut AdapterState, scope_ids: &[String]) {
    for attempt in &mut state.attempts {
        if scope_ids
            .iter()
            .any(|scope_id| scope_id == &attempt.scope_id)
        {
            attempt.phase = AttemptPhase::PendingAbandon;
        }
    }
}

pub(crate) fn mark_recovery_pending_and_pending_abandon(
    git_dir: &Path,
    scope_ids: &[String],
) -> Result<()> {
    if scope_ids.is_empty() {
        return Ok(());
    }

    let _lock = AdapterStateLock::acquire(git_dir, DEFAULT_LOCK_TIMEOUT)
        .map_err(|err| anyhow!("Failed to acquire adapter-state lock: {err}"))?;

    let mut state = read_state(git_dir)?;
    state.recovery_pending = true;
    transition_to_pending_abandon(&mut state, scope_ids);
    write_state_durably(git_dir, &state)
}

pub(crate) fn reprove_pending_abandon(git_dir: &Path) -> Result<Option<Vec<AdapterAttempt>>> {
    let _lock = AdapterStateLock::acquire(git_dir, DEFAULT_LOCK_TIMEOUT)
        .map_err(|err| anyhow!("Failed to acquire adapter-state lock: {err}"))?;

    let state = read_state(git_dir)?;

    if !state.recovery_pending || state.attempts.is_empty() {
        return Ok(None);
    }

    let every_attempt_is_pending_abandon = state
        .attempts
        .iter()
        .all(|attempt| attempt.phase == AttemptPhase::PendingAbandon);

    if !every_attempt_is_pending_abandon {
        return Ok(None);
    }

    Ok(Some(state.attempts))
}

pub(crate) fn remove_attempt(git_dir: &Path, scope_id: &str) -> Result<()> {
    let _lock = AdapterStateLock::acquire(git_dir, DEFAULT_LOCK_TIMEOUT)
        .map_err(|err| anyhow!("Failed to acquire adapter-state lock: {err}"))?;

    let mut state = read_state(git_dir)?;
    let before = state.attempts.len();
    state
        .attempts
        .retain(|attempt| attempt.scope_id != scope_id);
    if state.attempts.len() == before {
        return Ok(());
    }
    write_state_durably(git_dir, &state)
}

pub(crate) fn mark_recovery_pending(git_dir: &Path) -> Result<()> {
    let _lock = AdapterStateLock::acquire(git_dir, DEFAULT_LOCK_TIMEOUT)
        .map_err(|err| anyhow!("Failed to acquire adapter-state lock: {err}"))?;

    let mut state = read_state(git_dir)?;
    state.recovery_pending = true;
    write_state_durably(git_dir, &state)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClearRecoveryOutcome {
    Cleared,
    StillPending,
}

pub(crate) fn clear_recovery_pending_if_quiescent(git_dir: &Path) -> Result<ClearRecoveryOutcome> {
    clear_recovery_pending_if_quiescent_inner(git_dir, |_, _| Ok(()))
}

fn clear_recovery_pending_if_quiescent_inner<F>(
    git_dir: &Path,
    before_rename: F,
) -> Result<ClearRecoveryOutcome>
where
    F: FnOnce(&Path, &Path) -> Result<()>,
{
    let _lock = AdapterStateLock::acquire(git_dir, DEFAULT_LOCK_TIMEOUT)
        .map_err(|err| anyhow!("Failed to acquire adapter-state lock: {err}"))?;

    let mut state = read_state(git_dir)?;

    if !state.recovery_pending {
        return Ok(ClearRecoveryOutcome::Cleared);
    }

    if !state.attempts.is_empty() {
        return Ok(ClearRecoveryOutcome::StillPending);
    }

    state.recovery_pending = false;
    write_state_durably_inner(git_dir, &state, before_rename)?;
    Ok(ClearRecoveryOutcome::Cleared)
}
