#![allow(
    dead_code,
    reason = "maintenance entrypoints are wired by later tasks of context/plans/mutation-cursor-ref-reconciliation-wiring.md"
)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub(super) const RECONCILIATION_ADVISORY_AFTER_MS: i64 = 24 * 60 * 60 * 1000;
pub(super) const MAINTENANCE_STATE_MAX_BYTES: u64 = 4096;
pub(super) const FUTURE_SKEW_TOLERANCE_MS: i64 = 5 * 60 * 1000;
pub(super) const MAX_FAILURE_MESSAGE_CHARS: usize = 256;

const MAINTENANCE_STATE_VERSION: u32 = 1;
const SCE_RUNTIME_DIR: &str = "sce";
const MAINTENANCE_STATE_FILE: &str = "ref-maintenance.json";
const STAGING_ATTEMPTS: u16 = 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AttemptOutcome {
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct StoredReport {
    pub retained: usize,
    pub deleted: usize,
    pub local_required: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct StoredFailure {
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct MaintenanceState {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_advised: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_outcome: Option<AttemptOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<StoredReport>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<StoredFailure>,
}

impl Default for MaintenanceState {
    fn default() -> Self {
        Self {
            version: MAINTENANCE_STATE_VERSION,
            anchor: None,
            last_success: None,
            last_attempt: None,
            last_advised: None,
            last_attempt_outcome: None,
            last_report: None,
            consecutive_failures: 0,
            last_failure: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum StateRead {
    Absent,
    Unusable,
    Valid(MaintenanceState),
}

impl StateRead {
    pub(super) fn into_state(self) -> MaintenanceState {
        match self {
            StateRead::Valid(state) => state,
            StateRead::Absent | StateRead::Unusable => MaintenanceState::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Evaluation {
    pub due: bool,
    pub advise: bool,
    pub invalid_timestamps: bool,
    pub reference_age_ms: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Recommendation {
    pub recommended: bool,
    pub invalid_timestamps: bool,
    pub last_success_age_ms: Option<i64>,
    pub last_attempt_age_ms: Option<i64>,
    pub last_advised_age_ms: Option<i64>,
    pub last_attempt_outcome: Option<AttemptOutcome>,
    pub consecutive_failures: u32,
}

pub(super) fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

pub(super) fn state_path(git_dir: &Path) -> PathBuf {
    git_dir.join(SCE_RUNTIME_DIR).join(MAINTENANCE_STATE_FILE)
}

fn is_valid_timestamp(timestamp: i64, now: i64) -> bool {
    timestamp <= now.saturating_add(FUTURE_SKEW_TOLERANCE_MS)
}

fn age_ms(timestamp: i64, now: i64) -> i64 {
    now.saturating_sub(timestamp).max(0)
}

fn usable(timestamp: Option<i64>, now: i64) -> Option<i64> {
    timestamp.filter(|value| is_valid_timestamp(*value, now))
}

pub(super) fn read_state(path: &Path) -> std::io::Result<StateRead> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(StateRead::Absent);
        }
        Err(error) => return Err(error),
    };

    let mut bytes = Vec::new();
    file.take(MAINTENANCE_STATE_MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAINTENANCE_STATE_MAX_BYTES) {
        return Ok(StateRead::Unusable);
    }

    Ok(match serde_json::from_slice::<MaintenanceState>(&bytes) {
        Ok(state) if state.version == MAINTENANCE_STATE_VERSION => StateRead::Valid(state),
        Ok(_) | Err(_) => StateRead::Unusable,
    })
}

pub(super) fn evaluate(state: &MaintenanceState, now: i64) -> Evaluation {
    let timestamps = [
        state.anchor,
        state.last_success,
        state.last_attempt,
        state.last_advised,
    ];
    let invalid_timestamps = timestamps
        .iter()
        .flatten()
        .any(|timestamp| !is_valid_timestamp(*timestamp, now));

    let reference = usable(state.last_success, now).or_else(|| usable(state.anchor, now));
    let reference_lost =
        reference.is_none() && (state.last_success.is_some() || state.anchor.is_some());
    let reference_age_ms = reference.map(|timestamp| age_ms(timestamp, now));
    let due = reference_lost
        || reference_age_ms.is_some_and(|age| age >= RECONCILIATION_ADVISORY_AFTER_MS);
    let advised_recently = usable(state.last_advised, now)
        .is_some_and(|timestamp| age_ms(timestamp, now) < RECONCILIATION_ADVISORY_AFTER_MS);

    Evaluation {
        due,
        advise: due && !advised_recently,
        invalid_timestamps,
        reference_age_ms,
    }
}

pub(super) fn evaluate_recommendation(state: &MaintenanceState, now: i64) -> Recommendation {
    let evaluation = evaluate(state, now);
    let age_of = |timestamp: Option<i64>| usable(timestamp, now).map(|value| age_ms(value, now));

    Recommendation {
        recommended: evaluation.due || evaluation.invalid_timestamps,
        invalid_timestamps: evaluation.invalid_timestamps,
        last_success_age_ms: age_of(state.last_success),
        last_attempt_age_ms: age_of(state.last_attempt),
        last_advised_age_ms: age_of(state.last_advised),
        last_attempt_outcome: state.last_attempt_outcome,
        consecutive_failures: state.consecutive_failures,
    }
}

pub(super) fn normalize(state: &MaintenanceState, now: i64) -> MaintenanceState {
    let mut normalized = state.clone();
    normalized.anchor = usable(state.anchor, now);
    normalized.last_success = usable(state.last_success, now);
    normalized.last_attempt = usable(state.last_attempt, now);
    normalized.last_advised = usable(state.last_advised, now);
    normalized
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AdvisoryDecision {
    Anchored,
    NoAction,
    Advised,
}

pub(super) fn plan_advisory(
    read: &StateRead,
    now: i64,
) -> (AdvisoryDecision, Option<MaintenanceState>) {
    let StateRead::Valid(state) = read else {
        let anchored = MaintenanceState {
            anchor: Some(now),
            ..MaintenanceState::default()
        };
        return (AdvisoryDecision::Anchored, Some(anchored));
    };

    let evaluation = evaluate(state, now);
    let mut next = normalize(state, now);
    let decision = if evaluation.advise {
        next.last_advised = Some(now);
        if next.anchor.is_none() && next.last_success.is_none() {
            next.anchor = Some(now);
        }
        AdvisoryDecision::Advised
    } else if next.anchor.is_none() && next.last_success.is_none() {
        next.anchor = Some(now);
        AdvisoryDecision::Anchored
    } else {
        AdvisoryDecision::NoAction
    };

    let changed = next != *state;
    (decision, changed.then_some(next))
}

pub(super) fn record_success(
    state: &MaintenanceState,
    now: i64,
    report: StoredReport,
) -> MaintenanceState {
    let mut next = normalize(state, now);
    next.last_attempt = Some(now);
    next.last_success = Some(now);
    next.last_attempt_outcome = Some(AttemptOutcome::Completed);
    next.last_report = Some(report);
    next.consecutive_failures = 0;
    next.last_failure = None;
    next
}

pub(super) fn record_failure(
    state: &MaintenanceState,
    now: i64,
    kind: &str,
    message: &str,
) -> MaintenanceState {
    let mut next = normalize(state, now);
    next.last_attempt = Some(now);
    next.last_attempt_outcome = Some(AttemptOutcome::Failed);
    next.consecutive_failures = state.consecutive_failures.saturating_add(1);
    next.last_failure = Some(StoredFailure {
        kind: kind.to_string(),
        message: message.chars().take(MAX_FAILURE_MESSAGE_CHARS).collect(),
    });
    next
}

pub(super) fn write_state_atomically<R>(
    path: &Path,
    state: &MaintenanceState,
    rename: R,
) -> std::io::Result<()>
where
    R: FnOnce(&Path, &Path) -> std::io::Result<()>,
{
    let bytes = serde_json::to_vec(state).map_err(std::io::Error::other)?;
    let directory = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "maintenance state path has no parent directory",
        )
    })?;

    let staging = create_staging_file(directory)?;
    let staged = write_staged(&staging, &bytes).and_then(|()| rename(&staging, path));
    if let Err(error) = staged {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }

    if let Ok(directory_handle) = std::fs::File::open(directory) {
        let _ = directory_handle.sync_all();
    }
    Ok(())
}

fn write_staged(staging: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).open(staging)?;
    file.write_all(bytes)?;
    file.sync_data()
}

fn create_staging_file(directory: &Path) -> std::io::Result<PathBuf> {
    let epoch_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos();

    for attempt in 0..STAGING_ATTEMPTS {
        let candidate = directory.join(format!(
            ".{MAINTENANCE_STATE_FILE}.staging-{epoch_nanos}-{}-{attempt}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique maintenance state staging file",
    ))
}

pub(super) fn derive_git_dir(start: &Path) -> Option<PathBuf> {
    if ["GIT_DIR", "GIT_COMMON_DIR", "GIT_WORK_TREE"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return None;
    }

    for ancestor in start.ancestors() {
        let dot_git = ancestor.join(".git");
        let Ok(metadata) = std::fs::symlink_metadata(&dot_git) else {
            continue;
        };

        let git_dir = if metadata.is_dir() {
            dot_git
        } else if metadata.is_file() {
            let contents = std::fs::read_to_string(&dot_git).ok()?;
            let target = contents.lines().next()?.strip_prefix("gitdir:")?.trim();
            let target = Path::new(target);
            if target.is_absolute() {
                target.to_path_buf()
            } else {
                ancestor.join(target)
            }
        } else {
            return None;
        };

        return git_dir
            .is_dir()
            .then(|| std::fs::canonicalize(&git_dir).ok())
            .flatten();
    }

    None
}
