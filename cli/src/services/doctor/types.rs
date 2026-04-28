use std::path::PathBuf;

pub(super) const OPENCODE_PLUGINS_LABEL: &str = "OpenCode plugins";
pub(super) const OPENCODE_AGENTS_LABEL: &str = "OpenCode agents";
pub(super) const OPENCODE_COMMANDS_LABEL: &str = "OpenCode commands";
pub(super) const OPENCODE_SKILLS_LABEL: &str = "OpenCode skills";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Readiness {
    Ready,
    NotReady,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HookPathSource {
    Default,
    LocalConfig,
    GlobalConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HookFileHealth {
    pub(super) name: &'static str,
    pub(super) path: PathBuf,
    pub(super) exists: bool,
    pub(super) executable: bool,
    pub(super) content_state: HookContentState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HookContentState {
    Current,
    Stale,
    Missing,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FileLocationHealth {
    pub(super) label: &'static str,
    pub(super) path: PathBuf,
    pub(super) state: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct GlobalStateHealth {
    pub(super) state_root: Option<FileLocationHealth>,
    pub(super) config_locations: Vec<FileLocationHealth>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LocalDbHealth {
    pub(super) db_path: Option<PathBuf>,
    pub(super) parent_path: Option<PathBuf>,
    pub(super) db_status: LocalDbStatus,
    pub(super) parent_status: LocalDbParentStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LocalDbStatus {
    Healthy,
    Unhealthy,
    Unresolvable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LocalDbParentStatus {
    Healthy,
    Missing,
    NotDirectory,
    NotWritable,
    Unresolvable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HookDoctorReport {
    pub(super) mode: super::DoctorMode,
    pub(super) readiness: Readiness,
    pub(super) state_root: Option<FileLocationHealth>,
    pub(super) repository_root: Option<PathBuf>,
    pub(super) hook_path_source: HookPathSource,
    pub(super) hooks_directory: Option<PathBuf>,
    pub(super) config_locations: Vec<FileLocationHealth>,
    pub(super) local_db: LocalDbHealth,
    pub(super) hooks: Vec<HookFileHealth>,
    pub(super) integration_groups: Vec<IntegrationGroupHealth>,
    pub(super) problems: Vec<DoctorProblem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IntegrationGroupHealth {
    pub(super) label: &'static str,
    pub(super) children: Vec<IntegrationChildHealth>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IntegrationChildHealth {
    pub(super) relative_path: String,
    pub(super) path: PathBuf,
    pub(super) content_state: IntegrationContentState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum IntegrationContentState {
    Match,
    Missing,
    Mismatch,
    ReadFailed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProblemCategory {
    GlobalState,
    LocalDatabase,
    RepositoryTargeting,
    HookRollout,
    RepoAssets,
    FilesystemPermissions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProblemSeverity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProblemFixability {
    AutoFixable,
    ManualOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProblemKind {
    GitUnavailable,
    BareRepository,
    NotInsideGitRepository,
    UnableToResolveGitHooksDirectory,
    UnableToResolveStateRoot,
    GlobalConfigValidationFailed,
    UnableToResolveGlobalConfigPath,
    LocalConfigValidationFailed,
    LocalDbPathUnresolvable,
    LocalDbParentMissing,
    LocalDbParentNotDirectory,
    LocalDbParentNotWritable,
    LocalDbHealthCheckFailed,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FixResult {
    Fixed,
    Skipped,
    Manual,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DoctorProblem {
    pub(super) kind: ProblemKind,
    pub(super) category: ProblemCategory,
    pub(super) severity: ProblemSeverity,
    pub(super) fixability: ProblemFixability,
    pub(super) summary: String,
    pub(super) remediation: String,
    pub(super) next_action: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DoctorFixResultRecord {
    pub(super) category: ProblemCategory,
    pub(super) outcome: FixResult,
    pub(super) detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HumanTextStatus {
    Pass,
    Fail,
    Miss,
}

pub(super) fn problem_category(category: ProblemCategory) -> &'static str {
    match category {
        ProblemCategory::GlobalState => "global_state",
        ProblemCategory::LocalDatabase => "local_database",
        ProblemCategory::RepositoryTargeting => "repository_targeting",
        ProblemCategory::HookRollout => "hook_rollout",
        ProblemCategory::RepoAssets => "repo_assets",
        ProblemCategory::FilesystemPermissions => "filesystem_permissions",
    }
}

pub(super) fn problem_severity(severity: ProblemSeverity) -> &'static str {
    match severity {
        ProblemSeverity::Error => "error",
        ProblemSeverity::Warning => "warning",
    }
}

pub(super) fn problem_fixability(fixability: ProblemFixability) -> &'static str {
    match fixability {
        ProblemFixability::AutoFixable => "auto_fixable",
        ProblemFixability::ManualOnly => "manual_only",
    }
}

pub(super) fn fix_result_outcome(outcome: FixResult) -> &'static str {
    match outcome {
        FixResult::Fixed => "fixed",
        FixResult::Skipped => "skipped",
        FixResult::Manual => "manual",
        FixResult::Failed => "failed",
    }
}
