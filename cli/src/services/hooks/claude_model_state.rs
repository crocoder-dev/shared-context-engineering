use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{ClaudeModelStateObservation, ObservationKind};
use crate::services::observability::traits::Logger;

use super::{
    current_unix_time_ms, normalize_claude_model_id, open_agent_trace_db_for_hook_runtime,
    prefixed_diff_trace_session_id, read_hook_stdin, CLAUDE_TOOL_NAME,
};

const SESSION_START_EVENT: &str = "SessionStart";
const POST_MODEL_SWITCH_EVENT: &str = "PostModelSwitch";
const POST_MODEL_SWITCH_SOURCES: &[&str] = &["command", "picker", "sdk", "auto", "resume"];
const ERROR_EVENT: &str = "sce.hooks.claude_model_state.error";
const DB_OPEN_FAILED_EVENT: &str = "sce.hooks.claude_model_state.agent_trace_db_open_failed";
const DB_WRITE_FAILED_EVENT: &str = "sce.hooks.claude_model_state.agent_trace_db_write_failed";

pub(super) fn run_claude_model_state_subcommand(
    repository_root: &Path,
    logger: Option<&dyn Logger>,
) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => {
            log_fail_open(logger, ERROR_EVENT, &error, None);
            return String::new();
        }
    };
    let session_id = fail_open_session_id(&stdin_payload);

    let observed_at_ms = match current_unix_time_ms() {
        Ok(observed_at_ms) => observed_at_ms,
        Err(error) => {
            log_fail_open(logger, ERROR_EVENT, &error, session_id.as_deref());
            return String::new();
        }
    };

    run_claude_model_state_from_payload(repository_root, &stdin_payload, logger, || {
        Ok(observed_at_ms)
    })
}

fn run_claude_model_state_from_payload<F>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    observed_at_ms: F,
) -> String
where
    F: FnOnce() -> Result<i64>,
{
    let session_id = fail_open_session_id(stdin_payload);
    let observed_at_ms = match observed_at_ms() {
        Ok(observed_at_ms) if observed_at_ms >= 0 => observed_at_ms,
        Ok(observed_at_ms) => {
            let error = anyhow!(
                "Invalid Claude model-state observation time: expected a non-negative millisecond value, got {observed_at_ms}."
            );
            log_fail_open(logger, ERROR_EVENT, &error, session_id.as_deref());
            return String::new();
        }
        Err(error) => {
            log_fail_open(logger, ERROR_EVENT, &error, session_id.as_deref());
            return String::new();
        }
    };

    let observation = match parse_claude_model_state_payload(stdin_payload, observed_at_ms) {
        Ok(observation) => observation,
        Err(error) => {
            log_fail_open(logger, ERROR_EVENT, &error, session_id.as_deref());
            return String::new();
        }
    };
    let Some(observation) = observation else {
        return String::new();
    };

    let db = match open_agent_trace_db_for_hook_runtime(
        repository_root,
        "Failed to open Agent Trace DB for Claude model-state persistence.",
    ) {
        Ok(db) => db,
        Err(error) => {
            log_fail_open(
                logger,
                DB_OPEN_FAILED_EVENT,
                &error,
                Some(&observation.session_id),
            );
            return String::new();
        }
    };

    if let Err(error) = persist_claude_model_state(&db, observation) {
        log_fail_open(logger, DB_WRITE_FAILED_EVENT, &error, session_id.as_deref());
    }

    String::new()
}

fn persist_claude_model_state(
    db: &RepositoryAgentTraceDb,
    observation: ClaudeModelStateObservation,
) -> Result<()> {
    db.upsert_claude_model_state(observation)
        .context("Failed to persist Claude model-state observation.")?;
    Ok(())
}

