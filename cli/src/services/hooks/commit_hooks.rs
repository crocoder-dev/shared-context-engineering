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
use crate::services::config::{self, ResolvedHookRuntimeConfig};
use crate::services::patch::{
    combine_patches as combine_patches_fn, intersect_patches as intersect_patches_fn,
    parse_patch as parse_patch_from_text, ParsedPatch,
};
use crate::services::sync::auto_sync;

use super::commit_patch_id::compute_commit_patch_id;
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

pub(crate) async fn run_commit_msg_subcommand_in_repo<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    message_file: &Path,
    logger: Option<&L>,
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
        match staged_diff_has_ai_overlap(repository_root, logger).await {
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

pub(crate) async fn run_commit_msg_subcommand_with_trace<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    _: &HookSubcommand,
    message_file: &Path,
    logger: Option<&L>,
) -> Result<String> {
    run_commit_msg_subcommand_in_repo(repository_root, message_file, logger).await
}

pub(crate) async fn run_post_commit_subcommand<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    logger: Option<&L>,
) -> Result<String> {
    run_post_commit_subcommand_with(
        repository_root,
        vcs_type,
        remote_url,
        run_post_commit_intersection_flow,
        run_post_commit_agent_trace_flow,
        config::resolve_hook_runtime_config,
        |root| {
            auto_sync::launch(root);
            Ok(())
        },
        run_post_commit_passive_checkpoint,
        logger,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_post_commit_subcommand_with<
    L: crate::services::observability::traits::Logger,
    F,
    B,
    C,
    Launch,
    K,
>(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    run_intersection_flow: F,
    run_agent_trace_flow: B,
    resolve_hook_config: C,
    launch_auto_sync: Launch,
    run_passive_checkpoint: K,
    logger: Option<&L>,
) -> Result<String>
where
    F: std::ops::AsyncFnOnce(&Path) -> Result<PostCommitIntersectionFlowResult>,
    B: std::ops::AsyncFnOnce(
        &Path,
        &PostCommitIntersectionFlowResult,
        Option<AgentTraceVcsType>,
        &str,
        Option<&L>,
    ) -> Result<AgentTrace>,
    C: FnOnce(&Path) -> Result<ResolvedHookRuntimeConfig>,
    Launch: FnOnce(&Path) -> Result<()>,
    K: std::ops::AsyncFnOnce(&Path) -> Result<()>,
{
    let result = run_intersection_flow(repository_root).await?;
    let _agent_trace =
        run_agent_trace_flow(repository_root, &result, vcs_type, remote_url, logger).await?;

    if let Err(error) = run_passive_checkpoint(repository_root).await {
        if let Some(log) = logger {
            log.warn(
                "sce.agent_trace_db.passive_checkpoint_failed",
                &error.to_string(),
                &[],
                None,
            );
        }
    }

    let hook_config = resolve_hook_config(repository_root)?;
    if hook_config
        .access_policy
        .auto_sync_allowed(hook_config.agent_trace_auto_sync)
    {
        let _ = launch_auto_sync(repository_root);
    } else if hook_config.agent_trace_auto_sync {
        if let Some(log) = logger {
            log.debug(
                AUTO_SYNC_SKIPPED_BY_POLICY_EVENT,
                AUTO_SYNC_SKIPPED_BY_POLICY_MESSAGE,
                &[],
                None,
            );
        }
    }

    Ok(format!(
        "post-commit hook processed intersection: commit={}, intersection_files={}",
        result.post_commit_data.commit_oid,
        result.combined_recent_patch.files.len()
    ))
}

pub(crate) async fn run_post_commit_passive_checkpoint(repository_root: &Path) -> Result<()> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit checkpoint.",
    )
    .await?;

    db.passive_checkpoint().await
}

const AUTO_SYNC_SKIPPED_BY_POLICY_EVENT: &str = "sce.agent_trace.auto_sync_skipped_by_policy";
const AUTO_SYNC_SKIPPED_BY_POLICY_MESSAGE: &str =
    "Post-commit auto-sync skipped: background cloud operations are not allowed by SCE config.";

pub(crate) const COMMIT_PATCH_ID_UNAVAILABLE_EVENT: &str =
    "sce.agent_trace.commit_patch_id_unavailable";

