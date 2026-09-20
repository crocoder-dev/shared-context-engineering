use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};

use crate::services::hooks;
use crate::services::mutation_trace::runtime::resolve_git_dir;
use crate::services::observability::traits::Logger;

use super::boundary_lock::{AdapterBoundaryLock, DEFAULT_BOUNDARY_LOCK_TIMEOUT};
use super::state::{self, AdmitDecision, RecoveryFlushCompletion};
use super::{
    abandon_payload, flush_payload, parse_pi_hook_event, pi_scope_close_event_id,
    pi_scope_provenance, pi_scope_start_event_id, scope_boundary_payload, scope_start_payload,
    AttemptKey, PiHookEvent, PiScopeProvenance, ToolClassification,
};

pub(crate) fn run_pi_mutation_scope_subcommand(logger: Option<&dyn Logger>) -> Result<String> {
    let stdin_payload = hooks::read_hook_stdin()?;
    run_pi_mutation_scope_from_payload(&stdin_payload, logger)
}

pub(crate) fn run_pi_mutation_scope_from_payload(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let resolve_git_dir_fn = |cwd: &str| resolve_git_dir(Path::new(cwd));
    let seam_fn = |repository_root: &Path, payload: &str, logger: Option<&dyn Logger>| {
        hooks::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
    };

    run_pi_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

#[cfg(test)]
pub(crate) fn run_pi_mutation_scope_from_payload_at_state_root(
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

    run_pi_mutation_scope_from_payload_with_seams(
        stdin_payload,
        logger,
        &resolve_git_dir_fn,
        &seam_fn,
    )
}

pub(super) type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

pub(super) type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

pub(super) const FAIL_CLOSED_MESSAGE: &str =
    "SCE could not establish Pi mutation attribution for this tool execution.";

pub(super) const FAIL_CLOSED_EVENT: &str = "sce.hooks.pi_mutation_scope.start_fail_closed";

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

pub(super) fn run_pi_mutation_scope_from_payload_with_seams(
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    let event = parse_pi_hook_event(stdin_payload)?;
    dispatch_pi_hook_event(event, logger, resolve_git_dir, seam)
}

pub(super) fn dispatch_pi_hook_event(
    event: PiHookEvent,
    logger: Option<&dyn Logger>,
    resolve_git_dir: GitDirResolver,
    seam: IngressSeam,
) -> Result<String> {
    match event {
        PiHookEvent::ExecutionStart(_identity) => Ok(String::new()),
        PiHookEvent::Call(call) => match call.identity.classification() {
            ToolClassification::TrackedMutation => {
                let provenance =
                    pi_scope_provenance(&call.identity.session_id, call.model.as_deref());
                establish_tracked_start(
                    &call.identity.cwd,
                    &call.identity.attempt_key(),
                    &call.identity.tool_name,
                    &provenance,
                    logger,
                    resolve_git_dir,
                    seam,
                )
            }
            ToolClassification::Untracked => Ok(String::new()),
        },
        PiHookEvent::Executed(identity) => {
            if !matches!(
                identity.classification(),
                ToolClassification::TrackedMutation
            ) {
                return Ok(String::new());
            }
            let git_dir = resolve_git_dir(&identity.cwd)?;
            state::mark_executed(&git_dir, &identity.attempt_key())?;
            Ok(String::new())
        }
        PiHookEvent::ExecutionEnd(identity) => {
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
                handle_tool_execution_end(&git_dir, repository_root, &key, logger, seam)
            })
        }
        PiHookEvent::ExecutionAbandon(identity) => {
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
                force_abandon_attempt(&git_dir, repository_root, &key, logger, seam)
            })
        }
    }
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
    provenance: &PiScopeProvenance,
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
    match reconcile_stale_owners(git_dir, repository_root, logger, seam)? {
        RecoveryResolution::Cleared => {}
        RecoveryResolution::Unresolved => return Ok(Admission::Denied),
    }

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

pub(super) fn reconcile_stale_owners(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<RecoveryResolution> {
    loop {
        let dead_scope_ids = state::find_definitely_dead_attempts(git_dir)?;
        if dead_scope_ids.is_empty() {
            return Ok(RecoveryResolution::Cleared);
        }

        let generation = state::begin_terminal_cleanup(git_dir, &dead_scope_ids)?;
        if matches!(
            resolve_recovery(git_dir, repository_root, generation, logger, seam)?,
            RecoveryResolution::Unresolved
        ) {
            return Ok(RecoveryResolution::Unresolved);
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
    _git_dir: &Path,
    repository_root: &Path,
    allocated: &state::AllocatedAttempt,
    provenance: &PiScopeProvenance,
    logger: Option<&dyn Logger>,
    seam: IngressSeam,
) -> Result<()> {
    let scope_id = &allocated.attempt.scope_id;

    let start_payload =
        scope_start_payload(scope_id, &pi_scope_start_event_id(scope_id), provenance);

    seam(repository_root, &start_payload, logger)?;
    Ok(())
}

pub(super) fn handle_tool_execution_end(
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
        .find(|attempt| {
            attempt.session_id == key.session_id && attempt.tool_call_id == key.tool_call_id
        })
        .cloned()
    else {
        return Ok(String::new());
    };

    let doomed_scope_id = attempt.scope_id.clone();

    if !matches!(attempt.phase, state::AttemptPhase::Executed) {
        return abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
            candidate.scope_id == doomed_scope_id
        });
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &pi_scope_close_event_id(&attempt.scope_id),
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

pub(super) fn force_abandon_attempt(
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
        .find(|attempt| {
            attempt.session_id == key.session_id && attempt.tool_call_id == key.tool_call_id
        })
        .cloned()
    else {
        return Ok(String::new());
    };

    let doomed_scope_id = attempt.scope_id.clone();
    abandon_and_consume(git_dir, repository_root, logger, seam, move |candidate| {
        candidate.scope_id == doomed_scope_id
    })
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

#[cfg(test)]
pub(crate) fn force_attempt_owner_dead_for_tests(git_dir: &Path, scope_id: &str) {
    let mut dead_child = std::process::Command::new("true")
        .spawn()
        .expect("spawning 'true' should succeed");
    let dead_pid = i32::try_from(dead_child.id()).expect("pid fits in i32");
    dead_child.wait().expect("child should exit and be reaped");
    state::set_attempt_owner_for_tests(
        git_dir,
        scope_id,
        super::process_owner::ProcessOwner {
            pid: dead_pid,
            instance_token: None,
        },
    );
}
