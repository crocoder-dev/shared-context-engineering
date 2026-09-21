use super::types::{
    DoctorFixResultRecord, DoctorProblem, FixResult, ProblemCategory, ProblemFixability,
};
use super::HookDoctorReport;

pub(super) fn build_manual_fix_results(report: &HookDoctorReport) -> Vec<DoctorFixResultRecord> {
    report
        .problems
        .iter()
        .filter(|problem| problem.fixability == ProblemFixability::ManualOnly)
        .map(|problem| DoctorFixResultRecord {
            category: problem.category,
            outcome: FixResult::Manual,
            detail: manual_fix_detail(problem),
        })
        .collect()
}

fn manual_fix_detail(problem: &DoctorProblem) -> String {
    if problem.category == ProblemCategory::MutationScopeHealth {
        problem.remediation.clone()
    } else {
        format!("{} Manual remediation is still required.", problem.summary)
    }
}
