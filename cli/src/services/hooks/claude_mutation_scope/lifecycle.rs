use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::services::hooks;
use crate::services::mutation_trace::runtime::resolve_git_dir;
use crate::services::observability::traits::Logger;

use super::state;
use super::{
    abandon_payload, classify_tool, claude_scope_close_event_id, claude_scope_start_event_id,
    flush_payload, is_explicit_background_shell, parse_claude_hook_event, pre_tool_use_deny_json,
    scope_boundary_payload, scope_start_payload, AttemptKey, ClaudeHookEvent, ClaudeToolExecution,
    ClaudeToolIdentity, ToolClassification,
};

pub(super) type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

pub(super) type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

pub(super) const ACTOR_KIND_CLAUDE_CODE: &str = "claude_code";

pub(super) const FAIL_CLOSED_DENY_REASON: &str =
    "SCE could not establish mutation attribution for this tool execution.";
pub(super) const EXPLICIT_BACKGROUND_SHELL_DENY_REASON: &str =
    "SCE mutation attribution does not yet support detached background shell execution. Run this command in the foreground.";

pub(super) const PRE_TOOL_USE_FAIL_CLOSED_EVENT: &str =
    "sce.hooks.claude_mutation_scope.pre_tool_use_fail_closed";
pub(super) const MODEL_STATE_UNAVAILABLE_EVENT: &str =
    "sce.hooks.claude_mutation_scope.model_state_unavailable";

pub(super) type ClaudeModelStateResolver<'a> =
    &'a dyn Fn(&Path, &str, &str) -> Result<Option<String>>;

pub(super) fn log_pre_tool_use_fail_closed(
    logger: Option<&dyn Logger>,
    context: &str,
    error: &anyhow::Error,
) {
    if let Some(log) = logger {
        log.warn(
            PRE_TOOL_USE_FAIL_CLOSED_EVENT,
            &error.to_string(),
            &[("context", context)],
            None,
        );
    }
}

pub(crate) fn run_claude_mutation_scope_subcommand(logger: Option<&dyn Logger>) -> Result<String> {
    let stdin_payload = hooks::read_hook_stdin()?;
    run_claude_mutation_scope_from_payload(&stdin_payload, logger)
}

pub(crate) fn run_claude_mutation_scope_from_payload(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let model_state_resolver =
        |repository_root: &Path, session_id: &str, agent_id: &str| -> Result<Option<String>> {
            let db = hooks::open_agent_trace_db_for_hook_runtime(
                repository_root,
                "Failed to open Agent Trace DB for Claude mutation-scope model resolution.",
            )?;
            Ok(db
                .claude_model_state_by_session_and_agent(session_id, agent_id)?
                .map(|state| state.model_id))
        };
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        hooks::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
    };

    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &model_state_resolver,
        &seam_fn,
    )
}

#[cfg(test)]
pub(crate) fn run_claude_mutation_scope_from_payload_at_state_root(
    state_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let model_state_root = state_root.to_path_buf();
    let seam_state_root = state_root.to_path_buf();
    let model_state_resolver =
        move |repository_root: &Path, session_id: &str, agent_id: &str| -> Result<Option<String>> {
            let db = hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                repository_root,
                &model_state_root,
                "Failed to open Agent Trace DB for Claude mutation-scope model resolution.",
            )?;
            Ok(db
                .claude_model_state_by_session_and_agent(session_id, agent_id)?
                .map(|state| state.model_id))
        };
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        hooks::mutation_scope::run_mutation_scope_from_payload_at_state_root(
            repository_root,
            &seam_state_root,
            payload,
            logger,
        )
    };

    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &model_state_resolver,
        &seam_fn,
    )
}

#[cfg(test)]
pub(super) fn run_claude_mutation_scope_from_payload_with(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let unavailable_model_state = |_repository_root: &Path,
                                   _session_id: &str,
                                   _agent_id: &str|
     -> Result<Option<String>> { Ok(None) };
    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        resolve_git_dir,
        &unavailable_model_state,
        seam,
    )
}

