use std::path::Path;

use super::maintenance_state::{
    derive_git_dir, evaluate_recommendation, read_state, state_path, system_now_ms, AttemptOutcome,
    MaintenanceState, PersistFailure, PersistPhase, StateRead,
};
use super::ref_maintenance::{reconcile_explicit, ExplicitOutcome, StatePersistWarning};
use super::ref_reconciliation::{ReconcileError, ReconciliationReport};
use crate::services::agent_trace_db::repository::ExistingRepositoryDbError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationCounts {
    pub(crate) deleted: usize,
    pub(crate) retained: usize,
    pub(crate) local_required: usize,
}

const NOT_APPLIED_MESSAGE: &str = "Maintenance-state update was not applied.";
const DURABILITY_UNCERTAIN_MESSAGE: &str = "Maintenance-state durability could not be confirmed.";
const PREVIOUS_STATE_UNREADABLE_MESSAGE: &str =
    "Maintenance state could not be read; the update was not attempted.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StateWarning {
    NotApplied { phase: &'static str, cause: String },
    DurabilityUncertain { phase: &'static str, cause: String },
    PreviousStateUnreadable { cause: String },
}

impl StateWarning {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            StateWarning::NotApplied { .. } => "not_applied",
            StateWarning::DurabilityUncertain { .. } => "durability_uncertain",
            StateWarning::PreviousStateUnreadable { .. } => "previous_state_unreadable",
        }
    }

    pub(crate) fn phase(&self) -> Option<&'static str> {
        match self {
            StateWarning::NotApplied { phase, .. }
            | StateWarning::DurabilityUncertain { phase, .. } => Some(phase),
            StateWarning::PreviousStateUnreadable { .. } => None,
        }
    }

    pub(crate) fn message(&self) -> &'static str {
        match self {
            StateWarning::NotApplied { .. } => NOT_APPLIED_MESSAGE,
            StateWarning::DurabilityUncertain { .. } => DURABILITY_UNCERTAIN_MESSAGE,
            StateWarning::PreviousStateUnreadable { .. } => PREVIOUS_STATE_UNREADABLE_MESSAGE,
        }
    }

    pub(crate) fn cause(&self) -> &str {
        match self {
            StateWarning::NotApplied { cause, .. }
            | StateWarning::DurabilityUncertain { cause, .. }
            | StateWarning::PreviousStateUnreadable { cause } => cause,
        }
    }
}

