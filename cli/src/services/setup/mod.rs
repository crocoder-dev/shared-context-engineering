use anyhow::{bail, Context, Result};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::services::config::{AuthMode, ResolvedAccessPolicy};
use crate::services::error::AuthPolicyConfigFile;
use crate::services::style::{label, success, value};
use crate::services::{default_paths, default_paths::RepoPaths};

#[allow(dead_code)]
pub(crate) mod access_preferences;
mod atomic_write;
pub(crate) mod claude_telemetry;
pub mod command;
pub(crate) mod config_merge;
pub(crate) mod hook_merge;

#[derive(Debug)]
struct MissingGitRemoteError {
    remote_name: String,
}

impl std::fmt::Display for MissingGitRemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Git remote '{}' has no configured URL. Try: run 'git remote add {} <url>', then rerun 'sce setup'.",
            self.remote_name, self.remote_name
        )
    }
}

impl std::error::Error for MissingGitRemoteError {}

pub(crate) fn is_missing_git_remote_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<MissingGitRemoteError>().is_some()
}

fn repo_local_config_bootstrap_payload() -> String {
    format!(
        "{{\n  \"$schema\": \"{}\",\n  \"agent_trace\": {{\n    \"auto_sync\": true\n  }},\n  \"policies\": {{\n    \"attribution_hooks\": {{\n      \"enabled\": true\n    }}\n  }}\n}}\n",
        crate::services::agent_trace::sce_config_schema_url()
    )
}

pub const NAME: &str = "setup";

#[derive(Debug)]
pub enum GitRepositoryResolutionError {
    NotGitRepository(anyhow::Error),

    Unexpected(anyhow::Error),
}

impl std::fmt::Display for GitRepositoryResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotGitRepository(source) | Self::Unexpected(source) => write!(f, "{source:#}"),
        }
    }
}

