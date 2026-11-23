use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{ClaudeModelStateObservation, ObservationKind};

use super::claude_model_state;
use super::claude_transcript;
use super::diff_trace::normalize_claude_model_id;
use super::runtime::{current_unix_time_ms, DIFF_TRACE_CLAUDE_SESSION_ID_PREFIX};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ClaudeModelEvidence {
    pub(crate) session_id: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) tool_use_id: Option<String>,
    pub(crate) transcript_path: Option<PathBuf>,
    pub(crate) direct_model_id: Option<String>,
}

pub(crate) async fn resolve_claude_model(
    db: &RepositoryAgentTraceDb,
    evidence: &ClaudeModelEvidence,
) -> Result<Option<String>> {
    if let Some(model_id) = resolve_claude_model_from_exact_evidence(evidence) {
        return Ok(Some(model_id));
    }
    if evidence.agent_id.is_some() {
        return Ok(None);
    }

    if let Some(state) = db
        .claude_model_state_by_session_and_agent(&evidence.session_id, "")
        .await?
    {
        return Ok(Some(state.model_id));
    }

    Ok(seed_main_agent_model_from_bridge_chain(db, evidence).await)
}

pub(crate) fn resolve_claude_model_from_exact_evidence(
    evidence: &ClaudeModelEvidence,
) -> Option<String> {
    if let Some(model_id) = evidence
        .direct_model_id
        .as_deref()
        .and_then(normalize_claude_model_id)
    {
        return Some(model_id);
    }

    let tool_use_id = evidence.tool_use_id.as_deref()?;
    let transcript_path = evidence.transcript_path.as_deref()?;
    let transcript_path = match evidence.agent_id.as_deref() {
        Some(agent_id) => {
            claude_subagent_transcript_path(transcript_path, &evidence.session_id, agent_id)?
        }
        None => transcript_path.to_path_buf(),
    };

    claude_transcript::find_claude_transcript_tool_use_model(&transcript_path, tool_use_id)
        .ok()
        .flatten()
        .and_then(|model| normalize_claude_model_id(&model))
}

pub(crate) fn claude_subagent_transcript_path(
    transcript_path: &Path,
    session_id: &str,
    agent_id: &str,
) -> Option<PathBuf> {
    let raw_session_id = session_id
        .strip_prefix(DIFF_TRACE_CLAUDE_SESSION_ID_PREFIX)
        .unwrap_or(session_id);
    if !is_safe_path_component(raw_session_id) || !is_safe_path_component(agent_id) {
        return None;
    }

    let file_name = format!("agent-{agent_id}.jsonl");
    let names_subagent_transcript = transcript_path
        .file_name()
        .is_some_and(|name| name == file_name.as_str())
        && transcript_path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "subagents");
    if names_subagent_transcript {
        return Some(transcript_path.to_path_buf());
    }

    Some(
        transcript_path
            .parent()?
            .join(raw_session_id)
            .join("subagents")
            .join(file_name),
    )
}

fn is_safe_path_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

