use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::services::hooks;
use crate::services::mutation_trace::runtime::resolve_git_dir;

use super::events::{
    opencode_scope_close_event_id, opencode_scope_provenance, opencode_scope_start_event_id,
    parse_opencode_hook_event, AttemptKey, OpenCodeHookEvent, OpenCodeScopeProvenance,
    ToolClassification, OPENCODE_TRACKED_TOOL_BASH,
};
use super::payload::{abandon_payload, flush_payload, scope_boundary_payload, scope_start_payload};
use super::state::{self, AdmitDecision, RecoveryFlushCompletion};

pub(crate) async fn run_opencode_mutation_scope_subcommand<
    L: crate::services::observability::traits::Logger,
>(
    logger: Option<&L>,
) -> Result<String> {
    let stdin_payload = hooks::read_hook_stdin()?;
    run_opencode_mutation_scope_from_payload(&stdin_payload, logger).await
}

pub(crate) async fn run_opencode_mutation_scope_from_payload<
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

    run_opencode_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
    .await
}

pub(super) const FAIL_CLOSED_MESSAGE: &str =
    "SCE could not establish OpenCode mutation attribution for this tool execution.";

pub(super) const FAIL_CLOSED_EVENT: &str = "sce.hooks.opencode_mutation_scope.start_fail_closed";

pub(super) fn log_fail_closed<L: crate::services::observability::traits::Logger>(
    logger: Option<&L>,
    context: &str,
    error: &anyhow::Error,
) {
    if let Some(log) = logger {
        log.warn(
            FAIL_CLOSED_EVENT,
            &error.to_string(),
            &[("context", context)],
            None,
        );
    }
}

pub(super) async fn run_opencode_mutation_scope_from_payload_with_seams<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    let event = parse_opencode_hook_event(stdin_payload)?;
    dispatch_opencode_hook_event(event, logger, resolve_git_dir, seam).await
}

pub(super) async fn dispatch_opencode_hook_event<
    L: crate::services::observability::traits::Logger,
>(
    event: OpenCodeHookEvent,
    logger: Option<&L>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    match event {
        OpenCodeHookEvent::ToolExecuteBefore(execution) => {
            match execution.identity.classification() {
                ToolClassification::TrackedMutation => {
                    if execution.identity.tool_name == OPENCODE_TRACKED_TOOL_BASH {
                        return Ok(String::new());
                    }
                    let provenance = opencode_scope_provenance(
                        &execution.identity.session_id,
                        execution.model.as_deref(),
                    );
                    establish_tracked_start(
                        &execution.identity.cwd,
                        &execution.identity.attempt_key(),
                        &execution.identity.tool_name,
                        &provenance,
                        logger,
                        resolve_git_dir,
                        seam,
                    )
                    .await
                }
                ToolClassification::Delegation | ToolClassification::Untracked => Ok(String::new()),
            }
        }
        OpenCodeHookEvent::ShellEnv(shell) => {
            let provenance = opencode_scope_provenance(&shell.session_id, shell.model.as_deref());
            establish_tracked_start(
                &shell.cwd,
                &shell.attempt_key(),
                OPENCODE_TRACKED_TOOL_BASH,
                &provenance,
                logger,
                resolve_git_dir,
                seam,
            )
            .await
        }
        OpenCodeHookEvent::ToolExecuteAfter(identity) => {
            if !matches!(
                identity.classification(),
                ToolClassification::TrackedMutation
            ) {
                return Ok(String::new());
            }
            let git_dir = resolve_git_dir(&identity.cwd).await?;
            let repository_root = Path::new(&identity.cwd);
            let key = identity.attempt_key();
            with_boundary_lock(&git_dir, async || {
                state::normalize_recovery_after_boundary_lock_acquired(&git_dir).await?;
                handle_close(&git_dir, repository_root, &key, logger, seam).await
            })
            .await
        }
        OpenCodeHookEvent::ToolError(call) => {
            if !matches!(call.classification(), ToolClassification::TrackedMutation) {
                return Ok(String::new());
            }
            let git_dir = resolve_git_dir(&call.cwd).await?;
            let repository_root = Path::new(&call.cwd);
            let key = call.attempt_key();
            with_boundary_lock(&git_dir, async || {
                state::normalize_recovery_after_boundary_lock_acquired(&git_dir).await?;
                abandon_and_consume(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == key.session_id && attempt.call_id == key.call_id
                })
                .await
            })
            .await
        }
        OpenCodeHookEvent::SessionIdle(_)
        | OpenCodeHookEvent::SessionError(_)
        | OpenCodeHookEvent::SessionDeleted(_)
        | OpenCodeHookEvent::ServerDisposed(_) => Ok(String::new()),
    }
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
    with_boundary_lock(git_dir, async || {
        state::normalize_recovery_after_boundary_lock_acquired(git_dir).await?;

        let Some(generation) =
            state::reprove_dead_owner_pending_start_and_begin_repair(git_dir).await?
        else {
            return Ok(RepairOutcome::NoOp);
        };

        match resolve_recovery(git_dir, repository_root, generation, logger, seam).await? {
            RecoveryResolution::Cleared => Ok(RepairOutcome::Repaired),
            RecoveryResolution::Unresolved => Ok(RepairOutcome::NoOp),
        }
    })
    .await
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

