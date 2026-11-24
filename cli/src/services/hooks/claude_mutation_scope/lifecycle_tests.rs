use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::run_claude_mutation_scope_from_payload_with_resolver;
use crate::services::hooks::claude_model_resolver::ClaudeModelEvidence;
use crate::services::hooks::claude_mutation_scope::routes::{self, RouteTarget};
use crate::services::hooks::claude_mutation_scope::state::{self, AdapterState};
use crate::services::hooks::claude_mutation_scope::AttemptKey;
use crate::services::mutation_trace::runtime::resolve_git_dir;
use crate::services::observability::traits::NoopLogger;

const SESSION: &str = "session-1";

#[derive(Clone, Debug, Eq, PartialEq)]
struct SeamCall {
    root: PathBuf,
    operation: String,
    scope_id: Option<String>,
}

struct Harness {
    _root: tempfile::TempDir,
    _outside: tempfile::TempDir,
    repo_a: PathBuf,
    nested_b: PathBuf,
    outside: PathBuf,
    calls: Mutex<Vec<SeamCall>>,
    failing_operations: Mutex<Vec<&'static str>>,
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

impl Harness {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temp dir");
        let outside = tempfile::tempdir().expect("outside temp dir");
        let repo_a = root.path().join("repoA");
        let nested_b = repo_a.join("nestedB");
        for repo in [&repo_a, &nested_b] {
            std::fs::create_dir_all(repo).expect("mkdir");
            git(repo, &["init", "--quiet"]);
        }
        Harness {
            repo_a,
            nested_b,
            outside: outside.path().to_path_buf(),
            _root: root,
            _outside: outside,
            calls: Mutex::new(Vec::new()),
            failing_operations: Mutex::new(Vec::new()),
        }
    }

    async fn send_with_project_dir(
        &self,
        project_dir: Option<&Path>,
        payload: Value,
    ) -> Result<String> {
        let resolver = async |dir: &str| resolve_git_dir(Path::new(dir)).await;
        let model_state = async |_: &Path, _: &ClaudeModelEvidence| Ok(None);
        let seam = async |root: &Path, payload: &str, _: Option<&NoopLogger>| {
            let parsed: Value = serde_json::from_str(payload).expect("seam payload is JSON");
            let operation = parsed["operation"].as_str().expect("operation").to_string();
            if self
                .failing_operations
                .lock()
                .unwrap()
                .contains(&operation.as_str())
            {
                return Err(anyhow!("injected seam failure for {operation}"));
            }
            self.calls.lock().unwrap().push(SeamCall {
                root: root.to_path_buf(),
                operation,
                scope_id: parsed["scope_id"].as_str().map(str::to_string),
            });
            Ok(String::new())
        };
        run_claude_mutation_scope_from_payload_with_resolver(
            &payload.to_string(),
            None::<&NoopLogger>,
            project_dir.and_then(Path::to_str),
            &resolver,
            &model_state,
            &seam,
        )
        .await
    }

    async fn send(&self, payload: Value) -> Result<String> {
        self.send_with_project_dir(Some(&self.repo_a.clone()), payload)
            .await
    }

    fn take_calls(&self) -> Vec<SeamCall> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }

    fn fail(&self, operation: &'static str) {
        self.failing_operations.lock().unwrap().push(operation);
    }

    fn heal(&self) {
        self.failing_operations.lock().unwrap().clear();
    }

    fn anchor(&self) -> PathBuf {
        self.repo_a.join(".git")
    }

    async fn state_of(&self, repo: &Path) -> AdapterState {
        state::read_state(&repo.join(".git"))
            .await
            .expect("state reads")
    }

    async fn route_count(&self) -> usize {
        routes::read_routes(&self.anchor())
            .await
            .expect("routes read")
            .routes
            .len()
    }
}

fn tool_event(event: &str, agent: Option<&str>, tool_use_id: &str, cwd: &Path) -> Value {
    json!({
        "hook_event_name": event,
        "session_id": SESSION,
        "agent_id": agent,
        "cwd": cwd.to_str().unwrap(),
        "tool_name": "Write",
        "tool_use_id": tool_use_id,
        "tool_input": {},
    })
}

fn pre(agent: Option<&str>, tool_use_id: &str, cwd: &Path) -> Value {
    tool_event("PreToolUse", agent, tool_use_id, cwd)
}

fn post(agent: Option<&str>, tool_use_id: &str, cwd: &Path) -> Value {
    tool_event("PostToolUse", agent, tool_use_id, cwd)
}

fn session_event(event: &str, cwd: &Path) -> Value {
    json!({ "hook_event_name": event, "session_id": SESSION, "cwd": cwd.to_str().unwrap() })
}

fn started_scope(calls: &[SeamCall]) -> String {
    let [call] = calls else {
        panic!("expected exactly one start call, got {calls:?}");
    };
    assert_eq!(call.operation, "start");
    call.scope_id.clone().expect("start carries a scope id")
}

