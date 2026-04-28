use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::services::hooks_lifecycle::HOOKS_SERVICE_ID;
use crate::services::lifecycle::{FixReport, FixRequest, LifecycleContext, LifecycleOutcome};
use crate::services::lifecycle_registry::LifecycleRegistry;

use super::types::{DoctorFixResultRecord, FixResult, ProblemCategory, ProblemFixability};
use super::{DoctorDependencies, HookDoctorReport};

pub(super) fn run_auto_fixes(
    report: &HookDoctorReport,
    _dependencies: &DoctorDependencies<'_>,
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

        match run_hook_lifecycle_fix(repository_root) {
            Ok(report) => fix_results.extend(build_hook_fix_results(&report)),
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

fn run_hook_lifecycle_fix(repository_root: &Path) -> Result<FixReport> {
    let hooks_lifecycle = LifecycleRegistry::fix_lifecycle(HOOKS_SERVICE_ID)
        .context("Hooks lifecycle fix capability is not registered")?;

    hooks_lifecycle.fix(FixRequest {
        context: LifecycleContext {
            repository: Some(repository_root.to_path_buf()),
            ..LifecycleContext::default()
        },
        problem_kinds: Vec::new(),
    })
}

fn build_hook_fix_results(report: &FixReport) -> Vec<DoctorFixResultRecord> {
    report
        .actions
        .iter()
        .map(|action| {
            let hook_path = PathBuf::from(&action.target);
            let hook_name = hook_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(action.target.as_str());
            DoctorFixResultRecord {
                category: ProblemCategory::HookRollout,
                outcome: fix_result_from_lifecycle_outcome(action.outcome)
                    .unwrap_or(FixResult::Failed),
                detail: format!(
                    "Hook '{}' {} at '{}'.",
                    hook_name,
                    hook_status_text_from_lifecycle_outcome(action.outcome)
                        .unwrap_or("repair failed"),
                    hook_path.display()
                ),
            }
        })
        .collect()
}

fn fix_result_from_lifecycle_outcome(outcome: LifecycleOutcome) -> Result<FixResult> {
    match outcome {
        LifecycleOutcome::Applied | LifecycleOutcome::Updated => Ok(FixResult::Fixed),
        LifecycleOutcome::Unchanged | LifecycleOutcome::Skipped => Ok(FixResult::Skipped),
        LifecycleOutcome::Failed => bail!("unsupported failed lifecycle fix outcome"),
    }
}

fn hook_status_text_from_lifecycle_outcome(outcome: LifecycleOutcome) -> Result<&'static str> {
    match outcome {
        LifecycleOutcome::Applied => Ok("installed"),
        LifecycleOutcome::Updated => Ok("updated"),
        LifecycleOutcome::Unchanged | LifecycleOutcome::Skipped => {
            Ok("already matched canonical content")
        }
        LifecycleOutcome::Failed => bail!("unsupported failed lifecycle fix outcome"),
    }
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
