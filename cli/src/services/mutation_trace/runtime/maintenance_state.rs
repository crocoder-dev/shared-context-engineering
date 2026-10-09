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
    pub recommended: bool,
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

fn failure_outstanding(state: &MaintenanceState) -> bool {
    state.last_attempt_outcome == Some(AttemptOutcome::Failed) || state.consecutive_failures > 0
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
    let advised = usable(state.last_advised, now)
        .filter(|timestamp| age_ms(*timestamp, now) < RECONCILIATION_ADVISORY_AFTER_MS);
    let failure_outstanding = failure_outstanding(state);
    let failure_reminded = advised.is_some_and(|advised_at| {
        usable(state.last_attempt, now).is_none_or(|attempt| advised_at >= attempt)
    });

    Evaluation {
        recommended: due || failure_outstanding || invalid_timestamps,
        advise: (due && advised.is_none()) || (failure_outstanding && !failure_reminded),
        invalid_timestamps,
        reference_age_ms,
    }
}

pub(super) fn evaluate_recommendation(state: &MaintenanceState, now: i64) -> Recommendation {
    let evaluation = evaluate(state, now);
    let age_of = |timestamp: Option<i64>| usable(timestamp, now).map(|value| age_ms(value, now));

    Recommendation {
        recommended: evaluation.recommended,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PersistPhase {
    Serialize,
    CreateStaging,
    WriteStaging,
    SyncStaging,
    Rename,
    OpenParentDirectory,
    SyncParentDirectory,
}

#[derive(Debug)]
pub(super) enum PersistFailure {
    NotApplied {
        phase: PersistPhase,
        source: std::io::Error,
        staging_cleanup: Option<std::io::Error>,
    },
    DurabilityUncertain {
        phase: PersistPhase,
        source: std::io::Error,
    },
}

impl std::fmt::Display for PersistFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistFailure::NotApplied {
                phase,
                source,
                staging_cleanup,
            } => {
                write!(
                    f,
                    "maintenance state was not recorded ({phase:?}): {source}; previous state preserved"
                )?;
                if let Some(cleanup) = staging_cleanup {
                    write!(f, "; staging file cleanup failed: {cleanup}")?;
                }
                Ok(())
            }
            PersistFailure::DurabilityUncertain { phase, source } => write!(
                f,
                "maintenance state was renamed into place but crash durability is unconfirmed ({phase:?}): {source}; the new state may be visible"
            ),
        }
    }
}

impl std::error::Error for PersistFailure {}

pub(super) trait StateFilesystem {
    fn create_staging(&self, directory: &Path) -> std::io::Result<PathBuf>;
    fn write_staging(&self, staging: &Path, bytes: &[u8]) -> std::io::Result<()>;
    fn sync_staging(&self, staging: &Path) -> std::io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn open_directory(&self, directory: &Path) -> std::io::Result<std::fs::File>;
    fn sync_directory(&self, directory: &std::fs::File) -> std::io::Result<()>;
    fn remove_staging(&self, staging: &Path) -> std::io::Result<()>;
}

pub(super) struct RealStateFilesystem;

impl StateFilesystem for RealStateFilesystem {
    fn create_staging(&self, directory: &Path) -> std::io::Result<PathBuf> {
        create_staging_file(directory)
    }

    fn write_staging(&self, staging: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).open(staging)?;
        file.write_all(bytes)
    }

    fn sync_staging(&self, staging: &Path) -> std::io::Result<()> {
        std::fs::OpenOptions::new()
            .write(true)
            .open(staging)?
            .sync_data()
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    fn open_directory(&self, directory: &Path) -> std::io::Result<std::fs::File> {
        std::fs::File::open(directory)
    }

    fn sync_directory(&self, directory: &std::fs::File) -> std::io::Result<()> {
        directory.sync_all()
    }

    fn remove_staging(&self, staging: &Path) -> std::io::Result<()> {
        std::fs::remove_file(staging)
    }
}

pub(super) fn write_state_atomically(
    path: &Path,
    state: &MaintenanceState,
) -> std::result::Result<(), PersistFailure> {
    write_state_atomically_with(&RealStateFilesystem, path, state)
}

fn not_applied(
    phase: PersistPhase,
    source: std::io::Error,
    staging_cleanup: Option<std::io::Error>,
) -> PersistFailure {
    PersistFailure::NotApplied {
        phase,
        source,
        staging_cleanup,
    }
}

