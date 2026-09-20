use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
};
use crate::services::mutation_trace::runtime::{
    coordinate, run_external_mutation_guard, GuardRequest, RuntimeBoundary,
};
use crate::services::mutation_trace::types::{ActorKind, EventId, ScopeId};

use super::*;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git should spawn");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct GuardRepo {
    _temp: tempfile::TempDir,
    root: PathBuf,
    state_root: PathBuf,
}

impl GuardRepo {
    fn new(label: &str) -> Self {
        let temp = tempfile::Builder::new()
            .prefix(&format!("sce-pi-guard-reconciliation-{label}-"))
            .tempdir()
            .expect("temp dir should be created");
        let root = temp.path().join("repo");
        fs::create_dir_all(&root).expect("repo dir should be created");
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "user.name", "SCE Test"]);
        git(
            &root,
            &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
        );
        fs::write(root.join("file.txt"), "one\n").expect("seed file should write");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-qm", "base"]);

        let state_root = temp.path().join("state");
        fs::create_dir_all(&state_root).expect("state root should be created");
        resolve_agent_trace_storage_at_state_root(
            &AgentTraceStorageContext {
                repository_root: &root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            &state_root,
        )
        .expect("state-root storage should initialize the repository DB");

        Self {
            _temp: temp,
            root,
            state_root,
        }
    }

    fn drive(&self, payload: &str) -> Result<String> {
        run_pi_mutation_scope_from_payload_at_state_root(&self.state_root, payload, None)
    }

    fn open_db(&self) -> anyhow::Result<RepositoryAgentTraceDb> {
        crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
            &self.root,
            &self.state_root,
            "Pi guard-reconciliation test assertions",
        )
    }

    fn db(&self) -> RepositoryAgentTraceDb {
        self.open_db().expect("assertion DB should open")
    }

    fn scope_status(&self, scope_id: &str) -> Option<(String, String)> {
        self.db()
            .query_map(
                "SELECT actor_kind, status FROM mutation_trace_scopes WHERE scope_id = ?1",
                (scope_id,),
                |row| {
                    let actor_kind = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let status = row.get::<String>(1).map_err(anyhow::Error::from)?;
                    Ok((actor_kind, status))
                },
            )
            .expect("scope query should succeed")
            .into_iter()
            .next()
    }

    fn write(&self, name: &str, contents: &str) {
        fs::write(self.root.join(name), contents).expect("write should succeed");
    }

    fn mutation_events(&self) -> Vec<(String, Option<String>)> {
        self.db()
            .query_map(
                "SELECT attribution_kind, attribution_scope_id \
                     FROM mutation_trace_events ORDER BY revision",
                (),
                |row| {
                    let attribution_kind = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let attribution_scope_id =
                        row.get::<Option<String>>(1).map_err(anyhow::Error::from)?;
                    Ok((attribution_kind, attribution_scope_id))
                },
            )
            .expect("mutation-events query should succeed")
    }
}

fn tool_call(repo: &GuardRepo, tool_call_id: &str, session_id: &str) -> String {
    json!({
        "hook_event_name": "ToolCall",
        "session_id": session_id,
        "tool_call_id": tool_call_id,
        "cwd": repo.root.to_string_lossy(),
        "tool_name": "bash",
        "model": "openai-codex/gpt-5.5",
    })
    .to_string()
}

fn tool_execution_end(repo: &GuardRepo, tool_call_id: &str, session_id: &str) -> String {
    json!({
        "hook_event_name": "ToolExecutionEnd",
        "session_id": session_id,
        "tool_call_id": tool_call_id,
        "cwd": repo.root.to_string_lossy(),
        "tool_name": "bash",
    })
    .to_string()
}

fn tool_result(repo: &GuardRepo, tool_call_id: &str, session_id: &str) -> String {
    json!({
        "hook_event_name": "ToolResult",
        "session_id": session_id,
        "tool_call_id": tool_call_id,
        "cwd": repo.root.to_string_lossy(),
        "tool_name": "bash",
    })
    .to_string()
}

