use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::services::hooks::mutation_scope_lock::AdapterLockSpec;

use super::{format_codex_scope_id, AttemptKey};

const SCE_STATE_DIR: &str = "sce";
const ADAPTER_STATE_FILE: &str = "codex-mutation-scope-state.json";

pub(crate) const STATE_LOCK: AdapterLockSpec =
    AdapterLockSpec::state("codex-mutation-scope-state.lock");
pub(crate) const BOUNDARY_LOCK: AdapterLockSpec =
    AdapterLockSpec::boundary("codex-mutation-scope-boundary.lock");

const ADAPTER_STATE_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttemptPhase {
    PendingStart,
    Active,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AdapterAttempt {
    pub attempt_seq: u64,
    pub scope_id: String,
    pub session_id: String,
    pub turn_id: String,
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

    pub(crate) fn in_builtin_lane(&self, session_id: &str, turn_id: &str) -> bool {
        self.session_id == session_id && self.turn_id == turn_id
    }
}

fn default_recovery_generation() -> u64 {
    1
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AdapterState {
    pub version: u32,
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
            next_attempt_seq: 1,
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
    StalePredecessorBlocked,
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

pub(crate) async fn read_state(git_dir: &Path) -> Result<AdapterState> {
    let git_dir = git_dir.to_owned();
    tokio::task::spawn_blocking(move || read_state_sync(&git_dir))
        .await
        .context("Adapter state read worker failed")?
}

fn read_state_sync(git_dir: &Path) -> Result<AdapterState> {
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

fn write_state_durably_sync(git_dir: &Path, state: &AdapterState) -> Result<()> {
    write_state_durably_sync_inner(git_dir, state, |_, _| Ok(()))
}

fn write_state_durably_sync_inner<F>(
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

enum StateTransaction<T> {
    Unchanged(T),
    Persist(T),
}

async fn with_locked_state<T, F>(git_dir: &Path, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut AdapterState) -> Result<StateTransaction<T>> + Send + 'static,
{
    let git_dir = git_dir.to_owned();
    STATE_LOCK
        .run_locked_blocking(&adapter_state_dir(&git_dir), move || {
            let mut state = read_state_sync(&git_dir)?;
            match operation(&mut state)? {
                StateTransaction::Unchanged(result) => Ok(result),
                StateTransaction::Persist(result) => {
                    write_state_durably_sync(&git_dir, &state)?;
                    Ok(result)
                }
            }
        })
        .await
}

fn allocate_pending_start(
    state: &mut AdapterState,
    key: &AttemptKey,
    turn_id: &str,
    tool_name: &str,
) -> AdapterAttempt {
    let attempt_seq = state.next_attempt_seq;
    let attempt = AdapterAttempt {
        attempt_seq,
        scope_id: format_codex_scope_id(attempt_seq, key),
        session_id: key.session_id.clone(),
        turn_id: turn_id.to_string(),
        agent_id: key.agent_id.clone(),
        tool_use_id: key.tool_use_id.clone(),
        tool_name: tool_name.to_string(),
        phase: AttemptPhase::PendingStart,
    };
    state.attempts.push(attempt.clone());
    state.next_attempt_seq += 1;
    attempt
}

pub(crate) async fn admit_tracked_attempt(
    git_dir: &Path,
    key: &AttemptKey,
    turn_id: &str,
    tool_name: &str,
) -> Result<AdmitDecision> {
    let key = key.clone();
    let turn_id = turn_id.to_string();
    let tool_name = tool_name.to_string();
    with_locked_state(git_dir, move |state| {
        match state.recovery {
            RecoveryState::Flushing { .. } => {
                return Ok(StateTransaction::Unchanged(AdmitDecision::RecoveryBlocked))
            }
            RecoveryState::Pending { generation } => {
                if !state.attempts.is_empty() {
                    return Ok(StateTransaction::Unchanged(AdmitDecision::RecoveryBlocked));
                }
                state.recovery = RecoveryState::Flushing { generation };
                return Ok(StateTransaction::Persist(AdmitDecision::FlushClaimed {
                    generation,
                }));
            }
            RecoveryState::Clear => {}
        }

        if let Some(existing) = state
            .attempts
            .iter()
            .find(|attempt| attempt.matches_key(&key))
        {
            return Ok(StateTransaction::Unchanged(AdmitDecision::Admitted(
                AllocatedAttempt {
                    attempt: existing.clone(),
                    reused: true,
                },
            )));
        }

        if state.attempts.iter().any(|attempt| {
            attempt.in_builtin_lane(&key.session_id, &turn_id) && !attempt.matches_key(&key)
        }) {
            return Ok(StateTransaction::Unchanged(
                AdmitDecision::StalePredecessorBlocked,
            ));
        }

        if state
            .attempts
            .iter()
            .any(|attempt| attempt.phase == AttemptPhase::PendingStart)
        {
            return Ok(StateTransaction::Unchanged(
                AdmitDecision::UncertainAttemptBlocked,
            ));
        }

        let attempt = allocate_pending_start(state, &key, &turn_id, &tool_name);
        Ok(StateTransaction::Persist(AdmitDecision::Admitted(
            AllocatedAttempt {
                attempt,
                reused: false,
            },
        )))
    })
    .await
}

pub(crate) async fn mark_active(git_dir: &Path, scope_id: &str) -> Result<()> {
    let scope_id = scope_id.to_string();
    with_locked_state(git_dir, move |state| {
        let attempt = state
            .attempts
            .iter_mut()
            .find(|attempt| attempt.scope_id == scope_id)
            .ok_or_else(|| anyhow!("No adapter-state attempt found for scope_id '{scope_id}'"))?;
        attempt.phase = AttemptPhase::Active;
        Ok(StateTransaction::Persist(()))
    })
    .await
}

pub(crate) async fn remove_attempt(git_dir: &Path, scope_id: &str) -> Result<()> {
    let scope_id = scope_id.to_string();
    with_locked_state(git_dir, move |state| {
        let before = state.attempts.len();
        state
            .attempts
            .retain(|attempt| attempt.scope_id != scope_id);
        if state.attempts.len() == before {
            return Ok(StateTransaction::Unchanged(()));
        }
        Ok(StateTransaction::Persist(()))
    })
    .await
}

pub(crate) async fn normalize_recovery_after_boundary_lock_acquired(git_dir: &Path) -> Result<()> {
    with_locked_state(git_dir, |state| {
        if let RecoveryState::Flushing { generation } = state.recovery {
            state.recovery = RecoveryState::Pending { generation };
            return Ok(StateTransaction::Persist(()));
        }
        Ok(StateTransaction::Unchanged(()))
    })
    .await
}

pub(crate) async fn arm_recovery(git_dir: &Path) -> Result<u64> {
    with_locked_state(git_dir, |state| {
        let generation = match state.recovery {
            RecoveryState::Pending { generation } => generation,
            RecoveryState::Clear | RecoveryState::Flushing { .. } => {
                let generation = state.next_recovery_generation;
                state.next_recovery_generation += 1;
                generation
            }
        };
        state.recovery = RecoveryState::Pending { generation };
        Ok(StateTransaction::Persist(generation))
    })
    .await
}

pub(crate) async fn complete_recovery_flush(
    git_dir: &Path,
    generation: u64,
) -> Result<RecoveryFlushCompletion> {
    with_locked_state(git_dir, move |state| match state.recovery {
        RecoveryState::Flushing { generation: owned } if owned == generation => {
            state.recovery = RecoveryState::Clear;
            Ok(StateTransaction::Persist(RecoveryFlushCompletion::Cleared))
        }
        _ => Ok(StateTransaction::Unchanged(
            RecoveryFlushCompletion::Superseded,
        )),
    })
    .await
}

pub(crate) async fn relinquish_recovery_flush(git_dir: &Path, generation: u64) -> Result<()> {
    with_locked_state(git_dir, move |state| {
        if let RecoveryState::Flushing { generation: owned } = state.recovery {
            if owned == generation {
                state.recovery = RecoveryState::Pending { generation: owned };
                return Ok(StateTransaction::Persist(()));
            }
        }
        Ok(StateTransaction::Unchanged(()))
    })
    .await
}
