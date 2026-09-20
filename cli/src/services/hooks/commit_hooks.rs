use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{to_string as serialize_to_json, Value};

use crate::services::agent_trace::{
    agent_trace_persisted_url, build_agent_trace_from_evidence, patch_has_touched_lines,
    patches_have_overlap, validate_agent_trace_value, AgentTrace, AgentTraceEvidence,
    AgentTraceMetadataInput, AgentTraceVcsType,
};
use crate::services::agent_trace_db::{
    AgentTraceInsert, PostCommitPatchIntersectionInsert, RecentDiffTracePatches,
};
use crate::services::config;
use crate::services::observability::traits::Logger;
use crate::services::patch::{
    combine_patches as combine_patches_fn, intersect_patches as intersect_patches_fn,
    parse_patch as parse_patch_from_text, ParsedPatch,
};
use crate::services::sync::auto_sync;

use super::runtime::{
    commit_msg_policy_gate_passed, current_unix_time_ms, open_agent_trace_db_for_hook_runtime,
    post_rewrite_no_op_reason, pre_commit_no_op_reason, read_hook_stdin, resolve_runtime_state,
    run_git_command_capture_stdout, HookRuntimeState,
};
use super::HookSubcommand;
use super::CANONICAL_SCE_COAUTHOR_TRAILER;

pub(crate) fn run_pre_commit_subcommand_with_trace(repository_root: &Path) -> Result<String> {
    run_pre_commit_subcommand(repository_root)
}

pub(crate) fn run_pre_commit_subcommand(repository_root: &Path) -> Result<String> {
    let runtime = resolve_runtime_state(repository_root)?;

    Ok(format!(
        "pre-commit hook executed with no-op runtime state: {:?}",
        pre_commit_no_op_reason(&runtime)
    ))
}

pub(crate) fn run_commit_msg_subcommand_in_repo(
    repository_root: &Path,
    message_file: &Path,
    logger: Option<&dyn Logger>,
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
    let ai_contribution_present = if gate_passed {
        match staged_diff_has_ai_overlap(repository_root, logger) {
            StagedDiffAiOverlapResult::Overlap => true,
            StagedDiffAiOverlapResult::NoOverlap | StagedDiffAiOverlapResult::Error => false,
        }
    } else {
        false
    };
    let transformed =
        apply_commit_msg_coauthor_policy(&runtime, ai_contribution_present, &original);
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

pub(crate) fn run_commit_msg_subcommand_with_trace(
    repository_root: &Path,
    _: &HookSubcommand,
    message_file: &Path,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_commit_msg_subcommand_in_repo(repository_root, message_file, logger)
}

pub(crate) fn run_post_commit_subcommand(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_post_commit_subcommand_with(
        repository_root,
        vcs_type,
        remote_url,
        run_post_commit_intersection_flow,
        run_post_commit_agent_trace_flow,
        |root| {
            config::resolve_hook_runtime_config(root).map(|runtime| runtime.agent_trace_auto_sync)
        },
        |root| {
            auto_sync::launch(root);
            Ok(())
        },
        run_post_commit_passive_checkpoint,
        logger,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_post_commit_subcommand_with<F, B, C, L, K>(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    run_intersection_flow: F,
    run_agent_trace_flow: B,
    resolve_auto_sync: C,
    launch_auto_sync: L,
    run_passive_checkpoint: K,
    logger: Option<&dyn Logger>,
) -> Result<String>
where
    F: FnOnce(&Path) -> Result<PostCommitIntersectionFlowResult>,
    B: FnOnce(
        &Path,
        &PostCommitIntersectionFlowResult,
        Option<AgentTraceVcsType>,
        &str,
    ) -> Result<AgentTrace>,
    C: FnOnce(&Path) -> Result<bool>,
    L: FnOnce(&Path) -> Result<()>,
    K: FnOnce(&Path) -> Result<()>,
{
    let result = run_intersection_flow(repository_root)?;
    let _agent_trace = run_agent_trace_flow(repository_root, &result, vcs_type, remote_url)?;

    if let Err(error) = run_passive_checkpoint(repository_root) {
        if let Some(log) = logger {
            log.warn(
                "sce.agent_trace_db.passive_checkpoint_failed",
                &error.to_string(),
                &[],
                None,
            );
        }
    }

    if resolve_auto_sync(repository_root)? {
        let _ = launch_auto_sync(repository_root);
    }

    Ok(format!(
        "post-commit hook processed intersection: commit={}, intersection_files={}",
        result.post_commit_data.commit_oid,
        result.combined_recent_patch.files.len()
    ))
}

pub(crate) fn run_post_commit_passive_checkpoint(repository_root: &Path) -> Result<()> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit checkpoint.",
    )?;

    db.passive_checkpoint()
}

pub(crate) fn run_post_commit_agent_trace_flow(
    repository_root: &Path,
    flow_result: &PostCommitIntersectionFlowResult,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
) -> Result<AgentTrace> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit trace.",
    )?;

    let direct_intersection = intersect_patches_fn(
        &flow_result.combined_recent_patch,
        &flow_result.post_commit_data.parsed_patch,
    );
    let mutation_ai_patch =
        crate::services::mutation_trace::runtime::resolve_post_commit_mutation_ai_patch(
            repository_root,
            &db,
            &direct_intersection,
            &flow_result.post_commit_data.parsed_patch,
        );

    run_post_commit_agent_trace_flow_with(
        flow_result,
        vcs_type,
        remote_url,
        &mutation_ai_patch,
        |trace_value| {
            validate_agent_trace_value(trace_value)
                .map_err(|error| anyhow!(error.to_string()))
                .context("Failed to verify built post-commit Agent Trace payload.")?;

            Ok(())
        },
        |insert_input| {
            db.insert_agent_trace(insert_input)
                .context("Failed to persist built post-commit Agent Trace payload.")?;

            Ok(())
        },
    )
}

