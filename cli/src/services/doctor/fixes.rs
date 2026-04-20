use crate::services::setup::{RequiredHookInstallStatus, RequiredHooksInstallOutcome};

use super::types::{DoctorFixResultRecord, FixResult, ProblemCategory, ProblemFixability};
use super::{DoctorDependencies, HookDoctorReport};

pub(super) fn run_auto_fixes(
    report: &HookDoctorReport,
    dependencies: &DoctorDependencies<'_>,
) -> Vec<DoctorFixResultRecord> {
    let auto_fixable_problems = report
        .problems
        .iter()
        .filter(|problem| problem.fixability == ProblemFixability::AutoFixable)
        .collect::<Vec<_>>();

    if auto_fixable_problems.is_empty() {
        return Vec::new();
    }

    let mut fix_results = Vec::new();

    if auto_fixable_problems
        .iter()
        .any(|problem| problem.category == ProblemCategory::HookRollout)
    {
        let Some(repository_root) = report.repository_root.as_deref() else {
            fix_results.push(DoctorFixResultRecord {
                category: ProblemCategory::HookRollout,
                outcome: FixResult::Failed,
                detail: String::from("Automatic hook repair could not start because the repository root was not resolved during diagnosis."),
            });
            return fix_results;
        };

        match (dependencies.install_required_git_hooks)(repository_root) {
            Ok(outcome) => fix_results.extend(build_hook_fix_results(&outcome)),
            Err(error) => fix_results.push(DoctorFixResultRecord {
                category: ProblemCategory::HookRollout,
                outcome: FixResult::Failed,
                detail: format!(
                    "Automatic hook repair failed while reusing the canonical setup flow: {error}"
                ),
            }),
        }
    }

    fix_results
}

fn build_hook_fix_results(outcome: &RequiredHooksInstallOutcome) -> Vec<DoctorFixResultRecord> {
    outcome
        .hook_results
        .iter()
        .map(|hook_result| DoctorFixResultRecord {
            category: ProblemCategory::HookRollout,
            outcome: match hook_result.status {
                RequiredHookInstallStatus::Installed | RequiredHookInstallStatus::Updated => {
                    FixResult::Fixed
                }
                RequiredHookInstallStatus::Skipped => FixResult::Skipped,
            },
            detail: format!(
                "Hook '{}' {} at '{}'.",
                hook_result.hook_name,
                match hook_result.status {
                    RequiredHookInstallStatus::Installed => "installed",
                    RequiredHookInstallStatus::Updated => "updated",
                    RequiredHookInstallStatus::Skipped => "already matched canonical content",
                },
                hook_result.hook_path.display()
            ),
        })
        .collect()
}

pub(super) fn build_manual_fix_results(report: &HookDoctorReport) -> Vec<DoctorFixResultRecord> {
    report
        .problems
        .iter()
        .filter(|problem| problem.fixability != ProblemFixability::AutoFixable)
        .map(|problem| DoctorFixResultRecord {
            category: problem.category,
            outcome: FixResult::Manual,
            detail: match problem.fixability {
                ProblemFixability::AutoFixable => {
                    unreachable!("auto-fixable problems should not be rendered as manual results")
                }
                ProblemFixability::ManualOnly => {
                    format!("{} Manual remediation is still required.", problem.summary)
                }
            },
        })
        .collect()
}
