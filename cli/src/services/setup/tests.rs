use std::{
    cell::Cell,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use crate::test_support::TestTempDir;
use anyhow::Result;

use super::{
    get_required_hook_asset, install_embedded_setup_assets,
    install_embedded_setup_assets_with_rename, install_required_git_hooks,
    install_required_git_hooks_with_rename, iter_embedded_assets_for_setup_target,
    iter_required_hook_assets, resolve_setup_dispatch, resolve_setup_request, run_setup_for_mode,
    run_setup_hooks, RequiredHookAsset, RequiredHookInstallStatus, SetupCliOptions, SetupDispatch,
    SetupMode, SetupTarget,
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
    let temp = TestTempDir::new("sce-setup-install-tests").expect("temp dir should be created");
    let error = run_setup_for_mode(temp.path(), SetupMode::Interactive)
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
fn run_setup_hooks_rejects_file_repo_path() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
    let file_path = temp.path().join("not-a-directory");
    fs::write(&file_path, b"not a repo")?;

    let error = run_setup_hooks(&file_path).expect_err("file path should fail");
    let message = format!("{error:#}");
    assert!(message.contains("is not a directory"));

    Ok(())
}

#[test]
fn run_setup_hooks_rejects_non_git_directory_with_git_init_guidance() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-hook-install-tests")?;

    let error = run_setup_hooks(temp.path()).expect_err("non-git directory should fail");
    let message = format!("{error:#}");
    assert!(message.contains("is not a git repository"));
    assert!(message.contains("git init"));
    assert!(message.contains("rerun 'sce setup'"));

    Ok(())
}

#[test]
fn run_setup_reports_selected_target_and_backup_status() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-install-tests")?;
    fs::create_dir_all(temp.path().join(".opencode/legacy"))?;
    fs::write(temp.path().join(".opencode/legacy/config.txt"), b"legacy")?;

    let message = run_setup_for_mode(
        temp.path(),
        SetupMode::NonInteractive(SetupTarget::OpenCode),
    )?;
    assert!(message.contains("Setup completed successfully."));
    assert!(message.contains("Selected target(s): OpenCode"));
    assert!(message.contains("OpenCode: installed"));
    assert!(message.contains("backup: existing target moved to"));
    assert!(message.contains(".opencode.backup"));

    Ok(())
}

#[test]
fn run_setup_reports_both_targets() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-install-tests")?;
    let message = run_setup_for_mode(temp.path(), SetupMode::NonInteractive(SetupTarget::Both))?;
    assert!(message.contains("Selected target(s): OpenCode, Claude"));
    assert!(message.contains("OpenCode: installed"));
    assert!(message.contains("Claude: installed"));
    assert!(message.contains("backup: not needed (no existing target)"));
    Ok(())
}

#[test]
fn interactive_dispatch_maps_selected_target() -> Result<()> {
    let dispatch = resolve_setup_dispatch(
        SetupMode::Interactive,
        &MockPrompter {
            response: SetupDispatch::Proceed(SetupMode::NonInteractive(SetupTarget::Claude)),
        },
    )?;

    assert_eq!(
        dispatch,
        SetupDispatch::Proceed(SetupMode::NonInteractive(SetupTarget::Claude))
    );
    Ok(())
}