pub(crate) fn run_post_commit_agent_trace_flow_with<V, I>(
    flow_result: &PostCommitIntersectionFlowResult,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    mutation_ai_patch: &ParsedPatch,
    validate_agent_trace: V,
    persist_agent_trace: I,
) -> Result<AgentTrace>
where
    V: FnOnce(&Value) -> Result<()>,
    I: for<'a> FnOnce(AgentTraceInsert<'a>) -> Result<()>,
{
    let commit_timestamp =
        DateTime::<Utc>::from_timestamp_millis(flow_result.post_commit_data.commit_time_ms)
            .ok_or_else(|| {
                anyhow!(
            "Invalid post-commit timestamp '{}': expected a valid Unix epoch millisecond value.",
            flow_result.post_commit_data.commit_time_ms
        )
            })?
            .to_rfc3339();

    let agent_trace = build_agent_trace_from_evidence(
        AgentTraceEvidence {
            direct_patch: &flow_result.combined_recent_patch,
            mutation_ai_patch,
        },
        &flow_result.post_commit_data.parsed_patch,
        AgentTraceMetadataInput {
            commit_timestamp: &commit_timestamp,
            commit_revision: &flow_result.post_commit_data.commit_oid,
            vcs_type,
            tool_name: flow_result.tool_name.as_deref(),
            tool_version: flow_result.tool_version.as_deref(),
        },
    )
    .context("Failed to build Agent Trace payload from post-commit intersection flow result.")?;

    let agent_trace_value = serde_json::to_value(&agent_trace)
        .context("Failed to serialize post-commit Agent Trace payload for validation.")?;
    validate_agent_trace(&agent_trace_value)
        .context("Failed to validate built post-commit Agent Trace payload.")?;

    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&agent_trace)
            .context("Failed to serialize post-commit Agent Trace payload for persistence.")?
    );

    let constructed_url = agent_trace_persisted_url(&agent_trace.id);

    let insert_input = AgentTraceInsert {
        commit_id: &flow_result.post_commit_data.commit_oid,
        commit_time_ms: flow_result.post_commit_data.commit_time_ms,
        trace_json: &serialized,
        agent_trace_id: &agent_trace.id,
        url: &constructed_url,
        remote_url,
    };
    persist_agent_trace(insert_input)?;

    Ok(agent_trace)
}