pub(super) enum Admission {
    Admitted(state::AllocatedAttempt),
    Denied,
}

pub(super) enum StartOutcome {
    Established,
    Denied,
}

pub(super) async fn establish_tracked_start<L: crate::services::observability::traits::Logger>(
    cwd: &str,
    key: &AttemptKey,
    tool_name: &str,
    provenance: &OpenCodeScopeProvenance,
    logger: Option<&L>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    let git_dir = match resolve_git_dir(cwd).await {
        Ok(git_dir) => git_dir,
        Err(error) => {
            log_fail_closed(logger, "resolve_git_dir", &error);
            return Err(error.context(FAIL_CLOSED_MESSAGE));
        }
    };
    let repository_root = Path::new(cwd);

    let outcome = with_boundary_lock(&git_dir, async || {
        state::normalize_recovery_after_boundary_lock_acquired(&git_dir).await?;

        match admit_or_recover(&git_dir, repository_root, key, tool_name, logger, seam).await? {
            Admission::Admitted(allocated) => {
                establish_start(
                    &git_dir,
                    repository_root,
                    &allocated,
                    provenance,
                    logger,
                    seam,
                )
                .await?;
                Ok(StartOutcome::Established)
            }
            Admission::Denied => Ok(StartOutcome::Denied),
        }
    })
    .await;

    match outcome {
        Ok(StartOutcome::Established) => Ok(String::new()),
        Ok(StartOutcome::Denied) => bail!(FAIL_CLOSED_MESSAGE),
        Err(error) => {
            log_fail_closed(logger, "establish_tracked_start", &error);
            Err(error.context(FAIL_CLOSED_MESSAGE))
        }
    }
}

pub(super) async fn admit_or_recover<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    tool_name: &str,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, tool_name).await? {
        AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        AdmitDecision::RecoveryBlocked
        | AdmitDecision::UncertainAttemptBlocked
        | AdmitDecision::TerminalAttemptBlocked => Ok(Admission::Denied),
        AdmitDecision::FlushClaimed { generation } => {
            match resolve_recovery(git_dir, repository_root, generation, logger, seam).await? {
                RecoveryResolution::Cleared => readmit_after_flush(git_dir, key, tool_name).await,
                RecoveryResolution::Unresolved => Ok(Admission::Denied),
            }
        }
    }
}

