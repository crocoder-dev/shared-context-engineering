use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
};

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

struct PiRepo {
    _temp: tempfile::TempDir,
    root: PathBuf,
    state_root: PathBuf,
}

impl PiRepo {
    fn new(label: &str) -> Self {
        let temp = tempfile::Builder::new()
            .prefix(&format!("sce-pi-mutation-scope-seam-{label}-"))
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

    fn cwd(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn drive(&self, payload: &str) -> Result<String> {
        run_pi_mutation_scope_from_payload_at_state_root(&self.state_root, payload, None)
    }

    fn write(&self, name: &str, contents: &str) {
        fs::write(self.root.join(name), contents).expect("write should succeed");
    }

    fn db(&self) -> RepositoryAgentTraceDb {
        crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
            &self.root,
            &self.state_root,
            "Pi mutation-scope seam test assertions",
        )
        .expect("assertion DB should open")
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

    fn scope_provenance(&self, scope_id: &str) -> Option<(String, Option<String>)> {
        self.db()
            .query_map(
                "SELECT session_id, model_id FROM mutation_trace_scope_provenance \
                     WHERE scope_id = ?1",
                (scope_id,),
                |row| {
                    let session_id = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let model_id = row.get::<Option<String>>(1).map_err(anyhow::Error::from)?;
                    Ok((session_id, model_id))
                },
            )
            .expect("scope-provenance query should succeed")
            .into_iter()
            .next()
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

fn tool_call(repo: &PiRepo, tool_name: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolCall",
        "session_id": "01a091f4-seam-session",
        "tool_call_id": tool_call_id,
        "cwd": repo.cwd(),
        "tool_name": tool_name,
        "model": "openai-codex/gpt-5.5",
    })
    .to_string()
}

fn tool_result(repo: &PiRepo, tool_name: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolResult",
        "session_id": "01a091f4-seam-session",
        "tool_call_id": tool_call_id,
        "cwd": repo.cwd(),
        "tool_name": tool_name,
    })
    .to_string()
}

fn tool_execution_end(repo: &PiRepo, tool_name: &str, tool_call_id: &str) -> String {
    json!({
        "hook_event_name": "ToolExecutionEnd",
        "session_id": "01a091f4-seam-session",
        "tool_call_id": tool_call_id,
        "cwd": repo.cwd(),
        "tool_name": tool_name,
    })
    .to_string()
}

#[test]
fn a_write_start_result_close_lands_a_real_ai_exclusive_event_with_pi_provenance() {
    let repo = PiRepo::new("real-lifecycle");
    let key = AttemptKey {
        session_id: "01a091f4-seam-session".to_string(),
        tool_call_id: "call_write".to_string(),
    };
    let scope_id = format_pi_scope_id(&key, 1);

    repo.drive(&tool_call(&repo, "write", "call_write"))
        .expect("Start should reach the real runtime");
    assert_eq!(
        repo.scope_status(&scope_id),
        Some(("pi".to_string(), "active".to_string()))
    );
    assert_eq!(
        repo.scope_provenance(&scope_id),
        Some((
            "pi_01a091f4-seam-session".to_string(),
            Some("openai-codex/gpt-5.5".to_string())
        ))
    );

    repo.write("file.txt", "one\ntwo\n");
    repo.drive(&tool_result(&repo, "write", "call_write"))
        .expect("tool_result should mark Executed");

    repo.drive(&tool_execution_end(&repo, "write", "call_write"))
        .expect("Close should reach the real runtime");

    assert_eq!(
        repo.scope_status(&scope_id),
        Some(("pi".to_string(), "closed".to_string()))
    );
    assert_eq!(
        repo.mutation_events(),
        vec![("ai_exclusive".to_string(), Some(scope_id))]
    );
    assert!(
        state::read_state(&resolve_git_dir(&repo.root).expect("git dir resolves"))
            .expect("state readable")
            .attempts
            .is_empty()
    );
}

#[test]
fn a_start_followed_by_no_execution_abandons_through_the_real_runtime() {
    let repo = PiRepo::new("real-abandon");
    let key = AttemptKey {
        session_id: "01a091f4-seam-session".to_string(),
        tool_call_id: "call_blocked".to_string(),
    };
    let scope_id = format_pi_scope_id(&key, 1);

    repo.drive(&tool_call(&repo, "bash", "call_blocked"))
        .expect("Start should reach the real runtime");

    repo.drive(&tool_execution_end(&repo, "bash", "call_blocked"))
        .expect("the terminal event must resolve via abandon, not surface an error");

    assert_eq!(
        repo.scope_status(&scope_id),
        Some(("pi".to_string(), "abandoned".to_string()))
    );
    assert!(repo.mutation_events().is_empty());
}
