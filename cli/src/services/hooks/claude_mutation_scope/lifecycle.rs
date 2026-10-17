use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::services::hooks;
use crate::services::managed_telemetry::trace_export::{in_child_span, ManagedChildSpan};
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

pub(super) async fn run_claude_mutation_scope_from_payload_with_resolver<
    L: crate::services::observability::traits::Logger,
>(
    stdin_payload: &str,
    logger: Option<&L>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &str, &str) -> Result<Option<String>>,
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
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
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
            let git_dir = resolve_git_dir(&identity.cwd).await?;
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
            let git_dir = resolve_git_dir(&identity.cwd).await?;
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
            let git_dir = resolve_git_dir(&session.cwd).await?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
            .await
        }
        ClaudeHookEvent::UserPromptSubmit(session) => {
            let git_dir = resolve_git_dir(&session.cwd).await?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.is_none()
            })
            .await
        }
        ClaudeHookEvent::SubagentStop(agent) => {
            let git_dir = resolve_git_dir(&agent.cwd).await?;
            let repository_root = Path::new(&agent.cwd);
            let session_id = agent.session_id.clone();
            let agent_id = agent.agent_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id && attempt.agent_id.as_deref() == Some(&agent_id)
            })
            .await
        }
        ClaudeHookEvent::SessionEnd(session) => {
            let git_dir = resolve_git_dir(&session.cwd).await?;
            let repository_root = Path::new(&session.cwd);
            let session_id = session.session_id.clone();
            cleanup_attempts_matching(&git_dir, repository_root, logger, seam, |attempt| {
                attempt.session_id == session_id
            })
            .await
        }
        ClaudeHookEvent::WorktreeRemove(worktree_remove) => {
            let git_dir = resolve_git_dir(&worktree_remove.worktree_path).await?;
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
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
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
    let git_dir = match resolve_git_dir(&identity.cwd).await {
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
    model_state_resolver: &impl std::ops::AsyncFn(&Path, &str, &str) -> Result<Option<String>>,
    seam: &impl std::ops::AsyncFn(&Path, &str, Option<&L>) -> Result<String>,
) -> Result<()> {
    let allocated =
        state::allocate_attempt(git_dir, &identity.attempt_key(), &identity.tool_name).await?;
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
        state::remove_attempt(git_dir, &attempt.scope_id).await?;
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
    let current = state::read_state(git_dir).await?;
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
mod tests {
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use tokio::net::TcpListener;

    use super::*;
    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::error::CliError;
    use crate::services::hooks::mutation_scope::{
        drive_mutation_scope, parse_mutation_scope_payload,
    };
    use crate::services::managed_telemetry::trace_export::tests::{
        capture_requests_until, loopback_hook_telemetry, trace_spans, DecodedSpan,
    };
    use crate::services::managed_telemetry::trace_export::{
        record_claude_hook_payload, SHUTDOWN_TIMEOUT,
    };
    use crate::services::mutation_trace::runtime::{
        abandon_scope, advise_after_completed_boundary, coordinate,
    };
    use crate::services::observability::traits::{NoopLogger, NoopTelemetry, Telemetry};

    const HOOK_SEQUENCE: [&str; 4] = [
        include_str!("fixtures/probe01-write-success.pre_tool_use.json"),
        include_str!("fixtures/probe01-write-success.post_tool_use.json"),
        include_str!("fixtures/probe02-bash-success.pre_tool_use.json"),
        include_str!("fixtures/probe14-run-in-background-true.pre_tool_use.json"),
    ];
    const FAILING_SEAM_CALL: usize = 3;
    const WRITE_PRE_TOOL_USE: &str =
        include_str!("fixtures/probe01-write-success.pre_tool_use.json");
    const TEST_REPOSITORY_ID: &str = "claude-hook-span-repository";
    const SHUTDOWN_MARGIN: Duration = Duration::from_secs(1);

    #[derive(Debug, Eq, PartialEq)]
    struct HookRun {
        outputs: Vec<Option<String>>,
        seam_payloads: Vec<String>,
        state: state::AdapterState,
    }

    async fn run_hook_sequence<T: Telemetry>(telemetry: &T) -> HookRun {
        let git_dir = tempfile::tempdir().expect("temp dir");
        let seam_payloads = Mutex::new(Vec::new());
        let resolve = async |_: &str| Ok::<_, anyhow::Error>(git_dir.path().to_path_buf());
        let model_state = async |_: &Path, _: &str, _: &str| Ok::<_, anyhow::Error>(None);
        let seam = async |_: &Path, payload: &str, _: Option<&NoopLogger>| {
            let mut payloads = seam_payloads.lock().expect("seam payloads");
            payloads.push(payload.to_string());
            if payloads.len() == FAILING_SEAM_CALL {
                anyhow::bail!("seam failure");
            }
            Ok(String::new())
        };
        let mut outputs = Vec::new();
        for payload in HOOK_SEQUENCE {
            let output = telemetry
                .with_default_subscriber(&mut || async {
                    record_claude_hook_payload(payload);
                    run_claude_mutation_scope_from_payload_with_resolver::<NoopLogger>(
                        payload,
                        None,
                        &resolve,
                        &model_state,
                        &seam,
                    )
                    .await
                    .map_err(CliError::runtime)
                })
                .await;
            outputs.push(output.ok());
        }
        HookRun {
            outputs,
            seam_payloads: seam_payloads.into_inner().expect("seam payloads"),
            state: state::read_state(git_dir.path()).await.expect("state"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failing_managed_telemetry_cannot_change_claude_hook_behavior() {
        let baseline = run_hook_sequence(&NoopTelemetry).await;
        assert!(baseline
            .outputs
            .iter()
            .flatten()
            .any(|output| output.contains("\"permissionDecision\":\"deny\"")));

        let refused = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let refused_address = refused.local_addr().expect("addr");
        drop(refused);
        let stalled = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let stalled_address = stalled.local_addr().expect("addr");
        let holder = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = stalled.accept().await {
                held.push(stream);
            }
        });

        for address in [refused_address, stalled_address] {
            let telemetry = loopback_hook_telemetry(address, "bao-token-value");
            let observed = run_hook_sequence(&telemetry).await;
            let started = Instant::now();
            telemetry.shutdown().await;
            assert!(started.elapsed() < SHUTDOWN_TIMEOUT + SHUTDOWN_MARGIN);
            assert_eq!(observed, baseline);
        }
        holder.abort();
    }

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_repo(root: &Path) {
        std::fs::create_dir_all(root).expect("repo dir");
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "t@example.invalid"]);
        git(root, &["config", "user.name", "T"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("tracked.txt"), "tracked\n").expect("write file");
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-q", "-m", "initial"]);
    }

    async fn export_claude_hook_spans(payload: &str) -> (Option<String>, Vec<DecodedSpan>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("repo");
        init_repo(&root);
        let git_dir = root.join(".git");
        let db_path = dir.path().join("agent-trace.db");
        RepositoryAgentTraceDb::new_at(&db_path)
            .await
            .expect("create db")
            .verify_or_initialize_repository_metadata(TEST_REPOSITORY_ID)
            .await
            .expect("initialize metadata");
        let resolve = async |_: &str| Ok::<_, anyhow::Error>(git_dir.clone());
        let model_state = async |_: &Path, _: &str, _: &str| Ok::<_, anyhow::Error>(None);
        let seam = async |_: &Path, payload: &str, _: Option<&NoopLogger>| {
            drive_mutation_scope(
                &root,
                parse_mutation_scope_payload(payload)?,
                None::<&NoopLogger>,
                &mut Vec::new(),
                async |root: &Path, boundary| {
                    coordinate(root, boundary, async || {
                        RepositoryAgentTraceDb::open_without_migrations_at(&db_path).await
                    })
                    .await
                },
                async |root: &Path, scope| {
                    abandon_scope(root, scope, async || {
                        RepositoryAgentTraceDb::open_without_migrations_at(&db_path).await
                    })
                    .await
                },
                advise_after_completed_boundary,
            )
            .await
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let receiver = tokio::spawn(capture_requests_until(listener, stopped));
        let telemetry = loopback_hook_telemetry(address, "bao-token-value");
        let output = telemetry
            .with_default_subscriber(&mut || async {
                record_claude_hook_payload(payload);
                run_claude_mutation_scope_from_payload_with_resolver::<NoopLogger>(
                    payload,
                    None,
                    &resolve,
                    &model_state,
                    &seam,
                )
                .await
                .map_err(CliError::runtime)
            })
            .await;
        telemetry.shutdown().await;
        stop.send(()).expect("stop receiver");
        let spans = trace_spans(&receiver.await.expect("receiver join"));
        (output.ok(), spans)
    }

    fn span_named<'a>(spans: &'a [DecodedSpan], name: &str) -> &'a DecodedSpan {
        spans
            .iter()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("missing {name}: {spans:?}"))
    }

    fn sorted_names(spans: &[DecodedSpan]) -> Vec<&str> {
        let mut names = spans
            .iter()
            .map(|span| span.name.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracked_claude_hook_exports_mutation_snapshot_and_persistence_children() {
        let (output, spans) = export_claude_hook_spans(WRITE_PRE_TOOL_USE).await;

        assert_eq!(output.as_deref(), Some(""));
        assert_eq!(
            sorted_names(&spans),
            [
                "sce.git_snapshot",
                "sce.hook",
                "sce.mutation_scope",
                "sce.persistence"
            ]
        );
        let hook = span_named(&spans, "sce.hook");
        let mutation_scope = span_named(&spans, "sce.mutation_scope");
        let git_snapshot = span_named(&spans, "sce.git_snapshot");
        let persistence = span_named(&spans, "sce.persistence");
        assert!(hook.parent_span_id.is_empty());
        assert_eq!(mutation_scope.parent_span_id, hook.span_id);
        assert_eq!(git_snapshot.parent_span_id, mutation_scope.span_id);
        assert_eq!(persistence.parent_span_id, mutation_scope.span_id);
        let success = vec![("sce.outcome".to_string(), "success".to_string())];
        for child in [mutation_scope, git_snapshot, persistence] {
            assert_eq!(child.trace_id, hook.trace_id, "{}", child.name);
            assert_eq!(child.sce_attributes(), success, "{}", child.name);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn read_only_claude_hook_exports_no_operation_children() {
        let payload =
            WRITE_PRE_TOOL_USE.replace("\"tool_name\":\"Write\"", "\"tool_name\":\"Read\"");
        assert_ne!(payload, WRITE_PRE_TOOL_USE);

        let (output, spans) = export_claude_hook_spans(&payload).await;

        assert_eq!(output.as_deref(), Some(""));
        assert_eq!(sorted_names(&spans), ["sce.hook"]);
    }
}
