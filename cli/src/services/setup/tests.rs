use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{
    get_required_hook_asset, iter_embedded_assets_for_setup_target, iter_required_hook_assets,
    resolve_setup_dispatch, resolve_setup_request, run_setup_for_mode, run_setup_hooks,
    RequiredHookAsset, SetupCliOptions, SetupDispatch, SetupMode, SetupTarget,
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
