use anyhow::{bail, Context, Result};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::services::style::{label, success, value};
use crate::services::{default_paths, default_paths::RepoPaths};

pub mod command;

/// Canonical JSON payload for a newly bootstrapped repo-local `.sce/config.json`.
/// Contains only the `$schema` declaration pointing to the SCE config JSON Schema.
fn repo_local_config_bootstrap_payload() -> String {
    format!(
        "{{\n  \"$schema\": \"{}/config.json\"\n}}\n",
        crate::services::agent_trace::SCE_WEB_BASE_URL
    )
}

pub const NAME: &str = "setup";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupTarget {
    OpenCode,
    Claude,
    Pi,
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

/// A workflow that is generated for every target like any other, but whose
/// assets are installed only when a repository explicitly selects it. The
/// catalog below is generated from the Pkl workflow catalog at build time.
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
        SetupTarget::All => {
            unreachable!("meta targets are expanded into concrete targets")
        }
    }
}

/// The directory names a target uses for workflow commands and workflow skills.
/// Optional workflow asset membership is derived from these plus the catalog's
/// slugs rather than from an enumerated file list, so a new optional workflow
/// needs no Rust change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WorkflowAssetLayout {
    command_dir: &'static str,
    skills_dir: &'static str,
}

fn workflow_asset_layout(target: SetupTarget) -> WorkflowAssetLayout {
    match target {
        SetupTarget::OpenCode => WorkflowAssetLayout {
            command_dir: default_paths::opencode_asset::OPENCODE_COMMAND_DIR,
            skills_dir: default_paths::opencode_asset::SKILLS_DIR,
        },
        SetupTarget::Claude => WorkflowAssetLayout {
            command_dir: default_paths::claude_asset::COMMANDS_DIR,
            skills_dir: default_paths::claude_asset::SKILLS_DIR,
        },
        SetupTarget::Pi => WorkflowAssetLayout {
            command_dir: default_paths::pi_asset::PROMPTS_DIR,
            skills_dir: default_paths::pi_asset::SKILLS_DIR,
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
    let command_path = format!("{}/{}.md", layout.command_dir, workflow.command_slug);
    let skill_prefix = format!("{}/{}/", layout.skills_dir, workflow.skill_slug);

    relative_path == command_path || relative_path.starts_with(&skill_prefix)
}

/// Embedded assets for `target`, minus the command and skill assets of every
/// optional workflow the repository has not selected. Assets that belong to no
/// optional workflow are always yielded.
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
        /// The optional workflows this run installs. `None` means no selection
        /// was resolved here, so the persisted selection is reused downstream.
        optional_workflows: Option<Vec<String>>,
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
    pub all: bool,
    pub hooks: bool,
    pub repo_path: Option<PathBuf>,
    pub bootstrap_context: bool,
    /// Repeated `--workflow <slug>` values. Empty means the flag was absent.
    pub workflows: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub config_mode: Option<SetupMode>,
    pub install_hooks: bool,
    pub hooks_repo_path: Option<PathBuf>,
    pub context_only: bool,
    /// The optional workflows this run installs. `None` means no selection was
    /// supplied, so the persisted `integrations.optional_workflows` is reused.
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
    if options.all {
        selected_targets.push(SetupTarget::All);
    }

    if selected_targets.len() > 1 {
        bail!(
            "Options '--opencode', '--claude', '--pi', and '--all' are mutually exclusive. Try: choose exactly one target flag (for example 'sce setup --opencode --non-interactive') or omit all target flags for interactive mode."
        );
    }

    if options.non_interactive && selected_targets.is_empty() && !options.hooks {
        bail!(
            "Option '--non-interactive' requires a target flag. Try: 'sce setup --opencode --non-interactive', 'sce setup --claude --non-interactive', 'sce setup --pi --non-interactive', or 'sce setup --all --non-interactive'."
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

/// Validate repeated `--workflow` slugs against the build-generated catalog,
/// deduping while preserving order. The error lists every available slug so a
/// new optional workflow needs no Rust change here.
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
) -> Result<String> {
    let target = match mode {
        SetupMode::Interactive => {
            bail!("Interactive setup mode must be resolved before installation")
        }
        SetupMode::NonInteractive(target) => target,
    };

    // A supplied selection is the exact selection for this run; without one the
    // persisted selection is reused so a repeat run does not uninstall it.
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

    // Persist selected integration targets and optional workflows in repo-local config.
    persist_integration_targets(repository_root, target, &selected_optional_workflows)
        .with_context(|| {
            format!(
                "Setup assets were installed for {} but failed to update repo-local config",
                setup_target_label(target)
            )
        })?;

    Ok(format_setup_install_success_message(&outcome))
}

/// The optional workflows recorded in repo-local `.sce/config.json`, or an empty
/// selection when the file is absent, unreadable, or records none.
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

/// Preflight check that verifies the given directory is inside a git repository.
/// Returns the resolved repository root path on success.
/// Returns an actionable error telling the operator to run `git init` on failure.
pub fn ensure_git_repository(directory: &Path) -> Result<PathBuf> {
    install::ensure_git_repository(directory)
}

/// Bootstraps the repo-local `.sce/config.json` file if it does not already exist.
///
/// Creates the `.sce/` parent directory as needed, then writes the canonical
/// schema-only JSON payload. If the file already exists, it is left untouched.
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

/// Creates the baseline durable-context tree additively.
///
/// Missing directories and baseline files are created with neutral templates.
/// Existing files and directory contents are never overwritten.
pub fn bootstrap_context_baseline(repository_root: &Path) -> Result<String> {
    use crate::adapters::inbound::cli::setup::render_context_baseline_report;
    use crate::adapters::outbound::filesystem::context_store::FilesystemContextStore;
    use crate::application::use_cases::ensure_context_baseline::{
        EnsureContextBaseline, EnsureContextBaselineRequest,
    };

    let use_case = EnsureContextBaseline::new(FilesystemContextStore);
    let report = use_case
        .execute(EnsureContextBaselineRequest {
            repository_root: repository_root.to_path_buf(),
        })
        .with_context(|| {
            format!(
                "Failed to ensure context baseline for '{}'",
                repository_root.display()
            )
        })?;

    Ok(render_context_baseline_report(&report))
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
    use crate::adapters::outbound::assets::embedded_integration_assets::EmbeddedIntegrationAssetCatalog;
    use crate::adapters::outbound::filesystem::integration_installer::FilesystemIntegrationInstaller;
    use crate::application::use_cases::install_integration_assets::{
        InstallIntegrationAssets, InstallIntegrationAssetsError,
    };
    use crate::domain::integration::{IntegrationTarget, IntegrationTargetSelection};

    let selection = match target {
        SetupTarget::OpenCode => IntegrationTargetSelection::One(IntegrationTarget::OpenCode),
        SetupTarget::Claude => IntegrationTargetSelection::One(IntegrationTarget::Claude),
        SetupTarget::Pi => IntegrationTargetSelection::One(IntegrationTarget::Pi),
        SetupTarget::All => IntegrationTargetSelection::All,
    };

    let use_case = InstallIntegrationAssets::new(
        EmbeddedIntegrationAssetCatalog,
        FilesystemIntegrationInstaller,
    );

    let report = use_case
        .execute(repository_root, selection, selected_optional_workflows)
        .map_err(|error| match error {
            InstallIntegrationAssetsError::Catalog(never) => match never {},
            InstallIntegrationAssetsError::Installer(error) => error,
        })?;

    let target_results = report
        .targets
        .into_iter()
        .map(|installed| SetupInstallTargetResult {
            target: setup_target_for_integration_target(installed.target),
            destination_root: installed.destination_root,
            installed_file_count: installed.installed_file_count,
        })
        .collect();

    Ok(SetupInstallOutcome { target_results })
}

fn setup_target_for_integration_target(
    target: crate::domain::integration::IntegrationTarget,
) -> SetupTarget {
    use crate::domain::integration::IntegrationTarget;

    match target {
        IntegrationTarget::OpenCode => SetupTarget::OpenCode,
        IntegrationTarget::Claude => SetupTarget::Claude,
        IntegrationTarget::Pi => SetupTarget::Pi,
    }
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

    // Best-effort cleanup; log errors but don't fail the operation
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
        SetupTarget::All => &[SetupTarget::OpenCode, SetupTarget::Claude, SetupTarget::Pi],
    }
}

/// Convert a concrete [`SetupTarget`] (not `All`) to its canonical
/// `integrations.target` string representation.
fn integration_target_id_str(target: SetupTarget) -> &'static str {
    match target {
        SetupTarget::OpenCode => "opencode",
        SetupTarget::Claude => "claude",
        SetupTarget::Pi => "pi",
        SetupTarget::All => {
            unreachable!("integration_target_id_str must not be called with meta targets")
        }
    }
}

/// Persist a successfully installed setup target into the repo-local config file.
///
/// Reads the existing `.sce/config.json`, merges the new concrete target(s) into
/// `integrations.target` (deduped, preserving existing unrelated fields), records
/// the run's resolved optional-workflow selection in
/// `integrations.optional_workflows`, and writes the file back. Creates the file
/// with the bootstrap payload if it does not already exist.
pub fn persist_integration_targets(
    repository_root: &Path,
    target: SetupTarget,
    selected_optional_workflows: &[String],
) -> Result<()> {
    let repo_paths = RepoPaths::new(repository_root);
    let config_file = repo_paths.sce_config_file();

    // Read existing config or start with bootstrap payload.
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

    // Collect existing integration target values, if any.
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

    // Add new concrete targets (expanding All), deduping as we go.
    let new_targets = concrete_targets_for(target);
    for concrete in new_targets {
        let id_str = integration_target_id_str(*concrete);
        let id_owned = id_str.to_string();
        if !existing_targets.contains(&id_owned) {
            existing_targets.push(id_owned);
        }
    }

    // Write the merged integrations block back. The optional-workflow selection
    // resolved for this run replaces any previously recorded selection.
    config_obj.insert(
        "integrations".to_string(),
        json!({
            "target": existing_targets,
            "optional_workflows": selected_optional_workflows,
        }),
    );

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

mod install {
    use anyhow::{bail, Context, Result};
    use std::{
        fs, io,
        path::{Component, Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::services::security::{ensure_directory_is_writable, redact_sensitive_text};

    use super::{
        cleanup_path_if_exists, hook_install_recovery_guidance, iter_required_hook_assets,
        EmbeddedAsset, RequiredHookInstallResult, RequiredHookInstallStatus,
        RequiredHooksInstallOutcome,
    };

    pub(super) fn prepare_setup_hooks_repository(repository_root: &Path) -> Result<PathBuf> {
        let normalized_repository_root = normalize_user_repository_path(repository_root)?;
        resolve_git_repository_root(&normalized_repository_root)
    }

    pub(super) fn ensure_git_repository(directory: &Path) -> Result<PathBuf> {
        resolve_git_repository_root(directory)
    }

    pub(super) fn install_required_git_hooks(
        repository_root: &Path,
    ) -> Result<RequiredHooksInstallOutcome> {
        let resolved_repository_root = prepare_setup_hooks_repository(repository_root)?;
        install_required_git_hooks_in_resolved_repository(&resolved_repository_root, |from, to| {
            fs::rename(from, to)
        })
    }

    fn install_required_git_hooks_in_resolved_repository<F>(
        resolved_repository_root: &Path,
        mut rename_fn: F,
    ) -> Result<RequiredHooksInstallOutcome>
    where
        F: FnMut(&Path, &Path) -> io::Result<()>,
    {
        ensure_directory_is_writable(resolved_repository_root, "repository root")?;
        let hooks_directory = resolve_git_hooks_directory(resolved_repository_root)?;
        fs::create_dir_all(&hooks_directory).with_context(|| {
            format!(
                "Failed to create git hooks directory '{}'",
                hooks_directory.display()
            )
        })?;
        ensure_directory_is_writable(&hooks_directory, "git hooks directory")?;

        let mut hook_results = Vec::new();
        for hook_asset in iter_required_hook_assets() {
            let hook_result = install_single_required_hook_with_rename(
                &hooks_directory,
                hook_asset,
                &mut rename_fn,
            )?;
            hook_results.push(hook_result);
        }

        Ok(RequiredHooksInstallOutcome {
            repository_root: resolved_repository_root.to_path_buf(),
            hooks_directory,
            hook_results,
        })
    }

    fn install_single_required_hook_with_rename<F>(
        hooks_directory: &Path,
        hook_asset: &EmbeddedAsset,
        rename_fn: &mut F,
    ) -> Result<RequiredHookInstallResult>
    where
        F: FnMut(&Path, &Path) -> io::Result<()>,
    {
        validate_hook_relative_path(hook_asset.relative_path)?;

        let hook_path = hooks_directory.join(hook_asset.relative_path);
        let existing_metadata = fs::metadata(&hook_path).ok();

        if existing_metadata
            .as_ref()
            .is_some_and(std::fs::Metadata::is_file)
        {
            let existing_bytes = fs::read(&hook_path).with_context(|| {
                format!("Failed to read existing hook '{}'", hook_path.display())
            })?;
            let executable = is_executable_file(&hook_path)?;

            if existing_bytes == hook_asset.bytes && executable {
                return Ok(RequiredHookInstallResult {
                    hook_name: hook_asset.relative_path.to_string(),
                    hook_path,
                    status: RequiredHookInstallStatus::Skipped,
                });
            }
        } else if existing_metadata.is_some() {
            bail!(
                "Existing hook target '{}' is not a file",
                hook_path.display()
            );
        }

        let hook_staging_path =
            create_hook_staging_path(hooks_directory, hook_asset.relative_path)?;
        if let Err(error) = write_hook_payload_to_staging(&hook_staging_path, hook_asset.bytes) {
            cleanup_path_if_exists(&hook_staging_path);
            return Err(error);
        }

        if existing_metadata.is_none() {
            if let Err(error) = rename_fn(&hook_staging_path, &hook_path).with_context(|| {
                format!(
                    "Failed to install required hook '{}' at '{}'",
                    hook_asset.relative_path,
                    hook_path.display()
                )
            }) {
                cleanup_path_if_exists(&hook_staging_path);
                return Err(error);
            }

            return Ok(RequiredHookInstallResult {
                hook_name: hook_asset.relative_path.to_string(),
                hook_path,
                status: RequiredHookInstallStatus::Installed,
            });
        }

        remove_existing_hook_target(&hook_path).with_context(|| {
            format!(
                "Failed to replace existing hook '{}' without creating a backup",
                hook_path.display()
            )
        })?;

        if let Err(error) = rename_fn(&hook_staging_path, &hook_path).with_context(|| {
            format!(
                "Failed to update required hook '{}' at '{}'",
                hook_asset.relative_path,
                hook_path.display()
            )
        }) {
            cleanup_path_if_exists(&hook_staging_path);
            return Err(error.context(hook_install_recovery_guidance(&hook_path)));
        }

        Ok(RequiredHookInstallResult {
            hook_name: hook_asset.relative_path.to_string(),
            hook_path,
            status: RequiredHookInstallStatus::Updated,
        })
    }

    fn write_hook_payload_to_staging(staging_path: &Path, bytes: &[u8]) -> Result<()> {
        fs::write(staging_path, bytes).with_context(|| {
            format!(
                "Failed to write staged hook payload '{}'",
                staging_path.display()
            )
        })?;
        ensure_executable_permissions(staging_path)?;
        Ok(())
    }

    fn create_hook_staging_path(hooks_directory: &Path, hook_name: &str) -> Result<PathBuf> {
        let epoch_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System clock is before UNIX_EPOCH")?
            .as_nanos();
        let sanitized_hook_name = hook_name.replace('/', "-");

        for attempt in 0..1000_u16 {
            let candidate = hooks_directory.join(format!(
                ".sce-hook-staging-{sanitized_hook_name}-{epoch_nanos}-{}-{attempt}",
                std::process::id()
            ));

            match fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&candidate)
            {
                Ok(_) => return Ok(candidate),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "Failed to allocate hook staging file '{}'",
                            candidate.display()
                        )
                    });
                }
            }
        }

        bail!(
            "Could not allocate a unique hook staging file under '{}'",
            hooks_directory.display()
        )
    }

    fn normalize_user_repository_path(repository_root: &Path) -> Result<PathBuf> {
        if repository_root.as_os_str().is_empty() {
            bail!(
                "Option '--repo' must not be empty. Try: pass a path to an existing git repository."
            );
        }

        let canonical_repository_root = fs::canonicalize(repository_root).with_context(|| {
            format!(
                "Failed to resolve repository path '{}'. Try: pass a path to an existing git repository.",
                repository_root.display()
            )
        })?;

        let metadata = fs::metadata(&canonical_repository_root).with_context(|| {
            format!(
                "Failed to inspect repository path '{}'.",
                canonical_repository_root.display()
            )
        })?;

        if !metadata.is_dir() {
            bail!(
                "Repository path '{}' is not a directory. Try: pass a path to an existing git repository.",
                canonical_repository_root.display()
            );
        }

        Ok(canonical_repository_root)
    }

    fn resolve_git_repository_root(repository_root: &Path) -> Result<PathBuf> {
        let repository_root_output = run_git_command_in_directory(
            repository_root,
            &["rev-parse", "--show-toplevel"],
            "Failed to resolve repository root. Ensure '--repo' points to an accessible git repository.",
        )
        .map_err(|error| map_setup_non_git_repository_error(repository_root, error))?;
        Ok(PathBuf::from(repository_root_output))
    }

    fn map_setup_non_git_repository_error(
        repository_root: &Path,
        error: anyhow::Error,
    ) -> anyhow::Error {
        let message = error.to_string();
        if message.contains("not a git repository") {
            anyhow::anyhow!(
                "Directory '{}' is not a git repository. Try: run 'git init' in '{}', then rerun 'sce setup'.",
                repository_root.display(),
                repository_root.display()
            )
        } else {
            error
        }
    }

    fn resolve_git_hooks_directory(repository_root: &Path) -> Result<PathBuf> {
        let hooks_directory_output = run_git_command_in_directory(
            repository_root,
            &["rev-parse", "--git-path", "hooks"],
            "Failed to resolve effective git hooks path.",
        )?;

        let hooks_directory = PathBuf::from(&hooks_directory_output);
        if hooks_directory.is_absolute() {
            return Ok(hooks_directory);
        }

        Ok(repository_root.join(hooks_directory))
    }

    fn run_git_command_in_directory(
        repository_root: &Path,
        args: &[&str],
        context_message: &str,
    ) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(repository_root)
            .output()
            .with_context(|| {
                format!(
                    "{} (directory: '{}')",
                    context_message,
                    repository_root.display()
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let diagnostic = if stderr.is_empty() {
                String::from("git command exited with a non-zero status")
            } else {
                redact_sensitive_text(&stderr)
            };
            bail!("{context_message} {diagnostic}");
        }

        let stdout = String::from_utf8(output.stdout)
            .context("git command output contained invalid UTF-8")?
            .trim()
            .to_string();
        if stdout.is_empty() {
            bail!("{context_message} git command returned empty output");
        }

        Ok(stdout)
    }

    #[cfg(unix)]
    fn ensure_executable_permissions(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(path)
            .with_context(|| format!("Failed to read metadata for '{}'", path.display()))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        fs::set_permissions(path, permissions).with_context(|| {
            format!(
                "Failed to set executable permissions for '{}'",
                path.display()
            )
        })?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn ensure_executable_permissions(_path: &Path) -> Result<()> {
        Ok(())
    }

    #[cfg(unix)]
    fn is_executable_file(path: &Path) -> Result<bool> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(path)
            .with_context(|| format!("Failed to read metadata for '{}'", path.display()))?;
        Ok(metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }

    #[cfg(not(unix))]
    fn is_executable_file(path: &Path) -> Result<bool> {
        let metadata = fs::metadata(path)
            .with_context(|| format!("Failed to read metadata for '{}'", path.display()))?;
        Ok(metadata.is_file())
    }

    fn remove_existing_hook_target(destination_root: &Path) -> Result<()> {
        let metadata = fs::metadata(destination_root).with_context(|| {
            format!(
                "Failed to inspect existing setup target '{}'",
                destination_root.display()
            )
        })?;

        if metadata.is_dir() {
            fs::remove_dir_all(destination_root).with_context(|| {
                format!(
                    "Failed to remove existing setup target directory '{}'",
                    destination_root.display()
                )
            })?;
        } else {
            fs::remove_file(destination_root).with_context(|| {
                format!(
                    "Failed to remove existing setup target file '{}'",
                    destination_root.display()
                )
            })?;
        }

        Ok(())
    }

    fn validate_hook_relative_path(relative_path: &str) -> Result<()> {
        let path = Path::new(relative_path);

        if path.is_absolute() {
            bail!("Embedded asset path '{relative_path}' must be relative, not absolute");
        }

        for component in path.components() {
            match component {
                Component::Normal(_) => {}
                _ => {
                    bail!("Embedded asset path '{relative_path}' contains disallowed component");
                }
            }
        }

        Ok(())
    }
}

pub trait SetupTargetPrompter {
    fn prompt_target(&self) -> Result<SetupDispatch>;

    /// The optional workflows to install, pre-checked from `defaults`.
    /// `None` means the operator cancelled the prompt.
    fn prompt_optional_workflows(&self, defaults: &[String]) -> Result<Option<Vec<String>>>;
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupPromptTarget {
    OpenCode,
    Claude,
    Pi,
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

mod prompt {
    use anyhow::{bail, Result};
    use inquire::{InquireError, MultiSelect, Select};

    use crate::services::style::{
        prompt_label, prompt_label_with_color_policy, prompt_value_with_color_policy,
    };

    use super::{OptionalWorkflow, SetupDispatch, SetupMode, SetupPromptTarget, SetupTarget};

    fn proceed(target: SetupTarget) -> SetupDispatch {
        SetupDispatch::Proceed {
            mode: SetupMode::NonInteractive(target),
            optional_workflows: None,
        }
    }

    pub(super) fn prompt_target() -> Result<SetupDispatch> {
        let options = vec![
            SetupPromptTarget::OpenCode,
            SetupPromptTarget::Claude,
            SetupPromptTarget::Pi,
            SetupPromptTarget::All,
        ];

        let selection = Select::new(&setup_prompt_title(), options).prompt();

        match selection {
            Ok(SetupPromptTarget::OpenCode) => Ok(proceed(SetupTarget::OpenCode)),
            Ok(SetupPromptTarget::Claude) => Ok(proceed(SetupTarget::Claude)),
            Ok(SetupPromptTarget::Pi) => Ok(proceed(SetupTarget::Pi)),
            Ok(SetupPromptTarget::All) => Ok(proceed(SetupTarget::All)),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                Ok(SetupDispatch::Cancelled)
            }
            Err(InquireError::NotTTY) => bail!(
                "Interactive setup requires a TTY. Re-run with '--non-interactive' and one of '--opencode', '--claude', '--pi', or '--all'."
            ),
            Err(error) => Err(error.into()),
        }
    }

    /// The optional workflows to install, pre-checked from `defaults`. `None`
    /// means the operator cancelled. An empty catalog skips the prompt entirely
    /// and resolves to an empty selection.
    pub(super) fn prompt_optional_workflows(defaults: &[String]) -> Result<Option<Vec<String>>> {
        let Some((rows, default_indices)) =
            optional_workflow_prompt_inputs(super::OPTIONAL_WORKFLOWS, defaults)
        else {
            return Ok(Some(Vec::new()));
        };

        let selection = MultiSelect::new(&optional_workflow_prompt_title(), rows)
            .with_default(&default_indices)
            .prompt();

        match selection {
            Ok(selected) => Ok(Some(
                selected
                    .into_iter()
                    .map(|row| row.workflow.id.to_string())
                    .collect(),
            )),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
            Err(InquireError::NotTTY) => bail!(
                "Interactive setup requires a TTY. Re-run with '--non-interactive' and one of '--opencode', '--claude', '--pi', or '--all', adding '--workflow <slug>' for each optional workflow to install."
            ),
            Err(error) => Err(error.into()),
        }
    }

    /// One selectable row per optional workflow, in catalog order.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) struct OptionalWorkflowRow {
        pub(super) workflow: &'static OptionalWorkflow,
    }

    impl std::fmt::Display for OptionalWorkflowRow {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", optional_workflow_row_label(self.workflow))
        }
    }

    /// The prompt's rows and pre-checked indices, or `None` when the catalog
    /// carries no optional workflow and the prompt is skipped entirely.
    pub(super) fn optional_workflow_prompt_inputs(
        catalog: &'static [OptionalWorkflow],
        defaults: &[String],
    ) -> Option<(Vec<OptionalWorkflowRow>, Vec<usize>)> {
        if catalog.is_empty() {
            return None;
        }

        Some((
            optional_workflow_rows(catalog),
            optional_workflow_default_indices(catalog, defaults),
        ))
    }

    pub(super) fn optional_workflow_rows(
        catalog: &'static [OptionalWorkflow],
    ) -> Vec<OptionalWorkflowRow> {
        catalog
            .iter()
            .map(|workflow| OptionalWorkflowRow { workflow })
            .collect()
    }

    /// Row indices to pre-check, in catalog order. Ids that are not in the
    /// catalog are ignored so a stale persisted selection cannot panic the
    /// prompt.
    pub(super) fn optional_workflow_default_indices(
        catalog: &'static [OptionalWorkflow],
        defaults: &[String],
    ) -> Vec<usize> {
        catalog
            .iter()
            .enumerate()
            .filter(|(_, workflow)| defaults.iter().any(|id| id == workflow.id))
            .map(|(index, _)| index)
            .collect()
    }

    pub(super) fn optional_workflow_prompt_title() -> String {
        prompt_label("Select optional workflows")
    }

    pub(super) fn optional_workflow_row_label(workflow: &OptionalWorkflow) -> String {
        optional_workflow_row_label_with_color_policy(
            workflow,
            crate::services::style::supports_color(),
        )
    }

    pub(super) fn optional_workflow_row_label_with_color_policy(
        workflow: &OptionalWorkflow,
        color_enabled: bool,
    ) -> String {
        format!(
            "{} — {}",
            prompt_value_with_color_policy(workflow.title, color_enabled),
            workflow.description
        )
    }

    pub(super) fn setup_prompt_title() -> String {
        prompt_label("Select setup target")
    }

    pub(super) fn setup_prompt_target_label(target: SetupPromptTarget) -> String {
        setup_prompt_target_label_with_color_policy(
            target,
            crate::services::style::supports_color(),
        )
    }

    pub(super) fn setup_prompt_target_label_with_color_policy(
        target: SetupPromptTarget,
        color_enabled: bool,
    ) -> String {
        let label = match target {
            SetupPromptTarget::OpenCode => "OpenCode",
            SetupPromptTarget::Claude => "Claude",
            SetupPromptTarget::Pi => "Pi",
            SetupPromptTarget::All => "All (OpenCode + Claude + Pi)",
        };

        prompt_value_with_color_policy(label, color_enabled)
    }

    #[allow(dead_code)]
    pub(super) fn setup_prompt_title_with_color_policy(color_enabled: bool) -> String {
        prompt_label_with_color_policy("Select setup target", color_enabled)
    }
}