fn parse_claude_model_state_payload(
    stdin_payload: &str,
    observed_at_ms: i64,
) -> Result<Option<ClaudeModelStateObservation>> {
    let parsed: Value = serde_json::from_str(stdin_payload)
        .context("Invalid Claude model-state payload from STDIN: expected valid JSON.")?;
    let payload = parsed.as_object().ok_or_else(|| {
        anyhow!("Invalid Claude model-state payload from STDIN: expected a JSON object.")
    })?;

    let event_name = required_non_empty_string(payload, "hook_event_name")?;
    let observation_kind = match event_name.as_str() {
        SESSION_START_EVENT => ObservationKind::SessionStart,
        POST_MODEL_SWITCH_EVENT => ObservationKind::PostModelSwitch,
        _ => return Ok(None),
    };

    let session_id = prefixed_diff_trace_session_id(
        CLAUDE_TOOL_NAME,
        required_non_empty_string(payload, "session_id")?.as_str(),
    );
    let agent_id = optional_agent_id(payload)?;

    match observation_kind {
        ObservationKind::SessionStart => {
            let Some(model_id) = optional_model_id(payload, "model")? else {
                return Ok(None);
            };
            let source = required_non_empty_string(payload, "source")?;

            Ok(Some(ClaudeModelStateObservation {
                session_id,
                agent_id,
                model_id,
                observation_kind,
                source,
                observed_at_ms,
            }))
        }
        ObservationKind::PostModelSwitch => {
            let _from_model = required_model_id(payload, "from_model")?;
            let to_model = required_model_id(payload, "to_model")?;
            let source = required_non_empty_string(payload, "source")?;
            if !POST_MODEL_SWITCH_SOURCES.contains(&source.as_str()) {
                return Err(anyhow!(
                    "Invalid Claude model-state payload from STDIN: field 'source' must be one of 'command', 'picker', 'sdk', 'auto' or 'resume'."
                ));
            }

            Ok(Some(ClaudeModelStateObservation {
                session_id,
                agent_id,
                model_id: to_model,
                observation_kind,
                source,
                observed_at_ms,
            }))
        }
    }
}

fn required_model_id(payload: &serde_json::Map<String, Value>, field_name: &str) -> Result<String> {
    let value = required_non_empty_string(payload, field_name)?;
    normalize_claude_model_id(&value).ok_or_else(|| {
        anyhow!(
            "Invalid Claude model-state payload from STDIN: field '{field_name}' must be a non-empty model identifier."
        )
    })
}

fn optional_model_id(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
) -> Result<Option<String>> {
    let Some(value) = payload.get(field_name) else {
        return Ok(None);
    };

    if value.is_null() {
        return Ok(None);
    }

    let value = value.as_str().ok_or_else(|| {
        anyhow!(
            "Invalid Claude model-state payload from STDIN: field '{field_name}' must be null or a string."
        )
    })?;
    Ok(normalize_claude_model_id(value))
}

fn optional_agent_id(payload: &serde_json::Map<String, Value>) -> Result<String> {
    let Some(value) = payload.get("agent_id") else {
        return Ok(String::new());
    };
    if value.is_null() {
        return Ok(String::new());
    }

    value
        .as_str()
        .map(str::trim)
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "Invalid Claude model-state payload from STDIN: field 'agent_id' must be null or a string."
            )
        })
}

fn required_non_empty_string(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
) -> Result<String> {
    let value = payload.get(field_name).ok_or_else(|| {
        anyhow!(
            "Invalid Claude model-state payload from STDIN: missing required field '{field_name}'."
        )
    })?;
    let value = value.as_str().ok_or_else(|| {
        anyhow!(
            "Invalid Claude model-state payload from STDIN: field '{field_name}' must be a non-empty string."
        )
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!(
            "Invalid Claude model-state payload from STDIN: field '{field_name}' must be a non-empty string."
        ));
    }
    Ok(value.to_string())
}