#[test]
fn interactive_dispatch_returns_cancelled_without_side_effects() -> Result<()> {
    let dispatch = resolve_setup_dispatch(
        SetupMode::Interactive,
        &MockPrompter {
            response: SetupDispatch::Cancelled,
        },
    )?;

    assert_eq!(dispatch, SetupDispatch::Cancelled);
    Ok(())
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
fn embedded_manifest_matches_runtime_config_tree() -> Result<()> {
    let opencode_expected =
        collect_runtime_relative_paths(&runtime_target_root(SetupTarget::OpenCode))?;
    let claude_expected =
        collect_runtime_relative_paths(&runtime_target_root(SetupTarget::Claude))?;

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
    Ok(())
}

#[test]
fn embedded_setup_target_iterator_scopes_assets_per_target() {
    let opencode_count = assets_for_target(SetupTarget::OpenCode).len();
    let claude_count = assets_for_target(SetupTarget::Claude).len();

    let iter_opencode_count = iter_embedded_assets_for_setup_target(SetupTarget::OpenCode).count();
    let iter_claude_count = iter_embedded_assets_for_setup_target(SetupTarget::Claude).count();
    let iter_both_count = iter_embedded_assets_for_setup_target(SetupTarget::Both).count();

    assert_eq!(iter_opencode_count, opencode_count);
    assert_eq!(iter_claude_count, claude_count);
    assert_eq!(iter_both_count, opencode_count + claude_count);
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
fn install_engine_replaces_existing_target_with_backup() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-install-tests")?;
    let existing_target = temp.path().join(".opencode");
    fs::create_dir_all(existing_target.join("legacy"))?;
    fs::write(existing_target.join("legacy/config.txt"), b"legacy")?;

    let outcome = install_embedded_setup_assets(temp.path(), SetupTarget::OpenCode)?;
    assert_eq!(outcome.target_results.len(), 1);

    let result = &outcome.target_results[0];
    assert_eq!(result.target, SetupTarget::OpenCode);
    assert_eq!(result.destination_root, temp.path().join(".opencode"));
    assert_eq!(
        result.installed_file_count,
        assets_for_target(SetupTarget::OpenCode).len()
    );

    let backup_root = result
        .backup_root
        .as_ref()
        .expect("existing target should have backup path");
    assert!(backup_root.exists());
    assert!(backup_root.join("legacy/config.txt").exists());

    let installed_paths = collect_runtime_relative_paths(&result.destination_root)?;
    let expected_paths: Vec<String> = assets_for_target(SetupTarget::OpenCode)
        .iter()
        .map(|asset| asset.relative_path.to_string())
        .collect();
    assert_eq!(installed_paths, expected_paths);
    Ok(())
}

#[test]
fn install_engine_installs_both_targets() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-install-tests")?;

    let outcome = install_embedded_setup_assets(temp.path(), SetupTarget::Both)?;
    assert_eq!(outcome.target_results.len(), 2);

    let opencode_paths = collect_runtime_relative_paths(&temp.path().join(".opencode"))?;
    let claude_paths = collect_runtime_relative_paths(&temp.path().join(".claude"))?;

    let expected_opencode: Vec<String> = assets_for_target(SetupTarget::OpenCode)
        .iter()
        .map(|asset| asset.relative_path.to_string())
        .collect();
    let expected_claude: Vec<String> = assets_for_target(SetupTarget::Claude)
        .iter()
        .map(|asset| asset.relative_path.to_string())
        .collect();

    assert_eq!(opencode_paths, expected_opencode);
    assert_eq!(claude_paths, expected_claude);
    Ok(())
}

#[test]
fn install_engine_rolls_back_when_swap_fails() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-install-tests")?;
    let destination = temp.path().join(".opencode");
    fs::create_dir_all(&destination)?;
    fs::write(destination.join("legacy.txt"), b"legacy")?;

    let rename_calls = Cell::new(0_u8);
    let error = install_embedded_setup_assets_with_rename(
        temp.path(),
        SetupTarget::OpenCode,
        |from, to| {
            rename_calls.set(rename_calls.get() + 1);
            if rename_calls.get() == 2 {
                return Err(io::Error::other("injected swap failure"));
            }

            fs::rename(from, to)
        },
    )
    .expect_err("swap failure should bubble up as an error");

    assert!(error.to_string().contains("Failed to swap staged install"));
    assert!(destination.exists());
    assert!(destination.join("legacy.txt").exists());

    let backup = temp.path().join(".opencode.backup");
    assert!(!backup.exists(), "rollback should restore original path");

    for entry in fs::read_dir(temp.path())? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.starts_with(".sce-setup-staging-opencode-"),
            "staging directory should be cleaned up after failure"
        );
    }

    Ok(())
}

#[test]
fn required_hook_install_installs_missing_hooks_in_default_directory() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
    init_git_repo(temp.path())?;

    let outcome = install_required_git_hooks(temp.path())?;
    assert_eq!(outcome.repository_root, temp.path().to_path_buf());
    assert_eq!(outcome.hook_results.len(), 3);
    for hook in outcome.hook_results {
        assert_eq!(hook.status, RequiredHookInstallStatus::Installed);
        assert!(hook.hook_path.exists());
        assert!(hook.backup_path.is_none());
        assert_hook_is_executable(&hook.hook_path)?;
    }

    Ok(())
}

