use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};

use crate::services::hooks;
use crate::services::mutation_trace::runtime::resolve_git_dir;
use crate::services::observability::traits::Logger;

use super::boundary_lock::{AdapterBoundaryLock, DEFAULT_BOUNDARY_LOCK_TIMEOUT};
use super::events::{
    opencode_scope_close_event_id, opencode_scope_provenance, opencode_scope_start_event_id,
    parse_opencode_hook_event, AttemptKey, OpenCodeHookEvent, OpenCodeScopeProvenance,
    ToolClassification, OPENCODE_TRACKED_TOOL_BASH,
};
use super::payload::{abandon_payload, flush_payload, scope_boundary_payload, scope_start_payload};
use super::state::{self, AdmitDecision, RecoveryFlushCompletion};

pub(crate) fn run_opencode_mutation_scope_subcommand(
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let stdin_payload = hooks::read_hook_stdin()?;
    run_opencode_mutation_scope_from_payload(&stdin_payload, logger)
}

pub(crate) fn run_opencode_mutation_scope_from_payload(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        hooks::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
    };

    run_opencode_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

#[cfg(test)]
pub(crate) fn run_opencode_mutation_scope_from_payload_at_state_root(
    state_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        hooks::mutation_scope::run_mutation_scope_from_payload_at_state_root(
            repository_root,
            state_root,
            payload,
            logger,
        )
    };

    run_opencode_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

pub(super) type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

pub(super) type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

pub(super) const FAIL_CLOSED_MESSAGE: &str =
    "SCE could not establish OpenCode mutation attribution for this tool execution.";

pub(super) const FAIL_CLOSED_EVENT: &str = "sce.hooks.opencode_mutation_scope.start_fail_closed";

pub(super) fn log_fail_closed(logger: Option<&dyn Logger>, context: &str, error: &anyhow::Error) {
    if let Some(log) = logger {
        log.warn(
            FAIL_CLOSED_EVENT,
            &error.to_string(),
            &[("context", context)],
            None,
        );
    }
}

pub(super) fn run_opencode_mutation_scope_from_payload_with_seams(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let event = parse_opencode_hook_event(stdin_payload)?;
    dispatch_opencode_hook_event(event, logger, resolve_git_dir, seam)
}

pub(super) fn dispatch_opencode_hook_event(
    event: OpenCodeHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
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
        }
        OpenCodeHookEvent::ToolExecuteAfter(identity) => {
            if !matches!(
                identity.classification(),
                ToolClassification::TrackedMutation
            ) {
                return Ok(String::new());
            }
            let git_dir = resolve_git_dir(&identity.cwd)?;
            let repository_root = Path::new(&identity.cwd);
            let key = identity.attempt_key();
            with_boundary_lock(&git_dir, || {
                state::normalize_recovery_after_boundary_lock_acquired(&git_dir)?;
                handle_close(&git_dir, repository_root, &key, logger, seam)
            })
        }
        OpenCodeHookEvent::ToolError(call) => {
            if !matches!(call.classification(), ToolClassification::TrackedMutation) {
                return Ok(String::new());
            }
            let git_dir = resolve_git_dir(&call.cwd)?;
            let repository_root = Path::new(&call.cwd);
            let key = call.attempt_key();
            with_boundary_lock(&git_dir, || {
                state::normalize_recovery_after_boundary_lock_acquired(&git_dir)?;
                abandon_and_consume(&git_dir, repository_root, logger, seam, |attempt| {
                    attempt.session_id == key.session_id && attempt.call_id == key.call_id
                })
            })
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

pub(crate) fn repair_blocked(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<RepairOutcome> {
    with_boundary_lock(git_dir, || {
        state::normalize_recovery_after_boundary_lock_acquired(git_dir)?;

        let Some(generation) = state::reprove_dead_owner_pending_start_and_begin_repair(git_dir)?
        else {
            return Ok(RepairOutcome::NoOp);
        };

        match resolve_recovery(git_dir, repository_root, generation, logger, seam)? {
            RecoveryResolution::Cleared => Ok(RepairOutcome::Repaired),
            RecoveryResolution::Unresolved => Ok(RepairOutcome::NoOp),
        }
    })
}

pub(super) fn with_boundary_lock<T>(
    git_dir: &Path,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let _boundary = AdapterBoundaryLock::acquire(git_dir, DEFAULT_BOUNDARY_LOCK_TIMEOUT)
        .map_err(|error| anyhow!("Failed to acquire adapter boundary lock: {error}"))?;
    operation()
}

pub(super) enum Admission {
    Admitted(state::AllocatedAttempt),
    Denied,
}

pub(super) enum StartOutcome {
    Established,
    Denied,
}

pub(super) fn establish_tracked_start(
    cwd: &str,
    key: &AttemptKey,
    tool_name: &str,
    provenance: &OpenCodeScopeProvenance,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let git_dir = match resolve_git_dir(cwd) {
        Ok(git_dir) => git_dir,
        Err(error) => {
            log_fail_closed(logger, "resolve_git_dir", &error);
            return Err(error.context(FAIL_CLOSED_MESSAGE));
        }
    };
    let repository_root = Path::new(cwd);

    let outcome = with_boundary_lock(&git_dir, || {
        state::normalize_recovery_after_boundary_lock_acquired(&git_dir)?;

        match admit_or_recover(&git_dir, repository_root, key, tool_name, logger, seam)? {
            Admission::Admitted(allocated) => {
                establish_start(
                    &git_dir,
                    repository_root,
                    &allocated,
                    provenance,
                    logger,
                    seam,
                )?;
                Ok(StartOutcome::Established)
            }
            Admission::Denied => Ok(StartOutcome::Denied),
        }
    });

    match outcome {
        Ok(StartOutcome::Established) => Ok(String::new()),
        Ok(StartOutcome::Denied) => bail!(FAIL_CLOSED_MESSAGE),
        Err(error) => {
            log_fail_closed(logger, "establish_tracked_start", &error);
            Err(error.context(FAIL_CLOSED_MESSAGE))
        }
    }
}

pub(super) fn admit_or_recover(
    git_dir: &Path,
    repository_root: &Path,
    key: &AttemptKey,
    tool_name: &str,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, tool_name)? {
        AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        AdmitDecision::RecoveryBlocked
        | AdmitDecision::UncertainAttemptBlocked
        | AdmitDecision::TerminalAttemptBlocked => Ok(Admission::Denied),
        AdmitDecision::FlushClaimed { generation } => {
            match resolve_recovery(git_dir, repository_root, generation, logger, seam)? {
                RecoveryResolution::Cleared => readmit_after_flush(git_dir, key, tool_name),
                RecoveryResolution::Unresolved => Ok(Admission::Denied),
            }
        }
    }
}

pub(super) fn readmit_after_flush(
    git_dir: &Path,
    key: &AttemptKey,
    tool_name: &str,
) -> Result<Admission> {
    match state::admit_tracked_attempt(git_dir, key, tool_name)? {
        AdmitDecision::Admitted(allocated) => Ok(Admission::Admitted(allocated)),
        AdmitDecision::FlushClaimed { generation } => {
            state::relinquish_recovery_flush(git_dir, generation)?;
            Ok(Admission::Denied)
        }
        AdmitDecision::RecoveryBlocked
        | AdmitDecision::UncertainAttemptBlocked
        | AdmitDecision::TerminalAttemptBlocked => Ok(Admission::Denied),
    }
}

pub(super) fn establish_start(
    git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    provenance: &OpenCodeScopeProvenance,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
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
        });
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &opencode_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).is_ok() {
        state::remove_attempt(git_dir, &attempt.scope_id)?;
        Ok(String::new())
    } else {
        abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
            candidate.scope_id == doomed_scope_id
        })
    }
}

