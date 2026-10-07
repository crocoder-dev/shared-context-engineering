use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::services::hooks;
use crate::services::mutation_trace::runtime::resolve_git_dir;

use super::events::BASH_TOOL_NAME;
use super::state;
use super::{
    abandon_payload, classify_tool, claude_scope_close_event_id, claude_scope_start_event_id,
    flush_payload, is_explicit_background_shell, parse_claude_hook_event, pre_tool_use_deny_json,
    scope_boundary_payload, scope_start_payload, AttemptKey, ClaudeHookEvent, ClaudeToolExecution,
    ClaudeToolIdentity, ToolClassification,
};

pub(super) const ACTOR_KIND_CLAUDE_CODE: &str = "claude_code";

pub(super) const FAIL_CLOSED_DENY_REASON: &str =
    "SCE could not establish mutation attribution for this tool execution.";
pub(super) const EXPLICIT_BACKGROUND_SHELL_DENY_REASON: &str =
    "SCE mutation attribution does not yet support detached background shell execution. Run this command in the foreground.";

pub(super) const PRE_TOOL_USE_FAIL_CLOSED_EVENT: &str =
    "sce.hooks.claude_mutation_scope.pre_tool_use_fail_closed";
pub(super) const MODEL_STATE_UNAVAILABLE_EVENT: &str =
    "sce.hooks.claude_mutation_scope.model_state_unavailable";

pub(super) fn log_pre_tool_use_fail_closed<L: crate::services::observability::traits::Logger>(
    logger: Option<&L>,
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

pub(crate) async fn run_claude_mutation_scope_subcommand<
    L: crate::services::observability::traits::Logger,
>(
    logger: Option<&L>,
) -> Result<String> {
    let stdin_payload = hooks::read_hook_stdin()?;
    run_claude_mutation_scope_from_payload(&stdin_payload, logger).await
}

pub(crate) async fn run_claude_mutation_scope_from_payload<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let model_state_resolver = async |repository_root: &Path, session_id: &str, agent_id: &str| {
        let db = hooks::open_agent_trace_db_for_hook_runtime(
            repository_root,
            "Failed to open Agent Trace DB for Claude mutation-scope model resolution.",
        )
        .await?;
        Ok(db
            .claude_model_state_by_session_and_agent(session_id, agent_id)
            .await?
            .map(|state| state.model_id))
    };
    let seam_fn = async |repository_root: &Path, payload: &str, logger: Option<&L>| {
        hooks::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
            .await
    };

    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &model_state_resolver,
        &seam_fn,
    )
    .await
}

#[cfg(any())]
pub(crate) async fn run_claude_mutation_scope_from_payload_at_state_root<
    L: crate::services::observability::traits::Logger,
>(
    state_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let model_state_root = state_root.to_path_buf();
    let seam_state_root = state_root.to_path_buf();
    let model_state_resolver = async move |repository_root: &Path,
                                           session_id: &str,
                                           agent_id: &str|
                -> Result<Option<String>> {
        let db = hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
            repository_root,
            &model_state_root,
            "Failed to open Agent Trace DB for Claude mutation-scope model resolution.",
        )
        .await?;
        Ok(db
            .claude_model_state_by_session_and_agent(session_id, agent_id)
            .await?
            .map(|state| state.model_id))
    };
    let seam_fn = async |repository_root: &Path, payload: &str, logger: Option<&L>| {
        hooks::mutation_scope::run_mutation_scope_from_payload_at_state_root(
            repository_root,
            &seam_state_root,
            payload,
            logger,
        )
        .await
    };

    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &model_state_resolver,
        &seam_fn,
    )
    .await
}

#[cfg(any())]
pub(super) async fn run_claude_mutation_scope_from_payload_with<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
    resolve_git_dir: &impl Fn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    let unavailable_model_state = async |_repository_root: &Path,
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
    .await
}

pub(super) async fn run_claude_mutation_scope_from_payload_with_resolver<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
    resolve_git_dir: &impl Fn(&str) -> Result<PathBuf>,
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &str, &str) -> Result<Option<String>>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    let event = parse_claude_hook_event(stdin_payload)?;
    dispatch_claude_hook_event(event, logger, resolve_git_dir, model_state_resolver, seam).await
}

pub(super) async fn dispatch_claude_hook_event<
    L: crate::services::observability::traits::Logger,
>(
    event: ClaudeHookEvent,
    logger: Option<&L>,
    resolve_git_dir: &impl Fn(&str) -> Result<PathBuf>,
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &str, &str) -> Result<Option<String>>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    match event {
        ClaudeHookEvent::PreToolUse(execution) => Ok(handle_pre_tool_use(
            &execution,
            logger,
            resolve_git_dir,
            model_state_resolver,
            seam,
        )
        .await),
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
            .await
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
            .await
        }
        ClaudeHookEvent::Stop(session) | ClaudeHookEvent::StopFailure(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
            .await
        }
        ClaudeHookEvent::UserPromptSubmit(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
            .await
        }
        ClaudeHookEvent::SubagentStop(agent) => {
            let git_dir = resolve_git_dir(&agent.cwd)?;
            let repository_root = Path::new(&agent.cwd);
            let session_id = agent.session_id.clone();
            let agent_id = agent.agent_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.as_deref() == Some(&agent_id)
            })
            .await
        }
        ClaudeHookEvent::SessionEnd(session) => {
            let git_dir = resolve_git_dir(&session.cwd)?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id
            })
            .await
        }
        ClaudeHookEvent::WorktreeRemove(worktree_remove) => {
            let git_dir = resolve_git_dir(&worktree_remove.worktree_path)?;
            let repository_root = Path::new(&worktree_remove.worktree_path);
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |_attempt| true)
                .await
        }
        ClaudeHookEvent::SessionStart | ClaudeHookEvent::SubagentStart => Ok(String::new()),
    }
}

