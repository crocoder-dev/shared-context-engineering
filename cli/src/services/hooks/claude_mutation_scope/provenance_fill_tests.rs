use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use serde_json::Value;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{ClaudeModelStateObservation, ObservationKind};
use crate::services::error::CliError;
use crate::services::hooks::claude_model_resolver::{resolve_claude_model, ClaudeModelEvidence};
use crate::services::mutation_trace::store::{
    MutationTraceStore, ScopeProvenance, ScopeProvenanceModelFill,
};
use crate::services::mutation_trace::types::{ActorKind, ScopeId, TreeId, WorktreeId};
use crate::services::observability::traits::Logger;

use super::lifecycle::{
    establish_start, fill_claude_scope_provenance_model, handle_close,
    PROVENANCE_MODEL_FILL_FAILED_EVENT,
};
use super::{state, ClaudeToolIdentity};

const SESSION: &str = "7d1e9b20-aaaa-4bbb-8ccc-1234567890ab";
const CANONICAL_SESSION: &str = "cc_7d1e9b20-aaaa-4bbb-8ccc-1234567890ab";
const AGENT: &str = "f00dfeed1234";
const TOOL_USE: &str = "toolu_write_1";
const HAIKU: &str = "claude-haiku-5-5";
const OPUS: &str = "claude-opus-5-5";
const MAIN_MODEL: &str = "claude-sonnet-5-5";

#[derive(Default)]
struct RecordingLogger {
    warning_events: Mutex<Vec<String>>,
}

