use super::types::{
    DoctorFixResultRecord, DoctorProblem, FixResult, IntegrationTarget, ProblemCategory,
    ProblemFixability,
};
use super::HookDoctorReport;

pub(super) fn build_manual_fix_results(
    report: &HookDoctorReport,
    attempted_mutation_scope_targets: &[IntegrationTarget],
) -> Vec<DoctorFixResultRecord> {
    report
        .problems
        .iter()
        .filter(|problem| problem.fixability == ProblemFixability::ManualOnly)
        .filter(|problem| {
            !owned_by_attempted_mutation_scope_repair(problem, attempted_mutation_scope_targets)
        })
        .map(|problem| DoctorFixResultRecord {
            category: problem.category,
            outcome: FixResult::Manual,
            detail: manual_fix_detail(problem),
        })
        .collect()
}

fn owned_by_attempted_mutation_scope_repair(
    problem: &DoctorProblem,
    attempted_mutation_scope_targets: &[IntegrationTarget],
) -> bool {
    problem.category == ProblemCategory::MutationScopeHealth
        && problem
            .mutation_scope_target
            .is_some_and(|target| attempted_mutation_scope_targets.contains(&target))
}

fn manual_fix_detail(problem: &DoctorProblem) -> String {
    if problem.category == ProblemCategory::MutationScopeHealth {
        problem.remediation.clone()
    } else {
        format!("{} Manual remediation is still required.", problem.summary)
    }
}
