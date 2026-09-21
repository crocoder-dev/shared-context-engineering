use anyhow::{bail, Context, Result};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::services::style::{label, success, value};
use crate::services::{default_paths, default_paths::RepoPaths};

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
pub struct EmbeddedAsset {
    pub relative_path: &'static str,
    pub bytes: &'static [u8],
    pub sha256: [u8; 32],
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredHookAsset {
    PreCommit,
    CommitMsg,
    PostCommit,
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

#[allow(dead_code)]
pub fn get_required_hook_asset(hook: RequiredHookAsset) -> Option<&'static EmbeddedAsset> {
    let hook_name = match hook {
        RequiredHookAsset::PreCommit => default_paths::hook_dir::PRE_COMMIT,
        RequiredHookAsset::CommitMsg => default_paths::hook_dir::COMMIT_MSG,
        RequiredHookAsset::PostCommit => default_paths::hook_dir::POST_COMMIT,
    };

    HOOK_EMBEDDED_ASSETS
        .iter()
        .find(|asset| asset.relative_path == hook_name)
}

fn embedded_assets_for_concrete_target(target: SetupTarget) -> &'static [EmbeddedAsset] {
    match target {
        SetupTarget::OpenCode => OPENCODE_EMBEDDED_ASSETS,
        SetupTarget::Claude => CLAUDE_EMBEDDED_ASSETS,
        SetupTarget::Pi => PI_EMBEDDED_ASSETS,
        SetupTarget::Codex => CODEX_EMBEDDED_ASSETS,
        SetupTarget::All => {
            unreachable!("meta targets are expanded into concrete targets")
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WorkflowAssetLayout {
    command_dir: Option<&'static str>,
    skills_dir: &'static str,
}

fn workflow_asset_layout(target: SetupTarget) -> WorkflowAssetLayout {
    match target {
        SetupTarget::OpenCode => WorkflowAssetLayout {
            command_dir: Some(default_paths::opencode_asset::OPENCODE_COMMAND_DIR),
            skills_dir: default_paths::opencode_asset::SKILLS_DIR,
        },
        SetupTarget::Claude => WorkflowAssetLayout {
            command_dir: Some(default_paths::claude_asset::COMMANDS_DIR),
            skills_dir: default_paths::claude_asset::SKILLS_DIR,
        },
        SetupTarget::Pi => WorkflowAssetLayout {
            command_dir: Some(default_paths::pi_asset::PROMPTS_DIR),
            skills_dir: default_paths::pi_asset::SKILLS_DIR,
        },
        SetupTarget::Codex => WorkflowAssetLayout {
            command_dir: None,
            skills_dir: default_paths::codex_asset::SKILLS_DIR,
        },
        SetupTarget::All => {
            unreachable!("meta targets are expanded into concrete targets")
        }
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
    target: SetupTarget,
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

    let mut assets: Vec<&'static EmbeddedAsset> = Vec::new();
    for concrete in concrete_targets_for(target) {
        let layout = workflow_asset_layout(*concrete);
        assets.extend(
            embedded_assets_for_concrete_target(*concrete)
                .iter()
                .filter(|asset| {
                    !unselected.iter().any(|workflow| {
                        asset_belongs_to_optional_workflow(asset.relative_path, workflow, layout)
                    })
                }),
        );
    }

    assets.into_iter()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupMode {
    Interactive,
    NonInteractive(SetupTarget),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetupDispatch {
    Proceed {
        mode: SetupMode,
        optional_workflows: Option<Vec<String>>,
        agent_trace_auto_sync: Option<bool>,
        attribution_hooks_enabled: Option<bool>,
    },
    Cancelled,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SetupCliOptions {
    pub help: bool,
    pub non_interactive: bool,
    pub opencode: bool,
    pub claude: bool,
    pub pi: bool,
    pub codex: bool,
    pub all: bool,
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
            || options.opencode
            || options.claude
            || options.pi
            || options.codex
            || options.all
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

    let mut selected_targets = Vec::new();

    if options.opencode {
        selected_targets.push(SetupTarget::OpenCode);
    }
    if options.claude {
        selected_targets.push(SetupTarget::Claude);
    }
    if options.pi {
        selected_targets.push(SetupTarget::Pi);
    }
    if options.codex {
        selected_targets.push(SetupTarget::Codex);
    }
    if options.all {
        selected_targets.push(SetupTarget::All);
    }

    if selected_targets.len() > 1 {
        bail!(
            "Options '--opencode', '--claude', '--pi', '--codex', and '--all' are mutually exclusive. Try: choose exactly one target flag (for example 'sce setup --opencode --non-interactive') or omit all target flags for interactive mode."
        );
    }

    if options.non_interactive && selected_targets.is_empty() && !options.hooks {
        bail!(
            "Option '--non-interactive' requires a target flag. Try: 'sce setup --opencode --non-interactive', 'sce setup --claude --non-interactive', 'sce setup --pi --non-interactive', 'sce setup --codex --non-interactive', or 'sce setup --all --non-interactive'."
        );
    }

    let config_mode = match selected_targets.as_slice() {
        [target] => Some(SetupMode::NonInteractive(*target)),
        [] if options.hooks => None,
        [] => Some(SetupMode::Interactive),
        _ => unreachable!("target count already validated"),
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

pub fn run_setup_for_mode(
    repository_root: &Path,
    mode: SetupMode,
    optional_workflows: Option<&[String]>,
    agent_trace_auto_sync: Option<bool>,
    attribution_hooks_enabled: Option<bool>,
) -> Result<String> {
    let target = match mode {
        SetupMode::Interactive => {
            bail!("Interactive setup mode must be resolved before installation")
        }
        SetupMode::NonInteractive(target) => target,
    };

    let selected_optional_workflows = match optional_workflows {
        Some(selection) => selection.to_vec(),
        None => persisted_optional_workflows(repository_root),
    };

    let outcome =
        install_embedded_setup_assets(repository_root, target, &selected_optional_workflows)
            .with_context(|| {
                format!(
                    "Setup installation failed for {}",
                    setup_target_label(target)
                )
            })?;

    persist_integration_targets(
        repository_root,
        target,
        &selected_optional_workflows,
        agent_trace_auto_sync,
        attribution_hooks_enabled,
    )
    .with_context(|| {
        format!(
            "Setup assets were installed for {} but failed to update repo-local config",
            setup_target_label(target)
        )
    })?;

    Ok(format_setup_install_success_message(&outcome))
}

pub fn persisted_optional_workflows(repository_root: &Path) -> Vec<String> {
    use crate::services::config::schema::parse_file_config;
    use crate::services::config::ConfigPathSource;

    let config_path = RepoPaths::new(repository_root).sce_config_file();

    let Ok(raw) = fs::read_to_string(&config_path) else {
        return Vec::new();
    };

    let Ok(config) =
        parse_file_config(&raw, &config_path, ConfigPathSource::DefaultDiscoveredLocal)
    else {
        return Vec::new();
    };

    config
        .integrations
        .map(|integrations| integrations.value.optional_workflows)
        .unwrap_or_default()
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
        .map(|result| setup_target_label(result.target))
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
            label(&format!("{}:", setup_target_label(result.target))),
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
    pub target: SetupTarget,
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
    target: SetupTarget,
    selected_optional_workflows: &[String],
) -> Result<SetupInstallOutcome> {
    install::install_embedded_setup_assets(repository_root, target, selected_optional_workflows)
}

pub(crate) fn repair_merge_target_asset(
    repository_root: &Path,
    target: SetupTarget,
    relative_path: &str,
) -> Result<()> {
    install::repair_merge_target_asset(repository_root, target, relative_path)
}

pub(crate) fn setup_install_recovery_guidance(
    target: SetupTarget,
    destination_root: &Path,
) -> String {
    format!(
        "Setup for {} does not create backups. Recover '{}' from version control if needed.",
        setup_target_label(target),
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

pub(crate) fn concrete_targets_for(target: SetupTarget) -> &'static [SetupTarget] {
    match target {
        SetupTarget::OpenCode => &[SetupTarget::OpenCode],
        SetupTarget::Claude => &[SetupTarget::Claude],
        SetupTarget::Pi => &[SetupTarget::Pi],
        SetupTarget::Codex => &[SetupTarget::Codex],
        SetupTarget::All => &[
            SetupTarget::OpenCode,
            SetupTarget::Claude,
            SetupTarget::Pi,
            SetupTarget::Codex,
        ],
    }
}

fn integration_target_id_str(target: SetupTarget) -> &'static str {
    match target {
        SetupTarget::OpenCode => "opencode",
        SetupTarget::Claude => "claude",
        SetupTarget::Pi => "pi",
        SetupTarget::Codex => "codex",
        SetupTarget::All => {
            unreachable!("integration_target_id_str must not be called with meta targets")
        }
    }
}

pub fn persist_integration_targets(
    repository_root: &Path,
    target: SetupTarget,
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

    let mut existing_targets: Vec<String> = config_obj
        .get("integrations")
        .and_then(|i| i.get("target"))
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let new_targets = concrete_targets_for(target);
    for concrete in new_targets {
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
    fn prompt_target(&self) -> Result<SetupDispatch>;
    fn prompt_optional_workflows(&self, defaults: &[String]) -> Result<Option<Vec<String>>>;
    fn prompt_agent_trace_auto_sync(&self) -> Result<Option<bool>>;
    fn prompt_attribution_hooks_enabled(&self) -> Result<Option<bool>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct InquireSetupTargetPrompter;

impl SetupTargetPrompter for InquireSetupTargetPrompter {
    fn prompt_target(&self) -> Result<SetupDispatch> {
        prompt::prompt_target()
    }

    fn prompt_optional_workflows(&self, defaults: &[String]) -> Result<Option<Vec<String>>> {
        prompt::prompt_optional_workflows(defaults)
    }

    fn prompt_agent_trace_auto_sync(&self) -> Result<Option<bool>> {
        prompt::prompt_agent_trace_auto_sync()
    }

    fn prompt_attribution_hooks_enabled(&self) -> Result<Option<bool>> {
        prompt::prompt_attribution_hooks_enabled()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupPromptTarget {
    OpenCode,
    Claude,
    Pi,
    Codex,
    All,
}

impl std::fmt::Display for SetupPromptTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", setup_prompt_target_label(*self))
    }
}

fn setup_prompt_target_label(target: SetupPromptTarget) -> String {
    prompt::setup_prompt_target_label(target)
}

#[allow(dead_code)]
fn setup_prompt_target_label_with_color_policy(
    target: SetupPromptTarget,
    color_enabled: bool,
) -> String {
    prompt::setup_prompt_target_label_with_color_policy(target, color_enabled)
}

#[allow(dead_code)]
fn setup_prompt_title_with_color_policy(color_enabled: bool) -> String {
    prompt::setup_prompt_title_with_color_policy(color_enabled)
}

mod prompt;

pub fn resolve_setup_dispatch<P>(
    mode: SetupMode,
    prompter: &P,
    optional_workflow_defaults: &[String],
) -> Result<SetupDispatch>
where
    P: SetupTargetPrompter,
{
    match mode {
        SetupMode::Interactive => {
            let target_dispatch = prompter.prompt_target()?;
            let SetupDispatch::Proceed { mode, .. } = target_dispatch else {
                return Ok(SetupDispatch::Cancelled);
            };

            let Some(optional_workflows) =
                prompter.prompt_optional_workflows(optional_workflow_defaults)?
            else {
                return Ok(SetupDispatch::Cancelled);
            };

            let Some(agent_trace_auto_sync) = prompter.prompt_agent_trace_auto_sync()? else {
                return Ok(SetupDispatch::Cancelled);
            };

            let Some(attribution_hooks_enabled) = prompter.prompt_attribution_hooks_enabled()?
            else {
                return Ok(SetupDispatch::Cancelled);
            };

            Ok(SetupDispatch::Proceed {
                mode,
                optional_workflows: Some(optional_workflows),
                agent_trace_auto_sync: Some(agent_trace_auto_sync),
                attribution_hooks_enabled: Some(attribution_hooks_enabled),
            })
        }
        SetupMode::NonInteractive(target) => Ok(SetupDispatch::Proceed {
            mode: SetupMode::NonInteractive(target),
            optional_workflows: None,
            agent_trace_auto_sync: None,
            attribution_hooks_enabled: None,
        }),
    }
}

pub fn setup_cancelled_text() -> String {
    value("Setup cancelled. No files were changed.")
}

#[cfg(test)]
mod tests;
