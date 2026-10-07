use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::{format_claude_scope_id, AttemptKey};
use crate::services::hooks::mutation_scope_lock::AdapterLockSpec;

const SCE_STATE_DIR: &str = "sce";
const ADAPTER_STATE_FILE: &str = "claude-mutation-scope-state.json";

const STATE_LOCK: AdapterLockSpec = AdapterLockSpec::state("claude-mutation-scope-state.lock");

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

#[cfg(test)]
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

enum StateTransaction<T> {
    Unchanged(T),
    Persist(T),
}

async fn with_locked_state<T, F>(git_dir: &Path, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut AdapterState) -> Result<StateTransaction<T>> + Send + 'static,
{
    with_locked_state_inner(git_dir, |_, _| Ok(()), operation).await
}

async fn with_locked_state_inner<T, B, F>(
    git_dir: &Path,
    before_rename: B,
    operation: F,
) -> Result<T>
where
    T: Send + 'static,
    B: FnOnce(&Path, &Path) -> Result<()> + Send + 'static,
    F: FnOnce(&mut AdapterState) -> Result<StateTransaction<T>> + Send + 'static,
{
    let git_dir = git_dir.to_owned();
    STATE_LOCK
        .run_locked_blocking(&state_dir(&git_dir), move || {
            let mut state = read_state_sync(&git_dir)?;
            match operation(&mut state)? {
                StateTransaction::Unchanged(result) => Ok(result),
                StateTransaction::Persist(result) => {
                    write_state_durably_sync_inner(&git_dir, &state, before_rename)?;
                    Ok(result)
                }
            }
        })
        .await
}