pub(super) fn write_state_atomically_with<F: StateFilesystem>(
    fs: &F,
    path: &Path,
    state: &MaintenanceState,
) -> std::result::Result<(), PersistFailure> {
    let bytes = serde_json::to_vec(state)
        .map_err(|error| not_applied(PersistPhase::Serialize, error.into(), None))?;
    let directory = path.parent().ok_or_else(|| {
        not_applied(
            PersistPhase::CreateStaging,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "maintenance state path has no parent directory",
            ),
            None,
        )
    })?;

    let staging = fs
        .create_staging(directory)
        .map_err(|error| not_applied(PersistPhase::CreateStaging, error, None))?;

    let staged = fs
        .write_staging(&staging, &bytes)
        .map_err(|error| (PersistPhase::WriteStaging, error))
        .and_then(|()| {
            fs.sync_staging(&staging)
                .map_err(|error| (PersistPhase::SyncStaging, error))
        })
        .and_then(|()| {
            fs.rename(&staging, path)
                .map_err(|error| (PersistPhase::Rename, error))
        });
    if let Err((phase, source)) = staged {
        return Err(not_applied(
            phase,
            source,
            fs.remove_staging(&staging).err(),
        ));
    }

    let directory_handle =
        fs.open_directory(directory)
            .map_err(|source| PersistFailure::DurabilityUncertain {
                phase: PersistPhase::OpenParentDirectory,
                source,
            })?;
    fs.sync_directory(&directory_handle)
        .map_err(|source| PersistFailure::DurabilityUncertain {
            phase: PersistPhase::SyncParentDirectory,
            source,
        })
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

#[cfg(test)]
pub(super) struct FaultInjectingFilesystem {
    pub fail_at: Option<PersistPhase>,
    pub fail_staging_cleanup: bool,
}

#[cfg(test)]
impl FaultInjectingFilesystem {
    pub(super) const fn failing_at(phase: PersistPhase) -> Self {
        Self {
            fail_at: Some(phase),
            fail_staging_cleanup: false,
        }
    }

    fn check(&self, phase: PersistPhase) -> std::io::Result<()> {
        if self.fail_at == Some(phase) {
            return Err(std::io::Error::other(format!("injected {phase:?} failure")));
        }
        Ok(())
    }
}

#[cfg(test)]
impl StateFilesystem for FaultInjectingFilesystem {
    fn create_staging(&self, directory: &Path) -> std::io::Result<PathBuf> {
        self.check(PersistPhase::CreateStaging)?;
        RealStateFilesystem.create_staging(directory)
    }

    fn write_staging(&self, staging: &Path, bytes: &[u8]) -> std::io::Result<()> {
        self.check(PersistPhase::WriteStaging)?;
        RealStateFilesystem.write_staging(staging, bytes)
    }

    fn sync_staging(&self, staging: &Path) -> std::io::Result<()> {
        self.check(PersistPhase::SyncStaging)?;
        RealStateFilesystem.sync_staging(staging)
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.check(PersistPhase::Rename)?;
        RealStateFilesystem.rename(from, to)
    }

    fn open_directory(&self, directory: &Path) -> std::io::Result<std::fs::File> {
        self.check(PersistPhase::OpenParentDirectory)?;
        RealStateFilesystem.open_directory(directory)
    }

    fn sync_directory(&self, directory: &std::fs::File) -> std::io::Result<()> {
        self.check(PersistPhase::SyncParentDirectory)?;
        RealStateFilesystem.sync_directory(directory)
    }

    fn remove_staging(&self, staging: &Path) -> std::io::Result<()> {
        if self.fail_staging_cleanup {
            return Err(std::io::Error::other("injected staging cleanup failure"));
        }
        RealStateFilesystem.remove_staging(staging)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        read_state, record_success, write_state_atomically, write_state_atomically_with,
        FaultInjectingFilesystem, MaintenanceState, PersistFailure, PersistPhase, StateRead,
        StoredReport,
    };

    const NOW: i64 = 1_000_000_000_000;

    fn state(marker: usize) -> MaintenanceState {
        record_success(
            &MaintenanceState::default(),
            NOW,
            StoredReport {
                retained: marker,
                deleted: 0,
                local_required: 0,
            },
        )
    }

    fn state_file(dir: &Path) -> PathBuf {
        dir.join("ref-maintenance.json")
    }

    fn stored_marker(path: &Path) -> Option<usize> {
        match read_state(path).expect("read state") {
            StateRead::Valid(state) => state.last_report.map(|report| report.retained),
            StateRead::Absent => None,
            StateRead::Unusable => panic!("state unusable"),
        }
    }