pub(crate) const RECENT_DAYS_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

pub(crate) fn run_post_commit_intersection_flow(
    repository_root: &Path,
) -> Result<PostCommitIntersectionFlowResult> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit intersection.",
    )?;

    run_post_commit_intersection_flow_with(
        repository_root,
        capture_post_commit_patch_from_git,
        current_unix_time_ms,
        |cutoff_ms, end_ms| {
            db.recent_diff_trace_patches(cutoff_ms, end_ms)
                .context("Failed to query recent diff trace patches.")
        },
        |insert_input| {
            db.insert_post_commit_patch_intersection(insert_input)
                .context("Failed to persist post-commit patch intersection.")?;

            Ok(())
        },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StagedDiffAiOverlapResult {
    Overlap,
    NoOverlap,
    Error,
}

pub(crate) fn staged_diff_has_ai_overlap(
    repository_root: &Path,
    logger: Option<&dyn Logger>,
) -> StagedDiffAiOverlapResult {
    let db_open_result = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for staged AI-overlap evidence check.",
    );

    let db = match db_open_result {
        Ok(db) => db,
        Err(error) => {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.commit_msg.ai_overlap_error",
                    &format!("Staged AI-overlap evidence check failed: {error}."),
                    &[],
                    None,
                );
            }
            return StagedDiffAiOverlapResult::Error;
        }
    };

    let result = staged_diff_has_ai_overlap_with(
        repository_root,
        capture_staged_patch_from_git,
        current_unix_time_ms,
        |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms),
    );

    if result == StagedDiffAiOverlapResult::Error {
        if let Some(log) = logger {
            log.error(
                "sce.hooks.commit_msg.ai_overlap_error",
                "Staged AI-overlap evidence check failed: error during staged-diff or trace query.",
                &[],
                None,
            );
        }
    }

    result
}

pub(crate) fn staged_diff_has_ai_overlap_with<C, N, Q>(
    repository_root: &Path,
    capture_staged_patch: C,
    now_ms: N,
    query_recent_patches: Q,
) -> StagedDiffAiOverlapResult
where
    C: FnOnce(&Path) -> Result<ParsedPatch>,
    N: FnOnce() -> Result<i64>,
    Q: FnOnce(i64, i64) -> Result<RecentDiffTracePatches>,
{
    let Ok(staged_patch) = capture_staged_patch(repository_root) else {
        return StagedDiffAiOverlapResult::Error;
    };

    if !patch_has_touched_lines(&staged_patch) {
        return StagedDiffAiOverlapResult::NoOverlap;
    }

    let Ok(now_ms) = now_ms() else {
        return StagedDiffAiOverlapResult::Error;
    };
    let cutoff_ms = now_ms - RECENT_DAYS_MILLIS;

    let Ok(recent_patches) = query_recent_patches(cutoff_ms, now_ms) else {
        return StagedDiffAiOverlapResult::Error;
    };

    let has_overlap = recent_patches.patches.into_iter().any(|recent_patch| {
        let combined_recent_patch = combine_patches_fn(&[recent_patch.patch]);
        patches_have_overlap(&combined_recent_patch, &staged_patch)
    });

    if has_overlap {
        StagedDiffAiOverlapResult::Overlap
    } else {
        StagedDiffAiOverlapResult::NoOverlap
    }
}

pub(crate) fn capture_staged_patch_from_git(repository_root: &Path) -> Result<ParsedPatch> {
    let patch_text = capture_staged_diff_from_git(repository_root)?;

    if patch_text.trim().is_empty() {
        return Ok(ParsedPatch { files: Vec::new() });
    }

    parse_patch_from_text(&patch_text, None).map_err(|error| {
        anyhow!(staged_patch_error(
            "failed to parse staged patch",
            &error.to_string()
        ))
    })
}

