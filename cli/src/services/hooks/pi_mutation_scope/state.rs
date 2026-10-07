use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::services::hooks::mutation_scope_lock::{AdapterLockSpec, OsAdvisoryLock};

use super::{format_pi_scope_id, AttemptKey};
use crate::services::hooks::mutation_scope_owner::{
    current_process_owner, is_definitely_dead, ProcessOwner,
};

const SCE_STATE_DIR: &str = "sce";
const ADAPTER_STATE_FILE: &str = "pi-mutation-scope-state.json";

pub(crate) const STATE_LOCK: AdapterLockSpec =
    AdapterLockSpec::state("pi-mutation-scope-state.lock");
pub(crate) const BOUNDARY_LOCK: AdapterLockSpec =
    AdapterLockSpec::boundary("pi-mutation-scope-boundary.lock");

const ADAPTER_STATE_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttemptPhase {
    PendingStart,
    Executed,
    PendingAbandon,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub(crate) enum RecoveryState {
    #[default]
    Clear,
    Pending {
        generation: u64,
    },
    Flushing {
        generation: u64,
    },
}

impl RecoveryState {
    pub(crate) fn is_clear(&self) -> bool {
        matches!(self, RecoveryState::Clear)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AdapterAttempt {
    pub attempt_seq: u64,
    pub scope_id: String,
    pub session_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub phase: AttemptPhase,
    pub owner: ProcessOwner,
}

impl AdapterAttempt {
    fn matches_key(&self, key: &AttemptKey) -> bool {
        self.session_id == key.session_id && self.tool_call_id == key.tool_call_id
    }
}

fn default_recovery_generation() -> u64 {
    1
}

fn default_next_attempt_seq() -> u64 {
    1
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AdapterState {
    pub version: u32,
    #[serde(default = "default_next_attempt_seq")]
    pub next_attempt_seq: u64,
    #[serde(default = "default_recovery_generation")]
    pub next_recovery_generation: u64,
    #[serde(default)]
    pub recovery: RecoveryState,
    pub attempts: Vec<AdapterAttempt>,
}

impl Default for AdapterState {
    fn default() -> Self {
        AdapterState {
            version: ADAPTER_STATE_VERSION,
            next_attempt_seq: default_next_attempt_seq(),
            next_recovery_generation: default_recovery_generation(),
            recovery: RecoveryState::Clear,
            attempts: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AllocatedAttempt {
    pub attempt: AdapterAttempt,
    pub reused: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AdmitDecision {
    Admitted(AllocatedAttempt),
    RecoveryBlocked,
    UncertainAttemptBlocked,
    TerminalAttemptBlocked,
    FlushClaimed { generation: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryFlushCompletion {
    Cleared,
    Superseded,
}

pub(crate) fn adapter_state_dir(git_dir: &Path) -> PathBuf {
    git_dir.join(SCE_STATE_DIR)
}

pub(crate) fn state_path(git_dir: &Path) -> PathBuf {
    adapter_state_dir(git_dir).join(ADAPTER_STATE_FILE)
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
    let dir = adapter_state_dir(git_dir);
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

async fn acquire_lock(git_dir: &Path) -> Result<OsAdvisoryLock> {
    STATE_LOCK.acquire_async(&adapter_state_dir(git_dir)).await
}

fn allocate_pending_start(
    state: &mut AdapterState,
    key: &AttemptKey,
    tool_name: &str,
) -> AdapterAttempt {
    let attempt_seq = state.next_attempt_seq;
    state.next_attempt_seq += 1;

    let attempt = AdapterAttempt {
        attempt_seq,
        scope_id: format_pi_scope_id(key, attempt_seq),
        session_id: key.session_id.clone(),
        tool_call_id: key.tool_call_id.clone(),
        tool_name: tool_name.to_string(),
        phase: AttemptPhase::PendingStart,
        owner: current_process_owner(),
    };
    state.attempts.push(attempt.clone());
    attempt
}

pub(crate) async fn admit_tracked_attempt(
    git_dir: &Path,
    key: &AttemptKey,
    tool_name: &str,
) -> Result<AdmitDecision> {
    let _lock = acquire_lock(git_dir).await?;
    let mut state = read_state(git_dir)?;

    if let Some(existing) = state
        .attempts
        .iter()
        .find(|attempt| attempt.matches_key(key))
    {
        if existing.phase == AttemptPhase::PendingAbandon {
            return Ok(AdmitDecision::TerminalAttemptBlocked);
        }
        return Ok(AdmitDecision::Admitted(AllocatedAttempt {
            attempt: existing.clone(),
            reused: true,
        }));
    }

    match state.recovery {
        RecoveryState::Flushing { .. } => return Ok(AdmitDecision::RecoveryBlocked),
        RecoveryState::Pending { generation } => {
            state.recovery = RecoveryState::Flushing { generation };
            write_state_durably(git_dir, &state)?;
            return Ok(AdmitDecision::FlushClaimed { generation });
        }
        RecoveryState::Clear => {}
    }

    if state
        .attempts
        .iter()
        .any(|attempt| attempt.phase == AttemptPhase::PendingAbandon)
    {
        return Ok(AdmitDecision::UncertainAttemptBlocked);
    }

    let attempt = allocate_pending_start(&mut state, key, tool_name);
    write_state_durably(git_dir, &state)?;
    Ok(AdmitDecision::Admitted(AllocatedAttempt {
        attempt,
        reused: false,
    }))
}

pub(crate) async fn find_definitely_dead_attempts(git_dir: &Path) -> Result<Vec<String>> {
    let _lock = acquire_lock(git_dir).await?;
    let state = read_state(git_dir)?;
    Ok(state
        .attempts
        .iter()
        .filter(|attempt| {
            matches!(
                attempt.phase,
                AttemptPhase::PendingStart | AttemptPhase::Executed
            ) && is_definitely_dead(&attempt.owner)
        })
        .map(|attempt| attempt.scope_id.clone())
        .collect())
}

pub(crate) async fn mark_executed(git_dir: &Path, key: &AttemptKey) -> Result<()> {
    let _lock = acquire_lock(git_dir).await?;

    let mut state = read_state(git_dir)?;
    let Some(attempt) = state
        .attempts
        .iter_mut()
        .find(|attempt| attempt.matches_key(key))
    else {
        return Ok(());
    };

    if attempt.phase == AttemptPhase::PendingStart {
        attempt.phase = AttemptPhase::Executed;
        write_state_durably(git_dir, &state)?;
    }
    Ok(())
}

pub(crate) async fn remove_attempt(git_dir: &Path, scope_id: &str) -> Result<()> {
    let _lock = acquire_lock(git_dir).await?;

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

pub(crate) async fn normalize_recovery_after_boundary_lock_acquired(git_dir: &Path) -> Result<()> {
    let _lock = acquire_lock(git_dir).await?;
    let mut state = read_state(git_dir)?;

    if let RecoveryState::Flushing { generation } = state.recovery {
        state.recovery = RecoveryState::Pending { generation };
        write_state_durably(git_dir, &state)?;
    }
    Ok(())
}

pub(crate) async fn begin_terminal_cleanup(git_dir: &Path, scope_ids: &[String]) -> Result<u64> {
    let _lock = acquire_lock(git_dir).await?;
    let mut state = read_state(git_dir)?;

    for attempt in &mut state.attempts {
        if scope_ids
            .iter()
            .any(|scope_id| scope_id == &attempt.scope_id)
        {
            attempt.phase = AttemptPhase::PendingAbandon;
        }
    }

    let generation = match state.recovery {
        RecoveryState::Pending { generation } | RecoveryState::Flushing { generation } => {
            generation
        }
        RecoveryState::Clear => {
            let generation = state.next_recovery_generation;
            state.next_recovery_generation += 1;
            generation
        }
    };
    state.recovery = RecoveryState::Flushing { generation };
    write_state_durably(git_dir, &state)?;
    Ok(generation)
}

pub(crate) async fn complete_recovery_flush(
    git_dir: &Path,
    generation: u64,
) -> Result<RecoveryFlushCompletion> {
    let _lock = acquire_lock(git_dir).await?;
    let mut state = read_state(git_dir)?;

    match state.recovery {
        RecoveryState::Flushing { generation: owned } if owned == generation => {
            state.recovery = RecoveryState::Clear;
            write_state_durably(git_dir, &state)?;
            Ok(RecoveryFlushCompletion::Cleared)
        }
        _ => Ok(RecoveryFlushCompletion::Superseded),
    }
}

pub(crate) async fn relinquish_recovery_flush(git_dir: &Path, generation: u64) -> Result<()> {
    let _lock = acquire_lock(git_dir).await?;
    let mut state = read_state(git_dir)?;

    if let RecoveryState::Flushing { generation: owned } = state.recovery {
        if owned == generation {
            state.recovery = RecoveryState::Pending { generation: owned };
            write_state_durably(git_dir, &state)?;
        }
    }
    Ok(())
}
