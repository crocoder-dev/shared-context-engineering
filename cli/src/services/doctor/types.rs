use std::path::PathBuf;

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
pub(super) struct HookDoctorReport {
    pub(super) mode: super::DoctorMode,
    pub(super) readiness: Readiness,
    pub(super) state_root: Option<FileLocationHealth>,
    pub(super) checkout_identity: Option<CheckoutIdentityHealth>,
    pub(super) agent_trace_db: Option<AgentTraceDbHealth>,
    pub(super) repository_root: Option<PathBuf>,
    pub(super) hook_path_source: HookPathSource,
    pub(super) hooks_directory: Option<PathBuf>,
    pub(super) post_commit_auto_sync: PostCommitAutoSyncHealth,
    pub(super) config_locations: Vec<FileLocationHealth>,
    pub(super) hooks: Vec<HookFileHealth>,
    pub(super) integration_groups: Vec<IntegrationGroupHealth>,
    pub(super) integration_targets_absent: bool,
    pub(super) problems: Vec<DoctorProblem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PostCommitAutoSyncHealth {
    pub(super) state: PostCommitAutoSyncState,
    pub(super) enabled: bool,
    pub(super) source: &'static str,
    pub(super) config_source: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PostCommitAutoSyncState {
    Ready,
    Disabled,
    NotReady,
    NotApplicable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckoutIdentityHealth {
    pub(super) checkout_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AgentTraceDbHealth {
    pub(super) label: &'static str,
    pub(super) path: PathBuf,
    pub(super) state: &'static str,
    pub(super) repository_id: String,
    pub(super) canonical_identity: String,
    pub(super) identity_source: String,
    pub(super) configured_remote: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum IntegrationTarget {
    OpenCode,
    ClaudeCode,
    Pi,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum IntegrationArea {
    Plugins,
    Agents,
    Commands,
    Skills,
    Prompts,
    Extensions,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct IntegrationGroupKey {
    pub(super) target: IntegrationTarget,
    pub(super) area: IntegrationArea,
}

impl IntegrationGroupKey {
    pub(super) const fn new(target: IntegrationTarget, area: IntegrationArea) -> Self {
        Self { target, area }
    }

    pub(super) const fn display_label(self) -> &'static str {
        match (self.target, self.area) {
            (IntegrationTarget::OpenCode, IntegrationArea::Plugins) => "OpenCode plugins",
            (IntegrationTarget::OpenCode, IntegrationArea::Agents) => "OpenCode agents",
            (IntegrationTarget::OpenCode, IntegrationArea::Commands) => "OpenCode commands",
            (IntegrationTarget::OpenCode, IntegrationArea::Skills) => "OpenCode skills",
            (IntegrationTarget::ClaudeCode, IntegrationArea::Plugins) => "ClaudeCode plugins",
            (IntegrationTarget::ClaudeCode, IntegrationArea::Commands) => "ClaudeCode commands",
            (IntegrationTarget::ClaudeCode, IntegrationArea::Skills) => "ClaudeCode skills",
            (IntegrationTarget::Pi, IntegrationArea::Prompts) => "Pi prompts",
            (IntegrationTarget::Pi, IntegrationArea::Skills) => "Pi skills",
            (IntegrationTarget::Pi, IntegrationArea::Extensions) => "Pi extensions",
            // These combinations are not produced by inspection, but retaining
            // deterministic labels keeps the key total for future targets/areas.
            (IntegrationTarget::Pi, IntegrationArea::Plugins) => "Pi plugins",
            (IntegrationTarget::Pi, IntegrationArea::Agents) => "Pi agents",
            (IntegrationTarget::Pi, IntegrationArea::Commands) => "Pi commands",
            (IntegrationTarget::ClaudeCode, IntegrationArea::Prompts) => "ClaudeCode prompts",
            (IntegrationTarget::ClaudeCode, IntegrationArea::Extensions) => "ClaudeCode extensions",
            (IntegrationTarget::ClaudeCode, IntegrationArea::Agents) => "Unsupported Claude area",
            (IntegrationTarget::OpenCode, IntegrationArea::Prompts) => "OpenCode prompts",
            (IntegrationTarget::OpenCode, IntegrationArea::Extensions) => "OpenCode extensions",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IntegrationGroupHealth {
    pub(super) key: IntegrationGroupKey,
    pub(super) children: Vec<IntegrationChildHealth>,
}

impl IntegrationGroupHealth {
    pub(super) fn new(key: IntegrationGroupKey, children: Vec<IntegrationChildHealth>) -> Self {
        Self { key, children }
    }

    pub(super) const fn display_label(&self) -> &'static str {
        self.key.display_label()
    }

    #[allow(dead_code)]
    pub(super) fn display_node(&self) -> DoctorDisplayNode {
        let children = self
            .children
            .iter()
            .map(IntegrationChildHealth::display_node)
            .collect::<Vec<_>>();
        let status = children
            .iter()
            .fold(DoctorDisplayStatus::Pass, |status, child| {
                status.worst(if child.status == DoctorDisplayStatus::Miss {
                    DoctorDisplayStatus::Fail
                } else {
                    child.status
                })
            });
        DoctorDisplayNode::branch_with_status(
            DoctorDisplayNodeKind::Group,
            self.display_label(),
            status,
            Vec::new(),
            children,
        )
    }
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[allow(dead_code)]
pub(super) enum DoctorDisplayStatus {
    Pass,
    Warn,
    Miss,
    Fail,
}

impl DoctorDisplayStatus {
    #[allow(dead_code)]
    pub(super) const fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::Fail, _) | (_, Self::Fail) => Self::Fail,
            (Self::Miss, _) | (_, Self::Miss) => Self::Miss,
            (Self::Warn, _) | (_, Self::Warn) => Self::Warn,
            (Self::Pass, Self::Pass) => Self::Pass,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) enum DoctorDisplayDetail {
    MissingPath(PathBuf),
    ContentMismatch {
        path: PathBuf,
    },
    ReadFailed {
        path: PathBuf,
        error: String,
    },
    Problem {
        summary: String,
        remediation: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) enum DoctorDisplayNodeKind {
    Domain,
    Group,
    Asset,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) struct DoctorDisplayNode {
    pub(super) kind: DoctorDisplayNodeKind,
    pub(super) label: String,
    pub(super) status: DoctorDisplayStatus,
    pub(super) details: Vec<DoctorDisplayDetail>,
    pub(super) children: Vec<DoctorDisplayNode>,
}

impl DoctorDisplayNode {
    #[allow(dead_code)]
    pub(super) fn domain(label: impl Into<String>, children: Vec<Self>) -> Self {
        Self::branch(DoctorDisplayNodeKind::Domain, label, children)
    }

    #[allow(dead_code)]
    pub(super) fn group(label: impl Into<String>, children: Vec<Self>) -> Self {
        Self::branch(DoctorDisplayNodeKind::Group, label, children)
    }

    #[allow(dead_code)]
    fn asset(
        label: impl Into<String>,
        status: DoctorDisplayStatus,
        detail: Option<DoctorDisplayDetail>,
    ) -> Self {
        Self {
            kind: DoctorDisplayNodeKind::Asset,
            label: label.into(),
            status,
            details: detail.into_iter().collect(),
            children: Vec::new(),
        }
    }

    #[allow(dead_code)]
    fn branch(kind: DoctorDisplayNodeKind, label: impl Into<String>, children: Vec<Self>) -> Self {
        Self::branch_with_status(kind, label, DoctorDisplayStatus::Pass, Vec::new(), children)
    }

    pub(super) fn branch_with_status(
        kind: DoctorDisplayNodeKind,
        label: impl Into<String>,
        status: DoctorDisplayStatus,
        details: Vec<DoctorDisplayDetail>,
        children: Vec<Self>,
    ) -> Self {
        let status = children.iter().fold(status, |status, child| {
            status.worst(
                if matches!(
                    kind,
                    DoctorDisplayNodeKind::Domain | DoctorDisplayNodeKind::Group
                ) && child.status == DoctorDisplayStatus::Miss
                {
                    DoctorDisplayStatus::Fail
                } else {
                    child.status
                },
            )
        });
        Self {
            kind,
            label: label.into(),
            status,
            details,
            children,
        }
    }
}

impl IntegrationChildHealth {
    #[allow(dead_code)]
    pub(super) fn display_node(&self) -> DoctorDisplayNode {
        let (status, detail) = match &self.content_state {
            IntegrationContentState::Match => (DoctorDisplayStatus::Pass, None),
            IntegrationContentState::Missing => (
                DoctorDisplayStatus::Miss,
                Some(DoctorDisplayDetail::MissingPath(self.path.clone())),
            ),
            IntegrationContentState::Mismatch => (
                DoctorDisplayStatus::Fail,
                Some(DoctorDisplayDetail::ContentMismatch {
                    path: self.path.clone(),
                }),
            ),
            IntegrationContentState::ReadFailed(error) => (
                DoctorDisplayStatus::Fail,
                Some(DoctorDisplayDetail::ReadFailed {
                    path: self.path.clone(),
                    error: error.clone(),
                }),
            ),
        };
        DoctorDisplayNode::asset(self.relative_path.clone(), status, detail)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProblemCategory {
    GlobalState,
    RepositoryTargeting,
    HookRollout,
    RepoAssets,
    FilesystemPermissions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProblemSeverity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProblemFixability {
    AutoFixable,
    ManualOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProblemKind {
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
    NoIntegrationsInstalled,
    OpenCodeIntegrationFilesMissing,
    OpenCodeIntegrationContentMismatch,
    ClaudeIntegrationFilesMissing,
    ClaudeIntegrationContentMismatch,
    PiIntegrationFilesMissing,
    PiIntegrationContentMismatch,
    OpenCodePluginRegistryInvalid,
    OpenCodeAssetMissingOrInvalid,
    HookReadFailed,
    OpenCodeAssetReadFailed,
    ClaudeAssetReadFailed,
    PiAssetReadFailed,
    AgentTraceDbConnectionFailed,
    AgentTraceDbSchemaNotReady,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FixResult {
    Fixed,
    Skipped,
    Manual,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DoctorProblem {
    pub(crate) kind: ProblemKind,
    pub(crate) category: ProblemCategory,
    pub(crate) severity: ProblemSeverity,
    pub(crate) fixability: ProblemFixability,
    pub(crate) summary: String,
    pub(crate) remediation: String,
    pub(crate) next_action: &'static str,
    pub(super) scope: Option<IntegrationGroupKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DoctorFixResultRecord {
    pub(crate) category: ProblemCategory,
    pub(crate) outcome: FixResult,
    pub(crate) detail: String,
}

pub(super) fn problem_category(category: ProblemCategory) -> &'static str {
    match category {
        ProblemCategory::GlobalState => "global_state",
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
