#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::services::lifecycle::{
    ActionPlan, DiagnoseRequest, DiagnosticFixability, DiagnosticLifecycle, DiagnosticRecord,
    DiagnosticReport, DiagnosticSeverity, FixLifecycle, FixReport, FixRequest, LifecycleAction,
    LifecycleContext, LifecycleOperation, LifecycleOutcome, LifecycleService, PreviewLifecycle,
    PreviewRequest, ServiceId, ServiceMetadata, SetupLifecycle, SetupReport, SetupRequest,
};
use crate::services::setup::{
    self, RequiredHookInstallResult, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
};

pub const HOOKS_SERVICE_ID: ServiceId = ServiceId("hooks");
pub const HOOKS_DIRECTORY_MISSING: &str = "hooks_directory_missing";
pub const HOOKS_PATH_NOT_DIRECTORY: &str = "hooks_path_not_directory";
pub const REQUIRED_HOOK_MISSING: &str = "required_hook_missing";
pub const HOOK_NOT_EXECUTABLE: &str = "hook_not_executable";
pub const HOOK_CONTENT_STALE: &str = "hook_content_stale";
pub const HOOK_READ_FAILED: &str = "hook_read_failed";

#[derive(Clone, Copy, Debug, Default)]
pub struct HooksLifecycleService;

impl LifecycleService for HooksLifecycleService {
    fn metadata(&self) -> ServiceMetadata {
        ServiceMetadata {
            id: HOOKS_SERVICE_ID,
            display_name: "Git hooks",
            description: "Canonical SCE-managed required git hook lifecycle capability",
        }
    }
}

impl SetupLifecycle for HooksLifecycleService {
    fn setup(&self, request: SetupRequest) -> Result<SetupReport> {
        let repository_root = required_repository(&request.context)?;
        let outcome = setup::install_required_git_hooks(repository_root)
            .context("Hook lifecycle setup failed while installing required git hooks")?;

        Ok(SetupReport {
            service_id: HOOKS_SERVICE_ID,
            actions: lifecycle_actions_from_hook_outcome(&outcome, LifecycleOperation::Setup),
        })
    }
}

impl DiagnosticLifecycle for HooksLifecycleService {
    fn diagnose(&self, request: DiagnoseRequest) -> Result<DiagnosticReport> {
        let hooks_directory = request.context.state.as_deref().context(
            "Hooks lifecycle diagnosis requires a hooks directory path in LifecycleContext.state",
        )?;
        Ok(diagnose_required_hooks(hooks_directory).diagnostics)
    }
}

impl FixLifecycle for HooksLifecycleService {
    fn fix(&self, request: FixRequest) -> Result<FixReport> {
        let repository_root = required_repository(&request.context)?;
        let outcome = setup::install_required_git_hooks(repository_root)
            .context("Hook lifecycle fix failed while installing required git hooks")?;

        Ok(FixReport {
            service_id: HOOKS_SERVICE_ID,
            actions: lifecycle_actions_from_hook_outcome(&outcome, LifecycleOperation::Fix),
        })
    }
}

impl PreviewLifecycle for HooksLifecycleService {
    fn preview(&self, request: PreviewRequest) -> Result<ActionPlan> {
        let repository_root = required_repository(&request.context)?;
        let outcome = setup::preview_required_git_hooks(repository_root)
            .context("Hook lifecycle preview failed while planning required git hook actions")?;

        Ok(ActionPlan {
            service_id: HOOKS_SERVICE_ID,
            operation: request.operation,
            actions: lifecycle_actions_from_hook_outcome(&outcome, request.operation),
        })
    }
}

fn required_repository(context: &LifecycleContext) -> Result<&Path> {
    context
        .repository
        .as_deref()
        .context("Hooks lifecycle requires a repository path in LifecycleContext")
}

