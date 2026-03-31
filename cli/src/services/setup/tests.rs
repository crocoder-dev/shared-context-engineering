use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;

use super::{
    get_required_hook_asset, iter_embedded_assets_for_setup_target, iter_required_hook_assets,
    resolve_setup_dispatch, resolve_setup_request, run_setup_for_mode, run_setup_hooks,
    RequiredHookAsset, RequiredHookInstallResult, RequiredHookInstallStatus,
    RequiredHooksInstallOutcome, SetupBackupPolicy, SetupCliOptions, SetupDispatch,
    SetupInstallOutcome, SetupInstallTargetResult, SetupMode, SetupTarget,
};

#[derive(Clone, Copy, Debug)]
struct MockPrompter {
    response: SetupDispatch,
}

impl super::SetupTargetPrompter for MockPrompter {
    fn prompt_target(&self) -> Result<SetupDispatch> {
        Ok(self.response)
    }
}

#[test]
fn run_setup_rejects_unresolved_interactive_mode() {
    let error = run_setup_for_mode(std::path::Path::new("/nonexistent"), SetupMode::Interactive)
        .expect_err("interactive mode should be resolved before install");
    assert_eq!(
        error.to_string(),
        "Interactive setup mode must be resolved before installation"
    );
}

#[test]
fn setup_options_reject_mutually_exclusive_flags() {
    let error = resolve_setup_request(SetupCliOptions {
        help: false,
        non_interactive: false,
        opencode: true,
        claude: true,
        both: false,
        hooks: false,
        repo_path: None,
    })
    .expect_err("multiple target flags should fail");

    assert_eq!(
        error.to_string(),
        "Options '--opencode', '--claude', and '--both' are mutually exclusive. Try: choose exactly one target flag (for example 'sce setup --opencode --non-interactive') or omit all target flags for interactive mode."
    );
}

#[test]
fn setup_options_reject_non_interactive_without_target() {
    let error = resolve_setup_request(SetupCliOptions {
        help: false,
        non_interactive: true,
        opencode: false,
        claude: false,
        both: false,
        hooks: false,
        repo_path: None,
    })
    .expect_err("--non-interactive without a target should fail validation");
    assert_eq!(
        error.to_string(),
        "Option '--non-interactive' requires a target flag. Try: 'sce setup --opencode --non-interactive', 'sce setup --claude --non-interactive', or 'sce setup --both --non-interactive'."
    );
}

#[test]
fn setup_options_reject_repo_without_hooks() {
    let error = resolve_setup_request(SetupCliOptions {
        help: false,
        non_interactive: false,
        opencode: false,
        claude: false,
        both: false,
        hooks: false,
        repo_path: Some(PathBuf::from("tmp/repo")),
    })
    .expect_err("--repo without --hooks should fail");
    assert_eq!(
        error.to_string(),
        "Option '--repo' requires '--hooks'. Try: run 'sce setup --hooks --repo <path>' or remove '--repo'."
    );
}

#[test]
fn run_setup_hooks_rejects_missing_repo_path() {
    let missing_path = PathBuf::from("/definitely/missing/sce-test-repo");
    let error = run_setup_hooks(&missing_path).expect_err("missing repo path should fail");
    let message = format!("{error:#}");
    assert!(message.contains("Failed to resolve repository path"));
}

#[test]
fn interactive_dispatch_maps_selected_target() {
    let dispatch = resolve_setup_dispatch(
        SetupMode::Interactive,
        &MockPrompter {
            response: SetupDispatch::Proceed(SetupMode::NonInteractive(SetupTarget::Claude)),
        },
    )
    .expect("interactive dispatch should resolve selected target");

    assert_eq!(
        dispatch,
        SetupDispatch::Proceed(SetupMode::NonInteractive(SetupTarget::Claude))
    );
}

#[test]
fn interactive_dispatch_returns_cancelled_without_side_effects() {
    let dispatch = resolve_setup_dispatch(
        SetupMode::Interactive,
        &MockPrompter {
            response: SetupDispatch::Cancelled,
        },
    )
    .expect("interactive dispatch should return cancelled response");

    assert_eq!(dispatch, SetupDispatch::Cancelled);
}

