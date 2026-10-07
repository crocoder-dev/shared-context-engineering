use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::services::hooks;
use crate::services::hooks::codex::bash_policy::{
    bash_command_from_tool_input, evaluate_codex_bash_policy, CodexBashPolicyDecision,
};
use crate::services::hooks::{
    normalize_codex_model_id, prefixed_diff_trace_session_id, CODEX_TOOL_NAME,
};
use crate::services::mutation_trace::runtime::resolve_git_dir;

use super::events::CODEX_TRACKED_TOOL_BASH;
use super::state;
use super::{
    abandon_payload, classify_tool, codex_scope_close_event_id, codex_scope_start_event_id,
    flush_payload, parse_codex_hook_event, pre_tool_use_deny_json, scope_boundary_payload,
    scope_start_payload, AttemptKey, CodexHookEvent, CodexToolExecution, ToolClassification,
};

pub(super) const ACTOR_KIND_CODEX: &str = "codex";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexScopeProvenance {
    pub session_id: String,
    pub model_id: Option<String>,
}

pub(super) fn codex_scope_provenance(execution: &CodexToolExecution) -> CodexScopeProvenance {
    CodexScopeProvenance {
        session_id: prefixed_diff_trace_session_id(CODEX_TOOL_NAME, &execution.identity.session_id),
        model_id: execution
            .model
            .as_deref()
            .and_then(normalize_codex_model_id),
    }
}

pub(super) const FAIL_CLOSED_DENY_REASON: &str =
    "SCE could not establish mutation attribution for this tool execution.";

pub(super) const PRE_TOOL_USE_FAIL_CLOSED_EVENT: &str =
    "sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed";

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

pub(crate) async fn run_codex_mutation_scope_subcommand<
    L: crate::services::observability::traits::Logger,
>(
    logger: Option<&L>,
) -> Result<String> {
    let stdin_payload = hooks::read_hook_stdin()?;
    run_codex_mutation_scope_from_payload(&stdin_payload, logger).await
}

pub(crate) async fn run_codex_mutation_scope_from_payload<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
) -> Result<String> {
    let resolve_git_dir_fn = async |cwd: &str| resolve_git_dir(Path::new(cwd)).await;
    let seam_fn = async |repository_root: &Path, payload: &str, logger: Option<&L>| {
        hooks::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
            .await
    };
    let bash_policy_fn = |repository_root: &Path, command: &str| {
        evaluate_codex_bash_policy(repository_root, command)
    };

    run_codex_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
        &bash_policy_fn,
    )
    .await
}

pub(super) async fn run_codex_mutation_scope_from_payload_with_seams<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
    evaluate_bash_policy: &impl Fn(&Path, &str) -> Result<CodexBashPolicyDecision>,
) -> Result<String> {
    let event = parse_codex_hook_event(stdin_payload)?;
    dispatch_codex_hook_event(event, logger, resolve_git_dir, seam, evaluate_bash_policy).await
}

pub(super) async fn dispatch_codex_hook_event<L: crate::services::observability::traits::Logger>(
    event: CodexHookEvent,
    logger: Option<&L>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
    evaluate_bash_policy: &impl Fn(&Path, &str) -> Result<CodexBashPolicyDecision>,
) -> Result<String> {
    match event {
        CodexHookEvent::PreToolUse(execution) => Ok(handle_pre_tool_use(
            &execution,
            logger,
            resolve_git_dir,
            seam,
            evaluate_bash_policy,
        )
        .await),
        CodexHookEvent::PostToolUse(identity) => {
            if !matches!(
                classify_tool(&identity.tool_name),
                ToolClassification::TrackedMutation
            ) {
                return Ok(String::new());
            }

            let git_dir = resolve_git_dir(&identity.cwd).await?;
            let repository_root = Path::new(&identity.cwd);
            with_boundary_lock(&git_dir, async || {
                handle_close(
                    &git_dir,
                    repository_root,
                    &identity.attempt_key(),
                    logger,
                    seam,
                )
                .await
            })
            .await
        }
        CodexHookEvent::Stop(turn) => {
            let git_dir = resolve_git_dir(&turn.cwd).await?;
            let repository_root = Path::new(&turn.cwd);
            let session_id = turn.session_id.clone();
            with_boundary_lock(&git_dir, async || {
                cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == session_id && attempt.agent_id.is_none()
                })
                .await
            })
            .await
        }
        CodexHookEvent::Interrupt(turn) => {
            let git_dir = resolve_git_dir(&turn.cwd).await?;
            let repository_root = Path::new(&turn.cwd);
            let session_id = turn.session_id.clone();
            with_boundary_lock(&git_dir, async || {
                cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == session_id
                })
                .await
            })
            .await
        }
        CodexHookEvent::SubagentStop(agent) => {
            let git_dir = resolve_git_dir(&agent.cwd).await?;
            let repository_root = Path::new(&agent.cwd);
            let session_id = agent.session_id.clone();
            let agent_id = agent.agent_id.clone();
            with_boundary_lock(&git_dir, async || {
                cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == session_id
                        && attempt.agent_id.as_deref() == Some(&agent_id)
                })
                .await
            })
            .await
        }
        CodexHookEvent::SessionEnd(session) => {
            let git_dir = resolve_git_dir(&session.cwd).await?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            with_boundary_lock(&git_dir, async || {
                cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == session_id
                })
                .await
            })
            .await
        }
    }
}