impl std::fmt::Display for StateWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())?;
        match self.phase() {
            Some(phase) => write!(f, " Cause ({phase}): {}", self.cause()),
            None => write!(f, " Cause: {}", self.cause()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReconciliationFix {
    Completed(ReconciliationCounts),
    CompletedStatePersistFailed {
        counts: ReconciliationCounts,
        warning: StateWarning,
    },
    Failed {
        kind: &'static str,
        message: String,
        state_warning: Option<StateWarning>,
    },
    Busy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationRecommendation {
    pub(crate) invalid_timestamps: bool,
    pub(crate) state_unusable: bool,
    pub(crate) last_success_age_ms: Option<i64>,
    pub(crate) last_attempt_age_ms: Option<i64>,
    pub(crate) last_advised_age_ms: Option<i64>,
    pub(crate) last_attempt_failed: bool,
    pub(crate) consecutive_failures: u32,
}

pub(crate) async fn run_explicit_reconciliation(repository_root: &Path) -> ReconciliationFix {
    fix_from_outcome(reconcile_explicit(repository_root).await)
}

pub(crate) fn inspect_reconciliation_recommendation(
    repository_root: &Path,
) -> Option<ReconciliationRecommendation> {
    inspect_recommendation_at(repository_root, system_now_ms())
}

fn inspect_recommendation_at(
    repository_root: &Path,
    now: i64,
) -> Option<ReconciliationRecommendation> {
    let git_dir = derive_git_dir(repository_root)?;
    let (state, state_unusable) = match read_state(&state_path(&git_dir)) {
        Ok(StateRead::Valid(state)) => (state, false),
        Ok(StateRead::Absent) => (MaintenanceState::default(), false),
        Ok(StateRead::Unusable) | Err(_) => (MaintenanceState::default(), true),
    };
    let recommendation = evaluate_recommendation(&state, now);
    if !recommendation.recommended && !state_unusable {
        return None;
    }
    Some(ReconciliationRecommendation {
        invalid_timestamps: recommendation.invalid_timestamps,
        state_unusable,
        last_success_age_ms: recommendation.last_success_age_ms,
        last_attempt_age_ms: recommendation.last_attempt_age_ms,
        last_advised_age_ms: recommendation.last_advised_age_ms,
        last_attempt_failed: recommendation.last_attempt_outcome == Some(AttemptOutcome::Failed),
        consecutive_failures: recommendation.consecutive_failures,
    })
}

fn fix_from_outcome(outcome: ExplicitOutcome) -> ReconciliationFix {
    match outcome {
        ExplicitOutcome::Completed(report) => ReconciliationFix::Completed(counts(report)),
        ExplicitOutcome::CompletedStatePersistFailed { report, warning } => {
            ReconciliationFix::CompletedStatePersistFailed {
                counts: counts(report),
                warning: state_warning_from(warning),
            }
        }
        ExplicitOutcome::Failed {
            error,
            state_warning,
        } => ReconciliationFix::Failed {
            kind: failure_kind(&error),
            message: error.to_string(),
            state_warning: state_warning.map(state_warning_from),
        },
        ExplicitOutcome::Skipped(_) => ReconciliationFix::Busy,
    }
}

fn state_warning_from(warning: StatePersistWarning) -> StateWarning {
    match warning {
        StatePersistWarning::PreviousStateUnreadable(error) => {
            StateWarning::PreviousStateUnreadable {
                cause: error.to_string(),
            }
        }
        StatePersistWarning::WriteFailed(PersistFailure::NotApplied { phase, source, .. }) => {
            StateWarning::NotApplied {
                phase: phase_name(phase),
                cause: source.to_string(),
            }
        }
        StatePersistWarning::WriteFailed(PersistFailure::DurabilityUncertain { phase, source }) => {
            StateWarning::DurabilityUncertain {
                phase: phase_name(phase),
                cause: source.to_string(),
            }
        }
    }
}

fn phase_name(phase: PersistPhase) -> &'static str {
    match phase {
        PersistPhase::Serialize => "serialize",
        PersistPhase::CreateStaging => "create_staging",
        PersistPhase::WriteStaging => "write_staging",
        PersistPhase::SyncStaging => "sync_staging",
        PersistPhase::Rename => "rename",
        PersistPhase::OpenParentDirectory => "open_parent_directory",
        PersistPhase::SyncParentDirectory => "sync_parent_directory",
    }
}

fn counts(report: ReconciliationReport) -> ReconciliationCounts {
    ReconciliationCounts {
        deleted: report.deleted,
        retained: report.retained,
        local_required: report.local_required,
    }
}

fn failure_kind(error: &ReconcileError) -> &'static str {
    match error {
        ReconcileError::GitDir(_) => "git_dir_unavailable",
        ReconcileError::Lock(_) => "lock_unavailable",
        ReconcileError::CheckoutIdentity(_) => "worktree_identity_unavailable",
        ReconcileError::AgentTraceDbUnavailable(source) => {
            match source.downcast_ref::<ExistingRepositoryDbError>() {
                Some(ExistingRepositoryDbError::Missing { .. }) => "agent_trace_db_missing",
                Some(ExistingRepositoryDbError::Unreadable(_)) => "agent_trace_db_unreadable",
                Some(ExistingRepositoryDbError::IncompatibleSchema(_)) => {
                    "agent_trace_db_incompatible_schema"
                }
                Some(ExistingRepositoryDbError::MissingMetadata) => {
                    "agent_trace_db_missing_metadata"
                }
                Some(ExistingRepositoryDbError::RepositoryMismatch { .. }) => {
                    "agent_trace_db_repository_mismatch"
                }
                None => "agent_trace_db_unavailable",
            }
        }
        ReconcileError::SnapshotService(_) => "snapshot_service_unavailable",
        ReconcileError::PinInventory(_) => "pin_inventory_failed",
        ReconcileError::MalformedPin { .. } => "malformed_ref",
        ReconcileError::DurableRoots(_) => "durable_roots_unavailable",
        ReconcileError::MissingRequiredPins { .. } => "missing_required_pins",
        ReconcileError::DeleteTransaction(_) => "delete_transaction_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::mutation_trace::runtime::maintenance_state::{
        record_failure, record_success, StoredReport,
    };

    fn write_state(git_dir: &Path, state: &MaintenanceState) {
        let path = state_path(git_dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(state).unwrap()).unwrap();
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        dir
    }

    #[test]
    fn recommendation_is_absent_for_fresh_and_recent_state() {
        let repo = repo();
        assert_eq!(inspect_recommendation_at(repo.path(), 1_000), None);

        let state = record_success(
            &MaintenanceState::default(),
            1_000,
            StoredReport {
                retained: 1,
                deleted: 0,
                local_required: 1,
            },
        );
        write_state(&repo.path().join(".git"), &state);
        assert_eq!(inspect_recommendation_at(repo.path(), 2_000), None);
    }

    #[test]
    fn recommendation_reports_failure_and_never_writes_state() {
        let repo = repo();
        let git_dir = repo.path().join(".git");
        let state = record_failure(&MaintenanceState::default(), 1_000, "kind", "message");
        write_state(&git_dir, &state);
        let before = std::fs::read(state_path(&git_dir)).unwrap();

        let recommendation = inspect_recommendation_at(repo.path(), 2_000).unwrap();
        assert!(recommendation.last_attempt_failed);
        assert_eq!(recommendation.consecutive_failures, 1);
        assert!(!recommendation.state_unusable);
        assert_eq!(std::fs::read(state_path(&git_dir)).unwrap(), before);
    }

    #[test]
    fn recommendation_flags_unusable_and_invalid_state_without_rewriting() {
        let repo = repo();
        let git_dir = repo.path().join(".git");
        let path = state_path(&git_dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json").unwrap();

        let unusable = inspect_recommendation_at(repo.path(), 2_000).unwrap();
        assert!(unusable.state_unusable);
        assert_eq!(std::fs::read(&path).unwrap(), b"not json");

        let state = MaintenanceState {
            last_success: Some(i64::MAX / 2),
            ..MaintenanceState::default()
        };
        write_state(&git_dir, &state);
        let before = std::fs::read(&path).unwrap();
        let invalid = inspect_recommendation_at(repo.path(), 2_000).unwrap();
        assert!(invalid.invalid_timestamps);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    fn report() -> ReconciliationReport {
        ReconciliationReport {
            local_required: 1,
            retained: 2,
            deleted: 3,
        }
    }

    fn io_error() -> std::io::Error {
        std::io::Error::other("injected")
    }

    #[test]
    fn pre_rename_persist_failure_maps_to_not_applied() {
        let fix = fix_from_outcome(ExplicitOutcome::CompletedStatePersistFailed {
            report: report(),
            warning: StatePersistWarning::WriteFailed(PersistFailure::NotApplied {
                phase: PersistPhase::Rename,
                source: io_error(),
                staging_cleanup: None,
            }),
        });
        let ReconciliationFix::CompletedStatePersistFailed { counts, warning } = fix else {
            panic!("expected persist-failed completion");
        };
        assert_eq!(counts.deleted, 3);
        assert_eq!(warning.kind(), "not_applied");
        assert_eq!(warning.phase(), Some("rename"));
        assert_eq!(
            warning.message(),
            "Maintenance-state update was not applied."
        );
    }

    #[test]
    fn post_rename_persist_failure_maps_to_durability_uncertain() {
        let fix = fix_from_outcome(ExplicitOutcome::CompletedStatePersistFailed {
            report: report(),
            warning: StatePersistWarning::WriteFailed(PersistFailure::DurabilityUncertain {
                phase: PersistPhase::SyncParentDirectory,
                source: io_error(),
            }),
        });
        let ReconciliationFix::CompletedStatePersistFailed { warning, .. } = fix else {
            panic!("expected persist-failed completion");
        };
        assert_eq!(warning.kind(), "durability_uncertain");
        assert_eq!(warning.phase(), Some("sync_parent_directory"));
        assert_eq!(
            warning.message(),
            "Maintenance-state durability could not be confirmed."
        );
    }

    #[test]
    fn unreadable_previous_state_maps_to_its_own_warning_on_failure() {
        let fix = fix_from_outcome(ExplicitOutcome::Failed {
            error: ReconcileError::MissingRequiredPins { missing: vec![] },
            state_warning: Some(StatePersistWarning::PreviousStateUnreadable(io_error())),
        });
        let ReconciliationFix::Failed { state_warning, .. } = fix else {
            panic!("expected failure");
        };
        let warning = state_warning.expect("warning retained");
        assert_eq!(warning.kind(), "previous_state_unreadable");
        assert_eq!(warning.phase(), None);
        assert!(!warning.message().contains("not recorded"));
    }

    #[test]
    fn outcomes_map_to_distinct_fix_results() {
        let report = ReconciliationReport {
            local_required: 1,
            retained: 2,
            deleted: 3,
        };
        assert_eq!(
            fix_from_outcome(ExplicitOutcome::Completed(report)),
            ReconciliationFix::Completed(ReconciliationCounts {
                deleted: 3,
                retained: 2,
                local_required: 1,
            })
        );
        assert_eq!(
            fix_from_outcome(ExplicitOutcome::Skipped(
                super::super::ref_maintenance::SkipReason::Busy
            )),
            ReconciliationFix::Busy
        );
        let failed = fix_from_outcome(ExplicitOutcome::Failed {
            error: ReconcileError::MissingRequiredPins { missing: vec![] },
            state_warning: None,
        });
        assert!(matches!(
            failed,
            ReconciliationFix::Failed {
                kind: "missing_required_pins",
                ..
            }
        ));
    }
}