pub(crate) async fn allocate_attempt(
    git_dir: &Path,
    key: &AttemptKey,
    tool_name: &str,
) -> Result<AllocatedAttempt> {
    let key = key.clone();
    let tool_name = tool_name.to_string();
    with_locked_state(git_dir, move |state| {
        if let Some(existing) = state
            .attempts
            .iter()
            .find(|attempt| attempt.matches_key(&key))
        {
            return Ok(StateTransaction::Unchanged(AllocatedAttempt {
                attempt: existing.clone(),
                reused: true,
            }));
        }

        let attempt_seq = state.next_attempt_seq;
        let scope_id = format_claude_scope_id(attempt_seq, &key);
        let attempt = AdapterAttempt {
            attempt_seq,
            scope_id,
            session_id: key.session_id.clone(),
            agent_id: key.agent_id.clone(),
            tool_use_id: key.tool_use_id.clone(),
            tool_name,
            phase: AttemptPhase::PendingStart,
        };

        state.attempts.push(attempt.clone());
        state.next_attempt_seq += 1;

        Ok(StateTransaction::Persist(AllocatedAttempt {
            attempt,
            reused: false,
        }))
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

        match attempt.phase {
            AttemptPhase::PendingStart => {
                attempt.phase = AttemptPhase::Active;
            }
            AttemptPhase::Active => return Ok(StateTransaction::Unchanged(())),
            AttemptPhase::PendingAbandon => {
                return Err(anyhow!(
                    "Cannot mark mutation-scope attempt '{scope_id}' active after abandonment was established"
                ));
            }
        }

        Ok(StateTransaction::Persist(()))
    })
    .await
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

pub(crate) async fn mark_recovery_pending_and_pending_abandon(
    git_dir: &Path,
    scope_ids: &[String],
) -> Result<()> {
    if scope_ids.is_empty() {
        return Ok(());
    }

    let scope_ids = scope_ids.to_vec();
    with_locked_state(git_dir, move |state| {
        state.recovery_pending = true;
        transition_to_pending_abandon(state, &scope_ids);
        Ok(StateTransaction::Persist(()))
    })
    .await
}

pub(crate) async fn reprove_pending_abandon(git_dir: &Path) -> Result<Option<Vec<AdapterAttempt>>> {
    with_locked_state(git_dir, |state| {
        if !state.recovery_pending || state.attempts.is_empty() {
            return Ok(StateTransaction::Unchanged(None));
        }

        let every_attempt_is_pending_abandon = state
            .attempts
            .iter()
            .all(|attempt| attempt.phase == AttemptPhase::PendingAbandon);

        if !every_attempt_is_pending_abandon {
            return Ok(StateTransaction::Unchanged(None));
        }

        Ok(StateTransaction::Unchanged(Some(state.attempts.clone())))
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

pub(crate) async fn mark_recovery_pending(git_dir: &Path) -> Result<()> {
    with_locked_state(git_dir, |state| {
        state.recovery_pending = true;
        Ok(StateTransaction::Persist(()))
    })
    .await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClearRecoveryOutcome {
    Cleared,
    StillPending,
}

pub(crate) async fn clear_recovery_pending_if_quiescent(
    git_dir: &Path,
) -> Result<ClearRecoveryOutcome> {
    clear_recovery_pending_if_quiescent_inner(git_dir, |_, _| Ok(())).await
}

async fn clear_recovery_pending_if_quiescent_inner<F>(
    git_dir: &Path,
    before_rename: F,
) -> Result<ClearRecoveryOutcome>
where
    F: FnOnce(&Path, &Path) -> Result<()> + Send + 'static,
{
    with_locked_state_inner(git_dir, before_rename, |state| {
        if !state.recovery_pending {
            return Ok(StateTransaction::Unchanged(ClearRecoveryOutcome::Cleared));
        }

        if !state.attempts.is_empty() {
            return Ok(StateTransaction::Unchanged(
                ClearRecoveryOutcome::StillPending,
            ));
        }

        state.recovery_pending = false;
        Ok(StateTransaction::Persist(ClearRecoveryOutcome::Cleared))
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::{
        clear_recovery_pending_if_quiescent_inner, read_state, write_state_durably_sync,
        AdapterState, ClearRecoveryOutcome,
    };

    const INJECTED_WRITE_DELAY: Duration = Duration::from_millis(500);
    const UNRELATED_TIMER: Duration = Duration::from_millis(10);
    const RESPONSIVENESS_BUDGET: Duration = Duration::from_millis(250);
    const HOOK_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

    fn seed_quiescent_recovery_pending(git_dir: &std::path::Path) {
        write_state_durably_sync(
            git_dir,
            &AdapterState {
                recovery_pending: true,
                ..AdapterState::default()
            },
        )
        .expect("seeding adapter state should succeed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn state_transaction_persistence_does_not_starve_the_tokio_worker() {
        let git_dir = tempfile::tempdir().expect("temp dir");
        seed_quiescent_recovery_pending(git_dir.path());

        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let transaction_git_dir = git_dir.path().to_owned();
        let transaction = tokio::spawn(async move {
            clear_recovery_pending_if_quiescent_inner(&transaction_git_dir, move |_, _| {
                let _ = entered_tx.send(Instant::now());
                std::thread::sleep(INJECTED_WRITE_DELAY);
                Ok(())
            })
            .await
        });

        let entered_at = entered_rx.await.expect("transaction should reach the hook");
        let unrelated = tokio::spawn(async {
            tokio::time::sleep(UNRELATED_TIMER).await;
            Instant::now()
        });
        let unrelated_done_at = unrelated.await.expect("unrelated task should complete");

        assert!(
            unrelated_done_at.duration_since(entered_at) < RESPONSIVENESS_BUDGET,
            "unrelated Tokio task was starved for {:?} by the state transaction",
            unrelated_done_at.duration_since(entered_at)
        );

        let outcome = transaction
            .await
            .expect("transaction task should join")
            .expect("transaction should succeed");
        assert_eq!(outcome, ClearRecoveryOutcome::Cleared);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn aborted_caller_does_not_interrupt_a_started_state_transaction() {
        let git_dir = tempfile::tempdir().expect("temp dir");
        seed_quiescent_recovery_pending(git_dir.path());

        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let transaction_git_dir = git_dir.path().to_owned();
        let transaction = tokio::spawn(async move {
            clear_recovery_pending_if_quiescent_inner(&transaction_git_dir, move |_, _| {
                let _ = entered_tx.send(());
                let _ = release_rx.recv_timeout(HOOK_RELEASE_TIMEOUT);
                Ok(())
            })
            .await
        });

        entered_rx.await.expect("transaction should reach the hook");
        transaction.abort();
        assert!(transaction
            .await
            .expect_err("aborted caller should not join successfully")
            .is_cancelled());
        release_tx.send(()).expect("hook should still be waiting");

        let outcome = clear_recovery_pending_if_quiescent_inner(git_dir.path(), |_, _| Ok(()))
            .await
            .expect("state lock should be released and state should parse");
        assert_eq!(outcome, ClearRecoveryOutcome::Cleared);

        let state = read_state(git_dir.path())
            .await
            .expect("persisted state should parse");
        assert!(!state.recovery_pending);
    }
}