#[test]
fn a_guard_triggered_worktree_abandonment_reconciles_with_the_pi_adapters_own_state() {
    let repo = GuardRepo::new("reconcile");
    let session = "01a091f4-guard-session";
    let key_a = AttemptKey {
        session_id: session.to_string(),
        tool_call_id: "call_a".to_string(),
    };
    let key_b = AttemptKey {
        session_id: session.to_string(),
        tool_call_id: "call_b".to_string(),
    };
    let scope_a = format_pi_scope_id(&key_a, 1);
    let scope_b = format_pi_scope_id(&key_b, 2);

    repo.drive(&tool_call(&repo, "call_a", session))
        .expect("A's Start should reach the real runtime");
    repo.drive(&tool_call(&repo, "call_b", session))
        .expect("B's Start should reach the real runtime");
    assert_eq!(
        repo.scope_status(&scope_a),
        Some(("pi".to_string(), "active".to_string()))
    );
    assert_eq!(
        repo.scope_status(&scope_b),
        Some(("pi".to_string(), "active".to_string()))
    );

    let (_cancel_tx, cancel_rx) = mpsc::channel();
    let root = repo.root.clone();
    let outcome = run_external_mutation_guard(
        &root,
        &GuardRequest {
            command: "printf changed >> file.txt".to_string(),
            cwd: None,
            env: Vec::new(),
        },
        || repo.open_db(),
        |_event| {},
        cancel_rx,
    )
    .expect("the guard should finish successfully");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(!outcome.marker_clear_failed);

    assert_eq!(
        repo.scope_status(&scope_a),
        Some(("pi".to_string(), "abandoned".to_string())),
        "the guard's finish-time forced recovery must abandon every scope live during the \
             guarded interval, regardless of which harness's boundary happened to observe \
             user_bash"
    );
    assert_eq!(
        repo.scope_status(&scope_b),
        Some(("pi".to_string(), "abandoned".to_string()))
    );

    repo.drive(&tool_execution_end(&repo, "call_a", session))
        .expect(
            "the adapter's next interaction for an already-abandoned scope must reconcile \
             safely (falling back through the existing Close-failure-to-abandon path) rather \
             than erroring or resurrecting the scope",
        );
    repo.drive(&tool_execution_end(&repo, "call_b", session))
        .expect("the same reconciliation must hold for every sibling abandoned by the guard");

    assert!(
        state::read_state(&resolve_git_dir(&repo.root).expect("git dir resolves"))
            .expect("state readable")
            .attempts
            .is_empty(),
        "the Pi adapter's own durable local attempt state must converge to empty once it \
             observes the terminal event for a scope the generic runtime already abandoned out \
             from under it"
    );
}

