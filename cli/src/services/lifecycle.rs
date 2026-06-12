use std::path::PathBuf;

use anyhow::Result;

use crate::app::HasRepoRoot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleProviderId {
    Config,
    LocalDb,
    AuthDb,
    AgentTraceDb,
    Hooks,
}

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
    fn id(&self) -> LifecycleProviderId;

    fn diagnose<C: HasRepoRoot>(&self, _ctx: &C) -> Vec<HealthProblem> {
        Vec::new()
    }

    fn fix<C: HasRepoRoot>(&self, _ctx: &C, _problems: &[HealthProblem]) -> Vec<FixResultRecord> {
        Vec::new()
    }

    fn setup<C: HasRepoRoot>(&self, _ctx: &C) -> Result<SetupOutcome> {
        Ok(SetupOutcome::default())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleProvider {
    Config,
    LocalDb,
    AuthDb,
    AgentTraceDb,
    Hooks,
}

impl LifecycleProvider {
    pub fn id(self) -> LifecycleProviderId {
        match self {
            Self::Config => LifecycleProviderId::Config,
            Self::LocalDb => LifecycleProviderId::LocalDb,
            Self::AuthDb => LifecycleProviderId::AuthDb,
            Self::AgentTraceDb => LifecycleProviderId::AgentTraceDb,
            Self::Hooks => LifecycleProviderId::Hooks,
        }
    }

    pub fn diagnose<C: HasRepoRoot>(self, ctx: &C) -> Vec<HealthProblem> {
        match self {
            Self::Config => crate::services::config::lifecycle::ConfigLifecycle.diagnose(ctx),
            Self::LocalDb => crate::services::local_db::lifecycle::LocalDbLifecycle.diagnose(ctx),
            Self::AuthDb => crate::services::auth_db::lifecycle::AuthDbLifecycle.diagnose(ctx),
            Self::AgentTraceDb => {
                crate::services::agent_trace_db::lifecycle::AgentTraceDbLifecycle.diagnose(ctx)
            }
            Self::Hooks => crate::services::hooks::lifecycle::HooksLifecycle.diagnose(ctx),
        }
    }

    pub fn fix<C: HasRepoRoot>(self, ctx: &C, problems: &[HealthProblem]) -> Vec<FixResultRecord> {
        match self {
            Self::Config => crate::services::config::lifecycle::ConfigLifecycle.fix(ctx, problems),
            Self::LocalDb => {
                crate::services::local_db::lifecycle::LocalDbLifecycle.fix(ctx, problems)
            }
            Self::AuthDb => crate::services::auth_db::lifecycle::AuthDbLifecycle.fix(ctx, problems),
            Self::AgentTraceDb => {
                crate::services::agent_trace_db::lifecycle::AgentTraceDbLifecycle.fix(ctx, problems)
            }
            Self::Hooks => crate::services::hooks::lifecycle::HooksLifecycle.fix(ctx, problems),
        }
    }

    pub fn setup<C: HasRepoRoot>(self, ctx: &C) -> Result<SetupOutcome> {
        match self {
            Self::Config => crate::services::config::lifecycle::ConfigLifecycle.setup(ctx),
            Self::LocalDb => crate::services::local_db::lifecycle::LocalDbLifecycle.setup(ctx),
            Self::AuthDb => crate::services::auth_db::lifecycle::AuthDbLifecycle.setup(ctx),
            Self::AgentTraceDb => {
                crate::services::agent_trace_db::lifecycle::AgentTraceDbLifecycle.setup(ctx)
            }
            Self::Hooks => crate::services::hooks::lifecycle::HooksLifecycle.setup(ctx),
        }
    }
}

/// Returns lifecycle providers in deterministic orchestration order.
///
/// Provider order is config → `local_db` → `auth_db` → `agent_trace_db` → hooks when hook lifecycle behavior is requested.
pub fn lifecycle_providers(include_hooks: bool) -> Vec<LifecycleProvider> {
    let mut providers = vec![
        LifecycleProvider::Config,
        LifecycleProvider::LocalDb,
        LifecycleProvider::AuthDb,
        LifecycleProvider::AgentTraceDb,
    ];

    if include_hooks {
        providers.push(LifecycleProvider::Hooks);
    }

    providers
}
