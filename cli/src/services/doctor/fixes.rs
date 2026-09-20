use super::types::{DoctorFixResultRecord, FixResult, ProblemFixability};
use super::HookDoctorReport;

pub(super) fn build_manual_fix_results(report: &HookDoctorReport) -> Vec<DoctorFixResultRecord> {
    report
        .problems
        .iter()
        .filter(|problem| problem.fixability == ProblemFixability::ManualOnly)
        .map(|problem| DoctorFixResultRecord {
            category: problem.category,
            outcome: FixResult::Manual,
            detail: format!("{} Manual remediation is still required.", problem.summary),
        })
        .collect()
}