#[test]
fn embedded_manifest_paths_are_sorted_and_normalized() {
    for target in [SetupTarget::OpenCode, SetupTarget::Claude] {
        let assets = assets_for_target(target);

        assert!(!assets.is_empty(), "embedded asset set should not be empty");

        let paths: Vec<&str> = assets.iter().map(|asset| asset.relative_path).collect();
        assert_eq!(paths.len(), assets.len());

        for asset in assets {
            assert!(!asset.relative_path.is_empty());
            assert!(!asset.relative_path.starts_with('/'));
            assert!(!asset.relative_path.contains('\\'));
            assert!(!asset.relative_path.starts_with("config/"));
            assert!(
                !asset.bytes.is_empty(),
                "embedded files should have content bytes"
            );
        }

        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(
            paths, sorted,
            "embedded paths should be deterministic and sorted"
        );
    }
}

#[test]
fn embedded_manifest_matches_runtime_config_tree() {
    let opencode_expected =
        collect_runtime_relative_paths(&runtime_target_root(SetupTarget::OpenCode))
            .expect("should collect OpenCode runtime asset paths");
    let claude_expected = collect_runtime_relative_paths(&runtime_target_root(SetupTarget::Claude))
        .expect("should collect Claude runtime asset paths");

    let opencode_actual: Vec<String> = assets_for_target(SetupTarget::OpenCode)
        .iter()
        .map(|asset| asset.relative_path.to_string())
        .collect();
    let claude_actual: Vec<String> = assets_for_target(SetupTarget::Claude)
        .iter()
        .map(|asset| asset.relative_path.to_string())
        .collect();

    assert_eq!(opencode_actual, opencode_expected);
    assert_eq!(claude_actual, claude_expected);
}

#[test]
fn embedded_setup_target_iterator_scopes_assets_per_target() {
    let opencode_count = expected_installable_paths(SetupTarget::OpenCode).len();
    let claude_count = expected_installable_paths(SetupTarget::Claude).len();

    let iter_opencode_count = iter_embedded_assets_for_setup_target(SetupTarget::OpenCode).count();
    let iter_claude_count = iter_embedded_assets_for_setup_target(SetupTarget::Claude).count();
    let iter_both_count = iter_embedded_assets_for_setup_target(SetupTarget::Both).count();

    assert_eq!(iter_opencode_count, opencode_count);
    assert_eq!(iter_claude_count, claude_count);
    assert_eq!(iter_both_count, opencode_count + claude_count);
}

#[test]
fn embedded_setup_target_iterator_excludes_skill_tile_manifests() {
    for target in [SetupTarget::OpenCode, SetupTarget::Claude] {
        let installed_paths: Vec<&str> = iter_embedded_assets_for_setup_target(target)
            .map(|asset| asset.relative_path)
            .collect();

        assert!(
            installed_paths
                .iter()
                .all(|path| !is_skill_tile_manifest(path)),
            "setup install iterator should omit skill tile manifests"
        );
        assert!(
            installed_paths
                .iter()
                .any(|path| matches!(*path, "skills/sce-plan-review/SKILL.md")),
            "setup install iterator should keep skill markdown payloads"
        );
    }
}

#[test]
fn embedded_hook_manifest_is_complete_sorted_and_normalized() {
    let hooks: Vec<&super::EmbeddedAsset> = iter_required_hook_assets().collect();
    let paths: Vec<&str> = hooks.iter().map(|asset| asset.relative_path).collect();

    assert_eq!(paths, vec!["commit-msg", "post-commit", "pre-commit"]);

    for hook in hooks {
        assert!(!hook.relative_path.is_empty());
        assert!(!hook.relative_path.contains('/'));
        assert!(!hook.relative_path.contains('\\'));
        assert!(!hook.bytes.is_empty());
        assert!(
            hook.bytes.starts_with(b"#!/bin/sh\n"),
            "embedded hook should start with shell shebang"
        );
    }
}

#[test]
fn required_hook_lookup_resolves_each_canonical_hook() {
    for hook in [
        RequiredHookAsset::PreCommit,
        RequiredHookAsset::CommitMsg,
        RequiredHookAsset::PostCommit,
    ] {
        let asset = get_required_hook_asset(hook).expect("required hook asset should exist");
        assert_eq!(asset.relative_path, hook_filename(hook));
        assert!(!asset.bytes.is_empty());
    }
}