pub(crate) fn capture_staged_diff_from_git(repository_root: &Path) -> Result<String> {
    run_git_command_capture_stdout(
        repository_root,
        &["diff", "--cached", "--patch", "--no-ext-diff"],
        "Failed to capture staged patch from git.",
    )
}

pub(crate) fn staged_patch_error(detail: &str, context: &str) -> String {
    format!("Staged patch capture error: {detail} ({context}).")
}

pub(crate) fn run_post_commit_intersection_flow_with<C, N, Q, P>(
    repository_root: &Path,
    capture_post_commit_patch: C,
    now_ms: N,
    query_recent_patches: Q,
    persist_intersection: P,
) -> Result<PostCommitIntersectionFlowResult>
where
    C: FnOnce(&Path) -> Result<PostCommitPatchData>,
    N: FnOnce() -> Result<i64>,
    Q: FnOnce(i64, i64) -> Result<RecentDiffTracePatches>,
    P: for<'a> FnOnce(PostCommitPatchIntersectionInsert<'a>) -> Result<()>,
{
    let post_commit_data = capture_post_commit_patch(repository_root)?;

    let now_ms = now_ms()?;
    let cutoff_ms = now_ms - RECENT_DAYS_MILLIS;

    let recent_patches = query_recent_patches(cutoff_ms, now_ms)?;

    #[allow(clippy::cast_possible_wrap)]
    let loaded_count = recent_patches.loaded_count() as i64;
    #[allow(clippy::cast_possible_wrap)]
    let skipped_count = recent_patches.skipped_count() as i64;

    let last_patch = recent_patches.patches.last();
    let tool_name = last_patch.and_then(|patch| patch.tool_name.clone());
    let tool_version = last_patch.and_then(|patch| patch.tool_version.clone());

    let recent_patches_slice: Vec<ParsedPatch> = recent_patches
        .patches
        .into_iter()
        .map(|p| p.patch)
        .collect();

    let combined_recent_patch = combine_patches_fn(&recent_patches_slice);

    let intersection_patch =
        intersect_patches_fn(&combined_recent_patch, &post_commit_data.parsed_patch);

    let serialized_intersection = serialize_to_json(&intersection_patch)
        .context("Failed to serialize intersection patch.")?;

    let insert_input = PostCommitPatchIntersectionInsert {
        commit_id: &post_commit_data.commit_oid,
        post_commit_time_ms: post_commit_data.commit_time_ms,
        recent_window_cutoff_ms: cutoff_ms,
        recent_window_end_ms: now_ms,
        loaded_diff_trace_count: loaded_count,
        skipped_diff_trace_count: skipped_count,
        intersection_patch: &serialized_intersection,
    };

    persist_intersection(insert_input)?;

    Ok(PostCommitIntersectionFlowResult {
        combined_recent_patch,
        post_commit_data,
        tool_name,
        tool_version,
    })
}

pub(crate) fn run_post_commit_subcommand_with_trace(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: Option<&str>,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_post_commit_subcommand(
        repository_root,
        vcs_type,
        remote_url.unwrap_or_default(),
        logger,
    )
}

pub(crate) fn run_post_rewrite_subcommand(
    repository_root: &Path,
    rewrite_method: &str,
) -> Result<String> {
    let runtime = resolve_runtime_state(repository_root)?;

    Ok(format!(
        "post-rewrite hook executed with no-op runtime state: {:?} (rewrite_method='{}')",
        post_rewrite_no_op_reason(&runtime),
        rewrite_method.trim()
    ))
}

pub(crate) fn run_post_rewrite_subcommand_with_trace(
    repository_root: &Path,
    _: &HookSubcommand,
    rewrite_method: &str,
) -> Result<String> {
    let stdin_payload = read_hook_stdin();
    stdin_payload.and_then(|_| run_post_rewrite_subcommand(repository_root, rewrite_method))
}