pub(super) enum RecoveryResolution {
    Cleared,
    Unresolved,
}

pub(super) fn abandon_and_consume(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
    doomed: impl Fn(&state::AdapterAttempt) -> bool,
) -> Result<String> {
    let doomed_scope_ids: Vec<String> = state::read_state(git_dir)?
        .attempts
        .into_iter()
        .filter(|attempt| doomed(attempt))
        .map(|attempt| attempt.scope_id)
        .collect();
    if doomed_scope_ids.is_empty() {
        return Ok(String::new());
    }

    let generation = state::begin_terminal_cleanup(git_dir, &doomed_scope_ids)?;
    resolve_recovery(git_dir, repository_root, generation, logger, seam)?;
    Ok(String::new())
}

pub(super) fn resolve_recovery(
    git_dir: &Path,
    repository_root: &Path,
    generation: u64,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<RecoveryResolution> {
    let pending_abandon: Vec<state::AdapterAttempt> = state::read_state(git_dir)?
        .attempts
        .into_iter()
        .filter(|attempt| attempt.phase == state::AttemptPhase::PendingAbandon)
        .collect();

    if let Err(error) = seam(repository_root, &flush_payload(), logger) {
        log_fail_closed(logger, "recovery_ambiguity_flush", &error);
        state::relinquish_recovery_flush(git_dir, generation)?;
        return Ok(RecoveryResolution::Unresolved);
    }

    for attempt in &pending_abandon {
        if let Err(error) = seam(repository_root, &abandon_payload(&attempt.scope_id), logger) {
            log_fail_closed(logger, "recovery_abandon", &error);
            state::relinquish_recovery_flush(git_dir, generation)?;
            return Ok(RecoveryResolution::Unresolved);
        }
        state::remove_attempt(git_dir, &attempt.scope_id)?;
    }

    if let Err(error) = seam(repository_root, &flush_payload(), logger) {
        log_fail_closed(logger, "recovery_rebaseline_flush", &error);
        state::relinquish_recovery_flush(git_dir, generation)?;
        return Ok(RecoveryResolution::Unresolved);
    }

    match state::complete_recovery_flush(git_dir, generation)? {
        RecoveryFlushCompletion::Cleared => Ok(RecoveryResolution::Cleared),
        RecoveryFlushCompletion::Superseded => Ok(RecoveryResolution::Unresolved),
    }
}
