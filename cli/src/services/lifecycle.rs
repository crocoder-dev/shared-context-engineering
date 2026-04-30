use std::path::PathBuf;

use anyhow::Result;

use crate::app::AppContext;

pub type LifecycleProvider = Box<dyn ServiceLifecycle>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthCategory {
    GlobalState,
    RepositoryTargeting,
    HookRollout,
    RepoAssets,
    FilesystemPermissions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthSeverity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthFixability {
    AutoFixable,
    ManualOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthProblemKind {
    GitUnavailable,
    BareRepository,
    NotInsideGitRepository,
    UnableToResolveGitHooksDirectory,
    UnableToResolveStateRoot,
    GlobalConfigValidationFailed,
    UnableToResolveGlobalConfigPath,
    LocalConfigValidationFailed,
    HooksDirectoryMissing,
    HooksPathNotDirectory,
    RequiredHookMissing,
    HookNotExecutable,
    HookContentStale,
    OpenCodeIntegrationFilesMissing,
    OpenCodeIntegrationContentMismatch,
    OpenCodePluginRegistryInvalid,
    OpenCodeAssetMissingOrInvalid,
    HookReadFailed,
    OpenCodeAssetReadFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthProblem {
    pub kind: HealthProblemKind,
    pub category: HealthCategory,
    pub severity: HealthSeverity,
    pub fixability: HealthFixability,
    pub summary: String,
    pub remediation: String,
    pub next_action: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixOutcome {
    Fixed,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixResultRecord {
    pub category: HealthCategory,
    pub outcome: FixOutcome,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredHookInstallStatus {
    Installed,
    Updated,
    Skipped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredHookInstallResult {
    pub hook_name: String,
    pub hook_path: PathBuf,
    pub status: RequiredHookInstallStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredHooksInstallOutcome {
    pub repository_root: PathBuf,
    pub hooks_directory: PathBuf,
    pub hook_results: Vec<RequiredHookInstallResult>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetupOutcome {
    pub required_hooks_install: Option<RequiredHooksInstallOutcome>,
}

#[allow(dead_code)]
pub trait ServiceLifecycle: Send + Sync {
    fn diagnose(&self, _ctx: &AppContext) -> Vec<HealthProblem> {
        Vec::new()
    }

    fn fix(&self, _ctx: &AppContext, _problems: &[HealthProblem]) -> Vec<FixResultRecord> {
        Vec::new()
    }

    fn setup(&self, _ctx: &AppContext) -> Result<SetupOutcome> {
        Ok(SetupOutcome::default())
    }
}

/// Returns lifecycle providers in deterministic orchestration order.
///
/// Provider order is config → `local_db` → hooks when hook lifecycle behavior is requested.
pub fn lifecycle_providers(include_hooks: bool) -> Vec<LifecycleProvider> {
    let mut providers: Vec<LifecycleProvider> = vec![
        Box::new(crate::services::config::lifecycle::ConfigLifecycle),
        Box::new(crate::services::local_db::lifecycle::LocalDbLifecycle),
    ];

    if include_hooks {
        providers.push(Box::new(crate::services::hooks::lifecycle::HooksLifecycle));
    }

    providers
}