fn lifecycle_actions_from_hook_outcome(
    outcome: &RequiredHooksInstallOutcome,
    operation: LifecycleOperation,
) -> Vec<LifecycleAction> {
    outcome
        .hook_results
        .iter()
        .map(|result| lifecycle_action_from_hook_result(result, operation))
        .collect()
}

fn lifecycle_action_from_hook_result(
    result: &RequiredHookInstallResult,
    operation: LifecycleOperation,
) -> LifecycleAction {
    LifecycleAction {
        service_id: HOOKS_SERVICE_ID,
        operation,
        target: result.hook_path.display().to_string(),
        description: format!(
            "{} required git hook '{}'",
            hook_action_verb(result.status, operation),
            result.hook_name
        ),
        outcome: lifecycle_outcome_from_hook_status(result.status),
    }
}

fn hook_action_verb(
    status: RequiredHookInstallStatus,
    operation: LifecycleOperation,
) -> &'static str {
    match (operation, status) {
        (LifecycleOperation::Preview, RequiredHookInstallStatus::Installed) => "would install",
        (LifecycleOperation::Preview, RequiredHookInstallStatus::Updated) => "would update",
        (LifecycleOperation::Preview, RequiredHookInstallStatus::Skipped) => "would keep",
        (_, RequiredHookInstallStatus::Installed) => "installed",
        (_, RequiredHookInstallStatus::Updated) => "updated",
        (_, RequiredHookInstallStatus::Skipped) => "kept",
    }
}