#[test]
fn setup_backup_policy_detects_non_git_repository() {
    assert_eq!(
        super::resolve_setup_backup_policy_with_probe(Path::new("/tmp/non-git"), |_| false),
        SetupBackupPolicy::CreateAndRestoreBackups
    );
}

#[test]
fn setup_backup_policy_detects_git_backed_repository() {
    assert_eq!(
        super::resolve_setup_backup_policy_with_probe(Path::new("/tmp/git"), |_| true),
        SetupBackupPolicy::GitBackedRepository
    );
}

#[test]
fn install_paths_share_git_backed_backup_policy_input() {
    let config_backup_policy =
        super::resolve_setup_backup_policy_with_probe(Path::new("/tmp/shared"), |_| true);
    let hook_backup_policy =
        super::resolve_setup_backup_policy_with_probe(Path::new("/tmp/shared"), |_| true);

    assert_eq!(config_backup_policy, SetupBackupPolicy::GitBackedRepository);
    assert_eq!(hook_backup_policy, config_backup_policy);
}

#[test]
fn git_backed_config_install_skips_backup_creation() -> Result<()> {
    let repository_root = TestDir::new("setup-git-backed-no-backup-success")?;
    let destination_root = repository_root.path().join(".opencode");
    write_existing_setup_target(&destination_root)?;

    let outcome = super::install_embedded_setup_assets_with_rename(
        repository_root.path(),
        SetupTarget::OpenCode,
        |from, to| std::fs::rename(from, to),
        SetupBackupPolicy::GitBackedRepository,
    )?;

    assert_eq!(outcome.target_results.len(), 1);
    let result = &outcome.target_results[0];
    assert_eq!(result.target, SetupTarget::OpenCode);
    assert_eq!(result.backup_root, None);
    assert!(destination_root.exists());
    assert!(!destination_root.join("existing.txt").exists());
    assert!(!repository_root.path().join(".opencode.backup").exists());

    Ok(())
}

#[test]
fn setup_success_output_reports_git_backed_no_backup_policy() {
    let message = super::format_setup_install_success_message(&SetupInstallOutcome {
        target_results: vec![SetupInstallTargetResult {
            target: SetupTarget::OpenCode,
            destination_root: PathBuf::from("/tmp/repo/.opencode"),
            backup_root: None,
            skipped_backup_in_git_backed_repo: true,
            installed_file_count: 3,
        }],
    });

    assert!(message.contains("backup: not created (git-backed repository)"));
    assert!(!message.contains("backup: not needed (no existing target)"));
}

#[test]
fn git_backed_config_install_failure_skips_rollback_and_emits_git_guidance() -> Result<()> {
    let repository_root = TestDir::new("setup-git-backed-no-backup-failure")?;
    let destination_root = repository_root.path().join(".opencode");
    write_existing_setup_target(&destination_root)?;

    let mut rename_calls = Vec::new();
    let error = super::install_embedded_setup_assets_with_rename(
        repository_root.path(),
        SetupTarget::OpenCode,
        |from, to| {
            rename_calls.push((from.to_path_buf(), to.to_path_buf()));
            Err(io::Error::other("injected swap failure"))
        },
        SetupBackupPolicy::GitBackedRepository,
    )
    .expect_err("git-backed swap failure should surface an error");

    let message = format!("{error:#}");
    assert!(message.contains("injected swap failure"));
    assert!(message.contains("Git-backed setup for OpenCode does not create backups"));
    assert!(message.contains(destination_root.to_string_lossy().as_ref()));
    assert_eq!(rename_calls.len(), 1);
    assert_eq!(rename_calls[0].1, destination_root);
    assert!(!repository_root.path().join(".opencode.backup").exists());
    assert!(!destination_root.exists());

    Ok(())
}