pub(crate) fn hook_runtime_invocation_name(subcommand: &HookSubcommand) -> &'static str {
    match subcommand {
        HookSubcommand::PreCommit => "pre-commit runtime invocation",
        HookSubcommand::CommitMsg { .. } => "commit-msg runtime invocation",
        HookSubcommand::PostCommit { .. } => "post-commit runtime invocation",
        HookSubcommand::PostRewrite { .. } => "post-rewrite runtime invocation",
        HookSubcommand::DiffTrace => "diff-trace runtime invocation",
        HookSubcommand::ConversationTrace => "conversation-trace runtime invocation",
        HookSubcommand::Codex => "codex runtime invocation",
        HookSubcommand::ClaudeModelState => "Claude model-state runtime invocation",
        HookSubcommand::MutationScope => "mutation-scope runtime invocation",
        HookSubcommand::ClaudeMutationScope => "Claude mutation-scope runtime invocation",
        HookSubcommand::CodexMutationScope => "Codex mutation-scope runtime invocation",
        HookSubcommand::OpenCodeMutationScope => "OpenCode mutation-scope runtime invocation",
        HookSubcommand::PiMutationScope => "Pi mutation-scope runtime invocation",
        HookSubcommand::ExternalMutationGuard => "external-mutation-guard runtime invocation",
    }
}
pub fn apply_commit_msg_coauthor_policy(
    runtime: &HookRuntimeState,
    ai_contribution_present: bool,
    commit_message: &str,
) -> String {
    if !commit_msg_policy_gate_passed(runtime) || !ai_contribution_present {
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
pub struct PostCommitPatchData {
    pub commit_oid: String,
    pub commit_time_ms: i64,
    pub parsed_patch: ParsedPatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostCommitIntersectionFlowResult {
    pub combined_recent_patch: ParsedPatch,
    pub post_commit_data: PostCommitPatchData,
    pub tool_name: Option<String>,
    pub tool_version: Option<String>,
}

pub fn capture_post_commit_patch_from_git(repository_root: &Path) -> Result<PostCommitPatchData> {
    let commit_oid = capture_head_oid_from_git(repository_root)?;
    let commit_time_ms = capture_head_timestamp_from_git(repository_root)?;
    let patch_text = capture_head_patch_from_git(repository_root)?;
    let parsed_patch = parse_patch_from_text(&patch_text, None).map_err(|e| {
        anyhow!(post_commit_patch_error(
            "failed to parse post-commit patch",
            &e.to_string()
        ))
    })?;

    Ok(PostCommitPatchData {
        commit_oid,
        commit_time_ms,
        parsed_patch,
    })
}

pub(crate) fn capture_head_oid_from_git(repository_root: &Path) -> Result<String> {
    let output = run_git_command_capture_stdout(
        repository_root,
        &["rev-parse", "HEAD"],
        "Failed to capture HEAD commit OID from git.",
    )?;
    Ok(output.trim().to_string())
}

pub(crate) fn capture_head_timestamp_from_git(repository_root: &Path) -> Result<i64> {
    let output = run_git_command_capture_stdout(
        repository_root,
        &["show", "--format=%ct", "--no-patch", "HEAD"],
        "Failed to capture HEAD commit timestamp from git.",
    )?;
    let timestamp_str = output.trim();
    let timestamp_seconds: i64 = timestamp_str.parse().map_err(|_| {
        anyhow!(post_commit_patch_error(
            "failed to parse HEAD timestamp",
            timestamp_str,
        ))
    })?;
    let timestamp_ms = timestamp_seconds.checked_mul(1000).ok_or_else(|| {
        anyhow!(post_commit_patch_error(
            "failed to parse HEAD timestamp",
            timestamp_str,
        ))
    })?;
    Ok(timestamp_ms)
}

pub(crate) fn capture_head_patch_from_git(repository_root: &Path) -> Result<String> {
    run_git_command_capture_stdout(
        repository_root,
        &["show", "--format=", "--patch", "--no-ext-diff", "HEAD"],
        "Failed to capture HEAD patch from git.",
    )
}

pub(crate) fn post_commit_patch_error(detail: &str, context: &str) -> String {
    format!("Post-commit patch capture error: {detail} ({context}).")
}