fn fail_open_session_id(stdin_payload: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(stdin_payload).ok()?;
    let payload = payload.as_object()?;
    payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn log_fail_open(
    logger: Option<&dyn Logger>,
    event_id: &str,
    error: &anyhow::Error,
    session_id: Option<&str>,
) {
    if let Some(log) = logger {
        log.error(event_id, &error.to_string(), &[], session_id);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::agent_trace_db::ObservationKind;
    use serde_json::json;

    fn parse(payload: Value, observed_at_ms: i64) -> Option<ClaudeModelStateObservation> {
        parse_claude_model_state_payload(&payload.to_string(), observed_at_ms)
            .expect("model-state payload should parse")
    }

    #[test]
    fn session_start_normalizes_main_session_and_model() {
        let observation = parse(
            json!({
                "hook_event_name": "SessionStart",
                "session_id": "session-1",
                "model": "claude-opus-4-1",
                "source": "startup"
            }),
            42,
        )
        .expect("SessionStart should persist");

        assert_eq!(observation.session_id, "cc_session-1");
        assert_eq!(observation.agent_id, "");
        assert_eq!(observation.model_id, "claude/claude-opus-4-1");
        assert_eq!(observation.observation_kind, ObservationKind::SessionStart);
        assert_eq!(observation.source, "startup");
        assert_eq!(observation.observed_at_ms, 42);
    }

    #[test]
    fn model_less_session_start_is_a_no_op() {
        assert_eq!(
            parse(
                json!({
                    "hook_event_name": "SessionStart",
                    "session_id": "session-1",
                    "source": "resume"
                }),
                42,
            ),
            None
        );
        assert_eq!(
            parse(
                json!({
                    "hook_event_name": "SessionStart",
                    "session_id": "session-1",
                    "model": null,
                    "source": "resume"
                }),
                42,
            ),
            None
        );
        assert_eq!(
            parse(
                json!({
                    "hook_event_name": "SessionStart",
                    "session_id": "session-1",
                    "model": "   ",
                    "source": "resume"
                }),
                42,
            ),
            None
        );
    }

    #[test]
    fn post_model_switch_uses_to_model_and_exact_agent_scope() {
        let observation = parse(
            json!({
                "hook_event_name": "PostModelSwitch",
                "session_id": "cc_session-1",
                "agent_id": "agent-1",
                "from_model": "old-model",
                "to_model": "claude/new-model",
                "source": "picker"
            }),
            43,
        )
        .expect("PostModelSwitch should persist");

        assert_eq!(observation.session_id, "cc_session-1");
        assert_eq!(observation.agent_id, "agent-1");
        assert_eq!(observation.model_id, "claude/new-model");
        assert_eq!(
            observation.observation_kind,
            ObservationKind::PostModelSwitch
        );
        assert_eq!(observation.source, "picker");
    }

    #[test]
    fn post_model_switch_accepts_all_supported_sources() {
        for source in POST_MODEL_SWITCH_SOURCES {
            let observation = parse(
                json!({
                    "hook_event_name": "PostModelSwitch",
                    "session_id": "session-1",
                    "from_model": "model-a",
                    "to_model": "model-b",
                    "source": source
                }),
                43,
            )
            .expect("source should produce an observation");
            assert_eq!(observation.source, *source);
        }
    }

    #[test]
    fn malformed_switch_does_not_accept_missing_or_invalid_fields() {
        for payload in [
            json!({
                "hook_event_name": "PostModelSwitch",
                "session_id": "session-1",
                "from_model": "model-a",
                "to_model": "model-b"
            }),
            json!({
                "hook_event_name": "PostModelSwitch",
                "session_id": "session-1",
                "from_model": "model-a",
                "to_model": "model-b",
                "source": "unknown"
            }),
            json!({
                "hook_event_name": "PostModelSwitch",
                "session_id": "session-1",
                "from_model": "",
                "to_model": "model-b",
                "source": "command"
            }),
        ] {
            assert!(parse_claude_model_state_payload(&payload.to_string(), 43).is_err());
        }
    }

    #[test]
    fn unsupported_event_is_a_no_op() {
        assert_eq!(
            parse(
                json!({
                    "hook_event_name": "Stop",
                    "session_id": "session-1"
                }),
                42,
            ),
            None
        );
    }

    #[test]
    fn lifecycle_handler_returns_empty_output_for_intake_failures_and_no_ops() {
        assert_eq!(
            run_claude_model_state_from_payload(Path::new("/unused"), "not-json", None, || Ok(42),),
            ""
        );
        assert_eq!(
            run_claude_model_state_from_payload(
                Path::new("/unused"),
                &json!({
                    "hook_event_name": "SessionStart",
                    "session_id": "session-1",
                    "model": null,
                    "source": "resume"
                })
                .to_string(),
                None,
                || Ok(42),
            ),
            ""
        );
    }

    #[test]
    fn lifecycle_observation_is_written_to_the_repository_state_register() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("sce-claude-model-state-{suffix}.db"));
        let db = RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open");
        let observation = parse(
            json!({
                "hook_event_name": "PostModelSwitch",
                "session_id": "session-1",
                "from_model": "model-a",
                "to_model": "model-b",
                "source": "command"
            }),
            42,
        )
        .expect("switch should produce an observation");

        persist_claude_model_state(&db, observation).expect("state write should succeed");

        let state = db
            .claude_model_state_by_session_and_agent("cc_session-1", "")
            .expect("state lookup should succeed")
            .expect("state should be present");
        assert_eq!(state.model_id, "claude/model-b");
        assert_eq!(state.observation_kind, ObservationKind::PostModelSwitch);
        assert_eq!(state.observed_at_ms, 42);

        drop(db);
        let _ = fs::remove_file(db_path);
    }
}