/// Resolve the interactive setup prompts into an installable dispatch.
///
/// `optional_workflow_defaults` pre-checks the optional-workflow prompt's rows;
/// callers pass the repository's persisted selection, or the `--workflow`
/// selection when one was supplied. A non-interactive mode prompts for nothing
/// and carries no selection, leaving that resolution to `run_setup_for_mode`.
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

            Ok(SetupDispatch::Proceed {
                mode,
                optional_workflows: Some(optional_workflows),
            })
        }
        SetupMode::NonInteractive(target) => Ok(SetupDispatch::Proceed {
            mode: SetupMode::NonInteractive(target),
            optional_workflows: None,
        }),
    }
}

pub fn setup_cancelled_text() -> String {
    value("Setup cancelled. No files were changed.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::command_surface;
    use crate::services::command_registry::CommandRegistry;
    use crate::services::command_registry::RuntimeCommand;
    use crate::services::default_paths::InstallTargetPaths;
    use crate::services::parse::command_runtime::parse_runtime_command;

    fn options_with(mutate: impl FnOnce(&mut SetupCliOptions)) -> SetupCliOptions {
        let mut options = SetupCliOptions::default();
        mutate(&mut options);
        options
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-setup-context-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn init_git_repo(label: &str) -> PathBuf {
        let repo = unique_temp_dir(label);
        let output = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .output()
            .expect("git init should spawn");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        repo
    }

    fn assert_baseline_paths_exist(repo: &Path) {
        for relative_path in [
            "context/overview.md",
            "context/architecture.md",
            "context/patterns.md",
            "context/glossary.md",
            "context/context-map.md",
            "context/plans",
            "context/handovers",
            "context/decisions",
            "context/tmp",
            "context/tmp/.gitignore",
        ] {
            let path = repo.join(relative_path);
            assert!(path.exists(), "expected baseline path {}", path.display());
        }
    }

    #[test]
    fn resolve_setup_request_accepts_pi_target() {
        let request = resolve_setup_request(options_with(|options| {
            options.pi = true;
            options.non_interactive = true;
        }))
        .expect("pi target should resolve");

        assert_eq!(
            request.config_mode,
            Some(SetupMode::NonInteractive(SetupTarget::Pi))
        );
        assert!(!request.context_only);
    }

    #[test]
    fn resolve_setup_request_accepts_all_target() {
        let request = resolve_setup_request(options_with(|options| {
            options.all = true;
            options.non_interactive = true;
        }))
        .expect("all target should resolve");

        assert_eq!(
            request.config_mode,
            Some(SetupMode::NonInteractive(SetupTarget::All))
        );
        assert!(!request.context_only);
    }

    #[test]
    fn resolve_setup_request_accepts_bootstrap_context_alone() {
        let request = resolve_setup_request(options_with(|options| {
            options.bootstrap_context = true;
        }))
        .expect("bootstrap-context alone should resolve");

        assert!(request.context_only);
        assert_eq!(request.config_mode, None);
        assert!(!request.install_hooks);
        assert_eq!(request.hooks_repo_path, None);
    }

    #[test]
    fn resolve_setup_request_rejects_bootstrap_context_with_target() {
        let error = resolve_setup_request(options_with(|options| {
            options.bootstrap_context = true;
            options.opencode = true;
        }))
        .expect_err("bootstrap-context with target must be rejected");

        assert!(error.to_string().contains("--bootstrap-context"));
        assert!(error.to_string().contains("alone"));
    }

    #[test]
    fn resolve_setup_request_rejects_combined_target_flags() {
        let error = resolve_setup_request(options_with(|options| {
            options.pi = true;
            options.all = true;
        }))
        .expect_err("combined target flags must be rejected");

        assert!(error.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn resolve_setup_request_non_interactive_error_lists_pi_and_all() {
        let error = resolve_setup_request(options_with(|options| {
            options.non_interactive = true;
        }))
        .expect_err("non-interactive without target must be rejected");

        let message = error.to_string();
        assert!(message.contains("--pi"));
        assert!(message.contains("--all"));
    }

    #[test]
    fn parser_routes_bootstrap_context_to_context_only_request() {
        let registry = CommandRegistry::default();
        let command = parse_runtime_command(
            [
                "sce".to_string(),
                "setup".to_string(),
                "--bootstrap-context".to_string(),
            ],
            &registry,
            None,
        )
        .expect("bootstrap-context should parse");

        match command {
            RuntimeCommand::Setup(setup_command) => {
                assert!(setup_command.request.context_only);
                assert_eq!(setup_command.request.config_mode, None);
                assert!(!setup_command.request.install_hooks);
            }
            _ => panic!("expected Setup command for --bootstrap-context"),
        }
    }

    #[test]
    fn help_documents_bootstrap_context_flag() {
        let top_level_help = command_surface::help_text();
        assert!(
            top_level_help.contains("--bootstrap-context"),
            "top-level help should document --bootstrap-context"
        );

        let registry = CommandRegistry::default();
        let command = parse_runtime_command(
            ["sce".to_string(), "setup".to_string(), "--help".to_string()],
            &registry,
            None,
        )
        .expect("setup --help should parse");

        match command {
            RuntimeCommand::HelpText(help) => {
                assert!(
                    help.text.contains("--bootstrap-context"),
                    "setup --help should document --bootstrap-context:\n{}",
                    help.text
                );
            }
            _ => panic!("expected HelpText for setup --help"),
        }
    }

    #[test]
    fn bootstrap_context_baseline_creates_expected_paths() {
        let repo = init_git_repo("create-baseline");
        let message = bootstrap_context_baseline(&repo).expect("bootstrap should create baseline");
        assert!(message.contains("Context baseline ensured."));
        assert_baseline_paths_exist(&repo);

        let paths = RepoPaths::new(&repo);
        assert!(!paths.opencode_dir().exists());
        assert!(!paths.claude_dir().exists());
        assert!(!paths.pi_dir().exists());

        let gitignore = fs::read_to_string(repo.join("context/tmp/.gitignore"))
            .expect("tmp gitignore should be readable");
        assert_eq!(gitignore, "*\n!.gitignore\n");

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn bootstrap_context_baseline_is_additive_and_idempotent() {
        let repo = init_git_repo("idempotent-baseline");
        bootstrap_context_baseline(&repo).expect("initial bootstrap");

        let overview_file = repo.join("context/overview.md");
        let context_map_file = repo.join("context/context-map.md");
        let tmp_gitignore_file = repo.join("context/tmp/.gitignore");
        let architecture_file = repo.join("context/architecture.md");
        let plans_dir = repo.join("context/plans");

        let sentinel = "SENTINEL_OVERVIEW_CONTENT\n";
        fs::write(&overview_file, sentinel).expect("seed overview sentinel");
        fs::write(&context_map_file, "SENTINEL_CONTEXT_MAP\n").expect("seed context-map sentinel");
        fs::write(&tmp_gitignore_file, "SENTINEL_GITIGNORE\n").expect("seed gitignore sentinel");

        fs::remove_file(&architecture_file).expect("remove architecture");
        fs::remove_dir_all(&plans_dir).expect("remove plans");

        bootstrap_context_baseline(&repo).expect("rerun bootstrap");

        assert_eq!(
            fs::read_to_string(&overview_file).expect("read overview"),
            sentinel
        );
        assert_eq!(
            fs::read_to_string(&context_map_file).expect("read context-map"),
            "SENTINEL_CONTEXT_MAP\n"
        );
        assert_eq!(
            fs::read_to_string(&tmp_gitignore_file).expect("read gitignore"),
            "SENTINEL_GITIGNORE\n"
        );
        assert!(architecture_file.exists());
        assert!(plans_dir.is_dir());

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn concrete_targets_for_all_expands_to_three_targets() {
        assert_eq!(
            concrete_targets_for(SetupTarget::All),
            &[SetupTarget::OpenCode, SetupTarget::Claude, SetupTarget::Pi]
        );
    }

    #[test]
    fn integration_target_id_str_maps_pi() {
        assert_eq!(integration_target_id_str(SetupTarget::Pi), "pi");
    }

    /// Every optional workflow selected, so filtering drops nothing.
    fn every_optional_workflow() -> Vec<String> {
        super::OPTIONAL_WORKFLOWS
            .iter()
            .map(|workflow| workflow.id.to_string())
            .collect()
    }

    #[test]
    fn iter_embedded_assets_for_all_covers_each_concrete_target() {
        let selection = every_optional_workflow();
        let count = |target| {
            iter_embedded_assets_for_setup_target_with_selection(target, &selection).count()
        };

        let concrete_sum =
            count(SetupTarget::OpenCode) + count(SetupTarget::Claude) + count(SetupTarget::Pi);

        assert!(count(SetupTarget::Pi) > 0);
        assert_eq!(count(SetupTarget::All), concrete_sum);
    }

    #[test]
    fn embedded_build_payload_contains_generated_targets_and_static_hooks() {
        let selection = every_optional_workflow();
        let contains = |target, path| {
            iter_embedded_assets_for_setup_target_with_selection(target, &selection)
                .any(|asset| asset.relative_path == path && !asset.bytes.is_empty())
        };

        assert!(contains(SetupTarget::OpenCode, "command/next-task.md"));
        assert!(contains(
            SetupTarget::OpenCode,
            "lib/bash-policy-presets.json"
        ));
        assert!(contains(SetupTarget::Claude, "commands/next-task.md"));
        assert!(contains(SetupTarget::Pi, "prompts/next-task.md"));
        assert!(contains(SetupTarget::Pi, "extensions/sce/index.ts"));
        assert!(iter_required_hook_assets().all(|asset| !asset.bytes.is_empty()));
    }

    #[test]
    fn facade_installs_every_target_with_expected_files() {
        let cases = [
            (
                SetupTarget::OpenCode,
                vec![(SetupTarget::OpenCode, "command/next-task.md")],
            ),
            (
                SetupTarget::Claude,
                vec![(SetupTarget::Claude, "commands/next-task.md")],
            ),
            (
                SetupTarget::Pi,
                vec![(SetupTarget::Pi, "prompts/next-task.md")],
            ),
            (
                SetupTarget::All,
                vec![
                    (SetupTarget::OpenCode, "command/next-task.md"),
                    (SetupTarget::Claude, "commands/next-task.md"),
                    (SetupTarget::Pi, "prompts/next-task.md"),
                ],
            ),
        ];
        let selection = every_optional_workflow();

        for (requested_target, expected_targets) in cases {
            let repo = unique_temp_dir("facade-all-targets");
            let outcome = install_embedded_setup_assets(&repo, requested_target, &selection)
                .expect("facade installation should succeed");

            assert_eq!(outcome.target_results.len(), expected_targets.len());
            for ((expected_target, representative_path), result) in
                expected_targets.iter().zip(&outcome.target_results)
            {
                assert_eq!(result.target, *expected_target);
                assert_eq!(
                    result.installed_file_count,
                    iter_embedded_assets_for_setup_target_with_selection(
                        *expected_target,
                        &selection,
                    )
                    .count()
                );
                assert!(result.destination_root.is_dir());
                assert!(
                    result.destination_root.join(representative_path).is_file(),
                    "expected representative asset for {:?} at {}",
                    expected_target,
                    result.destination_root.join(representative_path).display()
                );

                let expected_root = match expected_target {
                    SetupTarget::OpenCode => InstallTargetPaths::new(&repo).opencode_target_dir(),
                    SetupTarget::Claude => InstallTargetPaths::new(&repo).claude_target_dir(),
                    SetupTarget::Pi => InstallTargetPaths::new(&repo).pi_target_dir(),
                    SetupTarget::All => unreachable!("facade results are concrete targets"),
                };
                assert_eq!(result.destination_root, expected_root);
            }

            let _ = fs::remove_dir_all(&repo);
        }
    }

    #[test]
    fn facade_preserves_optional_workflow_selection() {
        if OPTIONAL_WORKFLOWS.is_empty() {
            // There is no optional workflow to filter in this generated catalog.
            return;
        }

        let workflow = &OPTIONAL_WORKFLOWS[0];
        let target = SetupTarget::Claude;
        let layout = workflow_asset_layout(target);
        let command_path = format!("{}/{}.md", layout.command_dir, workflow.command_slug);
        let skill_path = format!("{}/{}/SKILL.md", layout.skills_dir, workflow.skill_slug);
        let selected = vec![workflow.id.to_string()];
        let unchanged_path =
            iter_embedded_assets_for_setup_target_with_selection(target, &[] as &[String])
                .next()
                .expect("an unconditionally installed asset should exist")
                .relative_path;
        let repo = unique_temp_dir("facade-optional-workflow");
        let destination = InstallTargetPaths::new(&repo).claude_target_dir();

        install_embedded_setup_assets(&repo, target, &[])
            .expect("installation without optional workflow should succeed");
        assert!(!destination.join(&command_path).exists());
        assert!(!destination.join(&skill_path).exists());
        assert!(destination.join(unchanged_path).is_file());

        install_embedded_setup_assets(&repo, target, &selected)
            .expect("installation with optional workflow should succeed");
        assert!(destination.join(&command_path).is_file());
        assert!(destination.join(&skill_path).is_file());
        assert!(destination.join(unchanged_path).is_file());

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn facade_replaces_target_without_backup() {
        let repo = unique_temp_dir("facade-replace-target");
        let target = SetupTarget::Claude;
        let selection = every_optional_workflow();
        let destination = InstallTargetPaths::new(&repo).claude_target_dir();

        install_embedded_setup_assets(&repo, target, &selection)
            .expect("initial facade installation should succeed");
        let sentinel = destination.join("sentinel.txt");
        fs::write(&sentinel, b"must be removed").expect("write sentinel");

        install_embedded_setup_assets(&repo, target, &selection)
            .expect("replacement facade installation should succeed");

        assert!(!sentinel.exists());
        assert!(destination.join("commands/next-task.md").is_file());
        let backup_directory_exists = fs::read_dir(&repo)
            .expect("read repository root")
            .filter_map(std::result::Result::ok)
            .any(|entry| {
                entry.file_type().is_ok_and(|file_type| file_type.is_dir())
                    && entry.file_name().to_string_lossy().contains("backup")
            });
        assert!(
            !backup_directory_exists,
            "replacement must not create a backup"
        );

        let _ = fs::remove_dir_all(&repo);
    }
}