pub(super) async fn with_boundary_lock<T>(
    git_dir: &Path,
    operation: impl std::ops::AsyncFnOnce() -> Result<T>,
) -> Result<T> {
    let _boundary = state::BOUNDARY_LOCK
        .acquire_async(&state::adapter_state_dir(git_dir))
        .await?;
    operation().await
}

pub(super) async fn handle_pre_tool_use<L: crate::services::observability::traits::Logger>(
    execution: &CodexToolExecution,
    logger: Option<&L>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
    evaluate_bash_policy: &impl Fn(&Path, &str) -> Result<CodexBashPolicyDecision>,
) -> String {
    let identity = &execution.identity;

    if !matches!(
        classify_tool(&identity.tool_name),
        ToolClassification::TrackedMutation
    ) {
        return String::new();
    }

    let repository_root = Path::new(&identity.cwd);

    if identity.tool_name == CODEX_TRACKED_TOOL_BASH {
        match codex_bash_policy_preflight(repository_root, execution, evaluate_bash_policy) {
            BashPolicyPreflight::Allowed => {}
            BashPolicyPreflight::Blocked(response) => return response,
            BashPolicyPreflight::EvaluationFailed(error) => {
                log_pre_tool_use_fail_closed(logger, "bash_policy_preflight", &error);
                return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
            }
        }
    }

    let git_dir = match resolve_git_dir(&identity.cwd).await {
        Ok(git_dir) => git_dir,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "resolve_git_dir", &error);
            return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
        }
    };

    let key = identity.attempt_key();
    let turn_id = identity.turn_id.as_str();
    let provenance = codex_scope_provenance(execution);
    let outcome = with_boundary_lock(&git_dir, async || {
        state::normalize_recovery_after_boundary_lock_acquired(&git_dir).await?;

        sweep_stale_lane_predecessors(&git_dir, repository_root, &key, turn_id, logger, seam)
            .await?;

        match admit_or_recover(
            &git_dir,
            repository_root,
            &key,
            turn_id,
            &identity.tool_name,
            logger,
            seam,
        )
        .await?
        {
            Admission::Admitted(allocated) => {
                establish_start(
                    &git_dir,
                    repository_root,
                    &allocated,
                    &provenance,
                    logger,
                    seam,
                )
                .await?;
                Ok(PreToolUseOutcome::Continue)
            }
            Admission::Denied => Ok(PreToolUseOutcome::Deny),
        }
    })
    .await;

    match outcome {
        Ok(PreToolUseOutcome::Continue) => String::new(),
        Ok(PreToolUseOutcome::Deny) => pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON),
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "codex_mutation_scope_pre_tool_use", &error);
            pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON)
        }
    }
}

pub(super) enum PreToolUseOutcome {
    Continue,
    Deny,
}

pub(super) enum BashPolicyPreflight {
    Allowed,
    Blocked(String),
    EvaluationFailed(anyhow::Error),
}

