mod normalize;
mod parser;
mod path;

use std::path::Path;

use anyhow::{Context, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{DiffTraceInsert, PAYLOAD_TYPE_PATCH};
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_for_hook_runtime_at_state_root, AgentTraceStorageContext,
};

use normalize::normalize_codex_patch;
#[allow(unused_imports)]
use parser::{
    normalize_outer_apply_patch_input, parse_codex_apply_patch, CodexFileOperation, CodexHunk,
    CodexHunkLine, CodexPatch, CodexPatchParseError,
};
use path::resolve_codex_patch_paths;

use super::super::{
    current_unix_time_ms, normalize_codex_model_id, open_agent_trace_db_for_hook_runtime,
    prefixed_diff_trace_session_id, CODEX_TOOL_NAME,
};
use super::CodexHookEvent;

pub(super) async fn handle<L: crate::services::observability::traits::Logger>(
    repository_root: &Path,
    event: &CodexHookEvent,
    logger: Option<&L>,
) -> Result<String> {
    handle_with_state_root(repository_root, event, None, logger).await
}

pub(super) async fn handle_with_state_root<L: crate::services::observability::traits::Logger>(
    repository_root: &Path,
    event: &CodexHookEvent,
    state_root: Option<&Path>,
    logger: Option<&L>,
) -> Result<String> {
    required_session_id(event.session_id.as_deref())?;

    let Some(command) = apply_patch_command_from_event(event) else {
        return Ok(String::new());
    };

    let canonical_command = match normalize_outer_apply_patch_input(command) {
        Ok(command) => command,
        Err(parse_error) => {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.codex.apply_patch.parse_failed",
                    &parse_error.to_string(),
                    &[],
                    event.session_id.as_deref(),
                );
            }
            return Ok(String::new());
        }
    };

    let patch = match parse_codex_apply_patch(&canonical_command) {
        Ok(patch) => patch,
        Err(parse_error) => {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.codex.apply_patch.parse_failed",
                    &parse_error.to_string(),
                    &[],
                    event.session_id.as_deref(),
                );
            }
            return Ok(String::new());
        }
    };

    let mut patch = patch;
    if let Some(event_cwd) = event.cwd.as_deref() {
        if let Err(error) = resolve_codex_patch_paths(repository_root, event_cwd, &mut patch) {
            if let Some(log) = logger {
                log.error(
                    "sce.hooks.codex.apply_patch.path_resolution_failed",
                    &error.to_string(),
                    &[],
                    event.session_id.as_deref(),
                );
            }
            return Ok(String::new());
        }
    } else {
        if let Some(log) = logger {
            log.error(
                "sce.hooks.codex.apply_patch.path_resolution_failed",
                "Codex hook event cwd is missing or malformed.",
                &[],
                event.session_id.as_deref(),
            );
        }
        return Ok(String::new());
    }

    let normalized_patch =
        match normalize_codex_patch(&patch, event.tool_use_id.as_deref().unwrap_or_default()) {
            Ok(normalized_patch) => normalized_patch,
            Err(error) => {
                if let Some(log) = logger {
                    log.error(
                        "sce.hooks.codex.apply_patch.normalize_failed",
                        &error.to_string(),
                        &[],
                        event.session_id.as_deref(),
                    );
                }
                return Ok(String::new());
            }
        };
    if normalized_patch.is_empty() {
        return Ok(String::new());
    }

    let Ok(time_ms) = current_unix_time_ms() else {
        return Ok(String::new());
    };

    let db = match state_root {
        Some(state_root) => resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            state_root,
        )
        .await
        .map(|storage| storage.db)
        .context("Failed to open Agent Trace DB for Codex apply_patch persistence."),
        None => {
            open_agent_trace_db_for_hook_runtime(
                repository_root,
                "Failed to open Agent Trace DB for Codex apply_patch persistence.",
            )
            .await
        }
    }?;

    persist_with(&db, event, &normalized_patch, time_ms).await
}

fn apply_patch_command_from_event(event: &CodexHookEvent) -> Option<&str> {
    event
        .tool_input
        .as_ref()
        .and_then(|value| value.get("command"))
        .and_then(|value| value.as_str())
}

fn required_session_id(value: Option<&str>) -> Result<&str> {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => anyhow::bail!(
            "Invalid Codex apply_patch payload: field 'session_id' must be a trimmed, non-empty string."
        ),
    }
}

async fn persist_with(
    db: &RepositoryAgentTraceDb,
    event: &CodexHookEvent,
    normalized_patch: &str,
    time_ms: i64,
) -> Result<String> {
    let session_id = prefixed_diff_trace_session_id(
        CODEX_TOOL_NAME,
        required_session_id(event.session_id.as_deref())?,
    );
    let model_id = event.model.as_deref().and_then(normalize_codex_model_id);

    db.insert_diff_trace(DiffTraceInsert {
        time_ms,
        session_id: &session_id,
        patch: normalized_patch,
        model_id: model_id.as_deref(),
        tool_name: CODEX_TOOL_NAME,
        tool_version: None,
        payload_type: PAYLOAD_TYPE_PATCH,
    })
    .await
    .context("Failed to persist Codex apply_patch diff-trace row.")?;

    Ok(String::new())
}