#[test]
fn non_git_backed_config_install_still_creates_backup() -> Result<()> {
    let repository_root = TestDir::new("setup-non-git-backup-success")?;
    let destination_root = repository_root.path().join(".opencode");
    write_existing_setup_target(&destination_root)?;

    let outcome = super::install_embedded_setup_assets_with_rename(
        repository_root.path(),
        SetupTarget::OpenCode,
        |from, to| std::fs::rename(from, to),
        SetupBackupPolicy::CreateAndRestoreBackups,
    )?;

    let result = &outcome.target_results[0];
    let backup_root = result
        .backup_root
        .as_ref()
        .expect("non-git-backed install should preserve backups");
    assert!(backup_root.exists());
    assert!(backup_root.join("existing.txt").exists());
    assert!(destination_root.exists());

    Ok(())
}

#[test]
fn git_backed_hook_install_default_hooks_path_skips_backup_creation() -> Result<()> {
    let repository_root = TestDir::new("setup-hooks-git-backed-default-no-backup")?;
    init_git_repository(repository_root.path())?;

    let pre_commit_path = repository_root.path().join(".git/hooks/pre-commit");
    write_existing_hook(&pre_commit_path)?;

    let outcome =
        super::install_required_git_hooks_with_rename(repository_root.path(), |from, to| {
            std::fs::rename(from, to)
        })?;

    assert_eq!(
        outcome.hooks_directory,
        repository_root.path().join(".git/hooks")
    );
    assert_eq!(outcome.hook_results.len(), 3);

    let pre_commit_result = outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == "pre-commit")
        .expect("pre-commit result should exist");
    assert_eq!(
        pre_commit_result.status,
        super::RequiredHookInstallStatus::Updated
    );
    assert_eq!(pre_commit_result.backup_path, None);
    assert!(!repository_root
        .path()
        .join(".git/hooks/pre-commit.backup")
        .exists());

    let expected_hook = get_required_hook_asset(RequiredHookAsset::PreCommit)
        .expect("pre-commit asset should exist");
    assert_eq!(fs::read(&pre_commit_path)?, expected_hook.bytes);

    Ok(())
}

#[test]
fn hook_success_output_reports_git_backed_no_backup_policy() {
    let message =
        super::format_required_hook_install_success_message(&RequiredHooksInstallOutcome {
            repository_root: PathBuf::from("/tmp/repo"),
            hooks_directory: PathBuf::from("/tmp/repo/.git/hooks"),
            hook_results: vec![RequiredHookInstallResult {
                hook_name: "commit-msg".to_string(),
                hook_path: PathBuf::from("/tmp/repo/.git/hooks/commit-msg"),
                status: RequiredHookInstallStatus::Updated,
                backup_path: None,
                skipped_backup_in_git_backed_repo: true,
            }],
        });

    assert!(message.contains("backup: not created (git-backed repository)"));
    assert!(!message.contains("backup: not needed"));
}

#[test]
fn git_backed_hook_install_custom_hooks_path_skips_backup_creation() -> Result<()> {
    let repository_root = TestDir::new("setup-hooks-git-backed-custom-no-backup")?;
    init_git_repository(repository_root.path())?;
    run_git(
        repository_root.path(),
        &["config", "core.hooksPath", "custom-hooks"],
    )?;

    let custom_hooks_path = repository_root.path().join("custom-hooks");
    let commit_msg_path = custom_hooks_path.join("commit-msg");
    write_existing_hook(&commit_msg_path)?;

    let outcome =
        super::install_required_git_hooks_with_rename(repository_root.path(), |from, to| {
            std::fs::rename(from, to)
        })?;

    assert_eq!(outcome.hooks_directory, custom_hooks_path);

    let commit_msg_result = outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == "commit-msg")
        .expect("commit-msg result should exist");
    assert_eq!(
        commit_msg_result.status,
        super::RequiredHookInstallStatus::Updated
    );
    assert_eq!(commit_msg_result.backup_path, None);
    assert!(!repository_root
        .path()
        .join("custom-hooks/commit-msg.backup")
        .exists());

    let expected_hook = get_required_hook_asset(RequiredHookAsset::CommitMsg)
        .expect("commit-msg asset should exist");
    assert_eq!(fs::read(&commit_msg_path)?, expected_hook.bytes);

    Ok(())
}