pub(crate) async fn run_post_commit_agent_trace_flow<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    flow_result: &PostCommitIntersectionFlowResult,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    logger: Option<&L>,
) -> Result<AgentTrace> {
    let mut db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit trace.",
    )
    .await?;

    let direct_intersection = intersect_patches_fn(
        &flow_result.combined_recent_patch,
        &flow_result.post_commit_data.parsed_patch,
    );
    let mutation_ai_patch =
        crate::services::mutation_trace::runtime::resolve_post_commit_mutation_ai_patch(
            repository_root,
            &mut db,
            &direct_intersection,
            &flow_result.post_commit_data.parsed_patch,
        )
        .await;

    let commit_patch_id = resolve_post_commit_commit_patch_id(
        repository_root,
        &flow_result.post_commit_data.commit_oid,
        logger,
    )
    .await;

    run_post_commit_agent_trace_flow_with(
        flow_result,
        vcs_type,
        remote_url,
        &mutation_ai_patch,
        commit_patch_id.as_deref(),
        |trace_value| {
            validate_agent_trace_value(trace_value)
                .map_err(|error| anyhow!(error.to_string()))
                .context("Failed to verify built post-commit Agent Trace payload.")?;

            Ok(())
        },
        async |insert_input| {
            db.insert_agent_trace(insert_input)
                .await
                .context("Failed to persist built post-commit Agent Trace payload.")?;

            Ok(())
        },
    )
    .await
}

pub(crate) async fn resolve_post_commit_commit_patch_id<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    commit_oid: &str,
    logger: Option<&L>,
) -> Option<String> {
    match compute_commit_patch_id(repository_root, commit_oid).await {
        Ok(commit_patch_id) => commit_patch_id,
        Err(error) => {
            if let Some(log) = logger {
                log.warn(
                    COMMIT_PATCH_ID_UNAVAILABLE_EVENT,
                    &format!(
                        "Commit patch ID unavailable; persisting Agent Trace without it: {error}"
                    ),
                    &[],
                    None,
                );
            }
            None
        }
    }
}

pub(crate) async fn run_post_commit_agent_trace_flow_with<V, I>(
    flow_result: &PostCommitIntersectionFlowResult,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: &str,
    mutation_ai_patch: &ParsedPatch,
    commit_patch_id: Option<&str>,
    validate_agent_trace: V,
    persist_agent_trace: I,
) -> Result<AgentTrace>
where
    V: FnOnce(&Value) -> Result<()>,
    I: for<'a> std::ops::AsyncFnOnce(AgentTraceInsert<'a>) -> Result<()>,
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
        commit_patch_id,
    };
    persist_agent_trace(insert_input).await?;

    Ok(agent_trace)
}

pub(crate) const RECENT_DAYS_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

pub(crate) async fn run_post_commit_intersection_flow(
    repository_root: &Path,
) -> Result<PostCommitIntersectionFlowResult> {
    let db = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for post-commit intersection.",
    )
    .await?;

    run_post_commit_intersection_flow_with(
        repository_root,
        capture_post_commit_patch_from_git,
        current_unix_time_ms,
        async |cutoff_ms, end_ms| {
            {
                db.recent_diff_trace_patches(cutoff_ms, end_ms)
                    .await
                    .context("Failed to query recent diff trace patches.")
            }
        },
        async |insert_input| {
            db.insert_post_commit_patch_intersection(insert_input)
                .await
                .context("Failed to persist post-commit patch intersection.")?;

            Ok(())
        },
    )
    .await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StagedDiffAiOverlapResult {
    Overlap,
    NoOverlap,
    Error,
}

pub(crate) async fn staged_diff_has_ai_overlap<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    logger: Option<&L>,
) -> StagedDiffAiOverlapResult {
    let db_open_result = open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for staged AI-overlap evidence check.",
    )
    .await;

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
        async |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms).await,
    )
    .await;

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

pub(crate) async fn staged_diff_has_ai_overlap_with<C, N, Q>(
    repository_root: &Path,
    capture_staged_patch: C,
    now_ms: N,
    query_recent_patches: Q,
) -> StagedDiffAiOverlapResult
where
    C: FnOnce(&Path) -> Result<ParsedPatch>,
    N: FnOnce() -> Result<i64>,
    Q: std::ops::AsyncFnOnce(i64, i64) -> Result<RecentDiffTracePatches>,
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

    let Ok(recent_patches) = query_recent_patches(cutoff_ms, now_ms).await else {
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

pub(crate) async fn run_post_commit_intersection_flow_with<C, N, Q, P>(
    repository_root: &Path,
    capture_post_commit_patch: C,
    now_ms: N,
    query_recent_patches: Q,
    persist_intersection: P,
) -> Result<PostCommitIntersectionFlowResult>
where
    C: FnOnce(&Path) -> Result<PostCommitPatchData>,
    N: FnOnce() -> Result<i64>,
    Q: std::ops::AsyncFnOnce(i64, i64) -> Result<RecentDiffTracePatches>,
    P: for<'a> std::ops::AsyncFnOnce(PostCommitPatchIntersectionInsert<'a>) -> Result<()>,
{
    let post_commit_data = capture_post_commit_patch(repository_root)?;

    let now_ms = now_ms()?;
    let cutoff_ms = now_ms - RECENT_DAYS_MILLIS;

    let recent_patches = query_recent_patches(cutoff_ms, now_ms).await?;

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

    persist_intersection(insert_input).await?;

    Ok(PostCommitIntersectionFlowResult {
        combined_recent_patch,
        post_commit_data,
        tool_name,
        tool_version,
    })
}

