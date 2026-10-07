use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{ClaudeModelStateObservation, ObservationKind};

use super::{
    current_unix_time_ms, normalize_claude_model_id, prefixed_diff_trace_session_id,
    read_hook_stdin, CLAUDE_TOOL_NAME,
};

const SESSION_START_EVENT: &str = "SessionStart";
const POST_MODEL_SWITCH_EVENT: &str = "PostModelSwitch";
const ERROR_EVENT: &str = "sce.hooks.claude_model_state.error";
const DB_OPEN_FAILED_EVENT: &str = "sce.hooks.claude_model_state.agent_trace_db_open_failed";
const DB_WRITE_FAILED_EVENT: &str = "sce.hooks.claude_model_state.agent_trace_db_write_failed";

struct BridgeInheritanceCandidate {
    session: String,
    agent: String,
    transcript_path: PathBuf,
}

pub(super) async fn run_claude_model_state_subcommand<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    logger: Option<&L>,
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
    .await
}

async fn run_claude_model_state_from_payload<L: crate::services::observability::traits::Logger, F>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
    observed_at_ms: F,
) -> String
where
    F: FnOnce() -> Result<i64>,
{
    run_claude_model_state_from_payload_with(
        repository_root,
        stdin_payload,
        logger,
        observed_at_ms,
        super::open_agent_trace_db_for_hook_runtime,
    )
    .await
}

async fn run_claude_model_state_from_payload_with<
    L: crate::services::observability::traits::Logger,
    F,
    O,
>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
    observed_at_ms: F,
    open_db: O,
) -> String
where
    F: FnOnce() -> Result<i64>,
    O: std::ops::AsyncFnOnce(&Path, &'static str) -> Result<RepositoryAgentTraceDb>,
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

    let bridge_candidate = if observation.is_none() {
        match bridge_inheritance_candidate(stdin_payload) {
            Ok(candidate) => candidate,
            Err(error) => {
                log_fail_open(logger, ERROR_EVENT, &error, session_id.as_deref());
                return String::new();
            }
        }
    } else {
        None
    };
    if observation.is_none() && bridge_candidate.is_none() {
        return String::new();
    }

    let db = match open_db(
        repository_root,
        "Failed to open Agent Trace DB for Claude model-state persistence.",
    )
    .await
    {
        Ok(db) => db,
        Err(error) => {
            log_fail_open(
                logger,
                DB_OPEN_FAILED_EVENT,
                &error,
                observation
                    .as_ref()
                    .map(|observation| observation.session_id.as_str())
                    .or(session_id.as_deref()),
            );
            return String::new();
        }
    };

    let observation = if let Some(observation) = observation {
        observation
    } else {
        let candidate = bridge_candidate
            .expect("bridge candidate must exist when no direct observation exists");
        let Some(model_id) = newest_bridge_chain_model(&db, &candidate.transcript_path).await
        else {
            return String::new();
        };

        ClaudeModelStateObservation {
            session_id: candidate.session,
            agent_id: candidate.agent,
            model_id,
            observation_kind: ObservationKind::SessionStart,
            source: String::from("bridge_inherited"),
            observed_at_ms,
        }
    };

    if let Err(error) = persist_claude_model_state(&db, observation).await {
        log_fail_open(logger, DB_WRITE_FAILED_EVENT, &error, session_id.as_deref());
    }

    String::new()
}

fn bridge_inheritance_candidate(stdin_payload: &str) -> Result<Option<BridgeInheritanceCandidate>> {
    let parsed: Value = serde_json::from_str(stdin_payload)
        .context("Invalid Claude model-state payload from STDIN: expected valid JSON.")?;
    let payload = parsed.as_object().ok_or_else(|| {
        anyhow!("Invalid Claude model-state payload from STDIN: expected a JSON object.")
    })?;

    if required_non_empty_string(payload, "hook_event_name")?.as_str() != SESSION_START_EVENT {
        return Ok(None);
    }
    if optional_model_id(payload, "model")?.is_some() {
        return Ok(None);
    }

    required_non_empty_string(payload, "source")?;
    let session_id = prefixed_diff_trace_session_id(
        CLAUDE_TOOL_NAME,
        required_non_empty_string(payload, "session_id")?.as_str(),
    );
    let agent_id = optional_agent_id(payload)?;
    let Some(transcript_path) = payload
        .get("transcript_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
    else {
        return Ok(None);
    };
    if super::claude_bridge_session::extract_claude_bridge_session_id(&transcript_path).is_none() {
        return Ok(None);
    }

    Ok(Some(BridgeInheritanceCandidate {
        session: session_id,
        agent: agent_id,
        transcript_path,
    }))
}

pub(super) async fn newest_bridge_chain_model(
    db: &RepositoryAgentTraceDb,
    transcript_path: &Path,
) -> Option<String> {
    let bridge_session_id =
        super::claude_bridge_session::extract_claude_bridge_session_id(transcript_path)?;
    let members = super::claude_bridge_session::find_claude_bridge_chain_session_ids(
        transcript_path,
        &bridge_session_id,
    );

    let mut winner: Option<(i64, String, String)> = None;
    for member in members {
        let member_session_id = prefixed_diff_trace_session_id(CLAUDE_TOOL_NAME, &member);
        let Ok(Some(state)) = db
            .claude_model_state_by_session_and_agent(&member_session_id, "")
            .await
        else {
            continue;
        };

        let should_replace = match &winner {
            None => true,
            Some((best_ms, best_session_id, _)) => {
                state.observed_at_ms > *best_ms
                    || (state.observed_at_ms == *best_ms && member_session_id > *best_session_id)
            }
        };
        if should_replace {
            winner = Some((state.observed_at_ms, member_session_id, state.model_id));
        }
    }

    winner.map(|(_, _, model_id)| model_id)
}

async fn persist_claude_model_state(
    db: &RepositoryAgentTraceDb,
    observation: ClaudeModelStateObservation,
) -> Result<()> {
    db.upsert_claude_model_state(observation)
        .await
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

    let value = value.as_str().ok_or_else(|| {
        anyhow!(
            "Invalid Claude model-state payload from STDIN: field 'agent_id' must be null or a non-empty string."
        )
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!(
            "Invalid Claude model-state payload from STDIN: field 'agent_id' must be non-empty when present."
        ));
    }
    Ok(value.to_string())
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

fn log_fail_open<L: crate::services::observability::traits::Logger>(
    logger: Option<&L>,
    event_id: &str,
    error: &anyhow::Error,
    session_id: Option<&str>,
) {
    if let Some(log) = logger {
        log.error(event_id, &format!("{error:#}"), &[], session_id);
    }
}
