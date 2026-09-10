use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::os_lock::{AdvisoryLockError, OsAdvisoryLock};
use super::{format_opencode_scope_id, AttemptKey};

const SCE_STATE_DIR: &str = "sce";
const ADAPTER_STATE_FILE: &str = "opencode-mutation-scope-state.json";
const ADAPTER_STATE_LOCK_FILE: &str = "opencode-mutation-scope-state.lock";
const STATE_LOCK_WHAT: &str = "adapter-state";

const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

const ADAPTER_STATE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttemptPhase {
    PendingStart,
    Active,
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
    pub scope_id: String,
    pub session_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub phase: AttemptPhase,
}

impl AdapterAttempt {
    fn matches_key(&self, key: &AttemptKey) -> bool {
        self.session_id == key.session_id && self.call_id == key.call_id
    }
}

fn default_recovery_generation() -> u64 {
    1
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AdapterState {
    pub version: u32,
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

fn state_path(git_dir: &Path) -> PathBuf {
    adapter_state_dir(git_dir).join(ADAPTER_STATE_FILE)
}

fn lock_path(git_dir: &Path) -> PathBuf {
    adapter_state_dir(git_dir).join(ADAPTER_STATE_LOCK_FILE)
}

struct AdapterStateLock {
    _inner: OsAdvisoryLock,
}

impl AdapterStateLock {
    fn acquire(git_dir: &Path, timeout: Duration) -> Result<AdapterStateLock, AdvisoryLockError> {
        let inner = OsAdvisoryLock::acquire(
            &adapter_state_dir(git_dir),
            lock_path(git_dir),
            timeout,
            STATE_LOCK_WHAT,
        )?;
        Ok(AdapterStateLock { _inner: inner })
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

fn acquire_lock(git_dir: &Path) -> Result<AdapterStateLock> {
    AdapterStateLock::acquire(git_dir, DEFAULT_LOCK_TIMEOUT)
        .map_err(|err| anyhow!("Failed to acquire adapter-state lock: {err}"))
}

fn allocate_pending_start(
    state: &mut AdapterState,
    key: &AttemptKey,
    tool_name: &str,
) -> AdapterAttempt {
    let attempt = AdapterAttempt {
        scope_id: format_opencode_scope_id(key),
        session_id: key.session_id.clone(),
        call_id: key.call_id.clone(),
        tool_name: tool_name.to_string(),
        phase: AttemptPhase::PendingStart,
    };
    state.attempts.push(attempt.clone());
    attempt
}

pub(crate) fn admit_tracked_attempt(
    git_dir: &Path,
    key: &AttemptKey,
    tool_name: &str,
) -> Result<AdmitDecision> {
    let _lock = acquire_lock(git_dir)?;
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

    if state.attempts.iter().any(|attempt| {
        matches!(
            attempt.phase,
            AttemptPhase::PendingStart | AttemptPhase::PendingAbandon
        )
    }) {
        return Ok(AdmitDecision::UncertainAttemptBlocked);
    }

    let attempt = allocate_pending_start(&mut state, key, tool_name);
    write_state_durably(git_dir, &state)?;
    Ok(AdmitDecision::Admitted(AllocatedAttempt {
        attempt,
        reused: false,
    }))
}

pub(crate) fn mark_active(git_dir: &Path, scope_id: &str) -> Result<()> {
    let _lock = acquire_lock(git_dir)?;

    let mut state = read_state(git_dir)?;
    let attempt = state
        .attempts
        .iter_mut()
        .find(|attempt| attempt.scope_id == scope_id)
        .ok_or_else(|| anyhow!("No adapter-state attempt found for scope_id '{scope_id}'"))?;
    attempt.phase = AttemptPhase::Active;
    write_state_durably(git_dir, &state)
}

pub(crate) fn remove_attempt(git_dir: &Path, scope_id: &str) -> Result<()> {
    let _lock = acquire_lock(git_dir)?;

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

pub(crate) fn normalize_recovery_after_boundary_lock_acquired(git_dir: &Path) -> Result<()> {
    let _lock = acquire_lock(git_dir)?;
    let mut state = read_state(git_dir)?;

    if let RecoveryState::Flushing { generation } = state.recovery {
        state.recovery = RecoveryState::Pending { generation };
        write_state_durably(git_dir, &state)?;
    }
    Ok(())
}

pub(crate) fn arm_recovery(git_dir: &Path) -> Result<u64> {
    let _lock = acquire_lock(git_dir)?;
    let mut state = read_state(git_dir)?;

    let generation = match state.recovery {
        RecoveryState::Pending { generation } => generation,
        RecoveryState::Clear | RecoveryState::Flushing { .. } => {
            let generation = state.next_recovery_generation;
            state.next_recovery_generation += 1;
            generation
        }
    };
    state.recovery = RecoveryState::Pending { generation };
    write_state_durably(git_dir, &state)?;
    Ok(generation)
}

pub(crate) fn begin_terminal_cleanup(git_dir: &Path, scope_ids: &[String]) -> Result<u64> {
    let _lock = acquire_lock(git_dir)?;
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

pub(crate) fn arm_and_begin_recovery_flush(git_dir: &Path) -> Result<u64> {
    let _lock = acquire_lock(git_dir)?;
    let mut state = read_state(git_dir)?;

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

pub(crate) fn complete_recovery_flush(
    git_dir: &Path,
    generation: u64,
) -> Result<RecoveryFlushCompletion> {
    let _lock = acquire_lock(git_dir)?;
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

pub(crate) fn relinquish_recovery_flush(git_dir: &Path, generation: u64) -> Result<()> {
    let _lock = acquire_lock(git_dir)?;
    let mut state = read_state(git_dir)?;

    if let RecoveryState::Flushing { generation: owned } = state.recovery {
        if owned == generation {
            state.recovery = RecoveryState::Pending { generation: owned };
            write_state_durably(git_dir, &state)?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn seed_attempt_for_tests(
    git_dir: &Path,
    key: &AttemptKey,
    tool_name: &str,
    phase: AttemptPhase,
) -> AdapterAttempt {
    let _lock = acquire_lock(git_dir).expect("test seed lock");
    let mut state = read_state(git_dir).expect("test seed read");
    allocate_pending_start(&mut state, key, tool_name);
    let seeded = state.attempts.last_mut().expect("attempt was just pushed");
    seeded.phase = phase;
    let attempt = seeded.clone();
    write_state_durably(git_dir, &state).expect("test seed write");
    attempt
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use super::*;

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-opencode-mutation-scope-state-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    fn key(session_id: &str, call_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            call_id: call_id.to_string(),
        }
    }

    fn admit(git_dir: &Path, key: &AttemptKey, tool_name: &str) -> AdmitDecision {
        admit_tracked_attempt(git_dir, key, tool_name).expect("admit should not error")
    }

    fn admit_and_activate(git_dir: &Path, key: &AttemptKey, tool_name: &str) -> AdapterAttempt {
        match admit(git_dir, key, tool_name) {
            AdmitDecision::Admitted(allocated) => {
                mark_active(git_dir, &allocated.attempt.scope_id)
                    .expect("mark_active should succeed");
                allocated.attempt
            }
            other => panic!("expected Admitted, got {other:?}"),
        }
    }

    #[test]
    fn read_state_returns_default_when_file_is_absent() {
        let git_dir = unique_test_git_dir("read-default");

        let state = read_state(&git_dir).expect("missing state file should read as default");
        assert_eq!(state, AdapterState::default());
        assert_eq!(state.version, 1);
        assert!(state.recovery.is_clear());
        assert_eq!(state.next_recovery_generation, 1);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_persists_the_pending_start_attempt_before_returning() {
        let git_dir = unique_test_git_dir("admit-persists-pending-start");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let AdmitDecision::Admitted(allocated) = admit(&git_dir, &key("ses-1", "call-1"), "write")
        else {
            panic!("expected Admitted");
        };

        let state = read_state(&git_dir).expect("state should be readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0].scope_id, allocated.attempt.scope_id);
        assert_eq!(state.attempts[0].phase, AttemptPhase::PendingStart);
        assert_eq!(state.attempts[0].call_id, "call-1");

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_state_is_checkout_local_to_its_git_dir() {
        let git_dir_a = unique_test_git_dir("checkout-local-a");
        let git_dir_b = unique_test_git_dir("checkout-local-b");
        std::fs::create_dir_all(&git_dir_a).expect("git dir A should be created");
        std::fs::create_dir_all(&git_dir_b).expect("git dir B should be created");

        admit_and_activate(&git_dir_a, &key("ses-1", "call-1"), "bash");
        admit_and_activate(&git_dir_a, &key("ses-1", "call-2"), "bash");
        admit_and_activate(&git_dir_b, &key("ses-1", "call-1"), "bash");

        assert_eq!(
            read_state(&git_dir_a)
                .expect("state A readable")
                .attempts
                .len(),
            2
        );
        assert_eq!(
            read_state(&git_dir_b)
                .expect("state B readable")
                .attempts
                .len(),
            1
        );

        remove_test_git_dir(&git_dir_a);
        remove_test_git_dir(&git_dir_b);
    }

    #[test]
    fn duplicate_live_delivery_reuses_the_same_attempt_and_scope_id() {
        let git_dir = unique_test_git_dir("duplicate-reuse");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt_key = key("ses-1", "call-1");

        let AdmitDecision::Admitted(first) = admit(&git_dir, &attempt_key, "bash") else {
            panic!("first admission should be Admitted");
        };
        assert!(!first.reused);

        let AdmitDecision::Admitted(second) = admit(&git_dir, &attempt_key, "bash") else {
            panic!("duplicate delivery should still be Admitted");
        };
        assert!(second.reused);
        assert_eq!(first.attempt.scope_id, second.attempt.scope_id);

        let state = read_state(&git_dir).expect("state should be readable");
        assert_eq!(state.attempts.len(), 1);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_allows_a_new_key_alongside_an_active_attempt_in_the_same_session() {
        let git_dir = unique_test_git_dir("admit-alongside-active");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        admit_and_activate(&git_dir, &key("ses-1", "call-a"), "bash");

        let AdmitDecision::Admitted(second) = admit(&git_dir, &key("ses-1", "call-b"), "bash")
        else {
            panic!("D9: a distinct concurrent call may run alongside an Active attempt");
        };
        assert!(!second.reused);
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts.len(),
            2
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_blocks_a_successor_while_an_unresolved_pending_start_exists() {
        let git_dir = unique_test_git_dir("admit-blocks-on-pending-start");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        seed_attempt_for_tests(
            &git_dir,
            &key("ses-1", "call-a"),
            "bash",
            AttemptPhase::PendingStart,
        );

        assert_eq!(
            admit(&git_dir, &key("ses-1", "call-b"), "bash"),
            AdmitDecision::UncertainAttemptBlocked,
            "a lingering unconfirmed PendingStart from a crashed invocation must fail closed"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_never_treats_duplicate_delivery_of_a_pending_start_as_uncertain() {
        let git_dir = unique_test_git_dir("admit-duplicate-pending-start");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt_key = key("ses-1", "call-1");

        seed_attempt_for_tests(&git_dir, &attempt_key, "bash", AttemptPhase::PendingStart);

        let AdmitDecision::Admitted(again) = admit(&git_dir, &attempt_key, "bash") else {
            panic!("re-delivery of the same key must reuse, never UncertainAttemptBlocked");
        };
        assert!(again.reused);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_claims_the_flush_even_while_attempts_remain_outstanding() {
        let git_dir = unique_test_git_dir("admit-flush-with-outstanding");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        seed_attempt_for_tests(
            &git_dir,
            &key("ses-1", "call-live"),
            "bash",
            AttemptPhase::Active,
        );
        let generation = arm_recovery(&git_dir).expect("arming recovery should succeed");

        assert_eq!(
            admit(&git_dir, &key("ses-1", "call-new"), "bash"),
            AdmitDecision::FlushClaimed { generation },
            "a recovery/flush operation must be allowed to consume the ambiguous interval \
             even while other attempts remain live",
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn arm_and_begin_recovery_flush_claims_flushing_regardless_of_pending_generation() {
        let git_dir = unique_test_git_dir("arm-and-begin-flush");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let clear_gen = arm_and_begin_recovery_flush(&git_dir).expect("arm from clear");
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Flushing {
                generation: clear_gen
            },
        );

        relinquish_recovery_flush(&git_dir, clear_gen).expect("relinquish");
        let pending_gen = arm_and_begin_recovery_flush(&git_dir).expect("arm from pending");
        assert_eq!(pending_gen, clear_gen, "a pending generation is reused");
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Flushing {
                generation: pending_gen
            },
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn a_stale_flush_completion_never_clears_a_newer_recovery_generation() {
        let git_dir = unique_test_git_dir("stale-flush-completion");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let generation_a = arm_and_begin_recovery_flush(&git_dir).expect("claim generation A");
        assert_eq!(generation_a, 1);

        let generation_b = arm_recovery(&git_dir).expect("a newer abandonment arms generation B");
        assert_eq!(generation_b, 2);
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 2 },
        );

        assert_eq!(
            complete_recovery_flush(&git_dir, generation_a).expect("the stale completion returns"),
            RecoveryFlushCompletion::Superseded,
        );
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 2 },
            "generation A's completion must not clear generation B's recovery requirement",
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_claims_the_flush_when_recovery_is_quiescent() {
        let git_dir = unique_test_git_dir("admit-claims-flush");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let generation = arm_recovery(&git_dir).expect("arming recovery should succeed");

        assert_eq!(
            admit(&git_dir, &key("ses-1", "call-new"), "bash"),
            AdmitDecision::FlushClaimed { generation },
        );
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Flushing { generation },
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn only_one_concurrent_caller_claims_the_flush_for_a_generation() {
        let git_dir = unique_test_git_dir("one-flush-owner");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let generation = arm_recovery(&git_dir).expect("arming recovery should succeed");

        let handles: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|suffix| {
                let git_dir = git_dir.clone();
                thread::spawn(move || {
                    admit_tracked_attempt(
                        &git_dir,
                        &key("ses-1", &format!("call-{suffix}")),
                        "bash",
                    )
                    .expect("admit should not error")
                })
            })
            .collect();

        let decisions: Vec<AdmitDecision> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread should not panic"))
            .collect();

        let flush_claims = decisions
            .iter()
            .filter(|decision| matches!(decision, AdmitDecision::FlushClaimed { .. }))
            .count();
        let blocked = decisions
            .iter()
            .filter(|decision| matches!(decision, AdmitDecision::RecoveryBlocked))
            .count();
        assert_eq!(flush_claims, 1, "exactly one process may claim Flushing(g)");
        assert_eq!(
            blocked, 1,
            "the other concurrent caller must stay fail-closed"
        );
        assert_eq!(
            decisions.iter().find_map(|decision| match decision {
                AdmitDecision::FlushClaimed { generation } => Some(*generation),
                _ => None,
            }),
            Some(generation),
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn arm_recovery_moves_clear_to_pending_with_a_fresh_generation() {
        let git_dir = unique_test_git_dir("arm-clear-to-pending");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let generation = arm_recovery(&git_dir).expect("arming should succeed");
        assert_eq!(generation, 1);

        let state = read_state(&git_dir).expect("state readable");
        assert_eq!(state.recovery, RecoveryState::Pending { generation: 1 });
        assert_eq!(state.next_recovery_generation, 2);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn arm_recovery_keeps_the_same_generation_when_already_pending() {
        let git_dir = unique_test_git_dir("arm-pending-idempotent");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        assert_eq!(arm_recovery(&git_dir).expect("first arm"), 1);
        assert_eq!(arm_recovery(&git_dir).expect("second arm"), 1);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn arm_recovery_supersedes_a_flushing_generation_with_a_newer_one() {
        let git_dir = unique_test_git_dir("arm-supersedes-flushing");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        arm_recovery(&git_dir).expect("arm g1");
        admit(&git_dir, &key("ses-1", "call-new"), "bash");
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Flushing { generation: 1 },
        );

        let generation = arm_recovery(&git_dir).expect("re-arm during flush");
        assert_eq!(generation, 2);
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 2 },
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn complete_recovery_flush_clears_only_with_the_matching_generation() {
        let git_dir = unique_test_git_dir("complete-matching-generation");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        arm_recovery(&git_dir).expect("arm");
        admit(&git_dir, &key("ses-1", "call-new"), "bash");

        assert_eq!(
            complete_recovery_flush(&git_dir, 2).expect("wrong-generation completion"),
            RecoveryFlushCompletion::Superseded,
        );
        assert_eq!(
            complete_recovery_flush(&git_dir, 1).expect("matching completion"),
            RecoveryFlushCompletion::Cleared,
        );
        assert!(read_state(&git_dir)
            .expect("state readable")
            .recovery
            .is_clear());

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn normalize_after_boundary_lock_reclaims_orphaned_flushing_to_pending_same_generation() {
        let git_dir = unique_test_git_dir("normalize-orphaned-flushing");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        arm_recovery(&git_dir).expect("arm g1");
        admit(&git_dir, &key("ses-1", "call-new"), "bash");
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Flushing { generation: 1 },
        );

        normalize_recovery_after_boundary_lock_acquired(&git_dir)
            .expect("normalize should succeed");

        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 1 },
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn relinquish_recovery_flush_returns_a_claimed_generation_to_pending() {
        let git_dir = unique_test_git_dir("relinquish-to-pending");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        arm_recovery(&git_dir).expect("arm");
        admit(&git_dir, &key("ses-1", "call-new"), "bash");

        relinquish_recovery_flush(&git_dir, 1).expect("relinquish should succeed");
        assert_eq!(
            read_state(&git_dir).expect("state readable").recovery,
            RecoveryState::Pending { generation: 1 },
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn mark_active_transitions_phase_and_rejects_unknown_scope_ids() {
        let git_dir = unique_test_git_dir("mark-active");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let AdmitDecision::Admitted(allocated) = admit(&git_dir, &key("ses-1", "call-1"), "bash")
        else {
            panic!("expected Admitted");
        };

        mark_active(&git_dir, &allocated.attempt.scope_id).expect("mark_active should succeed");
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts[0].phase,
            AttemptPhase::Active,
        );

        let error = mark_active(&git_dir, "oc-tool-v1|s=1:x|c=1:y")
            .expect_err("marking an unknown scope active must be rejected");
        assert!(error.to_string().contains("No adapter-state attempt found"));

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn removing_an_already_removed_attempt_is_a_safe_no_op() {
        let git_dir = unique_test_git_dir("remove-idempotent");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt = admit_and_activate(&git_dir, &key("ses-1", "call-1"), "bash");

        remove_attempt(&git_dir, &attempt.scope_id).expect("first removal should succeed");
        remove_attempt(&git_dir, &attempt.scope_id)
            .expect("duplicate terminal delivery after cleanup must be a safe no-op");

        assert!(read_state(&git_dir)
            .expect("state readable")
            .attempts
            .is_empty());

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn state_round_trips_durably_through_the_canonical_path() {
        let git_dir = unique_test_git_dir("round-trip");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let attempt = admit_and_activate(&git_dir, &key("ses-7", "call-9"), "apply_patch");
        arm_recovery(&git_dir).expect("arming recovery should succeed");

        let reloaded = read_state(&git_dir).expect("state should reload");
        assert_eq!(reloaded.version, ADAPTER_STATE_VERSION);
        assert_eq!(reloaded.recovery, RecoveryState::Pending { generation: 1 });
        assert_eq!(reloaded.attempts.len(), 1);
        assert_eq!(reloaded.attempts[0].phase, AttemptPhase::Active);
        assert_eq!(reloaded.attempts[0].tool_name, "apply_patch");
        assert_eq!(reloaded.attempts[0].scope_id, attempt.scope_id);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn malformed_state_file_is_rejected_without_fabricating_bookkeeping() {
        let git_dir = unique_test_git_dir("malformed-json");
        let dir = adapter_state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(state_path(&git_dir), b"not json")
            .expect("malformed file should be written");

        let error = read_state(&git_dir).expect_err("malformed state file must be rejected");
        assert!(error.to_string().contains("malformed"));

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn an_unsupported_version_state_file_is_rejected() {
        let git_dir = unique_test_git_dir("unsupported-version");
        let dir = adapter_state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(
            state_path(&git_dir),
            serde_json::json!({
                "version": 99,
                "next_recovery_generation": 1,
                "recovery": { "phase": "clear" },
                "attempts": []
            })
            .to_string(),
        )
        .expect("state file should be written");

        let error = read_state(&git_dir).expect_err("unsupported version must be rejected");
        assert!(error.to_string().contains("unsupported version"));

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn interruption_before_rename_leaves_the_canonical_path_unaffected() {
        let git_dir = unique_test_git_dir("interrupted-before-rename");
        let dir = adapter_state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");

        let state = AdapterState::default();
        let result = write_state_durably_inner(&git_dir, &state, |tmp_path, canonical_path| {
            assert!(tmp_path.exists());
            assert!(!canonical_path.exists());
            Err(anyhow!("injected interruption before rename"))
        });

        assert!(result.is_err());
        assert!(!state_path(&git_dir).exists());
        assert_eq!(
            read_state(&git_dir).expect("read should not error on an absent canonical file"),
            AdapterState::default(),
        );

        remove_test_git_dir(&git_dir);
    }

    const PARALLEL_ADMISSION_COUNT: usize = 6;

    #[test]
    fn parallel_admissions_serialize_and_converge_without_lost_updates() {
        let git_dir = unique_test_git_dir("parallel-admissions");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let handles: Vec<_> = (0..PARALLEL_ADMISSION_COUNT)
            .map(|index| {
                let git_dir = git_dir.clone();
                thread::spawn(move || {
                    let attempt_key = key("ses-1", &format!("call-{index}"));
                    loop {
                        match admit_tracked_attempt(&git_dir, &attempt_key, "bash")
                            .expect("admit should not error")
                        {
                            AdmitDecision::Admitted(allocated) => {
                                mark_active(&git_dir, &allocated.attempt.scope_id)
                                    .expect("mark_active should succeed");
                                break allocated.attempt.scope_id;
                            }
                            AdmitDecision::UncertainAttemptBlocked => {
                                thread::sleep(Duration::from_millis(5));
                            }
                            other => panic!("unexpected admission decision: {other:?}"),
                        }
                    }
                })
            })
            .collect();

        let mut scope_ids: Vec<String> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread should not panic"))
            .collect();
        scope_ids.sort_unstable();
        scope_ids.dedup();
        assert_eq!(scope_ids.len(), PARALLEL_ADMISSION_COUNT);

        let state = read_state(&git_dir).expect("state should be readable");
        assert_eq!(state.attempts.len(), PARALLEL_ADMISSION_COUNT);
        assert!(state
            .attempts
            .iter()
            .all(|a| a.phase == AttemptPhase::Active));

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn adapter_state_files_live_only_below_git_dir_sce() {
        let git_dir = unique_test_git_dir("path-boundary");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        admit(&git_dir, &key("ses-1", "call-1"), "bash");

        let sce_dir = git_dir.join(SCE_STATE_DIR);
        assert!(state_path(&git_dir).starts_with(&sce_dir));
        assert!(lock_path(&git_dir).starts_with(&sce_dir));

        remove_test_git_dir(&git_dir);
    }
}