fn call(root: &Path, operation: &str, scope_id: &str) -> SeamCall {
    SeamCall {
        root: root.to_path_buf(),
        operation: operation.to_string(),
        scope_id: Some(scope_id.to_string()),
    }
}

#[tokio::test]
async fn post_tool_use_from_a_nested_repository_closes_the_attempt_in_the_original_repository() {
    let h = Harness::new();
    assert_eq!(h.send(pre(None, "t1", &h.repo_a)).await.unwrap(), "");
    let scope = started_scope(&h.take_calls());

    h.send(post(None, "t1", &h.nested_b)).await.unwrap();

    assert_eq!(h.take_calls(), vec![call(&h.repo_a, "close", &scope)]);
    assert!(h.state_of(&h.repo_a).await.attempts.is_empty());
    assert!(!state::state_path(&h.nested_b.join(".git")).exists());
    assert_eq!(h.route_count().await, 0);
}

#[tokio::test]
async fn concurrent_tools_in_two_repositories_close_only_their_own_attempts_in_reverse_order() {
    let h = Harness::new();
    h.send(pre(None, "t1", &h.repo_a)).await.unwrap();
    let main_a = started_scope(&h.take_calls());
    h.send(pre(None, "t2", &h.nested_b)).await.unwrap();
    let main_b = started_scope(&h.take_calls());
    h.send(pre(Some("sub"), "t1", &h.nested_b)).await.unwrap();
    let sub_b = started_scope(&h.take_calls());

    h.send(post(Some("sub"), "t1", &h.repo_a)).await.unwrap();
    h.send(post(None, "t2", &h.repo_a)).await.unwrap();
    assert_eq!(
        h.take_calls(),
        vec![
            call(&h.nested_b, "close", &sub_b),
            call(&h.nested_b, "close", &main_b),
        ]
    );
    let remaining = h.state_of(&h.repo_a).await.attempts;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].scope_id, main_a);

    h.send(post(None, "t1", &h.nested_b)).await.unwrap();
    assert_eq!(h.take_calls(), vec![call(&h.repo_a, "close", &main_a)]);
    assert_eq!(h.route_count().await, 0);
}

#[tokio::test]
async fn failure_and_permission_denial_terminate_through_the_original_route() {
    let h = Harness::new();
    h.send(pre(None, "t1", &h.repo_a)).await.unwrap();
    let failed = started_scope(&h.take_calls());
    h.send(pre(None, "t2", &h.repo_a)).await.unwrap();
    let denied = started_scope(&h.take_calls());

    h.send(tool_event("PostToolUseFailure", None, "t1", &h.nested_b))
        .await
        .unwrap();
    h.send(tool_event("PermissionDenied", None, "t2", &h.nested_b))
        .await
        .unwrap();

    assert_eq!(
        h.take_calls(),
        vec![
            call(&h.repo_a, "close", &failed),
            call(&h.repo_a, "abandon", &denied),
        ]
    );
    let state_a = h.state_of(&h.repo_a).await;
    assert!(state_a.attempts.is_empty());
    assert!(state_a.recovery_pending);
    assert!(!state::state_path(&h.nested_b.join(".git")).exists());
    assert_eq!(h.route_count().await, 0);
}

#[tokio::test]
async fn stop_and_subagent_stop_retire_only_their_own_routed_attempts() {
    let h = Harness::new();
    h.send(pre(None, "t1", &h.repo_a)).await.unwrap();
    let main = started_scope(&h.take_calls());
    h.send(pre(Some("sub"), "t1", &h.repo_a)).await.unwrap();
    let sub = started_scope(&h.take_calls());

    h.send(session_event("Stop", &h.nested_b)).await.unwrap();
    assert_eq!(h.take_calls(), vec![call(&h.repo_a, "abandon", &main)]);
    assert_eq!(h.route_count().await, 1);

    let subagent_stop = json!({
        "hook_event_name": "SubagentStop",
        "session_id": SESSION,
        "agent_id": "sub",
        "cwd": h.outside.to_str().unwrap(),
    });
    h.send(subagent_stop).await.unwrap();
    assert_eq!(h.take_calls(), vec![call(&h.repo_a, "abandon", &sub)]);
    assert!(h.state_of(&h.repo_a).await.attempts.is_empty());
    assert_eq!(h.route_count().await, 0);
}

#[tokio::test]
async fn failed_abandon_keeps_the_route_so_session_end_can_retry() {
    let h = Harness::new();
    h.send(pre(None, "t1", &h.repo_a)).await.unwrap();
    let scope = started_scope(&h.take_calls());

    h.fail("abandon");
    assert!(h.send(session_event("Stop", &h.nested_b)).await.is_err());
    assert_eq!(h.route_count().await, 1);
    assert_eq!(h.state_of(&h.repo_a).await.attempts.len(), 1);

    h.heal();
    h.send(session_event("SessionEnd", &h.outside))
        .await
        .unwrap();
    assert_eq!(h.take_calls(), vec![call(&h.repo_a, "abandon", &scope)]);
    assert!(h.state_of(&h.repo_a).await.attempts.is_empty());
    assert_eq!(h.route_count().await, 0);
}