#[test]
fn git_backed_hook_install_failure_skips_rollback_and_emits_git_guidance() -> Result<()> {
    let repository_root = TestDir::new("setup-hooks-git-backed-failure")?;
    init_git_repository(repository_root.path())?;

    let commit_msg_path = repository_root.path().join(".git/hooks/commit-msg");
    write_existing_hook(&commit_msg_path)?;

    let mut rename_calls = Vec::new();
    let error =
        super::install_required_git_hooks_with_rename(repository_root.path(), |from, to| {
            rename_calls.push((from.to_path_buf(), to.to_path_buf()));
            Err(io::Error::other("injected hook swap failure"))
        })
        .expect_err("git-backed hook swap failure should surface an error");

    let message = format!("{error:#}");
    assert!(message.contains("injected hook swap failure"));
    assert!(message.contains("Git-backed hook setup does not create backups"));
    assert!(message.contains(commit_msg_path.to_string_lossy().as_ref()));
    assert_eq!(rename_calls.len(), 1);
    assert_eq!(rename_calls[0].1, commit_msg_path);
    assert!(!repository_root
        .path()
        .join(".git/hooks/commit-msg.backup")
        .exists());
    assert!(!commit_msg_path.exists());

    Ok(())
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Result<Self> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("sce-setup-{label}-{unique}"));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_existing_setup_target(destination_root: &Path) -> Result<()> {
    fs::create_dir_all(destination_root)?;
    fs::write(destination_root.join("existing.txt"), "existing")?;
    Ok(())
}

fn write_existing_hook(hook_path: &Path) -> Result<()> {
    let parent = hook_path
        .parent()
        .expect("hook path should always have a parent directory");
    fs::create_dir_all(parent)?;
    fs::write(hook_path, "#!/bin/sh\nexit 0\n")?;
    Ok(())
}

fn init_git_repository(repository_root: &Path) -> Result<()> {
    run_git(repository_root, &["init"])?;
    Ok(())
}

fn run_git(repository_root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn runtime_target_root(target: SetupTarget) -> PathBuf {
    let target_relative = match target {
        SetupTarget::OpenCode => "config/.opencode",
        SetupTarget::Claude => "config/.claude",
        SetupTarget::Both => unreachable!("both is not a concrete filesystem root"),
    };

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cli crate should be nested under repository root")
        .join(target_relative)
}

fn assets_for_target(target: SetupTarget) -> &'static [super::EmbeddedAsset] {
    match target {
        SetupTarget::OpenCode => super::OPENCODE_EMBEDDED_ASSETS,
        SetupTarget::Claude => super::CLAUDE_EMBEDDED_ASSETS,
        SetupTarget::Both => unreachable!("both is not a single embedded target"),
    }
}

fn expected_installable_paths(target: SetupTarget) -> Vec<String> {
    assets_for_target(target)
        .iter()
        .map(|asset| asset.relative_path)
        .filter(|path| !is_skill_tile_manifest(path))
        .map(ToString::to_string)
        .collect()
}

fn collect_runtime_relative_paths(root: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    collect_runtime_files(root, root, &mut files)?;

    files.sort_unstable();

    let stable_paths = files
        .into_iter()
        .map(|path| {
            path.to_str()
                .expect("runtime config path should be UTF-8")
                .replace('\\', "/")
        })
        .collect();

    Ok(stable_paths)
}

fn collect_runtime_files(
    base_root: &Path,
    current_dir: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in std::fs::read_dir(current_dir)? {
        let entry = entry?;
        let path = entry.path();

        if entry.file_type()?.is_dir() {
            collect_runtime_files(base_root, &path, output)?;
            continue;
        }

        let relative = path
            .strip_prefix(base_root)
            .expect("relative path should be under root")
            .to_path_buf();
        output.push(relative);
    }

    Ok(())
}

fn hook_filename(hook: RequiredHookAsset) -> &'static str {
    match hook {
        RequiredHookAsset::PreCommit => "pre-commit",
        RequiredHookAsset::CommitMsg => "commit-msg",
        RequiredHookAsset::PostCommit => "post-commit",
    }
}

fn is_skill_tile_manifest(path: &str) -> bool {
    matches!(
        path.split('/').collect::<Vec<_>>().as_slice(),
        ["skills", _, "tile.json"]
    )
}