impl std::error::Error for GitRepositoryResolutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotGitRepository(source) | Self::Unexpected(source) => Some(source.as_ref()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GitExitKind {
    NotRepository,
    Other,
}

const NOT_GIT_REPOSITORY_PREFIX: &str = "fatal: not a git repository";

fn classify_git_exit(stderr: &str) -> GitExitKind {
    if stderr.starts_with(NOT_GIT_REPOSITORY_PREFIX) {
        GitExitKind::NotRepository
    } else {
        GitExitKind::Other
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupTarget {
    OpenCode,
    Claude,
    Pi,
    Codex,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConcreteSetupTarget {
    OpenCode,
    Claude,
    Pi,
    Codex,
}

impl From<ConcreteSetupTarget> for SetupTarget {
    fn from(target: ConcreteSetupTarget) -> Self {
        match target {
            ConcreteSetupTarget::OpenCode => Self::OpenCode,
            ConcreteSetupTarget::Claude => Self::Claude,
            ConcreteSetupTarget::Pi => Self::Pi,
            ConcreteSetupTarget::Codex => Self::Codex,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmbeddedAsset {
    pub relative_path: &'static str,
    pub bytes: &'static [u8],
    pub sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalWorkflow {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub command_slug: &'static str,
    pub skill_slug: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/setup_embedded_assets.rs"));
include!(concat!(env!("OUT_DIR"), "/optional_workflows.rs"));

pub fn iter_required_hook_assets() -> std::slice::Iter<'static, EmbeddedAsset> {
    HOOK_EMBEDDED_ASSETS.iter()
}

fn embedded_assets_for_concrete_target(target: ConcreteSetupTarget) -> &'static [EmbeddedAsset] {
    match target {
        ConcreteSetupTarget::OpenCode => OPENCODE_EMBEDDED_ASSETS,
        ConcreteSetupTarget::Claude => CLAUDE_EMBEDDED_ASSETS,
        ConcreteSetupTarget::Pi => PI_EMBEDDED_ASSETS,
        ConcreteSetupTarget::Codex => CODEX_EMBEDDED_ASSETS,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WorkflowAssetLayout {
    command_dir: Option<&'static str>,
    skills_dir: &'static str,
}

fn workflow_asset_layout(target: ConcreteSetupTarget) -> WorkflowAssetLayout {
    match target {
        ConcreteSetupTarget::OpenCode => WorkflowAssetLayout {
            command_dir: Some(default_paths::opencode_asset::OPENCODE_COMMAND_DIR),
            skills_dir: default_paths::opencode_asset::SKILLS_DIR,
        },
        ConcreteSetupTarget::Claude => WorkflowAssetLayout {
            command_dir: Some(default_paths::claude_asset::COMMANDS_DIR),
            skills_dir: default_paths::claude_asset::SKILLS_DIR,
        },
        ConcreteSetupTarget::Pi => WorkflowAssetLayout {
            command_dir: Some(default_paths::pi_asset::PROMPTS_DIR),
            skills_dir: default_paths::pi_asset::SKILLS_DIR,
        },
        ConcreteSetupTarget::Codex => WorkflowAssetLayout {
            command_dir: None,
            skills_dir: default_paths::codex_asset::SKILLS_DIR,
        },
    }
}

fn asset_belongs_to_optional_workflow(
    relative_path: &str,
    workflow: &OptionalWorkflow,
    layout: WorkflowAssetLayout,
) -> bool {
    let is_command_asset = layout.command_dir.is_some_and(|command_dir| {
        relative_path == format!("{command_dir}/{}.md", workflow.command_slug)
    });
    let skill_prefix = format!("{}/{}/", layout.skills_dir, workflow.skill_slug);

    is_command_asset || relative_path.starts_with(&skill_prefix)
}

pub fn iter_embedded_assets_for_setup_target_with_selection(
    target: ConcreteSetupTarget,
    selected_optional_workflows: &[impl AsRef<str>],
) -> std::vec::IntoIter<&'static EmbeddedAsset> {
    let unselected: Vec<&'static OptionalWorkflow> = OPTIONAL_WORKFLOWS
        .iter()
        .filter(|workflow| {
            !selected_optional_workflows
                .iter()
                .any(|selected| selected.as_ref() == workflow.id)
        })
        .collect();

    let layout = workflow_asset_layout(target);
    let assets: Vec<&'static EmbeddedAsset> = embedded_assets_for_concrete_target(target)
        .iter()
        .filter(|asset| {
            !unselected.iter().any(|workflow| {
                asset_belongs_to_optional_workflow(asset.relative_path, workflow, layout)
            })
        })
        .collect();

    assets.into_iter()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupMode {
    Interactive,
    NonInteractive(SetupTarget),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetPersistence {
    Merge,
    Replace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetupDispatch {
    Proceed {
        targets: Vec<ConcreteSetupTarget>,
        target_persistence: TargetPersistence,
        optional_workflows: Option<Vec<String>>,
        agent_trace_auto_sync: Option<bool>,
        attribution_hooks_enabled: Option<bool>,
        auth_mode: Option<AuthMode>,
        telemetry_enabled: Option<bool>,
        login_intent: SetupLoginIntent,
    },
    Cancelled,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetupCliOptions {
    pub target: Option<SetupTarget>,
    pub non_interactive: bool,
    pub hooks: bool,
    pub repo_path: Option<PathBuf>,
    pub bootstrap_context: bool,

    pub workflows: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub config_mode: Option<SetupMode>,
    pub install_hooks: bool,
    pub hooks_repo_path: Option<PathBuf>,
    pub context_only: bool,

    pub optional_workflows: Option<Vec<String>>,
}

pub fn resolve_setup_request(options: SetupCliOptions) -> Result<SetupRequest> {
    if options.repo_path.is_some() && !options.hooks {
        bail!(
            "Option '--repo' requires '--hooks'. Try: run 'sce setup --hooks --repo <path>' or remove '--repo'."
        );
    }

    let optional_workflows = if options.workflows.is_empty() {
        None
    } else {
        Some(validate_optional_workflow_slugs(&options.workflows)?)
    };

    if options.bootstrap_context {
        if optional_workflows.is_some() {
            bail!(
                "Option '--workflow' cannot be used with '--bootstrap-context'. Try: run 'sce setup --bootstrap-context' alone, then install optional workflows with a target run such as 'sce setup --claude --non-interactive --workflow <slug>'."
            );
        }

        let has_other_setup_options = options.non_interactive
            || options.target.is_some()
            || options.hooks
            || options.repo_path.is_some();
        if has_other_setup_options {
            bail!(
                "Option '--bootstrap-context' must be used alone. Try: run 'sce setup --bootstrap-context', or omit it because normal setup paths ensure the context baseline automatically."
            );
        }

        return Ok(SetupRequest {
            config_mode: None,
            install_hooks: false,
            hooks_repo_path: None,
            context_only: true,
            optional_workflows: None,
        });
    }

    let config_mode = match options.target {
        Some(target) => Some(SetupMode::NonInteractive(target)),
        None if options.non_interactive && !options.hooks => bail!(
            "Option '--non-interactive' requires a target flag. Try: 'sce setup --opencode --non-interactive', 'sce setup --claude --non-interactive', 'sce setup --pi --non-interactive', 'sce setup --codex --non-interactive', or 'sce setup --all --non-interactive'."
        ),
        None if options.hooks => None,
        None => Some(SetupMode::Interactive),
    };

    if config_mode.is_none() && optional_workflows.is_some() {
        bail!(
            "Option '--workflow' requires a target flag because a hooks-only run installs no target assets. Try: 'sce setup --claude --non-interactive --workflow <slug>', or drop '--workflow'."
        );
    }

    let install_hooks = options.hooks || (config_mode == Some(SetupMode::Interactive));

    Ok(SetupRequest {
        config_mode,
        install_hooks,
        hooks_repo_path: options.repo_path,
        context_only: false,
        optional_workflows,
    })
}

fn validate_optional_workflow_slugs(raw_slugs: &[String]) -> Result<Vec<String>> {
    let mut selected: Vec<String> = Vec::new();

    for raw in raw_slugs {
        let slug = raw.trim();
        let Some(workflow) = OPTIONAL_WORKFLOWS
            .iter()
            .find(|workflow| workflow.id == slug)
        else {
            bail!(
                "Unknown optional workflow '{raw}' for '--workflow'. Available workflows: {}. Try: rerun with one of those slugs, or omit '--workflow' to install no optional workflow.",
                available_optional_workflow_slugs()
            );
        };

        if !selected.iter().any(|id| id == workflow.id) {
            selected.push(workflow.id.to_string());
        }
    }

    Ok(selected)
}

fn available_optional_workflow_slugs() -> String {
    if OPTIONAL_WORKFLOWS.is_empty() {
        return "none".to_string();
    }

    OPTIONAL_WORKFLOWS
        .iter()
        .map(|workflow| workflow.id)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn run_setup_for_targets(
    repository_root: &Path,
    targets: &[ConcreteSetupTarget],
    target_persistence: TargetPersistence,
    optional_workflows: Option<&[String]>,
    agent_trace_auto_sync: Option<bool>,
    attribution_hooks_enabled: Option<bool>,
) -> Result<String> {
    let selected_optional_workflows = match optional_workflows {
        Some(selection) => selection.to_vec(),
        None => persisted_optional_workflows(repository_root),
    };

    let outcome =
        install_embedded_setup_assets(repository_root, targets, &selected_optional_workflows)
            .with_context(|| {
                format!(
                    "Setup installation failed for {}",
                    setup_targets_label(targets)
                )
            })?;

    persist_integration_targets(
        repository_root,
        targets,
        target_persistence,
        &selected_optional_workflows,
        agent_trace_auto_sync,
        attribution_hooks_enabled,
    )
    .with_context(|| {
        format!(
            "Setup assets were installed for {} but failed to update repo-local config",
            setup_targets_label(targets)
        )
    })?;

    Ok(format_setup_install_success_message(&outcome))
}

pub fn persisted_integration_targets(repository_root: &Path) -> Vec<ConcreteSetupTarget> {
    use crate::services::config::IntegrationTargetId;

    let Some(integrations) =
        persisted_file_config(repository_root).and_then(|config| config.integrations)
    else {
        return Vec::new();
    };

    let mut targets = Vec::new();
    for target in integrations.value.target {
        let concrete = match target {
            IntegrationTargetId::Opencode => ConcreteSetupTarget::OpenCode,
            IntegrationTargetId::Claude => ConcreteSetupTarget::Claude,
            IntegrationTargetId::Pi => ConcreteSetupTarget::Pi,
            IntegrationTargetId::Codex => ConcreteSetupTarget::Codex,
        };
        if !targets.contains(&concrete) {
            targets.push(concrete);
        }
    }
    targets
}

pub fn persisted_optional_workflows(repository_root: &Path) -> Vec<String> {
    persisted_file_config(repository_root)
        .and_then(|config| config.integrations)
        .map(|integrations| integrations.value.optional_workflows)
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PersistedSetupConfirmations {
    pub agent_trace_auto_sync: Option<bool>,
    pub attribution_hooks_enabled: Option<bool>,
    pub auth_mode: Option<AuthMode>,
    pub telemetry_enabled: Option<bool>,
    pub global_config_invalid: bool,
}

impl PersistedSetupConfirmations {
    pub(crate) fn with_global_access_preferences(
        self,
        preferences: access_preferences::GlobalAccessPreferences,
    ) -> Self {
        use access_preferences::GlobalAccessPreferences;

        match preferences {
            GlobalAccessPreferences::Missing => self,
            GlobalAccessPreferences::Valid {
                auth_mode,
                telemetry_enabled,
            } => Self {
                auth_mode,
                telemetry_enabled,
                ..self
            },
            GlobalAccessPreferences::Invalid => Self {
                global_config_invalid: true,
                ..self
            },
        }
    }
}

pub fn persisted_setup_confirmations(repository_root: &Path) -> PersistedSetupConfirmations {
    let Some(config) = persisted_file_config(repository_root) else {
        return PersistedSetupConfirmations::default();
    };

    PersistedSetupConfirmations {
        agent_trace_auto_sync: config.agent_trace_auto_sync.map(|setting| setting.value),
        attribution_hooks_enabled: config
            .attribution_hooks_enabled
            .map(|setting| setting.value),
        ..PersistedSetupConfirmations::default()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SetupAccessGate {
    pub auth_locked_by: Option<AuthPolicyConfigFile>,
    pub effective_mandatory: bool,
    pub auth_db_present: bool,
}

impl SetupAccessGate {
    pub(crate) fn from_policy(policy: &ResolvedAccessPolicy, auth_db_present: bool) -> Self {
        let auth_locked_by = policy
            .auth_disabled_config_file()
            .filter(|file| *file != AuthPolicyConfigFile::Global);

        Self {
            auth_locked_by,
            effective_mandatory: policy.auth_mode.value == AuthMode::Mandatory,
            auth_db_present,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupLoginIntent {
    NotOffered,
    Declined,
    Requested,
}

fn persisted_file_config(
    repository_root: &Path,
) -> Option<crate::services::config::schema::FileConfig> {
    use crate::services::config::schema::parse_file_config;
    use crate::services::config::ConfigPathSource;

    let config_path = RepoPaths::new(repository_root).sce_config_file();
    let raw = fs::read_to_string(&config_path).ok()?;

    parse_file_config(&raw, &config_path, ConfigPathSource::DefaultDiscoveredLocal).ok()
}

pub fn ensure_git_repository(directory: &Path) -> Result<PathBuf, GitRepositoryResolutionError> {
    install::ensure_git_repository(directory)
}

pub fn ensure_git_remote(repository_root: &Path, remote_name: &str) -> Result<()> {
    let remote_url = crate::services::repository_identity::resolve::lookup_remote_url_strict(
        repository_root,
        remote_name,
    )?;

    if remote_url.is_some() {
        return Ok(());
    }

    Err(anyhow::Error::new(MissingGitRemoteError {
        remote_name: remote_name.to_string(),
    }))
}

pub fn bootstrap_repo_local_config(repository_root: &Path) -> Result<()> {
    let repo_paths = RepoPaths::new(repository_root);
    let config_file = repo_paths.sce_config_file();

    if config_file.exists() {
        return Ok(());
    }

    let sce_dir = repo_paths.sce_dir();
    fs::create_dir_all(&sce_dir).with_context(|| {
        format!(
            "Failed to create repo-local config directory '{}'",
            sce_dir.display()
        )
    })?;

    fs::write(&config_file, repo_local_config_bootstrap_payload()).with_context(|| {
        format!(
            "Failed to write repo-local config file '{}'",
            config_file.display()
        )
    })?;

    Ok(())
}

const CONTEXT_TMP_GITIGNORE_CONTENT: &str = "*\n!.gitignore\n";

const CONTEXT_OVERVIEW_TEMPLATE: &str = "# Overview\n\n";
const CONTEXT_ARCHITECTURE_TEMPLATE: &str = "# Architecture\n\n";
const CONTEXT_PATTERNS_TEMPLATE: &str = "# Patterns\n\n";
const CONTEXT_GLOSSARY_TEMPLATE: &str = "# Glossary\n\n";
const CONTEXT_MAP_TEMPLATE: &str = "\
# Context Map

Primary context files:

- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`

Working areas:

- `context/plans/`
- `context/handovers/`
- `context/decisions/`
- `context/tmp/`
";

pub fn bootstrap_context_baseline(repository_root: &Path) -> Result<String> {
    let repo_paths = RepoPaths::new(repository_root);

    ensure_context_directory(&repo_paths.context_dir())?;
    ensure_context_directory(&repo_paths.context_plans_dir())?;
    ensure_context_directory(&repo_paths.context_handovers_dir())?;
    ensure_context_directory(&repo_paths.context_decisions_dir())?;
    ensure_context_directory(&repo_paths.context_tmp_dir())?;

    ensure_context_file(
        &repo_paths.context_overview_file(),
        CONTEXT_OVERVIEW_TEMPLATE,
    )?;
    ensure_context_file(
        &repo_paths.context_architecture_file(),
        CONTEXT_ARCHITECTURE_TEMPLATE,
    )?;
    ensure_context_file(
        &repo_paths.context_patterns_file(),
        CONTEXT_PATTERNS_TEMPLATE,
    )?;
    ensure_context_file(
        &repo_paths.context_glossary_file(),
        CONTEXT_GLOSSARY_TEMPLATE,
    )?;
    ensure_context_file(&repo_paths.context_map_file(), CONTEXT_MAP_TEMPLATE)?;
    ensure_context_file(
        &repo_paths.context_tmp_gitignore_file(),
        CONTEXT_TMP_GITIGNORE_CONTENT,
    )?;

    Ok(success("Context baseline ensured."))
}

fn ensure_context_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("Failed to create context directory '{}'", path.display()))
}

fn ensure_context_file(path: &Path, content: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        ensure_context_directory(parent)?;
    }

    fs::write(path, content)
        .with_context(|| format!("Failed to write context baseline file '{}'", path.display()))
}

fn format_setup_install_success_message(outcome: &SetupInstallOutcome) -> String {
    let selected_targets = outcome
        .target_results
        .iter()
        .map(|result| setup_target_label(result.target.into()))
        .collect::<Vec<_>>()
        .join(", ");

    let mut lines = vec![
        format!("{}", success("Setup completed successfully.")),
        format!(
            "{} {}",
            label("Selected target(s):"),
            value(&selected_targets)
        ),
    ];

    for result in &outcome.target_results {
        lines.push(format!(
            "- {}: {} {} {} '{}'",
            label(&format!("{}:", setup_target_label(result.target.into()))),
            success("installed"),
            value(&format!("{} file(s) to", result.installed_file_count)),
            value("'"),
            value(&format!("{}'", result.destination_root.display()))
        ));
    }

    lines.join("\n")
}

pub fn format_required_hook_install_success_message(
    outcome: &RequiredHooksInstallOutcome,
) -> String {
    let mut lines = vec![
        format!("{}", success("Hook setup completed successfully.")),
        format!(
            "{} {}",
            label("Repository root:"),
            value(&format!("'{}'", outcome.repository_root.display()))
        ),
        format!(
            "{} {}",
            label("Hooks directory:"),
            value(&format!("'{}'", outcome.hooks_directory.display()))
        ),
    ];

    for result in &outcome.hook_results {
        let status_text = required_hook_status_label(result.status);
        let styled_status = match result.status {
            RequiredHookInstallStatus::Installed | RequiredHookInstallStatus::Updated => {
                success(status_text)
            }
            RequiredHookInstallStatus::Skipped => value(status_text),
        };
        lines.push(format!(
            "- {}: {} {} '{}'",
            label(&format!("{}:", result.hook_name)),
            styled_status,
            value("at"),
            value(&format!("'{}'", result.hook_path.display()))
        ));

        if result.unreachable_block_advisory {
            lines.push(format!(
                "  {} '{}' ends with 'exec'/'exit' before the SCE managed block, so the block will not run. Move it above that line.",
                label("Advisory:"),
                result.hook_name
            ));
        }
    }

    lines.join("\n")
}

fn required_hook_status_label(status: RequiredHookInstallStatus) -> &'static str {
    match status {
        RequiredHookInstallStatus::Installed => "installed",
        RequiredHookInstallStatus::Updated => "updated",
        RequiredHookInstallStatus::Skipped => "skipped",
    }
}

fn setup_targets_label(targets: &[ConcreteSetupTarget]) -> String {
    targets
        .iter()
        .map(|target| setup_target_label((*target).into()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn setup_target_label(target: SetupTarget) -> &'static str {
    match target {
        SetupTarget::OpenCode => "OpenCode",
        SetupTarget::Claude => "Claude",
        SetupTarget::Pi => "Pi",
        SetupTarget::Codex => "Codex",
        SetupTarget::All => "All",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupInstallTargetResult {
    pub target: ConcreteSetupTarget,
    pub destination_root: PathBuf,
    pub installed_file_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupInstallOutcome {
    pub target_results: Vec<SetupInstallTargetResult>,
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

    pub unreachable_block_advisory: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredHooksInstallOutcome {
    pub repository_root: PathBuf,
    pub hooks_directory: PathBuf,
    pub hook_results: Vec<RequiredHookInstallResult>,
}

pub fn install_required_git_hooks(repository_root: &Path) -> Result<RequiredHooksInstallOutcome> {
    install::install_required_git_hooks(repository_root)
}

pub fn install_embedded_setup_assets(
    repository_root: &Path,
    targets: &[ConcreteSetupTarget],
    selected_optional_workflows: &[String],
) -> Result<SetupInstallOutcome> {
    install::install_embedded_setup_assets(repository_root, targets, selected_optional_workflows)
}

pub(crate) fn repair_merge_target_asset(
    repository_root: &Path,
    target: ConcreteSetupTarget,
    relative_path: &str,
) -> Result<()> {
    install::repair_merge_target_asset(repository_root, target, relative_path)
}

pub(crate) fn setup_install_recovery_guidance(
    target: ConcreteSetupTarget,
    destination_root: &Path,
) -> String {
    format!(
        "Setup for {} does not create backups. Recover '{}' from version control if needed.",
        setup_target_label(target.into()),
        destination_root.display()
    )
}

pub(crate) fn hook_install_recovery_guidance(hook_path: &Path) -> String {
    format!(
        "Hook setup does not create backups. Recover '{}' from version control if needed.",
        hook_path.display()
    )
}

pub(crate) fn cleanup_path_if_exists(path: &Path) {
    let cleanup_result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };

    if let Err(e) = cleanup_result {
        eprintln!(
            "Warning: Failed to clean up temporary path '{}': {}",
            path.display(),
            e
        );
    }
}

pub(crate) fn concrete_targets_for(target: SetupTarget) -> &'static [ConcreteSetupTarget] {
    match target {
        SetupTarget::OpenCode => &[ConcreteSetupTarget::OpenCode],
        SetupTarget::Claude => &[ConcreteSetupTarget::Claude],
        SetupTarget::Pi => &[ConcreteSetupTarget::Pi],
        SetupTarget::Codex => &[ConcreteSetupTarget::Codex],
        SetupTarget::All => &[
            ConcreteSetupTarget::OpenCode,
            ConcreteSetupTarget::Claude,
            ConcreteSetupTarget::Pi,
            ConcreteSetupTarget::Codex,
        ],
    }
}

fn integration_target_id_str(target: ConcreteSetupTarget) -> &'static str {
    match target {
        ConcreteSetupTarget::OpenCode => "opencode",
        ConcreteSetupTarget::Claude => "claude",
        ConcreteSetupTarget::Pi => "pi",
        ConcreteSetupTarget::Codex => "codex",
    }
}

pub fn persist_integration_targets(
    repository_root: &Path,
    targets: &[ConcreteSetupTarget],
    target_persistence: TargetPersistence,
    selected_optional_workflows: &[String],
    agent_trace_auto_sync: Option<bool>,
    attribution_hooks_enabled: Option<bool>,
) -> Result<()> {
    let repo_paths = RepoPaths::new(repository_root);
    let config_file = repo_paths.sce_config_file();

    if config_file.exists() && crate::services::config::validate_config_file(&config_file).is_err()
    {
        return Ok(());
    }

    let raw = if config_file.exists() {
        fs::read_to_string(&config_file)
            .with_context(|| format!("Failed to read config file '{}'", config_file.display()))?
    } else {
        bootstrap_repo_local_config(repository_root)?;
        fs::read_to_string(&config_file)
            .with_context(|| format!("Failed to read config file '{}'", config_file.display()))?
    };

    let mut config: serde_json::Value = serde_json::from_str(&raw).with_context(|| {
        format!(
            "Config file '{}' must contain valid JSON.",
            config_file.display()
        )
    })?;

    let config_obj = config.as_object_mut().with_context(|| {
        format!(
            "Config file '{}' must contain a top-level JSON object.",
            config_file.display()
        )
    })?;

    let mut existing_targets: Vec<String> = match target_persistence {
        TargetPersistence::Merge => config_obj
            .get("integrations")
            .and_then(|i| i.get("target"))
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        TargetPersistence::Replace => Vec::new(),
    };

    for concrete in targets {
        let id_str = integration_target_id_str(*concrete);
        let id_owned = id_str.to_string();
        if !existing_targets.contains(&id_owned) {
            existing_targets.push(id_owned);
        }
    }

    config_obj.insert(
        "integrations".to_string(),
        json!({
            "target": existing_targets,
            "optional_workflows": selected_optional_workflows,
        }),
    );

    if let Some(value) = agent_trace_auto_sync {
        let agent_trace = config_obj.entry("agent_trace").or_insert_with(|| json!({}));
        let agent_trace_obj = agent_trace.as_object_mut().with_context(|| {
            format!(
                "Config file '{}' must contain an object at 'agent_trace'.",
                config_file.display()
            )
        })?;
        agent_trace_obj.insert("auto_sync".to_string(), json!(value));
    }

    if let Some(value) = attribution_hooks_enabled {
        let policies = config_obj.entry("policies").or_insert_with(|| json!({}));
        let policies_obj = policies.as_object_mut().with_context(|| {
            format!(
                "Config file '{}' must contain an object at 'policies'.",
                config_file.display()
            )
        })?;
        let attribution_hooks = policies_obj
            .entry("attribution_hooks")
            .or_insert_with(|| json!({}));
        let attribution_hooks_obj = attribution_hooks.as_object_mut().with_context(|| {
            format!(
                "Config file '{}' must contain an object at 'policies.attribution_hooks'.",
                config_file.display()
            )
        })?;
        attribution_hooks_obj.insert("enabled".to_string(), json!(value));
    }

    let updated = serde_json::to_string_pretty(&config).with_context(|| {
        format!(
            "Failed to serialize updated config for '{}'",
            config_file.display()
        )
    })? + "\n";

    fs::write(&config_file, updated)
        .with_context(|| format!("Failed to write config file '{}'", config_file.display()))?;

    Ok(())
}

mod install;

pub trait SetupTargetPrompter {
    fn prompt_targets(
        &self,
        defaults: &[ConcreteSetupTarget],
    ) -> Result<Option<Vec<ConcreteSetupTarget>>>;
    fn prompt_optional_workflows(&self, defaults: &[String]) -> Result<Option<Vec<String>>>;
    fn prompt_agent_trace_auto_sync(&self, default: bool) -> Result<Option<bool>>;
    fn prompt_attribution_hooks_enabled(&self, default: bool) -> Result<Option<bool>>;
    fn prompt_auth_mode(&self, default: AuthMode) -> Result<Option<AuthMode>>;
    fn prompt_telemetry_consent(&self, default: bool) -> Result<Option<bool>>;
    fn prompt_login(&self) -> Result<Option<bool>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct InquireSetupTargetPrompter;

impl SetupTargetPrompter for InquireSetupTargetPrompter {
    fn prompt_targets(
        &self,
        defaults: &[ConcreteSetupTarget],
    ) -> Result<Option<Vec<ConcreteSetupTarget>>> {
        prompt::prompt_targets(defaults)
    }

    fn prompt_optional_workflows(&self, defaults: &[String]) -> Result<Option<Vec<String>>> {
        prompt::prompt_optional_workflows(defaults)
    }

    fn prompt_agent_trace_auto_sync(&self, default: bool) -> Result<Option<bool>> {
        prompt::prompt_agent_trace_auto_sync(default)
    }

    fn prompt_attribution_hooks_enabled(&self, default: bool) -> Result<Option<bool>> {
        prompt::prompt_attribution_hooks_enabled(default)
    }

    fn prompt_auth_mode(&self, default: AuthMode) -> Result<Option<AuthMode>> {
        prompt::prompt_auth_mode(default)
    }

    fn prompt_telemetry_consent(&self, default: bool) -> Result<Option<bool>> {
        prompt::prompt_telemetry_consent(default)
    }

    fn prompt_login(&self) -> Result<Option<bool>> {
        prompt::prompt_login()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SetupPromptTarget(ConcreteSetupTarget);

impl std::fmt::Display for SetupPromptTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", setup_prompt_target_label(*self))
    }
}

fn setup_prompt_target_label(target: SetupPromptTarget) -> String {
    prompt::setup_prompt_target_label(target)
}

mod prompt;

pub fn resolve_setup_dispatch<P>(
    mode: SetupMode,
    prompter: &P,
    optional_workflow_defaults: &[String],
    configured_targets: &[ConcreteSetupTarget],
    persisted_confirmations: PersistedSetupConfirmations,
    access_gate: SetupAccessGate,
) -> Result<SetupDispatch>
where
    P: SetupTargetPrompter,
{
    match mode {
        SetupMode::Interactive => {
            let Some(targets) = prompter.prompt_targets(configured_targets)? else {
                return Ok(SetupDispatch::Cancelled);
            };

            let Some(optional_workflows) =
                prompter.prompt_optional_workflows(optional_workflow_defaults)?
            else {
                return Ok(SetupDispatch::Cancelled);
            };

            let Some(agent_trace_auto_sync) =
                persisted_confirmations.agent_trace_auto_sync.map_or_else(
                    || prompter.prompt_agent_trace_auto_sync(true),
                    |value| Ok(Some(value)),
                )?
            else {
                return Ok(SetupDispatch::Cancelled);
            };

            let Some(attribution_hooks_enabled) = persisted_confirmations
                .attribution_hooks_enabled
                .map_or_else(
                    || prompter.prompt_attribution_hooks_enabled(true),
                    |value| Ok(Some(value)),
                )?
            else {
                return Ok(SetupDispatch::Cancelled);
            };

            let Some(access) =
                resolve_setup_access_choices(prompter, persisted_confirmations, access_gate)?
            else {
                return Ok(SetupDispatch::Cancelled);
            };

            Ok(SetupDispatch::Proceed {
                targets,
                target_persistence: TargetPersistence::Replace,
                optional_workflows: Some(optional_workflows),
                agent_trace_auto_sync: Some(agent_trace_auto_sync),
                attribution_hooks_enabled: Some(attribution_hooks_enabled),
                auth_mode: access.auth_mode,
                telemetry_enabled: access.telemetry_enabled,
                login_intent: access.login_intent,
            })
        }
        SetupMode::NonInteractive(target) => Ok(SetupDispatch::Proceed {
            targets: concrete_targets_for(target).to_vec(),
            target_persistence: TargetPersistence::Merge,
            optional_workflows: None,
            agent_trace_auto_sync: None,
            attribution_hooks_enabled: None,
            auth_mode: None,
            telemetry_enabled: None,
            login_intent: SetupLoginIntent::NotOffered,
        }),
    }
}

const FIRST_RUN_AUTH_MODE_DEFAULT: AuthMode = AuthMode::Optional;
const FIRST_RUN_TELEMETRY_CONSENT_DEFAULT: bool = false;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SetupAccessChoices {
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
    login_intent: SetupLoginIntent,
}

fn resolve_setup_access_choices<P>(
    prompter: &P,
    persisted: PersistedSetupConfirmations,
    gate: SetupAccessGate,
) -> Result<Option<SetupAccessChoices>>
where
    P: SetupTargetPrompter,
{
    let mut choices = SetupAccessChoices {
        auth_mode: None,
        telemetry_enabled: None,
        login_intent: SetupLoginIntent::NotOffered,
    };

    if gate.auth_locked_by.is_some() {
        return Ok(Some(choices));
    }

    let global_writable = !persisted.global_config_invalid;

    if global_writable && persisted.auth_mode.is_none() {
        let Some(mode) = prompter.prompt_auth_mode(FIRST_RUN_AUTH_MODE_DEFAULT)? else {
            return Ok(None);
        };
        choices.auth_mode = Some(mode);
    }

    let prospective_mode =
        choices
            .auth_mode
            .or(persisted.auth_mode)
            .unwrap_or(if gate.effective_mandatory {
                AuthMode::Mandatory
            } else {
                AuthMode::Optional
            });

    if prospective_mode == AuthMode::Disabled {
        return Ok(Some(choices));
    }

    if global_writable && persisted.telemetry_enabled.is_none() {
        let Some(consent) =
            prompter.prompt_telemetry_consent(FIRST_RUN_TELEMETRY_CONSENT_DEFAULT)?
        else {
            return Ok(None);
        };
        choices.telemetry_enabled = Some(consent);
    }

    let access_prompted = choices.auth_mode.is_some() || choices.telemetry_enabled.is_some();
    let login_offered = prospective_mode == AuthMode::Optional
        && !gate.effective_mandatory
        && (access_prompted || !gate.auth_db_present);

    if login_offered {
        let Some(accepted) = prompter.prompt_login()? else {
            return Ok(None);
        };
        choices.login_intent = if accepted {
            SetupLoginIntent::Requested
        } else {
            SetupLoginIntent::Declined
        };
    }

    Ok(Some(choices))
}

pub fn setup_cancelled_text() -> String {
    value("Setup cancelled. No files were changed.")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::services::config::{
        ConfigPathSource, ResolvedValue, TelemetryDisabledReason, TelemetryPolicyState, ValueSource,
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum PromptCall {
        Targets,
        Workflows,
        AutoSync(bool),
        Attribution(bool),
        AuthMode(AuthMode),
        Telemetry(bool),
        Login,
    }

    struct RecordingPrompter {
        calls: RefCell<Vec<PromptCall>>,
        auth_mode: Option<AuthMode>,
        telemetry: Option<bool>,
        login: Option<bool>,
    }

    impl RecordingPrompter {
        fn answering(
            auth_mode: Option<AuthMode>,
            telemetry: Option<bool>,
            login: Option<bool>,
        ) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                auth_mode,
                telemetry,
                login,
            }
        }

        fn record(&self, call: PromptCall) {
            self.calls.borrow_mut().push(call);
        }

        fn access_calls(&self) -> Vec<PromptCall> {
            self.calls
                .borrow()
                .iter()
                .copied()
                .filter(|call| {
                    matches!(
                        call,
                        PromptCall::AuthMode(_) | PromptCall::Telemetry(_) | PromptCall::Login
                    )
                })
                .collect()
        }
    }

    impl SetupTargetPrompter for RecordingPrompter {
        fn prompt_targets(
            &self,
            _defaults: &[ConcreteSetupTarget],
        ) -> Result<Option<Vec<ConcreteSetupTarget>>> {
            self.record(PromptCall::Targets);
            Ok(Some(vec![ConcreteSetupTarget::Claude]))
        }

        fn prompt_optional_workflows(&self, _defaults: &[String]) -> Result<Option<Vec<String>>> {
            self.record(PromptCall::Workflows);
            Ok(Some(Vec::new()))
        }

        fn prompt_agent_trace_auto_sync(&self, default: bool) -> Result<Option<bool>> {
            self.record(PromptCall::AutoSync(default));
            Ok(Some(true))
        }

        fn prompt_attribution_hooks_enabled(&self, default: bool) -> Result<Option<bool>> {
            self.record(PromptCall::Attribution(default));
            Ok(Some(true))
        }

        fn prompt_auth_mode(&self, default: AuthMode) -> Result<Option<AuthMode>> {
            self.record(PromptCall::AuthMode(default));
            Ok(self.auth_mode)
        }

        fn prompt_telemetry_consent(&self, default: bool) -> Result<Option<bool>> {
            self.record(PromptCall::Telemetry(default));
            Ok(self.telemetry)
        }

        fn prompt_login(&self) -> Result<Option<bool>> {
            self.record(PromptCall::Login);
            Ok(self.login)
        }
    }

    fn interactive_dispatch(
        prompter: &RecordingPrompter,
        persisted: PersistedSetupConfirmations,
        gate: SetupAccessGate,
    ) -> SetupDispatch {
        resolve_setup_dispatch(SetupMode::Interactive, prompter, &[], &[], persisted, gate)
            .expect("dispatch should resolve")
    }

    fn access_values(
        dispatch: &SetupDispatch,
    ) -> (Option<AuthMode>, Option<bool>, SetupLoginIntent) {
        match dispatch {
            SetupDispatch::Proceed {
                auth_mode,
                telemetry_enabled,
                login_intent,
                ..
            } => (*auth_mode, *telemetry_enabled, *login_intent),
            SetupDispatch::Cancelled => panic!("dispatch was cancelled"),
        }
    }

    #[test]
    fn first_interactive_run_asks_access_prompts_after_existing_prompts_with_defaults() {
        let prompter =
            RecordingPrompter::answering(Some(AuthMode::Optional), Some(true), Some(true));

        let dispatch = interactive_dispatch(
            &prompter,
            PersistedSetupConfirmations::default(),
            SetupAccessGate::default(),
        );

        assert_eq!(
            *prompter.calls.borrow(),
            vec![
                PromptCall::Targets,
                PromptCall::Workflows,
                PromptCall::AutoSync(true),
                PromptCall::Attribution(true),
                PromptCall::AuthMode(AuthMode::Optional),
                PromptCall::Telemetry(false),
                PromptCall::Login,
            ]
        );
        assert_eq!(
            access_values(&dispatch),
            (
                Some(AuthMode::Optional),
                Some(true),
                SetupLoginIntent::Requested
            )
        );
    }

    type AccessValues = (Option<AuthMode>, Option<bool>, SetupLoginIntent);
    type AccessCase = (
        &'static str,
        Option<AuthMode>,
        PersistedSetupConfirmations,
        SetupAccessGate,
        Vec<PromptCall>,
        AccessValues,
    );

    fn persisted_access(
        auth_mode: Option<AuthMode>,
        telemetry_enabled: Option<bool>,
        global_config_invalid: bool,
    ) -> PersistedSetupConfirmations {
        PersistedSetupConfirmations {
            auth_mode,
            telemetry_enabled,
            global_config_invalid,
            ..PersistedSetupConfirmations::default()
        }
    }

    fn access_gate(
        auth_locked_by: Option<AuthPolicyConfigFile>,
        effective_mandatory: bool,
        auth_db_present: bool,
    ) -> SetupAccessGate {
        SetupAccessGate {
            auth_locked_by,
            effective_mandatory,
            auth_db_present,
        }
    }

    fn assert_access_cases(cases: Vec<AccessCase>) {
        for (name, auth_answer, persisted, gate, expected_calls, expected_values) in cases {
            let prompter = RecordingPrompter::answering(
                auth_answer,
                expected_values.1.or(Some(true)),
                Some(expected_values.2 != SetupLoginIntent::Declined),
            );

            let dispatch = interactive_dispatch(&prompter, persisted, gate);

            assert_eq!(prompter.access_calls(), expected_calls, "{name}");
            assert_eq!(access_values(&dispatch), expected_values, "{name}");
        }
    }

    #[test]
    fn access_mode_and_telemetry_prompts_follow_persisted_values_locks_and_selected_mode() {
        use AuthMode::{Disabled, Mandatory, Optional};
        use PromptCall::{AuthMode as Mode, Login, Telemetry};
        use SetupLoginIntent::{Declined, NotOffered};

        assert_access_cases(vec![
            (
                "persisted globals suppress mode and telemetry; auth.db present skips login",
                Some(Optional),
                persisted_access(Some(Optional), Some(true), false),
                access_gate(None, false, true),
                vec![],
                (None, None, NotOffered),
            ),
            (
                "persisted mode with telemetry unanswered asks telemetry then login",
                None,
                persisted_access(Some(Optional), None, false),
                access_gate(None, false, true),
                vec![Telemetry(false), Login],
                (None, Some(false), Declined),
            ),
            (
                "selected disabled skips telemetry and login",
                Some(Disabled),
                PersistedSetupConfirmations::default(),
                access_gate(None, false, false),
                vec![Mode(Optional)],
                (Some(Disabled), None, NotOffered),
            ),
            (
                "persisted global disabled skips every access prompt",
                None,
                persisted_access(Some(Disabled), None, false),
                access_gate(None, false, false),
                vec![],
                (None, None, NotOffered),
            ),
            (
                "repository lock skips mode, telemetry, and login",
                Some(Mandatory),
                PersistedSetupConfirmations::default(),
                access_gate(Some(AuthPolicyConfigFile::Repository), false, false),
                vec![],
                (None, None, NotOffered),
            ),
            (
                "explicit config lock skips mode, telemetry, and login",
                Some(Optional),
                PersistedSetupConfirmations::default(),
                access_gate(Some(AuthPolicyConfigFile::ConfigEnv), false, false),
                vec![],
                (None, None, NotOffered),
            ),
            (
                "invalid global skips mode and telemetry",
                Some(Optional),
                persisted_access(None, None, true),
                access_gate(None, false, true),
                vec![],
                (None, None, NotOffered),
            ),
        ]);
    }

    #[test]
    fn login_offer_follows_mode_auth_db_and_prompts_asked_this_run() {
        use AuthMode::{Mandatory, Optional};
        use PromptCall::{AuthMode as Mode, Login, Telemetry};
        use SetupLoginIntent::{Declined, NotOffered, Requested};

        assert_access_cases(vec![
            (
                "persisted globals without auth.db still offer login",
                Some(Optional),
                persisted_access(Some(Optional), Some(false), false),
                access_gate(None, false, false),
                vec![Login],
                (None, None, Declined),
            ),
            (
                "selected mandatory skips login",
                Some(Mandatory),
                PersistedSetupConfirmations::default(),
                access_gate(None, false, false),
                vec![Mode(Optional), Telemetry(false)],
                (Some(Mandatory), Some(false), NotOffered),
            ),
            (
                "effective mandatory skips login after an optional answer",
                Some(Optional),
                PersistedSetupConfirmations::default(),
                access_gate(None, true, false),
                vec![Mode(Optional), Telemetry(false)],
                (Some(Optional), Some(false), NotOffered),
            ),
            (
                "invalid global offers login when auth.db is absent",
                Some(Optional),
                persisted_access(None, None, true),
                access_gate(None, false, false),
                vec![Login],
                (None, None, Requested),
            ),
            (
                "answers asked this run offer login even with auth.db present",
                Some(Optional),
                PersistedSetupConfirmations::default(),
                access_gate(None, false, true),
                vec![Mode(Optional), Telemetry(false), Login],
                (Some(Optional), Some(true), Requested),
            ),
        ]);
    }

    #[test]
    fn cancelling_any_access_prompt_cancels_setup() {
        let cases = [
            RecordingPrompter::answering(None, Some(true), Some(true)),
            RecordingPrompter::answering(Some(AuthMode::Optional), None, Some(true)),
            RecordingPrompter::answering(Some(AuthMode::Optional), Some(true), None),
        ];

        for (index, prompter) in cases.iter().enumerate() {
            let dispatch = interactive_dispatch(
                prompter,
                PersistedSetupConfirmations::default(),
                SetupAccessGate::default(),
            );

            assert_eq!(dispatch, SetupDispatch::Cancelled);
            assert_eq!(prompter.access_calls().len(), index + 1);
        }
    }

    #[test]
    fn telemetry_default_ignores_claude_target_and_auth_db() {
        for auth_db_present in [false, true] {
            let prompter =
                RecordingPrompter::answering(Some(AuthMode::Optional), Some(false), Some(false));

            interactive_dispatch(
                &prompter,
                PersistedSetupConfirmations {
                    agent_trace_auto_sync: Some(true),
                    ..PersistedSetupConfirmations::default()
                },
                SetupAccessGate {
                    auth_db_present,
                    ..SetupAccessGate::default()
                },
            );

            assert!(prompter
                .calls
                .borrow()
                .contains(&PromptCall::Telemetry(false)));
        }
    }

    #[test]
    fn non_interactive_targets_prompt_nothing_and_carry_no_access_values() {
        for target in [
            SetupTarget::OpenCode,
            SetupTarget::Claude,
            SetupTarget::Pi,
            SetupTarget::Codex,
            SetupTarget::All,
        ] {
            let prompter =
                RecordingPrompter::answering(Some(AuthMode::Mandatory), Some(true), Some(true));

            let dispatch = resolve_setup_dispatch(
                SetupMode::NonInteractive(target),
                &prompter,
                &[],
                &[],
                PersistedSetupConfirmations::default(),
                SetupAccessGate::default(),
            )
            .unwrap();

            assert!(prompter.calls.borrow().is_empty());
            assert_eq!(
                access_values(&dispatch),
                (None, None, SetupLoginIntent::NotOffered)
            );
        }
    }

    #[test]
    fn access_gate_locks_only_for_repository_or_explicit_disabled() {
        let policy = |mode, source| ResolvedAccessPolicy {
            auth_mode: ResolvedValue {
                value: mode,
                source,
            },
            telemetry_enabled: ResolvedValue {
                value: false,
                source: ValueSource::Default,
            },
            telemetry: TelemetryPolicyState::Disabled(TelemetryDisabledReason::NotEnabled),
            background_cloud_allowed: false,
        };
        let local = ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredLocal);
        let global = ValueSource::ConfigFile(ConfigPathSource::DefaultDiscoveredGlobal);

        let cases = [
            (
                policy(AuthMode::Disabled, local),
                Some(AuthPolicyConfigFile::Repository),
                false,
            ),
            (
                policy(
                    AuthMode::Disabled,
                    ValueSource::ConfigFile(ConfigPathSource::Flag),
                ),
                Some(AuthPolicyConfigFile::ConfigFlag),
                false,
            ),
            (
                policy(
                    AuthMode::Disabled,
                    ValueSource::ConfigFile(ConfigPathSource::Env),
                ),
                Some(AuthPolicyConfigFile::ConfigEnv),
                false,
            ),
            (policy(AuthMode::Disabled, global), None, false),
            (policy(AuthMode::Mandatory, local), None, true),
            (
                policy(AuthMode::Optional, ValueSource::Default),
                None,
                false,
            ),
        ];

        for (policy, locked_by, mandatory) in cases {
            let gate = SetupAccessGate::from_policy(&policy, true);

            assert_eq!(
                gate,
                SetupAccessGate {
                    auth_locked_by: locked_by,
                    effective_mandatory: mandatory,
                    auth_db_present: true,
                }
            );
        }
    }

    #[test]
    fn sce_info_assets_install_only_for_claude_and_exclude_development_files() {
        let expected = [
            "skills/sce-info/.claude-plugin/plugin.json",
            "skills/sce-info/hooks/hooks.json",
            "skills/sce-info/hooks/register.ts",
        ];

        for target in concrete_targets_for(SetupTarget::All) {
            let assets: Vec<_> = embedded_assets_for_concrete_target(*target)
                .iter()
                .filter(|asset| asset.relative_path.contains("/sce-info/"))
                .collect();
            if *target == ConcreteSetupTarget::Claude {
                assert_eq!(assets.len(), expected.len());
                for path in expected {
                    let asset = assets
                        .iter()
                        .find(|asset| asset.relative_path == path)
                        .unwrap();
                    assert!(!asset.bytes.is_empty());
                }
            } else {
                assert!(assets.is_empty(), "sce-info leaked into {target:?}");
            }
        }

        for selection in [
            SetupTarget::Claude,
            SetupTarget::All,
            SetupTarget::OpenCode,
            SetupTarget::Pi,
            SetupTarget::Codex,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let unrelated = root.join(".claude/skills/custom.txt");
            fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
            fs::write(&unrelated, "user content").unwrap();
            let targets = concrete_targets_for(selection);
            install_embedded_setup_assets(root, targets, &[]).unwrap();
            let installs_claude = targets.contains(&ConcreteSetupTarget::Claude);
            for path in expected {
                let installed = root.join(".claude").join(path);
                if installs_claude {
                    let asset = CLAUDE_EMBEDDED_ASSETS
                        .iter()
                        .find(|asset| asset.relative_path == path)
                        .unwrap();
                    assert_eq!(fs::read(installed).unwrap(), asset.bytes);
                } else {
                    assert!(!installed.exists());
                }
            }
            let package = root.join(".claude/skills/sce-info");
            assert!(!package.join("tests").exists());
            assert!(!package.join("tsconfig.json").exists());
            for path in [
                ".opencode/skills/sce-info",
                ".pi/skills/sce-info",
                ".agents/skills/sce-info",
                ".codex/skills/sce-info",
            ] {
                assert!(!root.join(path).exists());
            }
            assert_eq!(fs::read_to_string(&unrelated).unwrap(), "user content");

            if installs_claude {
                let local_test = package.join("tests/local.test.ts");
                fs::create_dir_all(local_test.parent().unwrap()).unwrap();
                fs::write(&local_test, "user test").unwrap();
                fs::write(package.join("tsconfig.json"), "user config").unwrap();
                install_embedded_setup_assets(root, targets, &[]).unwrap();
                assert_eq!(fs::read_to_string(local_test).unwrap(), "user test");
                assert_eq!(
                    fs::read_to_string(package.join("tsconfig.json")).unwrap(),
                    "user config"
                );
            }
        }
    }
}