#[test]
fn required_hook_install_rerun_reports_skipped_for_unchanged_hooks() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
    init_git_repo(temp.path())?;

    let first = install_required_git_hooks(temp.path())?;
    assert!(first
        .hook_results
        .iter()
        .all(|hook| hook.status == RequiredHookInstallStatus::Installed));

    let second = install_required_git_hooks(temp.path())?;
    assert!(second
        .hook_results
        .iter()
        .all(|hook| hook.status == RequiredHookInstallStatus::Skipped));
    assert!(second
        .hook_results
        .iter()
        .all(|hook| hook.backup_path.is_none()));

    Ok(())
}

#[test]
fn required_hook_install_updates_noncanonical_hook_in_custom_hooks_path() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
    init_git_repo(temp.path())?;

    run_git_in_repo(temp.path(), &["config", "core.hooksPath", ".githooks"])?;

    let custom_hooks_directory = temp.path().join(".githooks");
    fs::create_dir_all(&custom_hooks_directory)?;
    let commit_msg_path = custom_hooks_directory.join("commit-msg");
    fs::write(&commit_msg_path, b"#!/bin/sh\necho legacy\n")?;
    set_test_file_mode(&commit_msg_path, 0o644)?;

    let outcome = install_required_git_hooks(temp.path())?;
    assert_eq!(outcome.hooks_directory, custom_hooks_directory);

    let updated = outcome
        .hook_results
        .iter()
        .find(|hook| hook.hook_name == "commit-msg")
        .expect("commit-msg result should exist");
    assert_eq!(updated.status, RequiredHookInstallStatus::Updated);
    let backup_path = updated
        .backup_path
        .as_ref()
        .expect("updated hook should retain backup path");
    assert!(backup_path.exists());
    assert_eq!(fs::read(backup_path)?, b"#!/bin/sh\necho legacy\n");
    assert_hook_is_executable(&updated.hook_path)?;

    Ok(())
}

#[test]
fn required_hook_install_rolls_back_when_hook_swap_fails() -> Result<()> {
    let temp = TestTempDir::new("sce-setup-hook-install-tests")?;
    init_git_repo(temp.path())?;

    let hooks_directory = temp.path().join(".git/hooks");
    fs::create_dir_all(&hooks_directory)?;
    let commit_msg_path = hooks_directory.join("commit-msg");
    fs::write(&commit_msg_path, b"#!/bin/sh\necho legacy\n")?;

    let rename_calls = Cell::new(0_u8);
    let error = install_required_git_hooks_with_rename(temp.path(), |from, to| {
        rename_calls.set(rename_calls.get() + 1);
        if rename_calls.get() == 2 {
            return Err(io::Error::other("injected hook swap failure"));
        }

        fs::rename(from, to)
    })
    .expect_err("hook swap failure should bubble up");

    assert!(error
        .to_string()
        .contains("Failed to update required hook 'commit-msg'"));
    assert!(commit_msg_path.exists());
    assert_eq!(fs::read(&commit_msg_path)?, b"#!/bin/sh\necho legacy\n");
    assert!(!hooks_directory.join("commit-msg.backup").exists());

    for entry in fs::read_dir(&hooks_directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.starts_with(".sce-hook-staging-"),
            "hook staging file should be cleaned up after failure"
        );
    }

    Ok(())
}

fn init_git_repo(repository_root: &Path) -> Result<()> {
    run_git_in_repo(repository_root, &["init", "-q"])?;
    Ok(())
}

fn run_git_in_repo(repository_root: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .status()?;
    if !status.success() {
        anyhow::bail!("git command failed for test repository")
    }
    Ok(())
}

#[cfg(unix)]
fn set_test_file_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_test_file_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn assert_hook_is_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)?;
    assert!(metadata.permissions().mode() & 0o111 != 0);
    Ok(())
}

#[cfg(not(unix))]
fn assert_hook_is_executable(path: &Path) -> Result<()> {
    assert!(path.exists());
    Ok(())
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
    for entry in fs::read_dir(current_dir)? {
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