pub(super) fn run_claude_mutation_scope_from_payload_with_resolver(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    model_state_resolver: ClaudeModelStateResolver,
    seam: IngressSeam,
) -> Result<String> {
    let event = parse_claude_hook_event(stdin_payload)?;
    dispatch_claude_hook_event(event, logger, resolve_git_dir, model_state_resolver, seam)
}

pub(super) fn dispatch_claude_hook_event(
    event: ClaudeHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    model_state_resolver: ClaudeModelStateResolver,
    seam: IngressSeam,
) -> Result<String> {
    match event {
        ClaudeHookEvent::PreToolUse(execution) => Ok(handle_pre_tool_use(
            &execution,
            logger,
            resolve_git_dir,
            model_state_resolver,
            seam,
        )),
        ClaudeHookEvent::PostToolUse(identity) | ClaudeHookEvent::PostToolUseFailure(identity) => {
            let git_dir = resolve_git_dir(&identity.cwd)?;
            let repository_root = Path::new(&identity.cwd);
            handle_close(
                &git_dir,
                repository_root,
                &identity.attempt_key(),
                logger,
                seam,
            )
        }
        ClaudeHookEvent::PermissionDenied(identity) => {
            let git_dir = resolve_git_dir(&identity.cwd)?;
            let repository_root = Path::new(&identity.cwd);
            handle_permission_denied(
                &git_dir,
                repository_root,
                &identity.attempt_key(),
                logger,
                seam,
            )
        }
        ClaudeHookEvent::Stop(session) | ClaudeHookEvent::StopFailure(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
        }
        ClaudeHookEvent::UserPromptSubmit(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
        }
        ClaudeHookEvent::SubagentStop(agent) => {
            let git_dir = resolve_git_dir(&agent.cwd)?;
            let repository_root = Path::new(&agent.cwd);
            let session_id = agent.session_id.clone();
            let agent_id = agent.agent_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.as_deref() == Some(&agent_id)
            })
        }
        ClaudeHookEvent::SessionEnd(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id
            })
        }
        ClaudeHookEvent::WorktreeRemove(worktree_remove) => {
            let git_dir = resolve_git_dir(&worktree_remove.worktree_path)?;
            let repository_root = Path::new(&worktree_remove.worktree_path);
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |_attempt| true)
        }
        ClaudeHookEvent::SessionStart | ClaudeHookEvent::SubagentStart => Ok(String::new()),
    }
}

pub(super) fn handle_pre_tool_use(
    execution: &ClaudeToolExecution,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    model_state_resolver: ClaudeModelStateResolver,
    seam: IngressSeam,
) -> String {
    let identity = &execution.identity;

    if matches!(
        classify_tool(&identity.tool_name),
        ToolClassification::ReadOnly | ToolClassification::Delegation
    ) {
        return String::new();
    }

    if is_explicit_background_shell(&identity.tool_name, execution.run_in_background) {
        return pre_tool_use_deny_json(EXPLICIT_BACKGROUND_SHELL_DENY_REASON);
    }

    let repository_root = Path::new(&identity.cwd);
    let git_dir = match resolve_git_dir(&identity.cwd) {
        Ok(git_dir) => git_dir,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "resolve_git_dir", &error);
            return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
        }
    };

    if matches!(
        apply_recovery_barrier(&git_dir, repository_root, logger, seam),
        BarrierOutcome::Deny
    ) {
        return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
    }

    match establish_start(
        &git_dir,
        repository_root,
        identity,
        logger,
        model_state_resolver,
        seam,
    ) {
        Ok(()) => String::new(),
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "establish_start", &error);
            pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON)
        }
    }
}

pub(super) enum BarrierOutcome {
    Proceed,
    Deny,
}

pub(super) fn apply_recovery_barrier(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> BarrierOutcome {
    let state = match state::read_state(git_dir) {
        Ok(state) => state,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "recovery_barrier.read_state", &error);
            return BarrierOutcome::Deny;
        }
    };

    if !state.recovery_pending {
        return BarrierOutcome::Proceed;
    }

    if !state.attempts.is_empty() {
        return BarrierOutcome::Deny;
    }

    match seam(repository_root, &flush_payload(), logger) {
        Ok(_) => match state::clear_recovery_pending(git_dir) {
            Ok(()) => BarrierOutcome::Proceed,
            Err(error) => {
                log_pre_tool_use_fail_closed(
                    logger,
                    "recovery_barrier.clear_recovery_pending",
                    &error,
                );
                BarrierOutcome::Deny
            }
        },
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "recovery_barrier.flush", &error);
            BarrierOutcome::Deny
        }
    }
}