pub(super) async fn handle_pre_tool_use<L: crate::services::observability::traits::Logger>(
    execution: &ClaudeToolExecution,
    logger: Option<&L>,
    resolve_git_dir: &impl Fn(&str) -> Result<PathBuf>,
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &str, &str) -> Result<Option<String>>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> String {
    let identity = &execution.identity;

    if matches!(
        classify_tool(&identity.tool_name),
        ToolClassification::ReadOnly | ToolClassification::Delegation
    ) {
        return String::new();
    }

    if is_untracked_background_bash(execution) {
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
        apply_recovery_barrier(&git_dir, repository_root, logger, seam).await,
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
    )
    .await
    {
        Ok(()) => String::new(),
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "establish_start", &error);
            pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON)
        }
    }
}

fn is_untracked_background_bash(execution: &ClaudeToolExecution) -> bool {
    execution.run_in_background && execution.identity.tool_name == BASH_TOOL_NAME
}

pub(super) enum BarrierOutcome {
    Proceed,
    Deny,
}

pub(super) async fn apply_recovery_barrier<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
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

    match seam(repository_root, &flush_payload(), logger).await {
        Ok(_) => match state::clear_recovery_pending_if_quiescent(git_dir) {
            Ok(state::ClearRecoveryOutcome::Cleared) => BarrierOutcome::Proceed,
            Ok(state::ClearRecoveryOutcome::StillPending) => BarrierOutcome::Deny,
            Err(error) => {
                log_pre_tool_use_fail_closed(
                    logger,
                    "recovery_barrier.clear_recovery_pending_if_quiescent",
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

pub(super) async fn establish_start<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    identity: &ClaudeToolIdentity,
    logger: Option<&L>,
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &str, &str) -> Result<Option<String>>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    let allocated = state::allocate_attempt(git_dir, &identity.attempt_key(), &identity.tool_name)?;
    let scope_id = &allocated.attempt.scope_id;
    let canonical_session_id =
        hooks::prefixed_diff_trace_session_id(hooks::CLAUDE_TOOL_NAME, &identity.session_id);
    let agent_id = identity.agent_id.as_deref().unwrap_or("");
    let model_id =
        match model_state_resolver(repository_root, &canonical_session_id, agent_id).await {
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

    seam(repository_root, &start_payload, logger).await?;
    state::mark_active(git_dir, scope_id)?;
    Ok(())
}

pub(super) async fn handle_close<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
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
        abandon_attempt(git_dir, repository_root, &attempt, logger, seam).await?;
        return Ok(String::new());
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &claude_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).await.is_ok() {
        state::remove_attempt(git_dir, &attempt.scope_id)?;
    } else {
        abandon_attempt(git_dir, repository_root, &attempt, logger, seam).await?;
    }
    Ok(String::new())
}

pub(super) async fn handle_permission_denied<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
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

    abandon_attempt(git_dir, repository_root, &attempt, logger, seam).await?;
    Ok(String::new())
}

pub(super) async fn cleanup_attempts_matching<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
    predicate: impl Fn(&state::AdapterAttempt) -> bool,
) -> Result<String> {
    let current = state::read_state(git_dir)?;
    let stale: Vec<state::AdapterAttempt> = current
        .attempts
        .into_iter()
        .filter(|attempt| predicate(attempt))
        .collect();

    if stale.is_empty() {
        return Ok(String::new());
    }

    let stale_scope_ids: Vec<String> = stale
        .iter()
        .map(|attempt| attempt.scope_id.clone())
        .collect();
    state::mark_recovery_pending_and_pending_abandon(git_dir, &stale_scope_ids)?;

    let mut first_error: Option<anyhow::Error> = None;
    for attempt in &stale {
        if let Err(error) =
            abandon_marked_attempt(git_dir, repository_root, &attempt.scope_id, logger, seam).await
        {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(String::new()),
    }
}

pub(super) fn attempt_matches_key(attempt: &state::AdapterAttempt, key: &AttemptKey) -> bool {
    attempt.session_id == key.session_id
        && attempt.agent_id == key.agent_id
        && attempt.tool_use_id == key.tool_use_id
}

pub(super) async fn abandon_attempt<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    attempt: &state::AdapterAttempt,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    state::mark_recovery_pending_and_pending_abandon(
        git_dir,
        std::slice::from_ref(&attempt.scope_id),
    )?;

    abandon_marked_attempt(git_dir, repository_root, &attempt.scope_id, logger, seam).await
}

async fn abandon_marked_attempt<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    scope_id: &str,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    seam(repository_root, &abandon_payload(scope_id), logger).await?;
    state::remove_attempt(git_dir, scope_id)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepairOutcome {
    Repaired,
    NoOp,
}

pub(crate) async fn repair_blocked<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<RepairOutcome> {
    let Some(attempts) = state::reprove_pending_abandon(git_dir)? else {
        return Ok(RepairOutcome::NoOp);
    };

    let mut any_failed = false;
    for attempt in &attempts {
        if abandon_marked_attempt(git_dir, repository_root, &attempt.scope_id, logger, seam)
            .await
            .is_err()
        {
            any_failed = true;
        }
    }

    let cleared = matches!(
        state::clear_recovery_pending_if_quiescent(git_dir)?,
        state::ClearRecoveryOutcome::Cleared
    );

    if any_failed || !cleared {
        Ok(RepairOutcome::NoOp)
    } else {
        Ok(RepairOutcome::Repaired)
    }
}