fn lifecycle_outcome_from_hook_status(status: RequiredHookInstallStatus) -> LifecycleOutcome {
    match status {
        RequiredHookInstallStatus::Installed => LifecycleOutcome::Applied,
        RequiredHookInstallStatus::Updated => LifecycleOutcome::Updated,
        RequiredHookInstallStatus::Skipped => LifecycleOutcome::Unchanged,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HooksDiagnosticReport {
    pub hooks: Vec<RequiredHookHealth>,
    pub diagnostics: DiagnosticReport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredHookHealth {
    pub name: &'static str,
    pub path: PathBuf,
    pub exists: bool,
    pub executable: bool,
    pub content_state: RequiredHookContentState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredHookContentState {
    Current,
    Stale,
    Missing,
    Unknown,
}

pub fn diagnose_required_hooks(hooks_directory: &Path) -> HooksDiagnosticReport {
    let mut diagnostics = Vec::new();

    if !hooks_directory.exists() {
        diagnostics.push(DiagnosticRecord {
            service_id: HOOKS_SERVICE_ID,
            kind: HOOKS_DIRECTORY_MISSING.to_string(),
            target: hooks_directory.display().to_string(),
            severity: DiagnosticSeverity::Error,
            fixability: DiagnosticFixability::AutoFixable,
            summary: format!("Hooks directory '{}' does not exist.", hooks_directory.display()),
            remediation: format!(
                "Run 'sce doctor --fix' to install the canonical SCE-managed hooks into '{}', or run 'sce setup --hooks' directly.",
                hooks_directory.display()
            ),
        });
    } else if !hooks_directory.is_dir() {
        diagnostics.push(DiagnosticRecord {
            service_id: HOOKS_SERVICE_ID,
            kind: HOOKS_PATH_NOT_DIRECTORY.to_string(),
            target: hooks_directory.display().to_string(),
            severity: DiagnosticSeverity::Error,
            fixability: DiagnosticFixability::ManualOnly,
            summary: format!("Hooks path '{}' is not a directory.", hooks_directory.display()),
            remediation: format!(
                "Replace '{}' with a writable hooks directory, then rerun 'sce doctor' or 'sce setup --hooks'.",
                hooks_directory.display()
            ),
        });
    }

    let hooks = setup::iter_required_hook_assets()
        .map(|hook_asset| {
            inspect_required_hook(
                hooks_directory,
                hook_asset.relative_path,
                hook_asset.bytes,
                &mut diagnostics,
            )
        })
        .collect::<Vec<_>>();

    HooksDiagnosticReport {
        hooks,
        diagnostics: DiagnosticReport {
            service_id: HOOKS_SERVICE_ID,
            diagnostics,
        },
    }
}

fn inspect_required_hook(
    hooks_directory: &Path,
    hook_name: &'static str,
    expected_bytes: &[u8],
    diagnostics: &mut Vec<DiagnosticRecord>,
) -> RequiredHookHealth {
    let hook_path = hooks_directory.join(hook_name);
    let metadata = fs::metadata(&hook_path).ok();
    let exists = metadata.is_some();
    let executable = metadata
        .as_ref()
        .is_some_and(|entry| entry.is_file() && is_executable(entry));
    let content_state =
        inspect_required_hook_content(hook_name, &hook_path, exists, expected_bytes, diagnostics);

    if !exists {
        diagnostics.push(DiagnosticRecord {
            service_id: HOOKS_SERVICE_ID,
            kind: REQUIRED_HOOK_MISSING.to_string(),
            target: hook_path.display().to_string(),
            severity: DiagnosticSeverity::Error,
            fixability: DiagnosticFixability::AutoFixable,
            summary: format!("Missing required hook '{}' at '{}'.", hook_name, hook_path.display()),
            remediation: format!(
                "Run 'sce doctor --fix' to install the canonical '{hook_name}' hook, or run 'sce setup --hooks' directly."
            ),
        });
    } else if !executable {
        diagnostics.push(DiagnosticRecord {
            service_id: HOOKS_SERVICE_ID,
            kind: HOOK_NOT_EXECUTABLE.to_string(),
            target: hook_path.display().to_string(),
            severity: DiagnosticSeverity::Error,
            fixability: DiagnosticFixability::AutoFixable,
            summary: format!("Hook '{hook_name}' exists but is not executable."),
            remediation: format!(
                "Run 'sce doctor --fix' to restore the canonical executable hook, or run 'sce setup --hooks' / 'chmod +x {}' manually.",
                hook_path.display()
            ),
        });
    }

    if content_state == RequiredHookContentState::Stale {
        diagnostics.push(DiagnosticRecord {
            service_id: HOOKS_SERVICE_ID,
            kind: HOOK_CONTENT_STALE.to_string(),
            target: hook_path.display().to_string(),
            severity: DiagnosticSeverity::Error,
            fixability: DiagnosticFixability::AutoFixable,
            summary: format!(
                "Hook '{}' at '{}' differs from the canonical SCE-managed content.",
                hook_name,
                hook_path.display()
            ),
            remediation: format!(
                "Run 'sce doctor --fix' to reinstall the canonical '{hook_name}' hook content, or run 'sce setup --hooks' directly."
            ),
        });
    }

    RequiredHookHealth {
        name: hook_name,
        path: hook_path,
        exists,
        executable,
        content_state,
    }
}

fn inspect_required_hook_content(
    hook_name: &str,
    hook_path: &Path,
    exists: bool,
    expected_bytes: &[u8],
    diagnostics: &mut Vec<DiagnosticRecord>,
) -> RequiredHookContentState {
    if !exists {
        return RequiredHookContentState::Missing;
    }

    match fs::read(hook_path) {
        Ok(bytes) => {
            if bytes == expected_bytes {
                RequiredHookContentState::Current
            } else {
                RequiredHookContentState::Stale
            }
        }
        Err(error) => {
            diagnostics.push(DiagnosticRecord {
                service_id: HOOKS_SERVICE_ID,
                kind: HOOK_READ_FAILED.to_string(),
                target: hook_path.display().to_string(),
                severity: DiagnosticSeverity::Error,
                fixability: DiagnosticFixability::ManualOnly,
                summary: format!(
                    "Unable to read hook '{}' at '{}': {error}",
                    hook_name,
                    hook_path.display()
                ),
                remediation: format!(
                    "Verify that '{}' is readable before rerunning 'sce doctor'.",
                    hook_path.display()
                ),
            });
            RequiredHookContentState::Unknown
        }
    }
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
}
