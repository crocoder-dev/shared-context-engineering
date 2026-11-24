use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::hooks;
use crate::services::hooks::claude_model_resolver::{
    resolve_claude_model, resolve_claude_model_from_exact_evidence, ClaudeModelEvidence,
};
use crate::services::managed_telemetry::trace_export::{in_child_span, ManagedChildSpan};
use crate::services::mutation_trace::runtime::{resolve_git_dir, resolve_worktree_root};
use crate::services::mutation_trace::store::{MutationTraceStore, ScopeProvenanceModelFill};
use crate::services::mutation_trace::types::ScopeId;

use super::events::BASH_TOOL_NAME;
use super::routes::{self, PersistRouteOutcome, RouteRecord, RouteTarget};
use super::state;
use super::{
    abandon_payload, classify_tool, claude_scope_close_event_id, claude_scope_start_event_id,
    flush_payload, is_explicit_background_shell, parse_claude_hook_event, pre_tool_use_deny_json,
    scope_boundary_payload, scope_start_payload, AttemptKey, ClaudeHookEvent, ClaudeToolExecution,
    ClaudeToolIdentity, ClaudeWorktreeRemove, ToolClassification,
};

pub(super) const ACTOR_KIND_CLAUDE_CODE: &str = "claude_code";

const CLAUDE_PROJECT_DIR_ENV: &str = "CLAUDE_PROJECT_DIR";

pub(super) const FAIL_CLOSED_DENY_REASON: &str =
    "SCE could not establish mutation attribution for this tool execution.";
pub(super) const EXPLICIT_BACKGROUND_SHELL_DENY_REASON: &str =
    "SCE mutation attribution does not yet support detached background shell execution. Run this command in the foreground.";

pub(super) const PRE_TOOL_USE_FAIL_CLOSED_EVENT: &str =
    "sce.hooks.claude_mutation_scope.pre_tool_use_fail_closed";
pub(super) const MODEL_STATE_UNAVAILABLE_EVENT: &str =
    "sce.hooks.claude_mutation_scope.model_state_unavailable";
pub(super) const PROVENANCE_MODEL_FILL_FAILED_EVENT: &str =
    "sce.hooks.claude_mutation_scope.provenance_model_fill_failed";
pub(super) const ROUTE_UNRESOLVED_EVENT: &str = "sce.hooks.claude_mutation_scope.route_unresolved";

fn log_route_unresolved<L: crate::services::observability::traits::Logger>(
    logger: Option<&L>,
    context: &str,
    error: &dyn std::fmt::Display,
    session_id: &str,
) {
    if let Some(log) = logger {
        log.warn(
            ROUTE_UNRESOLVED_EVENT,
            &error.to_string(),
            &[("context", context)],
            Some(session_id),
        );
    }
}

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
    crate::services::managed_telemetry::trace_export::record_claude_hook_command(
        crate::services::managed_telemetry::trace_export::ClaudeHookCommand::ClaudeMutationScope,
    );
    let stdin_payload = hooks::read_hook_stdin()?;
    crate::services::managed_telemetry::trace_export::record_claude_hook_payload(&stdin_payload);
    run_claude_mutation_scope_from_payload(&stdin_payload, logger).await
}

pub(crate) async fn run_claude_mutation_scope_from_payload<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
) -> Result<String> {
    let resolve_git_dir_fn = async |cwd: &str| resolve_git_dir(Path::new(cwd)).await;
    let model_state_resolver = async |repository_root: &Path, evidence: &ClaudeModelEvidence| {
        let db = hooks::open_agent_trace_db_for_hook_runtime(
            repository_root,
            "Failed to open Agent Trace DB for Claude mutation-scope model resolution.",
        )
        .await?;
        resolve_claude_model(&db, evidence).await
    };
    let seam_fn = async |repository_root: &Path, payload: &str, logger: Option<&L>| {
        hooks::mutation_scope::run_mutation_scope_from_payload(repository_root, payload, logger)
            .await
    };

    let project_dir = std::env::var(CLAUDE_PROJECT_DIR_ENV).ok();

    run_claude_mutation_scope_from_payload_with_resolver(
        stdin_payload,
        logger,
        project_dir.as_deref(),
        &resolve_git_dir_fn,
        &model_state_resolver,
        &seam_fn,
    )
    .await
}