pub(super) fn establish_start(
    git_dir: &Path,
    repository_root: &Path,
    identity: &ClaudeToolIdentity,
    logger: Option<&dyn Logger>,
    model_state_resolver: ClaudeModelStateResolver,
    seam: IngressSeam,
) -> Result<()> {
    let allocated = state::allocate_attempt(git_dir, &identity.attempt_key(), &identity.tool_name)?;
    let scope_id = &allocated.attempt.scope_id;
    let canonical_session_id =
        hooks::prefixed_diff_trace_session_id(hooks::CLAUDE_TOOL_NAME, &identity.session_id);
    let agent_id = identity.agent_id.as_deref().unwrap_or("");
    let model_id = match model_state_resolver(repository_root, &canonical_session_id, agent_id) {
        Ok(model_id) => model_id.and_then(|model| hooks::normalize_claude_model_id(&model)),
        Err(error) => {
            if let Some(log) = logger {
                log.warn(
                    MODEL_STATE_UNAVAILABLE_EVENT,
                    &error.to_string(),
                    &[("agent_id", agent_id)],
                    Some(&canonical_session_id),
                );
            }
            None
        }
    };
    let start_payload = scope_start_payload(
        scope_id,
        &claude_scope_start_event_id(scope_id),
        &canonical_session_id,
        model_id.as_deref(),
    );

    seam(repository_root, &start_payload, logger)?;
    state::mark_active(git_dir, scope_id)?;
    Ok(())
}

pub(super) fn handle_close(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<String> {
    let current = state::read_state(git_dir)?;
    let Some(attempt) = current
        .attempts
        .iter()
        .find(|attempt| attempt_matches_key(attempt, key))
        .cloned()
    else {
        return Ok(String::new());
    };

    if attempt.phase == state::AttemptPhase::PendingStart {
        abandon_attempt(git_dir, repository_root, &attempt, logger, seam)?;
        return Ok(String::new());
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &claude_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).is_ok() {
        state::remove_attempt(git_dir, &attempt.scope_id)?;
    } else {
        abandon_attempt(git_dir, repository_root, &attempt, logger, seam)?;
    }
    Ok(String::new())
}

pub(super) fn handle_permission_denied(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<String> {
    let current = state::read_state(git_dir)?;
    let Some(attempt) = current
        .attempts
        .iter()
        .find(|attempt| attempt_matches_key(attempt, key))
        .cloned()
    else {
        return Ok(String::new());
    };

    abandon_attempt(git_dir, repository_root, &attempt, logger, seam)?;
    Ok(String::new())
}

pub(super) fn cleanup_attempts_matching(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
    predicate: impl Fn(&state::AdapterAttempt) -> bool,
) -> Result<String> {
    let current = state::read_state(git_dir)?;
    let stale: Vec<state::AdapterAttempt> = current
        .attempts
        .into_iter()
        .filter(|attempt| predicate(attempt))
        .collect();

    for attempt in &stale {
        abandon_attempt(git_dir, repository_root, attempt, logger, seam)?;
    }

    Ok(String::new())
}

pub(super) fn attempt_matches_key(attempt: &state::AdapterAttempt, key: &AttemptKey) -> bool {
    attempt.session_id == key.session_id
        && attempt.agent_id == key.agent_id
        && attempt.tool_use_id == key.tool_use_id
}

pub(super) fn abandon_attempt(
    git_dir: &Path,
    repository_root: &Path,
    attempt: &state::AdapterAttempt,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    state::mark_recovery_pending(git_dir)?;

    seam(repository_root, &abandon_payload(&attempt.scope_id), logger)?;
    state::remove_attempt(git_dir, &attempt.scope_id)?;
    Ok(())
}
