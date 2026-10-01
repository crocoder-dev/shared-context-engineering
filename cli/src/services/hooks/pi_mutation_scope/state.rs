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

fn acquire_lock(git_dir: &Path) -> Result<OsAdvisoryLock> {
    STATE_LOCK.acquire(&adapter_state_dir(git_dir))
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

pub(crate) fn find_definitely_dead_attempts(git_dir: &Path) -> Result<Vec<String>> {
    let _lock = acquire_lock(git_dir)?;
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

pub(crate) fn mark_executed(git_dir: &Path, key: &AttemptKey) -> Result<()> {
    let _lock = acquire_lock(git_dir)?;

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
pub(crate) fn set_attempt_owner_for_tests(
    git_dir: &Path,
    scope_id: &str,
    owner: ProcessOwner,
) -> AdapterAttempt {
    let _lock = acquire_lock(git_dir).expect("test owner-override lock");
    let mut state = read_state(git_dir).expect("test owner-override read");
    let attempt = state
        .attempts
        .iter_mut()
        .find(|attempt| attempt.scope_id == scope_id)
        .expect("attempt to override must already exist");
    attempt.owner = owner;
    let updated = attempt.clone();
    write_state_durably(git_dir, &state).expect("test owner-override write");
    updated
}

#[cfg(test)]
mod state_conformance {
    use super::*;
    use crate::services::hooks::mutation_scope_state_conformance::{
        mutation_scope_state_conformance_tests, CompletionView, FlushClaimView, RecoveryView,
        StateConformance,
    };

    const FIXTURE_SESSION: &str = "ses-conformance";
    const FIXTURE_PHASES: [AttemptPhase; 3] = [
        AttemptPhase::Executed,
        AttemptPhase::PendingAbandon,
        AttemptPhase::PendingStart,
    ];

    struct PiStateConformance;

    fn fixture_key(index: usize) -> AttemptKey {
        AttemptKey {
            session_id: FIXTURE_SESSION.to_string(),
            tool_call_id: format!("call-{index}"),
        }
    }

    impl StateConformance for PiStateConformance {
        type State = AdapterState;

        const ADAPTER: &'static str = "pi";
        const SUPPORTED_VERSION: u32 = ADAPTER_STATE_VERSION;

        fn state_path(git_dir: &Path) -> PathBuf {
            state_path(git_dir)
        }

        fn fixture_state(
            recovery: RecoveryView,
            next_recovery_generation: u64,
            attempt_count: usize,
        ) -> AdapterState {
            let mut state = AdapterState {
                next_recovery_generation,
                recovery: match recovery {
                    RecoveryView::Clear => RecoveryState::Clear,
                    RecoveryView::Pending(generation) => RecoveryState::Pending { generation },
                    RecoveryView::Flushing(generation) => RecoveryState::Flushing { generation },
                },
                ..AdapterState::default()
            };
            for index in 0..attempt_count {
                allocate_pending_start(&mut state, &fixture_key(index), "write");
                state.attempts[index].phase = FIXTURE_PHASES[index % FIXTURE_PHASES.len()];
            }
            state
        }

        fn read_state(git_dir: &Path) -> Result<AdapterState> {
            read_state(git_dir)
        }

        fn persist(git_dir: &Path, state: &AdapterState) -> Result<()> {
            write_state_durably(git_dir, state)
        }

        fn persist_with_before_rename_hook<F>(
            git_dir: &Path,
            state: &AdapterState,
            before_rename: F,
        ) -> Result<()>
        where
            F: FnOnce(&Path, &Path) -> Result<()>,
        {
            write_state_durably_inner(git_dir, state, before_rename)
        }

        fn recovery_view(state: &AdapterState) -> RecoveryView {
            match state.recovery {
                RecoveryState::Clear => RecoveryView::Clear,
                RecoveryState::Pending { generation } => RecoveryView::Pending(generation),
                RecoveryState::Flushing { generation } => RecoveryView::Flushing(generation),
            }
        }

        fn next_recovery_generation(state: &AdapterState) -> u64 {
            state.next_recovery_generation
        }

        fn scope_ids(state: &AdapterState) -> Vec<String> {
            state
                .attempts
                .iter()
                .map(|attempt| attempt.scope_id.clone())
                .collect()
        }

        fn remove_attempt(git_dir: &Path, scope_id: &str) -> Result<()> {
            remove_attempt(git_dir, scope_id)
        }

        fn normalize_recovery_after_boundary_lock_acquired(git_dir: &Path) -> Result<()> {
            normalize_recovery_after_boundary_lock_acquired(git_dir)
        }

        fn complete_recovery_flush(git_dir: &Path, generation: u64) -> Result<CompletionView> {
            Ok(match complete_recovery_flush(git_dir, generation)? {
                RecoveryFlushCompletion::Cleared => CompletionView::Cleared,
                RecoveryFlushCompletion::Superseded => CompletionView::Superseded,
            })
        }

        fn relinquish_recovery_flush(git_dir: &Path, generation: u64) -> Result<()> {
            relinquish_recovery_flush(git_dir, generation)
        }

        fn claim_flush_by_admitting_a_new_attempt(
            git_dir: &Path,
            contender: usize,
        ) -> Result<FlushClaimView> {
            let decision = admit_tracked_attempt(git_dir, &fixture_key(contender), "bash")?;
            Ok(match decision {
                AdmitDecision::FlushClaimed { generation } => FlushClaimView::Claimed(generation),
                AdmitDecision::RecoveryBlocked => FlushClaimView::Blocked,
                other => FlushClaimView::Other(format!("{other:?}")),
            })
        }
    }

    mutation_scope_state_conformance_tests!(PiStateConformance);
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
            "sce-pi-mutation-scope-state-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    fn key(session_id: &str, tool_call_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
        }
    }

    fn admit(git_dir: &Path, key: &AttemptKey, tool_name: &str) -> AdmitDecision {
        admit_tracked_attempt(git_dir, key, tool_name).expect("admit should not error")
    }

    #[test]
    fn admit_persists_the_pending_start_attempt_with_attempt_seq_one() {
        let git_dir = unique_test_git_dir("admit-persists-pending-start");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let AdmitDecision::Admitted(allocated) = admit(&git_dir, &key("ses-1", "call-1"), "write")
        else {
            panic!("expected Admitted");
        };
        assert_eq!(allocated.attempt.attempt_seq, 1);

        let state = read_state(&git_dir).expect("state should be readable");
        assert_eq!(state.attempts.len(), 1);
        assert_eq!(state.attempts[0], allocated.attempt);
        assert_eq!(state.attempts[0].phase, AttemptPhase::PendingStart);
        assert_eq!(state.attempts[0].session_id, "ses-1");
        assert_eq!(state.attempts[0].tool_call_id, "call-1");
        assert_eq!(state.attempts[0].tool_name, "write");
        assert_eq!(state.next_attempt_seq, 2);

        remove_test_git_dir(&git_dir);
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
    fn a_pending_start_attempt_never_blocks_a_concurrent_new_admission() {
        let git_dir = unique_test_git_dir("pending-start-does-not-block");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        seed_attempt_for_tests(
            &git_dir,
            &key("ses-1", "call-a"),
            "bash",
            AttemptPhase::PendingStart,
        );

        let AdmitDecision::Admitted(second) = admit(&git_dir, &key("ses-1", "call-b"), "bash")
        else {
            panic!(
                "D12: a lingering PendingStart from a still-executing tool call must not block \
                 a genuinely concurrent tool call"
            );
        };
        assert!(!second.reused);
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts.len(),
            2
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_blocks_a_new_admission_while_a_pending_abandon_exists() {
        let git_dir = unique_test_git_dir("admit-blocks-on-pending-abandon");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        seed_attempt_for_tests(
            &git_dir,
            &key("ses-1", "call-a"),
            "bash",
            AttemptPhase::PendingAbandon,
        );

        assert_eq!(
            admit(&git_dir, &key("ses-1", "call-b"), "bash"),
            AdmitDecision::UncertainAttemptBlocked,
            "an unresolved PendingAbandon must fail closed for a new admission"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_rejects_duplicate_delivery_of_a_pending_abandon_key() {
        let git_dir = unique_test_git_dir("admit-duplicate-pending-abandon");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt_key = key("ses-1", "call-1");

        seed_attempt_for_tests(&git_dir, &attempt_key, "bash", AttemptPhase::PendingAbandon);

        assert_eq!(
            admit(&git_dir, &attempt_key, "bash"),
            AdmitDecision::TerminalAttemptBlocked,
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn admit_claims_the_flush_when_recovery_is_pending() {
        let git_dir = unique_test_git_dir("admit-claims-flush");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        seed_attempt_for_tests(
            &git_dir,
            &key("ses-1", "call-doomed"),
            "bash",
            AttemptPhase::PendingAbandon,
        );
        let generation = begin_terminal_cleanup(&git_dir, &[]).expect("arming recovery succeeds");
        relinquish_recovery_flush(&git_dir, generation).expect("relinquish to Pending");

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
    fn mark_executed_transitions_pending_start_only() {
        let git_dir = unique_test_git_dir("mark-executed");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        admit(&git_dir, &key("ses-1", "call-1"), "bash");

        mark_executed(&git_dir, &key("ses-1", "call-1")).expect("mark_executed succeeds");
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts[0].phase,
            AttemptPhase::Executed,
        );

        mark_executed(&git_dir, &key("ses-unknown", "call-unknown"))
            .expect("marking an unknown key executed is a safe no-op");

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn mark_executed_does_not_resurrect_a_pending_abandon_attempt() {
        let git_dir = unique_test_git_dir("mark-executed-pending-abandon");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        seed_attempt_for_tests(
            &git_dir,
            &key("ses-1", "call-1"),
            "bash",
            AttemptPhase::PendingAbandon,
        );

        mark_executed(&git_dir, &key("ses-1", "call-1")).expect("mark_executed should not error");
        assert_eq!(
            read_state(&git_dir).expect("state readable").attempts[0].phase,
            AttemptPhase::PendingAbandon,
            "a late tool_result must never move a PendingAbandon attempt back to Executed"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn a_new_attempt_after_terminal_cleanup_gets_a_fresh_attempt_seq_and_scope_id() {
        let git_dir = unique_test_git_dir("terminal-scope-id-non-reuse");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt_key = key("ses-1", "call-1");

        let AdmitDecision::Admitted(first) = admit(&git_dir, &attempt_key, "bash") else {
            panic!("expected Admitted");
        };
        assert_eq!(first.attempt.attempt_seq, 1);
        remove_attempt(&git_dir, &first.attempt.scope_id).expect("terminal cleanup");

        let AdmitDecision::Admitted(second) = admit(&git_dir, &attempt_key, "bash") else {
            panic!("expected Admitted for the reused toolCallId");
        };
        assert!(!second.reused);
        assert_eq!(second.attempt.attempt_seq, 2);
        assert_ne!(
            first.attempt.scope_id, second.attempt.scope_id,
            "a reused toolCallId after terminal cleanup must never reactivate the old ScopeId"
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
                    admit_tracked_attempt(&git_dir, &attempt_key, "bash")
                        .expect("admit should not error")
                })
            })
            .collect();

        let mut scope_ids: Vec<String> = handles
            .into_iter()
            .map(|handle| {
                let AdmitDecision::Admitted(allocated) =
                    handle.join().expect("thread should not panic")
                else {
                    panic!("D12: every genuinely distinct concurrent call must be admitted");
                };
                allocated.attempt.scope_id
            })
            .collect();
        scope_ids.sort_unstable();
        scope_ids.dedup();
        assert_eq!(scope_ids.len(), PARALLEL_ADMISSION_COUNT);

        let state = read_state(&git_dir).expect("state should be readable");
        assert_eq!(state.attempts.len(), PARALLEL_ADMISSION_COUNT);
        assert!(state
            .attempts
            .iter()
            .all(|a| a.phase == AttemptPhase::PendingStart));

        remove_test_git_dir(&git_dir);
    }
}