impl Logger for RecordingLogger {
    fn info(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
    fn debug(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
    fn warn(&self, event_id: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {
        self.warning_events
            .lock()
            .unwrap()
            .push(event_id.to_owned());
    }
    fn error(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
    fn log_cli_error(&self, _: &CliError, _: Option<&str>) {}
}

struct Fixture {
    dir: tempfile::TempDir,
    db: RepositoryAgentTraceDb,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("project")).expect("project dir");
        std::fs::create_dir_all(dir.path().join("git")).expect("git dir");
        let db = RepositoryAgentTraceDb::new_at(dir.path().join("agent-trace.db"))
            .await
            .expect("db");
        Self { dir, db }
    }

    fn git_dir(&self) -> PathBuf {
        self.dir.path().join("git")
    }

    fn repository_root(&self) -> PathBuf {
        self.dir.path().join("not-a-repository")
    }

    fn main_transcript(&self) -> PathBuf {
        self.dir
            .path()
            .join("project")
            .join(format!("{SESSION}.jsonl"))
    }

    fn subagent_transcript(&self) -> PathBuf {
        self.dir
            .path()
            .join("project")
            .join(SESSION)
            .join("subagents")
            .join(format!("agent-{AGENT}.jsonl"))
    }

    fn identity(&self, agent_id: Option<&str>) -> ClaudeToolIdentity {
        ClaudeToolIdentity {
            session_id: SESSION.to_owned(),
            cwd: self.repository_root().display().to_string(),
            agent_id: agent_id.map(str::to_owned),
            tool_name: String::from("Write"),
            tool_use_id: TOOL_USE.to_owned(),
            transcript_path: Some(self.main_transcript().display().to_string()),
            direct_model_id: None,
        }
    }

    async fn seed_main_agent_state(&self) {
        self.db
            .upsert_claude_model_state(ClaudeModelStateObservation {
                session_id: CANONICAL_SESSION.to_owned(),
                agent_id: String::new(),
                model_id: MAIN_MODEL.to_owned(),
                observation_kind: ObservationKind::SessionStart,
                source: String::from("startup"),
                observed_at_ms: 1_000,
            })
            .await
            .expect("seed main-agent state");
    }

    async fn admit(&mut self, identity: &ClaudeToolIdentity) -> ScopeId {
        let payloads = RefCell::new(Vec::<String>::new());
        let db = &self.db;
        let resolver = async |_: &Path, evidence: &ClaudeModelEvidence| {
            resolve_claude_model(db, evidence).await
        };
        let seam = async |_: &Path, payload: &str, _: Option<&RecordingLogger>| -> Result<String> {
            payloads.borrow_mut().push(payload.to_owned());
            Ok(String::new())
        };
        establish_start(
            &self.git_dir(),
            &self.repository_root(),
            identity,
            None::<&RecordingLogger>,
            &resolver,
            &seam,
        )
        .await
        .expect("establish start");

        let start: Value =
            serde_json::from_str(&payloads.borrow()[0]).expect("start payload is JSON");
        let scope_id = ScopeId(start["scope_id"].as_str().unwrap().to_owned());
        let provenance = ScopeProvenance {
            scope_id: scope_id.clone(),
            session_id: start["provenance"]["session_id"]
                .as_str()
                .unwrap()
                .to_owned(),
            model_id: start["provenance"]["model_id"].as_str().map(str::to_owned),
        };

        let worktree = WorktreeId(String::from("main"));
        let store = MutationTraceStore::new(&mut self.db);
        store
            .initialize_worktree(&worktree, &TreeId(String::from("tree-0")))
            .await
            .expect("worktree");
        store
            .register_scope(&scope_id, &worktree, ActorKind::ClaudeCode)
            .await
            .expect("scope");
        store
            .register_scope_provenance(&provenance)
            .await
            .expect("provenance");
        scope_id
    }

    async fn complete(
        &mut self,
        scope_id: &ScopeId,
        identity: &ClaudeToolIdentity,
    ) -> Option<ScopeProvenanceModelFill> {
        fill_claude_scope_provenance_model(&mut self.db, &scope_id.0, &identity.model_evidence())
            .await
            .expect("fill")
    }

    async fn stored(&mut self, scope_id: &ScopeId) -> ScopeProvenance {
        MutationTraceStore::new(&mut self.db)
            .load_scope_provenance(scope_id)
            .await
            .expect("load provenance")
            .expect("provenance row")
    }

    async fn state_row_count(&self) -> usize {
        self.db
            .query_map("SELECT session_id FROM claude_model_state", (), |row| {
                Ok(row.get::<String>(0)?)
            })
            .await
            .expect("state rows")
            .len()
    }
}

fn write_transcript(path: &Path, records: &[String]) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("transcript dir");
    let mut contents = records.join("\n");
    contents.push('\n');
    std::fs::write(path, contents).expect("write transcript");
}

fn tool_use(tool_use_id: &str, model: &str) -> String {
    serde_json::json!({
        "type": "assistant",
        "isSidechain": true,
        "agentId": AGENT,
        "message": {
            "role": "assistant",
            "model": model,
            "content": [{"type": "tool_use", "id": tool_use_id, "name": "Write"}]
        }
    })
    .to_string()
}

fn claude(model: &str) -> String {
    format!("claude/{model}")
}

#[tokio::test]
async fn claude_provenance_fill_keeps_model_admitted_from_exact_record() {
    let mut fixture = Fixture::new().await;
    let identity = fixture.identity(Some(AGENT));
    write_transcript(&fixture.subagent_transcript(), &[tool_use(TOOL_USE, HAIKU)]);

    let scope_id = fixture.admit(&identity).await;
    assert_eq!(
        fixture.stored(&scope_id).await.model_id,
        Some(claude(HAIKU))
    );

    write_transcript(&fixture.subagent_transcript(), &[tool_use(TOOL_USE, OPUS)]);
    let outcome = fixture.complete(&scope_id, &identity).await;

    assert_eq!(outcome, Some(ScopeProvenanceModelFill::AlreadySet));
    let stored = fixture.stored(&scope_id).await;
    assert_eq!(stored.model_id, Some(claude(HAIKU)));
    assert_eq!(stored.session_id, CANONICAL_SESSION);
}

#[tokio::test]
async fn claude_provenance_fill_sets_model_when_record_appears_by_completion() {
    let mut fixture = Fixture::new().await;
    fixture.seed_main_agent_state().await;
    let identity = fixture.identity(Some(AGENT));
    write_transcript(
        &fixture.subagent_transcript(),
        &[tool_use("toolu_earlier", OPUS)],
    );

    let scope_id = fixture.admit(&identity).await;
    assert_eq!(fixture.stored(&scope_id).await.model_id, None);

    write_transcript(
        &fixture.subagent_transcript(),
        &[tool_use("toolu_earlier", OPUS), tool_use(TOOL_USE, HAIKU)],
    );
    let outcome = fixture.complete(&scope_id, &identity).await;

    assert_eq!(outcome, Some(ScopeProvenanceModelFill::Filled));
    let stored = fixture.stored(&scope_id).await;
    assert_eq!(stored.model_id, Some(claude(HAIKU)));
    assert_eq!(stored.session_id, CANONICAL_SESSION);
    assert_eq!(fixture.state_row_count().await, 1);
}

#[tokio::test]
async fn claude_provenance_fill_leaves_null_when_record_is_missing_at_both_boundaries() {
    let mut fixture = Fixture::new().await;
    let identity = fixture.identity(Some(AGENT));
    write_transcript(
        &fixture.subagent_transcript(),
        &[tool_use("toolu_other", OPUS)],
    );

    let scope_id = fixture.admit(&identity).await;
    let outcome = fixture.complete(&scope_id, &identity).await;

    assert_eq!(outcome, None);
    assert_eq!(fixture.stored(&scope_id).await.model_id, None);
}

#[tokio::test]
async fn claude_provenance_fill_never_overwrites_a_stored_model() {
    let mut fixture = Fixture::new().await;
    let identity = ClaudeToolIdentity {
        direct_model_id: Some(OPUS.to_owned()),
        ..fixture.identity(Some(AGENT))
    };
    let scope_id = fixture.admit(&identity).await;
    assert_eq!(fixture.stored(&scope_id).await.model_id, Some(claude(OPUS)));

    write_transcript(&fixture.subagent_transcript(), &[tool_use(TOOL_USE, HAIKU)]);
    let completion = ClaudeToolIdentity {
        direct_model_id: None,
        ..identity
    };
    let outcome = fixture.complete(&scope_id, &completion).await;

    assert_eq!(outcome, Some(ScopeProvenanceModelFill::AlreadySet));
    assert_eq!(fixture.stored(&scope_id).await.model_id, Some(claude(OPUS)));
}

#[tokio::test]
async fn claude_provenance_fill_ignores_model_state_and_bridge_chain() {
    let mut fixture = Fixture::new().await;
    let identity = fixture.identity(None);
    write_transcript(&fixture.main_transcript(), &[tool_use("toolu_other", OPUS)]);
    let scope_id = fixture.admit(&identity).await;
    assert_eq!(fixture.stored(&scope_id).await.model_id, None);

    fixture.seed_main_agent_state().await;
    let outcome = fixture.complete(&scope_id, &identity).await;

    assert_eq!(outcome, None);
    assert_eq!(fixture.stored(&scope_id).await.model_id, None);
}

#[tokio::test]
async fn claude_provenance_fill_failure_does_not_fail_close() {
    let fixture = Fixture::new().await;
    let identity = fixture.identity(Some(AGENT));
    let git_dir = fixture.git_dir();
    let allocated = state::allocate_attempt(&git_dir, &identity.attempt_key(), "Write")
        .await
        .expect("allocate");
    state::mark_active(&git_dir, &allocated.attempt.scope_id)
        .await
        .expect("mark active");
    let operations = RefCell::new(Vec::<String>::new());
    let seam = async |_: &Path, payload: &str, _: Option<&RecordingLogger>| -> Result<String> {
        let payload: Value = serde_json::from_str(payload)?;
        operations
            .borrow_mut()
            .push(payload["operation"].as_str().unwrap_or_default().to_owned());
        Ok(String::new())
    };
    let logger = RecordingLogger::default();

    let output = handle_close(
        &git_dir,
        &fixture.repository_root(),
        &identity,
        Some(&logger),
        &seam,
    )
    .await
    .expect("close succeeds");
    let duplicate = handle_close(
        &git_dir,
        &fixture.repository_root(),
        &identity,
        Some(&logger),
        &seam,
    )
    .await
    .expect("duplicate close succeeds");

    assert_eq!(output, "");
    assert_eq!(duplicate, "");
    assert_eq!(*operations.borrow(), vec![String::from("close")]);
    assert!(state::read_state(&git_dir)
        .await
        .expect("state")
        .attempts
        .is_empty());
    assert_eq!(
        *logger.warning_events.lock().unwrap(),
        vec![PROVENANCE_MODEL_FILL_FAILED_EVENT.to_owned()]
    );
}