pub(super) fn codex_bash_policy_preflight(
    repository_root: &Path,
    execution: &CodexToolExecution,
    evaluate_bash_policy: &impl Fn(&Path, &str) -> Result<CodexBashPolicyDecision>,
) -> BashPolicyPreflight {
    let command = match bash_command_from_tool_input(execution.tool_input.as_ref()) {
        Ok(command) => command,
        Err(error) => return BashPolicyPreflight::EvaluationFailed(error),
    };

    match evaluate_bash_policy(repository_root, command) {
        Ok(CodexBashPolicyDecision::Allowed) => BashPolicyPreflight::Allowed,
        Ok(CodexBashPolicyDecision::Blocked(response)) => BashPolicyPreflight::Blocked(response),
        Err(error) => BashPolicyPreflight::EvaluationFailed(error),
    }
}

pub(super) enum Admission {
    Admitted(state::AllocatedAttempt),
    Denied,
}

pub(super) async fn sweep_stale_lane_predecessors<
    L: crate::services::observability::traits::Logger,
>(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    turn_id: &str,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    loop {
        let current = state::read_state(git_dir)?;
        let Some(stale) = current
            .attempts
            .iter()
            .find(|attempt| {
                attempt.in_builtin_lane(&key.session_id, turn_id)
                    && !attempt_matches_key(attempt, key)
            })
            .cloned()
        else {
            return Ok(());
        };
        abandon_attempt(git_dir, repository_root, &stale, logger, seam).await?;
    }
}

pub(super) async fn admit_or_recover<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    turn_id: &str,
    tool_name: &str,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, turn_id, tool_name).await? {
        state::AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        state::AdmitDecision::RecoveryBlocked
        | state::AdmitDecision::UncertainAttemptBlocked
        | state::AdmitDecision::StalePredecessorBlocked => Ok(Admission::Denied),
        state::AdmitDecision::FlushClaimed { generation } => {
            match seam(repository_root, &flush_payload(), logger).await {
                Ok(_) => match state::complete_recovery_flush(git_dir, generation).await? {
                    state::RecoveryFlushCompletion::Cleared => {
                        readmit_after_flush(git_dir, key, turn_id, tool_name).await
                    }
                    state::RecoveryFlushCompletion::Superseded => Ok(Admission::Denied),
                },
                Err(error) => {
                    log_pre_tool_use_fail_closed(logger, "recovery_flush", &error);
                    state::relinquish_recovery_flush(git_dir, generation).await?;
                    Ok(Admission::Denied)
                }
            }
        }
    }
}

pub(super) async fn readmit_after_flush(
    git_dir: &Path,
    key: &AttemptKey,
    turn_id: &str,
    tool_name: &str,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, turn_id, tool_name).await? {
        state::AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        state::AdmitDecision::FlushClaimed { generation } => {
            state::relinquish_recovery_flush(git_dir, generation).await?;
            Ok(Admission::Denied)
        }
        state::AdmitDecision::RecoveryBlocked
        | state::AdmitDecision::UncertainAttemptBlocked
        | state::AdmitDecision::StalePredecessorBlocked => Ok(Admission::Denied),
    }
}

pub(super) async fn establish_start<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    provenance: &CodexScopeProvenance,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    let scope_id = &allocated.attempt.scope_id;

    if allocated.reused && allocated.attempt.phase == state::AttemptPhase::Active {
        return Ok(());
    }

    let start_payload =
        scope_start_payload(scope_id, &codex_scope_start_event_id(scope_id), provenance);

    seam(repository_root, &start_payload, logger).await?;
    state::mark_active(git_dir, scope_id).await?;
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
        &codex_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).await.is_ok() {
        state::remove_attempt(git_dir, &attempt.scope_id).await?;
    } else {
        abandon_attempt(git_dir, repository_root, &attempt, logger, seam).await?;
    }
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

    for attempt in &stale {
        abandon_attempt(git_dir, repository_root, attempt, logger, seam).await?;
    }

    Ok(String::new())
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
    state::arm_recovery(git_dir).await?;

    seam(repository_root, &abandon_payload(&attempt.scope_id), logger).await?;
    state::remove_attempt(git_dir, &attempt.scope_id).await?;
    Ok(())
}