pub(super) async fn readmit_after_flush(
    git_dir: &Path,
    key: &AttemptKey,
    tool_name: &str,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, tool_name).await? {
        AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        AdmitDecision::FlushClaimed { generation } => {
            state::relinquish_recovery_flush(git_dir, generation).await?;
            Ok(Admission::Denied)
        }
        AdmitDecision::RecoveryBlocked
        | AdmitDecision::UncertainAttemptBlocked
        | AdmitDecision::TerminalAttemptBlocked => Ok(Admission::Denied),
    }
}

pub(super) async fn establish_start<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    provenance: &OpenCodeScopeProvenance,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    let scope_id = &allocated.attempt.scope_id;

    if allocated.reused && allocated.attempt.phase == state::AttemptPhase::Active {
        return Ok(());
    }

    let start_payload = scope_start_payload(
        scope_id,
        &opencode_scope_start_event_id(scope_id),
        provenance,
    );

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
    let current = state::read_state(git_dir).await?;
    let Some(attempt) = current
        .attempts
        .iter()
        .find(|attempt| attempt.session_id == key.session_id && attempt.call_id == key.call_id)
        .cloned()
    else {
        return Ok(String::new());
    };

    let doomed_scope_id = attempt.scope_id.clone();

    if matches!(
        attempt.phase,
        state::AttemptPhase::PendingStart | state::AttemptPhase::PendingAbandon
    ) {
        return abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
            candidate.scope_id == doomed_scope_id
        })
        .await;
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &opencode_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).await.is_ok() {
        state::remove_attempt(git_dir, &attempt.scope_id).await?;
        Ok(String::new())
    } else {
        abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
            candidate.scope_id == doomed_scope_id
        })
        .await
    }
}

pub(super) enum RecoveryResolution {
    Cleared,
    Unresolved,
}

pub(super) async fn abandon_and_consume<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
    doomed: impl Fn(&state::AdapterAttempt) -> bool,
) -> Result<String> {
    let doomed_scope_ids: Vec<String> = state::read_state(git_dir)
        .await?
        .attempts
        .into_iter()
        .filter(|attempt| doomed(attempt))
        .map(|attempt| attempt.scope_id)
        .collect();
    if doomed_scope_ids.is_empty() {
        return Ok(String::new());
    }

    let generation = state::begin_terminal_cleanup(git_dir, &doomed_scope_ids).await?;
    resolve_recovery(git_dir, repository_root, generation, logger, seam).await?;
    Ok(String::new())
}

pub(super) async fn resolve_recovery<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    generation: u64,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<RecoveryResolution> {
    let pending_abandon: Vec<state::AdapterAttempt> = state::read_state(git_dir)
        .await?
        .attempts
        .into_iter()
        .filter(|attempt| attempt.phase == state::AttemptPhase::PendingAbandon)
        .collect();

    if let Err(error) = seam(repository_root, &flush_payload(), logger).await {
        log_fail_closed(logger, "recovery_ambiguity_flush", &error);
        state::relinquish_recovery_flush(git_dir, generation).await?;
        return Ok(RecoveryResolution::Unresolved);
    }

    for attempt in &pending_abandon {
        if let Err(error) = seam(repository_root, &abandon_payload(&attempt.scope_id), logger).await
        {
            log_fail_closed(logger, "recovery_abandon", &error);
            state::relinquish_recovery_flush(git_dir, generation).await?;
            return Ok(RecoveryResolution::Unresolved);
        }
        state::remove_attempt(git_dir, &attempt.scope_id).await?;
    }

    if let Err(error) = seam(repository_root, &flush_payload(), logger).await {
        log_fail_closed(logger, "recovery_rebaseline_flush", &error);
        state::relinquish_recovery_flush(git_dir, generation).await?;
        return Ok(RecoveryResolution::Unresolved);
    }

    match state::complete_recovery_flush(git_dir, generation).await? {
        RecoveryFlushCompletion::Cleared => Ok(RecoveryResolution::Cleared),
        RecoveryFlushCompletion::Superseded => Ok(RecoveryResolution::Unresolved),
    }
}