#[test]
fn a_guard_abandons_a_live_pi_scope_alongside_a_live_scope_from_another_harness() {
    let repo = GuardRepo::new("cross-harness");
    let key = AttemptKey {
        session_id: "01a091f4-guard-cross-session".to_string(),
        tool_call_id: "call_pi".to_string(),
    };
    let pi_scope_id = format_pi_scope_id(&key, 1);
    let claude_scope = ScopeId("claude-scope-under-guard".to_string());

    repo.drive(&tool_call(&repo, "call_pi", "01a091f4-guard-cross-session"))
        .expect("Pi's Start should reach the real runtime");
    coordinate(
        &repo.root,
        &RuntimeBoundary::Start {
            scope: claude_scope.clone(),
            event: EventId("claude-evt-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
            provenance: None,
        },
        || repo.open_db(),
    )
    .expect("Claude's Start should reach the real runtime");

    assert_eq!(
        repo.scope_status(&pi_scope_id),
        Some(("pi".to_string(), "active".to_string()))
    );
    assert_eq!(
        repo.scope_status(&claude_scope.0),
        Some(("claude_code".to_string(), "active".to_string()))
    );

    let (_cancel_tx, cancel_rx) = mpsc::channel();
    let root = repo.root.clone();
    run_external_mutation_guard(
        &root,
        &GuardRequest {
            command: "printf changed >> file.txt".to_string(),
            cwd: None,
            env: Vec::new(),
        },
        || repo.open_db(),
        |_event| {},
        cancel_rx,
    )
    .expect("the guard should finish successfully");

    assert_eq!(
        repo.scope_status(&pi_scope_id),
        Some(("pi".to_string(), "abandoned".to_string())),
        "the guard's forced recovery must abandon the Pi scope even though a different \
             harness's boundary is the one that happened to observe user_bash"
    );
    assert_eq!(
        repo.scope_status(&claude_scope.0),
        Some(("claude_code".to_string(), "abandoned".to_string())),
        "the guard's forced recovery must abandon every live scope on the worktree \
             regardless of which harness owns it"
    );

    repo.drive(&tool_execution_end(
        &repo,
        "call_pi",
        "01a091f4-guard-cross-session",
    ))
    .expect(
        "the Pi adapter must still reconcile cleanly with its own scope even when a \
                 sibling scope belonging to a different harness was abandoned by the same guard",
    );
    assert!(
        state::read_state(&resolve_git_dir(&repo.root).expect("git dir resolves"))
            .expect("state readable")
            .attempts
            .is_empty()
    );
}

#[test]
fn a_foreign_pi_start_racing_an_active_guard_fails_closed_touching_no_state_then_succeeds_on_retry()
{
    let repo = GuardRepo::new("race");
    let ready = repo.root.join("ready");
    let release = repo.root.join("release");
    let command = format!(
        "touch '{}'; while [ ! -f '{}' ]; do sleep 0.02; done",
        ready.display(),
        release.display(),
    );

    let root = repo.root.clone();
    let state_root = repo.state_root.clone();
    let guard_thread = std::thread::spawn(move || {
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        run_external_mutation_guard(
            &root,
            &GuardRequest {
                command,
                cwd: None,
                env: Vec::new(),
            },
            || {
                crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                    &root,
                    &state_root,
                    "guard race test",
                )
            },
            |_event| {},
            cancel_rx,
        )
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the guarded shell never reported ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let key = AttemptKey {
        session_id: "01a091f4-guard-race-session".to_string(),
        tool_call_id: "call_race".to_string(),
    };
    let scope_id = format_pi_scope_id(&key, 1);

    let error = repo
        .drive(&tool_call(
            &repo,
            "call_race",
            "01a091f4-guard-race-session",
        ))
        .expect_err(
            "a Pi Start racing an active external-mutation guard must block then fail \
                 closed with CoordinateError::LockAcquisition, never proceed",
        );
    assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE));
    assert!(
        repo.scope_status(&scope_id).is_none(),
        "a boundary that fails closed on lock acquisition must touch no protocol state"
    );

    fs::write(&release, "go").expect("release file should write");
    let outcome = guard_thread
        .join()
        .expect("guard thread should not panic")
        .expect("the guard should finish successfully once released");
    assert_eq!(outcome.exit_code, Some(0));

    repo.drive(&tool_call(
        &repo,
        "call_race",
        "01a091f4-guard-race-session",
    ))
    .expect("retrying the same Start after the guard finishes must succeed normally");
    assert_eq!(
        repo.scope_status(&scope_id),
        Some(("pi".to_string(), "active".to_string()))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_foreign_harnesss_boundary_racing_the_guard_fails_closed_then_succeeds_normally_on_retry() {
    let repo = GuardRepo::new("cross-harness-race");
    let claude_scope = ScopeId("claude-scope-racing-guard".to_string());
    let key = AttemptKey {
        session_id: "01a091f4-guard-cross-race-session".to_string(),
        tool_call_id: "call_pi_race".to_string(),
    };
    let pi_scope_id = format_pi_scope_id(&key, 1);

    repo.drive(&tool_call(
        &repo,
        "call_pi_race",
        "01a091f4-guard-cross-race-session",
    ))
    .expect("Pi's Start should reach the real runtime");
    coordinate(
        &repo.root,
        &RuntimeBoundary::Start {
            scope: claude_scope.clone(),
            event: EventId("claude-evt-race-start".to_string()),
            actor_kind: ActorKind::ClaudeCode,
            provenance: None,
        },
        || repo.open_db(),
    )
    .expect("Claude's Start should reach the real runtime");

    let ready = repo.root.join("ready");
    let release = repo.root.join("release");
    let command = format!(
            "touch '{}'; printf w1 >> file.txt; while [ ! -f '{}' ]; do sleep 0.02; done; printf w2 >> file.txt",
            ready.display(),
            release.display(),
        );

    let root = repo.root.clone();
    let state_root = repo.state_root.clone();
    let guard_thread = std::thread::spawn(move || {
        let (_cancel_tx, cancel_rx) = mpsc::channel();
        run_external_mutation_guard(
            &root,
            &GuardRequest {
                command,
                cwd: None,
                env: Vec::new(),
            },
            || {
                crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                    &root,
                    &state_root,
                    "guard cross-harness race test",
                )
            },
            |_event| {},
            cancel_rx,
        )
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the guarded shell never reported ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let close_while_active = coordinate(
        &repo.root,
        &RuntimeBoundary::Close {
            scope: claude_scope.clone(),
            event: EventId("claude-evt-race-close".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        || repo.open_db(),
    );
    assert!(
        matches!(
            close_while_active,
            Err(crate::services::mutation_trace::runtime::CoordinateError::LockAcquisition(_))
        ),
        "a foreign harness's boundary racing an active external-mutation guard must fail \
             closed with LockAcquisition, never proceed while the guard still holds the \
             worktree: {close_while_active:?}"
    );
    assert!(
        repo.mutation_events().is_empty(),
        "a boundary that fails closed on lock acquisition must touch no protocol state"
    );

    fs::write(&release, "go").expect("release file should write");
    let outcome = guard_thread
        .join()
        .expect("guard thread should not panic")
        .expect("the guard should finish successfully once released");
    assert_eq!(outcome.exit_code, Some(0));

    assert_eq!(
        repo.scope_status(&pi_scope_id),
        Some(("pi".to_string(), "abandoned".to_string()))
    );
    assert_eq!(
        repo.scope_status(&claude_scope.0),
        Some(("claude_code".to_string(), "abandoned".to_string()))
    );

    coordinate(
        &repo.root,
        &RuntimeBoundary::Close {
            scope: claude_scope.clone(),
            event: EventId("claude-evt-race-close-retry".to_string()),
            actor_kind: ActorKind::ClaudeCode,
        },
        || repo.open_db(),
    )
    .expect(
        "Claude's deferred boundary must succeed normally once retried against the \
             recovered worktree, rather than continuing to fail closed",
    );
    repo.drive(&tool_execution_end(
        &repo,
        "call_pi_race",
        "01a091f4-guard-cross-race-session",
    ))
    .expect("the Pi adapter must reconcile its own already-abandoned scope cleanly too");

    assert!(
        repo.mutation_events().is_empty(),
        "both human writes made under the guard must remain excluded from positive AI \
             attribution for the Pi scope and for the racing foreign-harness scope alike"
    );
}

#[test]
fn a_guard_triggered_abandonment_does_not_poison_the_checkout_for_a_fresh_pi_scope() {
    let repo = GuardRepo::new("post-recovery-fresh-start");
    let session = "01a091f4-guard-fresh-session";
    let key_a = AttemptKey {
        session_id: session.to_string(),
        tool_call_id: "call_doomed".to_string(),
    };
    let scope_a = format_pi_scope_id(&key_a, 1);

    repo.drive(&tool_call(&repo, "call_doomed", session))
        .expect("A's Start should reach the real runtime");
    assert_eq!(
        repo.scope_status(&scope_a),
        Some(("pi".to_string(), "active".to_string()))
    );

    let (_cancel_tx, cancel_rx) = mpsc::channel();
    let root = repo.root.clone();
    let outcome = run_external_mutation_guard(
        &root,
        &GuardRequest {
            command: "printf changed >> file.txt".to_string(),
            cwd: None,
            env: Vec::new(),
        },
        || repo.open_db(),
        |_event| {},
        cancel_rx,
    )
    .expect("the guard should finish successfully");
    assert_eq!(outcome.exit_code, Some(0));

    assert_eq!(
        repo.scope_status(&scope_a),
        Some(("pi".to_string(), "abandoned".to_string()))
    );
    repo.drive(&tool_execution_end(&repo, "call_doomed", session))
        .expect("the adapter must reconcile the guard-abandoned scope cleanly");

    let key_c = AttemptKey {
        session_id: session.to_string(),
        tool_call_id: "call_clean".to_string(),
    };
    let scope_c = format_pi_scope_id(&key_c, 2);

    repo.drive(&tool_call(&repo, "call_clean", session))
        .expect("a fresh Start on the same worktree after recovery must succeed normally");
    assert_eq!(
        repo.scope_status(&scope_c),
        Some(("pi".to_string(), "active".to_string()))
    );

    repo.write("file.txt", "one\nchanged\nclean\n");
    repo.drive(&tool_result(&repo, "call_clean", session))
        .expect("tool_result should mark Executed");
    repo.drive(&tool_execution_end(&repo, "call_clean", session))
        .expect("Close should reach the real runtime");

    assert_eq!(
        repo.scope_status(&scope_c),
        Some(("pi".to_string(), "closed".to_string()))
    );
    assert_eq!(
        repo.mutation_events(),
        vec![("ai_exclusive".to_string(), Some(scope_c))],
        "a clean Pi mutation after guard-triggered recovery must still reach AiExclusive; \
             recovery must not permanently poison the checkout for later, uninterfered-with work"
    );
}