async fn seed_main_agent_model_from_bridge_chain(
    db: &RepositoryAgentTraceDb,
    evidence: &ClaudeModelEvidence,
) -> Option<String> {
    let transcript_path = evidence.transcript_path.as_deref()?;
    let model_id = claude_model_state::newest_bridge_chain_model(db, transcript_path).await?;

    let observed_at_ms = current_unix_time_ms().ok()?;
    match db
        .upsert_claude_model_state(ClaudeModelStateObservation {
            session_id: evidence.session_id.clone(),
            agent_id: String::new(),
            model_id: model_id.clone(),
            observation_kind: ObservationKind::SessionStart,
            source: String::from("bridge_inherited"),
            observed_at_ms,
        })
        .await
    {
        Ok(_) => Some(model_id),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_db::{ClaudeModelStateObservation, ObservationKind};

    use super::{claude_subagent_transcript_path, resolve_claude_model, ClaudeModelEvidence};

    const SESSION: &str = "5f0c2a7e-1111-4222-8333-944445555666";
    const CANONICAL_SESSION: &str = "cc_5f0c2a7e-1111-4222-8333-944445555666";
    const AGENT: &str = "a1b2c3d4e5f6";

    struct Fixture {
        _dir: tempfile::TempDir,
        project_dir: PathBuf,
        db: RepositoryAgentTraceDb,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let project_dir = dir.path().join("project");
            std::fs::create_dir_all(&project_dir).expect("project dir");
            let db = RepositoryAgentTraceDb::new_at(dir.path().join("agent-trace.db"))
                .await
                .expect("db");
            Self {
                _dir: dir,
                project_dir,
                db,
            }
        }

        fn main_transcript(&self) -> PathBuf {
            self.project_dir.join(format!("{SESSION}.jsonl"))
        }

        fn subagent_transcript(&self) -> PathBuf {
            self.project_dir
                .join(SESSION)
                .join("subagents")
                .join(format!("agent-{AGENT}.jsonl"))
        }

        async fn seed_state(&self, session_id: &str, model_id: &str) {
            self.db
                .upsert_claude_model_state(ClaudeModelStateObservation {
                    session_id: session_id.to_string(),
                    agent_id: String::new(),
                    model_id: model_id.to_string(),
                    observation_kind: ObservationKind::SessionStart,
                    source: String::from("startup"),
                    observed_at_ms: 1_000,
                })
                .await
                .expect("seed state");
        }

        async fn state_rows(&self) -> Vec<(String, String, String)> {
            self.db
                .query_map(
                    "SELECT session_id, agent_id, model_id FROM claude_model_state ORDER BY session_id, agent_id",
                    (),
                    |row| {
                        Ok((
                            row.get::<String>(0)?,
                            row.get::<String>(1)?,
                            row.get::<String>(2)?,
                        ))
                    },
                )
                .await
                .expect("read state rows")
        }

        fn main_evidence(&self, tool_use_id: &str) -> ClaudeModelEvidence {
            ClaudeModelEvidence {
                session_id: CANONICAL_SESSION.to_string(),
                agent_id: None,
                tool_use_id: Some(tool_use_id.to_string()),
                transcript_path: Some(self.main_transcript()),
                direct_model_id: None,
            }
        }

        fn subagent_evidence(&self, tool_use_id: &str) -> ClaudeModelEvidence {
            ClaudeModelEvidence {
                agent_id: Some(AGENT.to_string()),
                ..self.main_evidence(tool_use_id)
            }
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

    fn text_only(model: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"role": "assistant", "model": model, "content": [{"type": "text", "text": "hi"}]}
        })
        .to_string()
    }

    #[tokio::test]
    async fn claude_subagent_model_prefers_direct_event_model() {
        let fixture = Fixture::new().await;
        write_transcript(
            &fixture.subagent_transcript(),
            &[tool_use("toolu_a", "claude-haiku-5-5")],
        );
        let evidence = ClaudeModelEvidence {
            direct_model_id: Some(String::from("claude-sonnet-5-5")),
            ..fixture.subagent_evidence("toolu_a")
        };

        let model = resolve_claude_model(&fixture.db, &evidence).await.unwrap();

        assert_eq!(model.as_deref(), Some("claude/claude-sonnet-5-5"));
    }

    #[tokio::test]
    async fn claude_subagent_model_uses_exact_record_not_latest_record() {
        let fixture = Fixture::new().await;
        write_transcript(
            &fixture.subagent_transcript(),
            &[
                tool_use("toolu_target", "claude-haiku-5-5"),
                tool_use("toolu_later", "claude-opus-5-5"),
                text_only("claude-opus-5-5"),
            ],
        );

        let model = resolve_claude_model(&fixture.db, &fixture.subagent_evidence("toolu_target"))
            .await
            .unwrap();

        assert_eq!(model.as_deref(), Some("claude/claude-haiku-5-5"));
    }

    #[tokio::test]
    async fn claude_subagent_model_is_null_when_exact_record_is_absent() {
        let fixture = Fixture::new().await;
        write_transcript(
            &fixture.subagent_transcript(),
            &[
                tool_use("toolu_other", "claude-opus-5-5"),
                text_only("claude-opus-5-5"),
            ],
        );

        let model = resolve_claude_model(&fixture.db, &fixture.subagent_evidence("toolu_target"))
            .await
            .unwrap();

        assert_eq!(model, None);
    }

    #[tokio::test]
    async fn claude_subagent_model_follows_each_tool_call_across_a_model_switch() {
        let fixture = Fixture::new().await;
        write_transcript(
            &fixture.subagent_transcript(),
            &[
                tool_use("toolu_a", "claude-sonnet-5-5"),
                tool_use("toolu_b", "claude-opus-5-5"),
            ],
        );

        let mut resolved = Vec::new();
        for tool_use_id in ["toolu_a", "toolu_b", "toolu_c"] {
            resolved.push(
                resolve_claude_model(&fixture.db, &fixture.subagent_evidence(tool_use_id))
                    .await
                    .unwrap(),
            );
        }

        assert_eq!(
            resolved,
            vec![
                Some(String::from("claude/claude-sonnet-5-5")),
                Some(String::from("claude/claude-opus-5-5")),
                None,
            ]
        );
    }

    #[tokio::test]
    async fn claude_subagent_model_never_reads_or_writes_claude_model_state() {
        let fixture = Fixture::new().await;
        fixture
            .seed_state(CANONICAL_SESSION, "claude/claude-opus-5-5")
            .await;
        write_transcript(
            &fixture.main_transcript(),
            &[tool_use("toolu_target", "claude-opus-5-5")],
        );
        let before = fixture.state_rows().await;

        let model = resolve_claude_model(&fixture.db, &fixture.subagent_evidence("toolu_target"))
            .await
            .unwrap();

        assert_eq!(model, None);
        assert_eq!(fixture.state_rows().await, before);
    }

    #[tokio::test]
    async fn claude_subagent_model_never_uses_bridge_session_chain() {
        let fixture = Fixture::new().await;
        let bridge_member = write_bridge_chain(&fixture);
        fixture
            .seed_state(&bridge_member, "claude/claude-opus-5-5")
            .await;
        let before = fixture.state_rows().await;

        let model = resolve_claude_model(&fixture.db, &fixture.subagent_evidence("toolu_target"))
            .await
            .unwrap();

        assert_eq!(model, None);
        assert_eq!(fixture.state_rows().await, before);
    }

    #[tokio::test]
    async fn claude_subagent_model_missing_transcript_is_null() {
        let fixture = Fixture::new().await;

        let model = resolve_claude_model(&fixture.db, &fixture.subagent_evidence("toolu_target"))
            .await
            .unwrap();

        assert_eq!(model, None);
    }

    #[test]
    fn claude_subagent_model_transcript_path_derivation() {
        let main = Path::new("/projects/repo/5f0c2a7e.jsonl");
        assert_eq!(
            claude_subagent_transcript_path(main, "cc_5f0c2a7e", "abc123"),
            Some(PathBuf::from(
                "/projects/repo/5f0c2a7e/subagents/agent-abc123.jsonl"
            ))
        );

        let already_subagent = Path::new("/projects/repo/5f0c2a7e/subagents/agent-abc123.jsonl");
        assert_eq!(
            claude_subagent_transcript_path(already_subagent, "cc_5f0c2a7e", "abc123"),
            Some(already_subagent.to_path_buf())
        );

        let other_subagent = Path::new("/projects/repo/5f0c2a7e/subagents/agent-zzz999.jsonl");
        assert_eq!(
            claude_subagent_transcript_path(other_subagent, "cc_5f0c2a7e", "abc123"),
            Some(PathBuf::from(
                "/projects/repo/5f0c2a7e/subagents/5f0c2a7e/subagents/agent-abc123.jsonl"
            ))
        );

        assert_eq!(
            claude_subagent_transcript_path(main, "cc_5f0c2a7e", "../escape"),
            None
        );
        assert_eq!(
            claude_subagent_transcript_path(main, "cc_../x", "abc123"),
            None
        );
    }

    #[tokio::test]
    async fn claude_model_attribution_main_agent_prefers_direct_model_over_transcript() {
        let fixture = Fixture::new().await;
        write_transcript(
            &fixture.main_transcript(),
            &[tool_use("toolu_a", "claude-haiku-5-5")],
        );
        let evidence = ClaudeModelEvidence {
            direct_model_id: Some(String::from("claude/claude-sonnet-5-5")),
            ..fixture.main_evidence("toolu_a")
        };

        let model = resolve_claude_model(&fixture.db, &evidence).await.unwrap();

        assert_eq!(model.as_deref(), Some("claude/claude-sonnet-5-5"));
    }

    #[tokio::test]
    async fn claude_model_attribution_main_agent_prefers_transcript_over_state_row() {
        let fixture = Fixture::new().await;
        fixture
            .seed_state(CANONICAL_SESSION, "claude/claude-opus-5-5")
            .await;
        write_transcript(
            &fixture.main_transcript(),
            &[tool_use("toolu_a", "claude-haiku-5-5")],
        );

        let model = resolve_claude_model(&fixture.db, &fixture.main_evidence("toolu_a"))
            .await
            .unwrap();

        assert_eq!(model.as_deref(), Some("claude/claude-haiku-5-5"));
    }

    #[tokio::test]
    async fn claude_model_attribution_main_agent_falls_back_to_state_row() {
        let fixture = Fixture::new().await;
        fixture
            .seed_state(CANONICAL_SESSION, "claude/claude-opus-5-5")
            .await;
        write_transcript(
            &fixture.main_transcript(),
            &[tool_use("toolu_other", "claude-haiku-5-5")],
        );

        let model = resolve_claude_model(&fixture.db, &fixture.main_evidence("toolu_a"))
            .await
            .unwrap();

        assert_eq!(model.as_deref(), Some("claude/claude-opus-5-5"));
    }

    #[tokio::test]
    async fn claude_model_attribution_main_agent_falls_back_to_bridge_chain_and_seeds_state() {
        let fixture = Fixture::new().await;
        let bridge_member = write_bridge_chain(&fixture);
        fixture
            .seed_state(&bridge_member, "claude/claude-opus-5-5")
            .await;

        let model = resolve_claude_model(&fixture.db, &fixture.main_evidence("toolu_a"))
            .await
            .unwrap();

        assert_eq!(model.as_deref(), Some("claude/claude-opus-5-5"));
        let seeded = fixture
            .db
            .claude_model_state_by_session_and_agent(CANONICAL_SESSION, "")
            .await
            .unwrap()
            .expect("bridge model seeded for the main agent");
        assert_eq!(seeded.model_id, "claude/claude-opus-5-5");
        assert_eq!(seeded.source, "bridge_inherited");
    }

    fn write_bridge_chain(fixture: &Fixture) -> String {
        let bridge_record =
            serde_json::json!({"type": "bridge-session", "bridgeSessionId": "bridge-1"})
                .to_string();
        write_transcript(
            &fixture.main_transcript(),
            std::slice::from_ref(&bridge_record),
        );
        let member_session = "0a0a0a0a-2222-4333-8444-955556666777";
        write_transcript(
            &fixture.project_dir.join(format!("{member_session}.jsonl")),
            &[bridge_record],
        );
        format!("cc_{member_session}")
    }
}