    fn staging_leftovers(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("staging"))
            .count()
    }

    fn assert_not_applied(
        phase: PersistPhase,
        previous_marker: Option<usize>,
    ) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = state_file(dir.path());
        if let Some(marker) = previous_marker {
            write_state_atomically(&path, &state(marker)).expect("seed");
        }

        let fs = FaultInjectingFilesystem::failing_at(phase);
        let failure =
            write_state_atomically_with(&fs, &path, &state(99)).expect_err("injected failure");
        let PersistFailure::NotApplied {
            phase: reported,
            staging_cleanup,
            ..
        } = failure
        else {
            panic!("expected NotApplied for {phase:?}, got {failure:?}");
        };
        assert_eq!(reported, phase);
        assert!(staging_cleanup.is_none());
        assert_eq!(stored_marker(&path), previous_marker);
        assert_eq!(staging_leftovers(dir.path()), 0);
        (dir, path)
    }

    #[test]
    fn maintenance_state_write_succeeds_end_to_end() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = state_file(dir.path());
        write_state_atomically(&path, &state(1)).expect("first write");
        write_state_atomically(&path, &state(2)).expect("second write");
        assert_eq!(stored_marker(&path), Some(2));
        assert_eq!(staging_leftovers(dir.path()), 0);
    }

    #[test]
    fn maintenance_state_staging_creation_failure_preserves_previous_state() {
        assert_not_applied(PersistPhase::CreateStaging, Some(1));
    }

    #[test]
    fn maintenance_state_staging_write_failure_preserves_previous_state_and_cleans_up() {
        assert_not_applied(PersistPhase::WriteStaging, Some(1));
    }

    #[test]
    fn maintenance_state_staging_sync_failure_preserves_previous_state_and_cleans_up() {
        assert_not_applied(PersistPhase::SyncStaging, Some(1));
    }

    #[test]
    fn maintenance_state_rename_failure_preserves_previous_state_and_cleans_up() {
        assert_not_applied(PersistPhase::Rename, Some(1));
    }

    #[test]
    fn maintenance_state_rename_failure_without_previous_state_leaves_no_state() {
        assert_not_applied(PersistPhase::Rename, None);
    }

    #[test]
    fn maintenance_state_parent_directory_open_failure_is_durability_uncertain() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = state_file(dir.path());
        write_state_atomically(&path, &state(1)).expect("seed");

        let fs = FaultInjectingFilesystem::failing_at(PersistPhase::OpenParentDirectory);
        let failure = write_state_atomically_with(&fs, &path, &state(2)).expect_err("failure");
        assert!(matches!(
            failure,
            PersistFailure::DurabilityUncertain {
                phase: PersistPhase::OpenParentDirectory,
                ..
            }
        ));
        assert!(failure.to_string().contains("may be visible"));
        assert_eq!(stored_marker(&path), Some(2));
        assert_eq!(staging_leftovers(dir.path()), 0);
    }

    #[test]
    fn maintenance_state_parent_directory_sync_failure_distinguishes_visibility_from_durability() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = state_file(dir.path());
        write_state_atomically(&path, &state(1)).expect("seed");

        let fs = FaultInjectingFilesystem::failing_at(PersistPhase::SyncParentDirectory);
        let failure = write_state_atomically_with(&fs, &path, &state(2)).expect_err("failure");
        assert!(matches!(
            failure,
            PersistFailure::DurabilityUncertain {
                phase: PersistPhase::SyncParentDirectory,
                ..
            }
        ));
        assert!(!failure.to_string().contains("previous state preserved"));
        assert_eq!(stored_marker(&path), Some(2));
        assert_eq!(staging_leftovers(dir.path()), 0);

        write_state_atomically(&path, &state(3)).expect("recovery write");
        assert_eq!(stored_marker(&path), Some(3));
    }

    #[test]
    fn maintenance_state_recovers_after_interrupted_pre_rename_write() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = state_file(dir.path());
        write_state_atomically(&path, &state(1)).expect("seed");

        let fs = FaultInjectingFilesystem {
            fail_at: Some(PersistPhase::Rename),
            fail_staging_cleanup: true,
        };
        let failure = write_state_atomically_with(&fs, &path, &state(2)).expect_err("failure");
        let PersistFailure::NotApplied {
            staging_cleanup, ..
        } = failure
        else {
            panic!("expected NotApplied");
        };
        assert!(staging_cleanup.is_some());
        assert_eq!(staging_leftovers(dir.path()), 1);
        assert_eq!(stored_marker(&path), Some(1));

        write_state_atomically(&path, &state(3)).expect("recovery write");
        assert_eq!(stored_marker(&path), Some(3));
    }
}