#[tokio::test]
async fn worktree_remove_retires_routes_into_the_removed_worktree_only() {
    let h = Harness::new();
    git(&h.repo_a, &["config", "user.name", "SCE Test"]);
    git(&h.repo_a, &["config", "user.email", "t@example.invalid"]);
    git(&h.repo_a, &["commit", "--allow-empty", "-q", "-m", "init"]);
    let worktree = h.outside.join("wt");
    git(
        &h.repo_a,
        &["worktree", "add", "-q", worktree.to_str().unwrap()],
    );
    h.send(pre(None, "t1", &worktree)).await.unwrap();
    let in_worktree = started_scope(&h.take_calls());
    h.send(pre(None, "t2", &h.repo_a)).await.unwrap();
    started_scope(&h.take_calls());

    let worktree_remove = json!({
        "hook_event_name": "WorktreeRemove",
        "session_id": SESSION,
        "worktree_path": worktree.to_str().unwrap(),
    });
    h.send(worktree_remove).await.unwrap();

    assert_eq!(
        h.take_calls(),
        vec![call(&worktree, "abandon", &in_worktree)]
    );
    let remaining = routes::routes_for_session(&h.anchor(), SESSION)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].tool_use_id, "t2");
    assert_eq!(h.state_of(&h.repo_a).await.attempts.len(), 1);
}

#[tokio::test]
async fn post_tool_use_outside_every_repository_still_closes_through_the_route() {
    let h = Harness::new();
    h.send(pre(None, "t1", &h.repo_a)).await.unwrap();
    let scope = started_scope(&h.take_calls());

    h.send(post(None, "t1", &h.outside)).await.unwrap();

    assert_eq!(h.take_calls(), vec![call(&h.repo_a, "close", &scope)]);
    assert_eq!(h.route_count().await, 0);
}

#[tokio::test]
async fn a_missing_route_never_closes_a_routed_attempt_but_legacy_attempts_close_by_cwd() {
    let h = Harness::new();
    h.send(pre(None, "routed", &h.repo_a)).await.unwrap();
    let routed_scope = started_scope(&h.take_calls());
    let route = routes::load_route(
        &h.anchor(),
        &AttemptKey {
            session_id: SESSION.to_string(),
            agent_id: None,
            tool_use_id: "routed".to_string(),
        },
    )
    .await
    .unwrap()
    .expect("route persisted before start");
    lose_route(&h, "routed", &route).await;

    let legacy_scope = "cc-tool-v1|legacy";
    let mut seeded = h.state_of(&h.repo_a).await;
    let mut legacy = seeded.attempts[0].clone();
    legacy.scope_id = legacy_scope.to_string();
    legacy.tool_use_id = "legacy".to_string();
    seeded.attempts.push(legacy);
    let mut on_disk = serde_json::to_value(&seeded).unwrap();
    on_disk["attempts"][1]
        .as_object_mut()
        .unwrap()
        .remove("routed");
    std::fs::write(
        state::state_path(&h.anchor()),
        serde_json::to_vec(&on_disk).unwrap(),
    )
    .unwrap();

    h.send(post(None, "routed", &h.repo_a)).await.unwrap();
    h.send(post(None, "legacy", &h.repo_a)).await.unwrap();

    assert_eq!(h.take_calls(), vec![call(&h.repo_a, "close", legacy_scope)]);
    let remaining = h.state_of(&h.repo_a).await.attempts;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].scope_id, routed_scope);
}

async fn lose_route(h: &Harness, tool_use_id: &str, route: &RouteTarget) {
    let key = AttemptKey {
        session_id: SESSION.to_string(),
        agent_id: None,
        tool_use_id: tool_use_id.to_string(),
    };
    routes::remove_route(&h.anchor(), &key, route)
        .await
        .expect("route removal");
}

#[tokio::test]
async fn an_unresolvable_route_anchor_denies_pre_tool_use_before_any_start() {
    let h = Harness::new();

    let unset = h
        .send_with_project_dir(None, pre(None, "t1", &h.repo_a))
        .await
        .unwrap();
    let outside = h
        .send_with_project_dir(Some(&h.outside.clone()), pre(None, "t2", &h.repo_a))
        .await
        .unwrap();

    for response in [unset, outside] {
        let decision: Value = serde_json::from_str(&response).expect("deny JSON");
        assert_eq!(decision["hookSpecificOutput"]["permissionDecision"], "deny");
    }
    assert!(h.take_calls().is_empty());
    assert!(h.state_of(&h.repo_a).await.attempts.is_empty());
    assert_eq!(h.route_count().await, 0);
}