pub(crate) async fn run_post_commit_subcommand_with_trace<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    vcs_type: Option<AgentTraceVcsType>,
    remote_url: Option<&str>,
    logger: Option<&L>,
) -> Result<String> {
    run_post_commit_subcommand(
        repository_root,
        vcs_type,
        remote_url.unwrap_or_default(),
        logger,
    )
    .await
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Mutex;

    use anyhow::{anyhow, Result};

    use crate::services::agent_trace::{validate_agent_trace_value, AgentTrace, AgentTraceVcsType};
    use crate::services::agent_trace_db::AgentTraceInsert;
    use crate::services::config::resolver::resolve_hook_runtime_config_with;
    use crate::services::error::CliError;
    use crate::services::observability::traits::Logger;
    use crate::services::patch::ParsedPatch;

    use super::super::commit_patch_id::compute_commit_patch_id;
    use super::{
        capture_post_commit_patch_from_git, resolve_post_commit_commit_patch_id,
        run_post_commit_agent_trace_flow_with, run_post_commit_subcommand_with,
        PostCommitIntersectionFlowResult, AUTO_SYNC_SKIPPED_BY_POLICY_EVENT,
        COMMIT_PATCH_ID_UNAVAILABLE_EVENT,
    };

    const REMOTE_URL: &str = "https://example.invalid/repo.git";
    const SECRET_FILE_CONTENT: &str = "commit-patch-id-secret-line";
    const UNKNOWN_COMMIT_OID: &str = "1111111111111111111111111111111111111111";
    const TRACE_ID_PLACEHOLDER: &str = "<agent-trace-id>";

    #[derive(Default)]
    struct RecordingLogger {
        warnings: Mutex<Vec<(String, String)>>,
        debugs: Mutex<Vec<String>>,
    }

    impl RecordingLogger {
        fn warnings(&self) -> Vec<(String, String)> {
            self.warnings.lock().unwrap().clone()
        }

        fn debugs(&self) -> Vec<String> {
            self.debugs.lock().unwrap().clone()
        }
    }

    impl Logger for RecordingLogger {
        fn info(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn debug(&self, event_id: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {
            self.debugs.lock().unwrap().push(event_id.to_string());
        }
        fn warn(&self, event_id: &str, message: &str, _: &[(&str, &str)], _: Option<&str>) {
            self.warnings
                .lock()
                .unwrap()
                .push((event_id.to_string(), message.to_string()));
        }
        fn error(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn log_cli_error(&self, _: &CliError, _: Option<&str>) {}
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct CapturedInsert {
        commit_id: String,
        commit_time_ms: i64,
        trace_json: String,
        agent_trace_id: String,
        remote_url: String,
        commit_patch_id: Option<String>,
    }

    impl CapturedInsert {
        fn from_insert(insert: &AgentTraceInsert<'_>) -> Self {
            Self {
                commit_id: insert.commit_id.to_string(),
                commit_time_ms: insert.commit_time_ms,
                trace_json: insert.trace_json.to_string(),
                agent_trace_id: insert.agent_trace_id.to_string(),
                remote_url: insert.remote_url.to_string(),
                commit_patch_id: insert.commit_patch_id.map(str::to_string),
            }
        }
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git output is UTF-8")
            .trim()
            .to_string()
    }

    fn init_repository_with_commit(root: &Path) -> String {
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["config", "user.email", "patch-id@example.invalid"]);
        git(root, &["config", "user.name", "Patch Id Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "one\n").expect("write a.txt");
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "root"]);
        std::fs::write(root.join("a.txt"), format!("one\n{SECRET_FILE_CONTENT}\n"))
            .expect("write a.txt");
        git(root, &["commit", "-q", "-am", "change"]);
        git(root, &["rev-parse", "HEAD"])
    }

    fn flow_result(root: &Path) -> PostCommitIntersectionFlowResult {
        PostCommitIntersectionFlowResult {
            combined_recent_patch: empty_patch(),
            post_commit_data: capture_post_commit_patch_from_git(root)
                .expect("post-commit patch capture"),
            tool_name: None,
            tool_version: None,
        }
    }

    fn empty_patch() -> ParsedPatch {
        ParsedPatch { files: Vec::new() }
    }

    async fn run_flow_capturing(
        flow_result: &PostCommitIntersectionFlowResult,
        commit_patch_id: Option<&str>,
    ) -> (AgentTrace, CapturedInsert) {
        let captured = RefCell::new(None);
        let agent_trace = run_post_commit_agent_trace_flow_with(
            flow_result,
            None,
            REMOTE_URL,
            &empty_patch(),
            commit_patch_id,
            |trace_value| {
                validate_agent_trace_value(trace_value).map_err(|error| anyhow!(error.to_string()))
            },
            async |insert: AgentTraceInsert<'_>| -> Result<()> {
                *captured.borrow_mut() = Some(CapturedInsert::from_insert(&insert));
                Ok(())
            },
        )
        .await
        .expect("post-commit agent trace flow");
        let insert = captured.into_inner().expect("agent trace insert captured");
        (agent_trace, insert)
    }

    fn normalized_trace_json(insert: &CapturedInsert) -> String {
        insert
            .trace_json
            .replace(&insert.agent_trace_id, TRACE_ID_PLACEHOLDER)
    }

    #[tokio::test]
    async fn commit_patch_id_is_attached_to_post_commit_insert_without_changing_trace_json() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let commit_oid = init_repository_with_commit(root);
        let flow_result = flow_result(root);
        assert_eq!(flow_result.post_commit_data.commit_oid, commit_oid);

        let logger = RecordingLogger::default();
        let commit_patch_id =
            resolve_post_commit_commit_patch_id(root, &commit_oid, Some(&logger)).await;
        let expected = compute_commit_patch_id(root, &commit_oid)
            .await
            .expect("canonical patch id")
            .expect("non-empty commit");
        assert_eq!(commit_patch_id.as_deref(), Some(expected.as_str()));
        assert!(logger.warnings().is_empty());

        let (with_trace, with_id) = run_flow_capturing(&flow_result, Some(&expected)).await;
        let (without_trace, without_id) = run_flow_capturing(&flow_result, None).await;

        assert_eq!(with_id.commit_id, commit_oid);
        assert_eq!(with_id.commit_patch_id.as_deref(), Some(expected.as_str()));
        assert_eq!(without_id.commit_id, commit_oid);
        assert_eq!(without_id.commit_patch_id, None);
        assert_eq!(with_id.commit_time_ms, without_id.commit_time_ms);
        assert_eq!(with_id.remote_url, REMOTE_URL);

        for (agent_trace, insert) in [(&with_trace, &with_id), (&without_trace, &without_id)] {
            assert_eq!(insert.agent_trace_id, agent_trace.id);
            assert_eq!(
                insert.trace_json,
                format!("{}\n", serde_json::to_string_pretty(agent_trace).unwrap())
            );
        }
        assert!(!with_id.trace_json.contains(&expected));
        assert_eq!(
            normalized_trace_json(&with_id),
            normalized_trace_json(&without_id)
        );
    }

    #[tokio::test]
    async fn commit_patch_id_is_none_without_warning_for_empty_post_commit() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository_with_commit(root);
        git(root, &["commit", "-q", "--allow-empty", "-m", "empty"]);
        let empty_oid = git(root, &["rev-parse", "HEAD"]);

        let logger = RecordingLogger::default();
        let commit_patch_id =
            resolve_post_commit_commit_patch_id(root, &empty_oid, Some(&logger)).await;

        assert_eq!(commit_patch_id, None);
        assert!(logger.warnings().is_empty());
    }

    #[tokio::test]
    async fn commit_patch_id_failure_logs_sanitized_warning_and_persists_null() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository_with_commit(root);
        let flow_result = flow_result(root);

        let logger = RecordingLogger::default();
        let commit_patch_id =
            resolve_post_commit_commit_patch_id(root, UNKNOWN_COMMIT_OID, Some(&logger)).await;
        assert_eq!(commit_patch_id, None);

        let warnings = logger.warnings();
        assert_eq!(warnings.len(), 1);
        let (event_id, message) = &warnings[0];
        assert_eq!(event_id, COMMIT_PATCH_ID_UNAVAILABLE_EVENT);
        assert!(
            message.starts_with("Commit patch ID unavailable; persisting Agent Trace without it: ")
        );
        assert!(!message.contains(SECRET_FILE_CONTENT));

        let (_, insert) = run_flow_capturing(&flow_result, commit_patch_id.as_deref()).await;
        assert_eq!(insert.commit_id, flow_result.post_commit_data.commit_oid);
        assert_eq!(insert.commit_patch_id, None);
    }

    #[tokio::test]
    async fn commit_patch_id_does_not_mask_agent_trace_validation_or_persistence_failures() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        init_repository_with_commit(root);
        let flow_result = flow_result(root);
        let commit_patch_id = "a".repeat(40);

        let persisted = RefCell::new(false);
        let validation_error = run_post_commit_agent_trace_flow_with(
            &flow_result,
            None,
            REMOTE_URL,
            &empty_patch(),
            Some(&commit_patch_id),
            |_| Err(anyhow!("validation rejected")),
            async |_insert: AgentTraceInsert<'_>| -> Result<()> {
                *persisted.borrow_mut() = true;
                Ok(())
            },
        )
        .await
        .expect_err("validation failure must propagate");
        assert_eq!(
            validation_error.to_string(),
            "Failed to validate built post-commit Agent Trace payload."
        );
        assert!(!*persisted.borrow());

        let persistence_error = run_post_commit_agent_trace_flow_with(
            &flow_result,
            None,
            REMOTE_URL,
            &empty_patch(),
            Some(&commit_patch_id),
            |_| Ok(()),
            async |_insert: AgentTraceInsert<'_>| -> Result<()> {
                Err(anyhow!("persistence rejected"))
            },
        )
        .await
        .expect_err("persistence failure must propagate");
        assert_eq!(persistence_error.to_string(), "persistence rejected");
    }

    #[tokio::test]
    async fn post_commit_launches_auto_sync_only_when_background_cloud_is_allowed() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).expect("repo dir");
        init_repository_with_commit(&root);
        let global = dir.path().join("global.json");
        let local = root.join(".sce").join("config.json");
        std::fs::create_dir_all(local.parent().expect("local config dir")).expect("local dir");

        for auto_sync in [false, true] {
            for (case, auth_mode, degraded) in [
                ("disabled", "disabled", false),
                ("optional", "optional", false),
                ("mandatory", "mandatory", false),
                ("degraded", "optional", true),
            ] {
                std::fs::write(
                    &global,
                    format!(
                        r#"{{"agent_trace":{{"auto_sync":{auto_sync}}},"auth":{{"mode":"{auth_mode}"}}}}"#
                    ),
                )
                .expect("global config");
                if degraded {
                    std::fs::write(&local, r#"{"auth":{"mode":"bogus"}}"#).expect("local config");
                } else {
                    let _ = std::fs::remove_file(&local);
                }

                let logger = RecordingLogger::default();
                let launches = RefCell::new(0_usize);
                let output = run_post_commit_subcommand_with(
                    &root,
                    None,
                    REMOTE_URL,
                    async |root: &Path| Ok(flow_result(root)),
                    async |_: &Path,
                           result: &PostCommitIntersectionFlowResult,
                           _: Option<AgentTraceVcsType>,
                           _: &str,
                           _: Option<&RecordingLogger>| {
                        Ok(run_flow_capturing(result, None).await.0)
                    },
                    |root| {
                        resolve_hook_runtime_config_with(
                            root,
                            |_| None,
                            |path| std::fs::read_to_string(path).map_err(anyhow::Error::from),
                            Path::exists,
                            || Ok(global.clone()),
                        )
                    },
                    |_| {
                        *launches.borrow_mut() += 1;
                        Ok(())
                    },
                    async |_: &Path| Ok(()),
                    Some(&logger),
                )
                .await
                .unwrap_or_else(|error| panic!("{case} auto_sync={auto_sync}: {error:#}"));

                let allowed = auth_mode != "disabled" && !degraded;
                assert!(
                    output.starts_with("post-commit hook processed intersection: commit="),
                    "{case} auto_sync={auto_sync}: {output}"
                );
                assert_eq!(
                    *launches.borrow(),
                    usize::from(auto_sync && allowed),
                    "{case} auto_sync={auto_sync}"
                );
                let expected_debugs = if auto_sync && !allowed {
                    vec![AUTO_SYNC_SKIPPED_BY_POLICY_EVENT.to_string()]
                } else {
                    Vec::new()
                };
                assert_eq!(
                    logger.debugs(),
                    expected_debugs,
                    "{case} auto_sync={auto_sync}"
                );
            }
        }
    }
}