pub(super) async fn run_claude_mutation_scope_from_payload_with_resolver<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
    project_dir: Option<&str>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &ClaudeModelEvidence) -> Result<Option<String>>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    let event = parse_claude_hook_event(stdin_payload)?;
    let traced_seam = async |repository_root: &Path, payload: &str, logger: Option<&L>| {
        in_child_span(
            ManagedChildSpan::MutationScope,
            seam(repository_root, payload, logger),
        )
        .await
    };
    dispatch_claude_hook_event(
        event,
        logger,
        project_dir,
        resolve_git_dir,
        model_state_resolver,
        &traced_seam,
    )
    .await
}

pub(super) async fn dispatch_claude_hook_event<
    L: crate::services::observability::traits::Logger,
>(
    event: ClaudeHookEvent,
    logger: Option<&L>,
    project_dir: Option<&str>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &ClaudeModelEvidence) -> Result<Option<String>>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    match event {
        ClaudeHookEvent::PreToolUse(execution) => Ok(handle_pre_tool_use(
            &execution,
            logger,
            project_dir,
            resolve_git_dir,
            model_state_resolver,
            seam,
        )
        .await),
        ClaudeHookEvent::PostToolUse(identity) | ClaudeHookEvent::PostToolUseFailure(identity) => {
            handle_tool_terminal(
                &identity,
                ToolTerminal::Completed,
                logger,
                project_dir,
                resolve_git_dir,
                seam,
            )
            .await
        }
        ClaudeHookEvent::PermissionDenied(identity) => {
            handle_tool_terminal(
                &identity,
                ToolTerminal::PermissionDenied,
                logger,
                project_dir,
                resolve_git_dir,
                seam,
            )
            .await
        }
        ClaudeHookEvent::Stop(session)
        | ClaudeHookEvent::StopFailure(session)
        | ClaudeHookEvent::UserPromptSubmit(session) => {
            cleanup_session_attempts(
                &session.session_id,
                &session.cwd,
                logger,
                project_dir,
                resolve_git_dir,
                seam,
                |agent_id| agent_id.is_none(),
            )
            .await
        }
        ClaudeHookEvent::SubagentStop(agent) => {
            let agent_id = agent.agent_id.clone();
            cleanup_session_attempts(
                &agent.session_id,
                &agent.cwd,
                logger,
                project_dir,
                resolve_git_dir,
                seam,
                |attempt_agent_id| attempt_agent_id == Some(agent_id.as_str()),
            )
            .await
        }
        ClaudeHookEvent::SessionEnd(session) => {
            cleanup_session_attempts(
                &session.session_id,
                &session.cwd,
                logger,
                project_dir,
                resolve_git_dir,
                seam,
                |_agent_id| true,
            )
            .await
        }
        ClaudeHookEvent::WorktreeRemove(worktree_remove) => {
            handle_worktree_remove(&worktree_remove, logger, project_dir, resolve_git_dir, seam)
                .await
        }
        ClaudeHookEvent::SessionStart | ClaudeHookEvent::SubagentStart => Ok(String::new()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolTerminal {
    Completed,
    PermissionDenied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttemptLookup {
    ByRoute,
    LegacyCwd,
}

impl AttemptLookup {
    fn admits(self, attempt: &state::AdapterAttempt) -> bool {
        match self {
            AttemptLookup::ByRoute => true,
            AttemptLookup::LegacyCwd => !attempt.routed,
        }
    }
}

enum ToolRoute {
    Routed {
        anchor_git_dir: PathBuf,
        target: RouteTarget,
    },
    Unrouted,
}

async fn resolve_route_anchor_or_log<L: crate::services::observability::traits::Logger>(
    project_dir: Option<&str>,
    session_id: &str,
    logger: Option<&L>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
) -> Option<PathBuf> {
    match routes::resolve_route_anchor(project_dir, resolve_git_dir).await {
        Ok(anchor_git_dir) => Some(anchor_git_dir),
        Err(error) => {
            log_route_unresolved(logger, "resolve_route_anchor", &error, session_id);
            None
        }
    }
}

async fn load_tool_route<L: crate::services::observability::traits::Logger>(
    key: &AttemptKey,
    logger: Option<&L>,
    project_dir: Option<&str>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
) -> ToolRoute {
    let Some(anchor_git_dir) =
        resolve_route_anchor_or_log(project_dir, &key.session_id, logger, resolve_git_dir).await
    else {
        return ToolRoute::Unrouted;
    };

    match routes::load_route(&anchor_git_dir, key).await {
        Ok(Some(target)) => ToolRoute::Routed {
            anchor_git_dir,
            target,
        },
        Ok(None) => ToolRoute::Unrouted,
        Err(error) => {
            log_route_unresolved(logger, "load_route", &error, &key.session_id);
            ToolRoute::Unrouted
        }
    }
}

async fn find_attempt(
    git_dir: &Path,
    key: &AttemptKey,
    lookup: AttemptLookup,
) -> Result<Option<state::AdapterAttempt>> {
    let current = state::read_state(git_dir).await?;
    Ok(current
        .attempts
        .into_iter()
        .find(|attempt| attempt_matches_key(attempt, key) && lookup.admits(attempt)))
}

async fn handle_tool_terminal<L: crate::services::observability::traits::Logger>(
    identity: &ClaudeToolIdentity,
    terminal: ToolTerminal,
    logger: Option<&L>,
    project_dir: Option<&str>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    let key = identity.attempt_key();
    match load_tool_route(&key, logger, project_dir, resolve_git_dir).await {
        ToolRoute::Routed {
            anchor_git_dir,
            target,
        } => {
            if let Some(attempt) =
                find_attempt(&target.git_dir, &key, AttemptLookup::ByRoute).await?
            {
                finish_attempt(
                    identity,
                    terminal,
                    &target.git_dir,
                    &target.checkout_path,
                    &attempt,
                    logger,
                    seam,
                )
                .await?;
            }
            retire_route_if_terminal(&anchor_git_dir, &key, &target).await?;
        }
        ToolRoute::Unrouted => {
            let Ok(git_dir) = resolve_git_dir(&identity.cwd).await else {
                return Ok(String::new());
            };
            if let Some(attempt) = find_attempt(&git_dir, &key, AttemptLookup::LegacyCwd).await? {
                finish_attempt(
                    identity,
                    terminal,
                    &git_dir,
                    Path::new(&identity.cwd),
                    &attempt,
                    logger,
                    seam,
                )
                .await?;
            }
        }
    }
    Ok(String::new())
}

async fn finish_attempt<L: crate::services::observability::traits::Logger>(
    identity: &ClaudeToolIdentity,
    terminal: ToolTerminal,
    git_dir: &Path,
    repository_root: &Path,
    attempt: &state::AdapterAttempt,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    match terminal {
        ToolTerminal::Completed => {
            close_attempt(git_dir, repository_root, attempt, identity, logger, seam).await
        }
        ToolTerminal::PermissionDenied => {
            abandon_attempt(git_dir, repository_root, attempt, logger, seam).await
        }
    }
}

async fn retire_route_if_terminal(
    anchor_git_dir: &Path,
    key: &AttemptKey,
    target: &RouteTarget,
) -> Result<()> {
    if find_attempt(&target.git_dir, key, AttemptLookup::ByRoute)
        .await?
        .is_none()
    {
        routes::remove_route(anchor_git_dir, key, target).await?;
    }
    Ok(())
}

fn group_routes_by_target(
    session_routes: impl IntoIterator<Item = RouteRecord>,
) -> Vec<(RouteTarget, Vec<AttemptKey>)> {
    let mut groups: Vec<(RouteTarget, Vec<AttemptKey>)> = Vec::new();
    for route in session_routes {
        let key = AttemptKey {
            session_id: route.session_id,
            agent_id: route.agent_id,
            tool_use_id: route.tool_use_id,
        };
        match groups
            .iter_mut()
            .find(|(target, _)| *target == route.target)
        {
            Some((_, keys)) => keys.push(key),
            None => groups.push((route.target, vec![key])),
        }
    }
    groups
}

async fn cleanup_routed_target<L: crate::services::observability::traits::Logger>(
    anchor_git_dir: &Path,
    target: &RouteTarget,
    keys: &[AttemptKey],
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    cleanup_attempts_matching(
        &target.git_dir,
        &target.checkout_path,
        logger,
        seam,
        |attempt| keys.iter().any(|key| attempt_matches_key(attempt, key)),
    )
    .await?;
    for key in keys {
        retire_route_if_terminal(anchor_git_dir, key, target).await?;
    }
    Ok(())
}

async fn cleanup_session_attempts<L: crate::services::observability::traits::Logger>(
    session_id: &str,
    cwd: &str,
    logger: Option<&L>,
    project_dir: Option<&str>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
    agent_matches: impl Fn(Option<&str>) -> bool,
) -> Result<String> {
    let mut first_error: Option<anyhow::Error> = None;

    if let Some(anchor_git_dir) =
        resolve_route_anchor_or_log(project_dir, session_id, logger, resolve_git_dir).await
    {
        match routes::routes_for_session(&anchor_git_dir, session_id).await {
            Ok(session_routes) => {
                let groups = group_routes_by_target(
                    session_routes
                        .into_iter()
                        .filter(|route| agent_matches(route.agent_id.as_deref())),
                );
                for (target, keys) in &groups {
                    if let Err(error) =
                        cleanup_routed_target(&anchor_git_dir, target, keys, logger, seam).await
                    {
                        first_error.get_or_insert(error);
                    }
                }
            }
            Err(error) => {
                log_route_unresolved(logger, "routes_for_session", &error, session_id);
            }
        }
    }

    if let Ok(git_dir) = resolve_git_dir(cwd).await {
        if let Err(error) =
            cleanup_attempts_matching(&git_dir, Path::new(cwd), logger, seam, |attempt| {
                AttemptLookup::LegacyCwd.admits(attempt)
                    && attempt.session_id == session_id
                    && agent_matches(attempt.agent_id.as_deref())
            })
            .await
        {
            first_error.get_or_insert(error);
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(String::new()),
    }
}

async fn handle_worktree_remove<L: crate::services::observability::traits::Logger>(
    worktree_remove: &ClaudeWorktreeRemove,
    logger: Option<&L>,
    project_dir: Option<&str>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    let git_dir = resolve_git_dir(&worktree_remove.worktree_path).await?;
    let repository_root = Path::new(&worktree_remove.worktree_path);
    cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |_attempt| true).await?;

    let session_id = &worktree_remove.session_id;
    let Some(anchor_git_dir) =
        resolve_route_anchor_or_log(project_dir, session_id, logger, resolve_git_dir).await
    else {
        return Ok(String::new());
    };
    let session_routes = match routes::routes_for_session(&anchor_git_dir, session_id).await {
        Ok(session_routes) => session_routes,
        Err(error) => {
            log_route_unresolved(logger, "routes_for_session", &error, session_id);
            return Ok(String::new());
        }
    };
    for route in session_routes {
        if route.target.git_dir != git_dir
            && !route.target.checkout_path.starts_with(repository_root)
        {
            continue;
        }
        let key = AttemptKey {
            session_id: route.session_id,
            agent_id: route.agent_id,
            tool_use_id: route.tool_use_id,
        };
        retire_route_if_terminal(&anchor_git_dir, &key, &route.target).await?;
    }
    Ok(String::new())
}

async fn persist_tool_route(
    anchor_git_dir: &Path,
    key: &AttemptKey,
    target: &RouteTarget,
) -> Result<()> {
    match routes::persist_route(anchor_git_dir, key, target).await? {
        PersistRouteOutcome::Inserted | PersistRouteOutcome::AlreadyRouted => Ok(()),
        PersistRouteOutcome::Conflict { existing } => Err(anyhow!(
            "Claude tool invocation '{}' is already routed to checkout '{}'",
            key.tool_use_id,
            existing.checkout_path.display()
        )),
    }
}

pub(super) async fn handle_pre_tool_use<L: crate::services::observability::traits::Logger>(
    execution: &ClaudeToolExecution,
    logger: Option<&L>,
    project_dir: Option<&str>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &ClaudeModelEvidence) -> Result<Option<String>>,
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

    let anchor_git_dir = match routes::resolve_route_anchor(project_dir, resolve_git_dir).await {
        Ok(anchor_git_dir) => anchor_git_dir,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "resolve_route_anchor", &anyhow!(error));
            return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
        }
    };

    let repository_root = match resolve_worktree_root(Path::new(&identity.cwd)).await {
        Ok(repository_root) => repository_root,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "resolve_worktree_root", &error);
            return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
        }
    };
    let git_dir = match resolve_checkout_git_dir(&repository_root, resolve_git_dir).await {
        Ok(git_dir) => git_dir,
        Err(error) => {
            log_pre_tool_use_fail_closed(logger, "resolve_git_dir", &error);
            return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
        }
    };

    if matches!(
        apply_recovery_barrier(&git_dir, &repository_root, logger, seam).await,
        BarrierOutcome::Deny
    ) {
        return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
    }

    let target = RouteTarget {
        checkout_path: repository_root.clone(),
        git_dir: git_dir.clone(),
    };
    if let Err(error) = persist_tool_route(&anchor_git_dir, &identity.attempt_key(), &target).await
    {
        log_pre_tool_use_fail_closed(logger, "persist_route", &error);
        return pre_tool_use_deny_json(FAIL_CLOSED_DENY_REASON);
    }

    match establish_start(
        &git_dir,
        &repository_root,
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

/// Resolves the Git directory from the canonical worktree root rather than the tool's working
/// directory, so the persisted root and Git directory always describe the same checkout.
async fn resolve_checkout_git_dir(
    repository_root: &Path,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
) -> Result<PathBuf> {
    let repository_root_str = repository_root.to_str().ok_or_else(|| {
        anyhow!(
            "worktree root '{}' is not valid UTF-8",
            repository_root.display()
        )
    })?;
    resolve_git_dir(repository_root_str).await
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
    let state = match state::read_state(git_dir).await {
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
        Ok(_) => match state::clear_recovery_pending_if_quiescent(git_dir).await {
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
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &ClaudeModelEvidence) -> Result<Option<String>>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    let allocated =
        state::allocate_attempt(git_dir, &identity.attempt_key(), &identity.tool_name).await?;
    let scope_id = &allocated.attempt.scope_id;
    let evidence = identity.model_evidence();
    let model_id = match model_state_resolver(repository_root, &evidence).await {
        Ok(model_id) => model_id.and_then(|model| hooks::normalize_claude_model_id(&model)),
        Err(error) => {
            if let Some(log) = logger {
                log.warn(
                    MODEL_STATE_UNAVAILABLE_EVENT,
                    &error.to_string(),
                    &[("agent_id", identity.agent_id.as_deref().unwrap_or(""))],
                    Some(&evidence.session_id),
                );
            }
            None
        }
    };
    let start_payload = scope_start_payload(
        scope_id,
        &claude_scope_start_event_id(scope_id),
        &evidence.session_id,
        model_id.as_deref(),
    );

    seam(repository_root, &start_payload, logger).await?;
    state::mark_active(git_dir, scope_id).await?;
    Ok(())
}

async fn close_attempt<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    attempt: &state::AdapterAttempt,
    identity: &ClaudeToolIdentity,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    if attempt.phase == state::AttemptPhase::PendingStart {
        return abandon_attempt(git_dir, repository_root, attempt, logger, seam).await;
    }

    let close_payload = scope_boundary_payload(
        "close",
        &attempt.scope_id,
        &claude_scope_close_event_id(&attempt.scope_id),
    );

    if seam(repository_root, &close_payload, logger).await.is_ok() {
        fill_provenance_model_after_close(repository_root, &attempt.scope_id, identity, logger)
            .await;
        state::remove_attempt(git_dir, &attempt.scope_id).await
    } else {
        abandon_attempt(git_dir, repository_root, attempt, logger, seam).await
    }
}

async fn fill_provenance_model_after_close<L: crate::services::observability::traits::Logger>(
    repository_root: &Path,
    scope_id: &str,
    identity: &ClaudeToolIdentity,
    logger: Option<&L>,
) {
    let evidence = identity.model_evidence();
    let filled = async {
        let mut db = hooks::open_agent_trace_db_for_hook_runtime(
            repository_root,
            "Failed to open Agent Trace DB for Claude mutation-scope provenance model fill.",
        )
        .await?;
        fill_claude_scope_provenance_model(&mut db, scope_id, &evidence).await
    }
    .await;

    if let (Err(error), Some(log)) = (filled, logger) {
        log.warn(
            PROVENANCE_MODEL_FILL_FAILED_EVENT,
            &error.to_string(),
            &[("scope_id", scope_id)],
            Some(&evidence.session_id),
        );
    }
}

pub(super) async fn fill_claude_scope_provenance_model(
    db: &mut RepositoryAgentTraceDb,
    scope_id: &str,
    evidence: &ClaudeModelEvidence,
) -> Result<Option<ScopeProvenanceModelFill>> {
    let scope_id = ScopeId(scope_id.to_owned());
    let store = MutationTraceStore::new(db);
    match store.load_scope_provenance(&scope_id).await? {
        None => return Ok(Some(ScopeProvenanceModelFill::NoRow)),
        Some(stored) if stored.model_id.is_some() => {
            return Ok(Some(ScopeProvenanceModelFill::AlreadySet));
        }
        Some(_) => {}
    }

    let Some(model_id) = resolve_claude_model_from_exact_evidence(evidence) else {
        return Ok(None);
    };
    store
        .fill_null_scope_provenance_model(&scope_id, &model_id)
        .await
        .map(Some)
}

#[cfg(test)]
pub(super) async fn handle_close<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    identity: &ClaudeToolIdentity,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<String> {
    let current = state::read_state(git_dir).await?;
    let Some(attempt) = current
        .attempts
        .iter()
        .find(|attempt| attempt_matches_key(attempt, &identity.attempt_key()))
        .cloned()
    else {
        return Ok(String::new());
    };

    close_attempt(git_dir, repository_root, &attempt, identity, logger, seam).await?;
    Ok(String::new())
}

pub(super) async fn cleanup_attempts_matching<L: crate::services::observability::traits::Logger>(
    git_dir: &Path,
    repository_root: &Path,
    logger: Option<&L>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
    predicate: impl Fn(&state::AdapterAttempt) -> bool,
) -> Result<String> {
    let current = state::read_state(git_dir).await?;
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
    state::mark_recovery_pending_and_pending_abandon(git_dir, &stale_scope_ids).await?;

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
    )
    .await?;

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
    state::remove_attempt(git_dir, scope_id).await?;
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
    let Some(attempts) = state::reprove_pending_abandon(git_dir).await? else {
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
        state::clear_recovery_pending_if_quiescent(git_dir).await?,
        state::ClearRecoveryOutcome::Cleared
    );

    if any_failed || !cleared {
        Ok(RepairOutcome::NoOp)
    } else {
        Ok(RepairOutcome::Repaired)
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
