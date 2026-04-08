use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::services::config;

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HookSubcommand {
    PreCommit,
    CommitMsg { message_file: PathBuf },
    PostCommit,
    PostRewrite { rewrite_method: String },
}

pub fn run_hooks_subcommand(subcommand: HookSubcommand) -> Result<String> {
    match subcommand {
        HookSubcommand::PreCommit => run_pre_commit_subcommand(),
        HookSubcommand::CommitMsg { message_file } => run_commit_msg_subcommand(&message_file),
        HookSubcommand::PostCommit => run_post_commit_subcommand(),
        HookSubcommand::PostRewrite { rewrite_method } => {
            run_post_rewrite_subcommand(&rewrite_method)
        }
    }
}

fn run_pre_commit_subcommand() -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for pre-commit runtime invocation.")?;
    let runtime = resolve_runtime_state(&repository_root)?;

    Ok(format!(
        "pre-commit hook executed with no-op runtime state: {:?}",
        pre_commit_no_op_reason(&runtime)
    ))
}

fn run_commit_msg_subcommand(message_file: &Path) -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for commit-msg runtime invocation.")?;
    run_commit_msg_subcommand_in_repo(&repository_root, message_file)
}

fn run_commit_msg_subcommand_in_repo(
    repository_root: &Path,
    message_file: &Path,
) -> Result<String> {
    let metadata = fs::metadata(message_file).with_context(|| {
        format!(
            "Invalid commit message file '{}': file does not exist or is not readable.",
            message_file.display()
        )
    })?;

    if !metadata.is_file() {
        bail!(
            "Invalid commit message file '{}': expected a regular file path.",
            message_file.display()
        );
    }

    let runtime = resolve_runtime_state(repository_root)?;
    let original = fs::read_to_string(message_file).with_context(|| {
        format!(
            "Invalid commit message file '{}': failed to read UTF-8 content.",
            message_file.display()
        )
    })?;

    let gate_passed = commit_msg_policy_gate_passed(&runtime);
    let transformed = apply_commit_msg_coauthor_policy(&runtime, &original);
    let trailer_applied = gate_passed && transformed != original;

    if trailer_applied {
        fs::write(message_file, transformed.as_bytes()).with_context(|| {
            format!(
                "Failed to update commit message file '{}' with canonical co-author trailer.",
                message_file.display()
            )
        })?;
    }

    Ok(format!(
        "commit-msg hook processed message file '{}' (policy_gate_passed={}, trailer_applied={}).",
        message_file.display(),
        gate_passed,
        trailer_applied
    ))
}

fn run_post_commit_subcommand() -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for post-commit runtime invocation.")?;
    let runtime = resolve_runtime_state(&repository_root)?;

    Ok(format!(
        "post-commit hook executed with no-op runtime state: {:?}",
        post_commit_no_op_reason(&runtime)
    ))
}

fn run_post_rewrite_subcommand(rewrite_method: &str) -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for post-rewrite runtime invocation.")?;
    let runtime = resolve_runtime_state(&repository_root)?;

    Ok(format!(
        "post-rewrite hook executed with no-op runtime state: {:?} (rewrite_method='{}')",
        post_rewrite_no_op_reason(&runtime),
        rewrite_method.trim()
    ))
}

fn resolve_runtime_state(repository_root: &Path) -> Result<HookRuntimeState> {
    Ok(HookRuntimeState {
        sce_disabled: env_flag_is_truthy("SCE_DISABLED"),
        attribution_hooks_enabled: config::resolve_hook_runtime_config(repository_root)?
            .attribution_hooks_enabled,
    })
}

fn env_flag_is_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| env_value_is_truthy(&value))
}

fn env_value_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn commit_msg_policy_gate_passed(runtime: &HookRuntimeState) -> bool {
    !runtime.sce_disabled && runtime.attribution_hooks_enabled
}

fn pre_commit_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

fn post_commit_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

fn post_rewrite_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

pub fn apply_commit_msg_coauthor_policy(
    runtime: &HookRuntimeState,
    commit_message: &str,
) -> String {
    if !commit_msg_policy_gate_passed(runtime) {
        return commit_message.to_string();
    }

    let mut lines: Vec<&str> = commit_message.lines().collect();
    lines.retain(|line| *line != CANONICAL_SCE_COAUTHOR_TRAILER);

    if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
        lines.push("");
    }
    lines.push(CANONICAL_SCE_COAUTHOR_TRAILER);

    let mut normalized = lines.join("\n");
    if commit_message.ends_with('\n') {
        normalized.push('\n');
    }

    normalized
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookRuntimeState {
    pub sce_disabled: bool,
    pub attribution_hooks_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookNoOpReason {
    Disabled,
    AttributionOnlyCommitMsgMode,
}
