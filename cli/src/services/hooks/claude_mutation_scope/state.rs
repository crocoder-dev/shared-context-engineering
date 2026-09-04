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

fn state_path(git_dir: &Path) -> PathBuf {
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
    attempt.phase = AttemptPhase::Active;
    write_state_durably(git_dir, &state)
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use super::*;

    static NEXT_TEST_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_git_dir(label: &str) -> PathBuf {
        let id = NEXT_TEST_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-claude-mutation-scope-state-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn remove_test_git_dir(git_dir: &Path) {
        let _ = std::fs::remove_dir_all(git_dir);
    }

    fn key(session_id: &str, agent_id: Option<&str>, tool_use_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            tool_use_id: tool_use_id.to_string(),
        }
    }

    #[test]
    fn read_state_returns_default_when_file_is_absent() {
        let git_dir = unique_test_git_dir("read-default");

        let state = read_state(&git_dir).expect("missing state file should read as default");
        assert_eq!(state, AdapterState::default());

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn attempt_seq_allocation_is_sequential_across_distinct_keys() {
        let git_dir = unique_test_git_dir("sequential-allocation");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let first = allocate_attempt(&git_dir, &key("session-1", None, "toolu_1"), "Write")
            .expect("first allocation should succeed");
        let second = allocate_attempt(&git_dir, &key("session-1", None, "toolu_2"), "Bash")
            .expect("second allocation should succeed");
        let third = allocate_attempt(
            &git_dir,
            &key("session-1", Some("agent-1"), "toolu_3"),
            "Edit",
        )
        .expect("third allocation should succeed");

        assert_eq!(first.attempt.attempt_seq, 1);
        assert_eq!(second.attempt.attempt_seq, 2);
        assert_eq!(third.attempt.attempt_seq, 3);
        assert!(!first.reused);
        assert!(!second.reused);
        assert!(!third.reused);

        let state = read_state(&git_dir).expect("state should be readable");
        assert_eq!(state.next_attempt_seq, 4);
        assert_eq!(state.attempts.len(), 3);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn duplicate_live_attempt_reuses_the_same_attempt_seq_and_scope_id() {
        let git_dir = unique_test_git_dir("duplicate-reuse");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt_key = key("session-1", None, "toolu_1");

        let first = allocate_attempt(&git_dir, &attempt_key, "Write")
            .expect("first allocation should succeed");
        let second = allocate_attempt(&git_dir, &attempt_key, "Write")
            .expect("duplicate delivery should still succeed");

        assert!(!first.reused, "the first allocation is not a reuse");
        assert!(
            second.reused,
            "AC4: duplicate live delivery must be reported as reused"
        );
        assert_eq!(
            first.attempt.attempt_seq, second.attempt.attempt_seq,
            "AC4: duplicate delivery must reuse the same attempt_seq"
        );
        assert_eq!(
            first.attempt.scope_id, second.attempt.scope_id,
            "AC4: duplicate delivery must reuse the same ScopeId"
        );

        let state = read_state(&git_dir).expect("state should be readable");
        assert_eq!(
            state.attempts.len(),
            1,
            "reusing a live attempt must not create a second bookkeeping entry"
        );
        assert_eq!(
            state.next_attempt_seq, 2,
            "reusing a live attempt must not advance the monotonic counter"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn a_terminal_attempt_is_followed_by_a_fresh_allocation_never_reusing_the_scope_id() {
        let git_dir = unique_test_git_dir("terminal-then-fresh");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let attempt_key = key("session-1", None, "toolu_1");

        let first = allocate_attempt(&git_dir, &attempt_key, "Write")
            .expect("first allocation should succeed");
        mark_active(&git_dir, &first.attempt.scope_id).expect("attempt should become active");
        remove_attempt(&git_dir, &first.attempt.scope_id)
            .expect("terminal attempt should be removable");

        let second = allocate_attempt(&git_dir, &attempt_key, "Write")
            .expect("a later execution of the same tool_use_id should allocate a fresh attempt");

        assert!(
            !second.reused,
            "the attempt is a fresh allocation, not a reuse"
        );
        assert_ne!(
            first.attempt.attempt_seq, second.attempt.attempt_seq,
            "AC5: attempt_seq must never be reused after the prior attempt became terminal"
        );
        assert_ne!(
            first.attempt.scope_id, second.attempt.scope_id,
            "AC5: a terminal ScopeId must never be reused"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn mark_active_transitions_phase_from_pending_start_to_active() {
        let git_dir = unique_test_git_dir("mark-active");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let allocated = allocate_attempt(&git_dir, &key("session-1", None, "toolu_1"), "Write")
            .expect("allocation should succeed");
        assert_eq!(allocated.attempt.phase, AttemptPhase::PendingStart);

        mark_active(&git_dir, &allocated.attempt.scope_id).expect("mark_active should succeed");

        let state = read_state(&git_dir).expect("state should be readable");
        let persisted = state
            .attempts
            .iter()
            .find(|attempt| attempt.scope_id == allocated.attempt.scope_id)
            .expect("attempt should still be present");
        assert_eq!(persisted.phase, AttemptPhase::Active);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn removing_an_already_removed_attempt_is_a_safe_no_op() {
        let git_dir = unique_test_git_dir("remove-idempotent");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");
        let allocated = allocate_attempt(&git_dir, &key("session-1", None, "toolu_1"), "Write")
            .expect("allocation should succeed");

        remove_attempt(&git_dir, &allocated.attempt.scope_id)
            .expect("first removal should succeed");
        remove_attempt(&git_dir, &allocated.attempt.scope_id)
            .expect("D9: duplicate terminal delivery after cleanup must be a safe no-op");

        let state = read_state(&git_dir).expect("state should be readable");
        assert!(state.attempts.is_empty());

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn malformed_state_file_is_rejected_without_fabricating_bookkeeping() {
        let git_dir = unique_test_git_dir("malformed-json");
        let dir = state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(state_path(&git_dir), b"not json")
            .expect("malformed file should be written");

        let error = read_state(&git_dir).expect_err("malformed state file must be rejected");
        assert!(error.to_string().contains("malformed"));

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn unsupported_version_is_rejected_without_fabricating_bookkeeping() {
        let git_dir = unique_test_git_dir("unsupported-version");
        let dir = state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(
            state_path(&git_dir),
            serde_json::json!({
                "version": 99,
                "next_attempt_seq": 1,
                "recovery_pending": false,
                "attempts": []
            })
            .to_string(),
        )
        .expect("state file with unsupported version should be written");

        let error = read_state(&git_dir).expect_err("unsupported version must be rejected");
        assert!(error.to_string().contains("unsupported version"));

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn interruption_before_rename_leaves_the_canonical_path_unaffected() {
        let git_dir = unique_test_git_dir("interrupted-before-rename");
        let dir = state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");

        let state = AdapterState {
            next_attempt_seq: 5,
            ..AdapterState::default()
        };
        let result = write_state_durably_inner(&git_dir, &state, |tmp_path, canonical_path| {
            assert!(
                tmp_path.exists(),
                "temp file should exist by the time the pre-rename hook runs"
            );
            assert!(
                !canonical_path.exists(),
                "canonical path should still be absent at the pre-rename hook"
            );
            Err(anyhow!("injected interruption before rename"))
        });

        assert!(
            result.is_err(),
            "write_state_durably_inner should surface the injected interruption"
        );
        assert!(
            !state_path(&git_dir).exists(),
            "atomic replacement: canonical state path must stay absent when interrupted before rename"
        );

        let after =
            read_state(&git_dir).expect("read should not error on an absent canonical file");
        assert_eq!(
            after,
            AdapterState::default(),
            "an interrupted write must not partially apply"
        );

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn a_leftover_lock_file_with_no_active_os_lock_does_not_block_a_new_acquirer() {
        let git_dir = unique_test_git_dir("leftover-lock-file");
        let dir = state_dir(&git_dir);
        std::fs::create_dir_all(&dir).expect("state dir should be created");
        std::fs::write(lock_path(&git_dir), b"leftover")
            .expect("leftover lock file should be writable");

        let result = allocate_attempt(&git_dir, &key("session-1", None, "toolu_1"), "Write");
        assert!(
            result.is_ok(),
            "a lock file with no active OS lock held against it must not block a new acquirer"
        );

        remove_test_git_dir(&git_dir);
    }

    const PARALLEL_WRITER_COUNT: u64 = 8;

    #[test]
    fn parallel_writers_for_distinct_keys_all_converge_without_lost_updates() {
        let git_dir = unique_test_git_dir("parallel-writers");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let handles: Vec<_> = (0..PARALLEL_WRITER_COUNT)
            .map(|index| {
                let git_dir = git_dir.clone();
                thread::spawn(move || {
                    let attempt_key = key("session-1", None, &format!("toolu_{index}"));
                    allocate_attempt(&git_dir, &attempt_key, "Write")
                        .expect("each concurrent allocation should durably succeed")
                })
            })
            .collect();

        let allocated: Vec<AllocatedAttempt> = handles
            .into_iter()
            .map(|handle| handle.join().expect("writer thread should not panic"))
            .collect();

        let mut attempt_seqs: Vec<u64> = allocated.iter().map(|a| a.attempt.attempt_seq).collect();
        attempt_seqs.sort_unstable();
        attempt_seqs.dedup();
        assert_eq!(
            attempt_seqs.len(),
            usize::try_from(PARALLEL_WRITER_COUNT).unwrap(),
            "concurrent writers must not lose updates or collide on attempt_seq"
        );

        let state = read_state(&git_dir).expect("state should be readable after concurrent writes");
        assert_eq!(
            state.attempts.len(),
            usize::try_from(PARALLEL_WRITER_COUNT).unwrap()
        );
        assert_eq!(state.next_attempt_seq, PARALLEL_WRITER_COUNT + 1);

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn a_second_acquirer_blocks_until_the_first_releases() {
        use std::sync::mpsc;

        let git_dir = unique_test_git_dir("lock-contention");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        let holder = AdapterStateLock::acquire(&git_dir, Duration::from_secs(5))
            .expect("first acquirer should succeed immediately");

        let (result_tx, result_rx) = mpsc::channel();
        let git_dir_clone = git_dir.clone();
        let handle = thread::spawn(move || {
            let result = AdapterStateLock::acquire(&git_dir_clone, Duration::from_secs(5));
            let _ = result_tx.send(());
            result
        });

        assert!(
            result_rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "second acquirer should not succeed while the first still holds the lock"
        );

        drop(holder);

        result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second acquirer should complete once the first releases the lock");
        assert!(handle
            .join()
            .expect("second acquirer thread should not panic")
            .is_ok());

        remove_test_git_dir(&git_dir);
    }

    #[test]
    fn adapter_state_files_live_only_below_git_dir_sce() {
        let git_dir = unique_test_git_dir("path-boundary");
        std::fs::create_dir_all(&git_dir).expect("git dir should be created");

        allocate_attempt(&git_dir, &key("session-1", None, "toolu_1"), "Write")
            .expect("allocation should succeed");

        let sce_dir = git_dir.join(SCE_STATE_DIR);
        assert!(state_path(&git_dir).starts_with(&sce_dir));
        assert!(lock_path(&git_dir).starts_with(&sce_dir));

        let mut found_state_file = false;
        for entry in std::fs::read_dir(&sce_dir).expect("sce dir should be readable") {
            let entry = entry.expect("dir entry should be readable");
            assert!(
                entry.path().starts_with(&sce_dir),
                "AC19: adapter state must live only under <git-dir>/sce/"
            );
            if entry.path() == state_path(&git_dir) {
                found_state_file = true;
            }
        }
        assert!(
            found_state_file,
            "state file should exist under <git-dir>/sce/"
        );

        remove_test_git_dir(&git_dir);
    }
}
