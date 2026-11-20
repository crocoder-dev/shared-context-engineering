use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{
    ClaudeModelStateObservation, ClaudeSessionAuthState, ObservationKind,
};

use super::claude_auth_detection::{detect_claude_auth, ClaudeAuthSignals};
use super::{
    current_unix_time_ms, normalize_claude_model_id, prefixed_diff_trace_session_id,
    read_hook_stdin, CLAUDE_TOOL_NAME,
};

const SESSION_START_EVENT: &str = "SessionStart";
const POST_MODEL_SWITCH_EVENT: &str = "PostModelSwitch";
const ERROR_EVENT: &str = "sce.hooks.claude_model_state.error";
const DB_OPEN_FAILED_EVENT: &str = "sce.hooks.claude_model_state.agent_trace_db_open_failed";
const DB_WRITE_FAILED_EVENT: &str = "sce.hooks.claude_model_state.agent_trace_db_write_failed";
const AUTH_DB_WRITE_FAILED_EVENT: &str =
    "sce.hooks.claude_model_state.auth_state_agent_trace_db_write_failed";

#[derive(Default)]
struct ClaudeModelStatePayload {
    observation: Option<ClaudeModelStateObservation>,
    session_start_session_id: Option<String>,
}

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
    crate::services::managed_telemetry::trace_export::record_claude_hook_command(
        crate::services::managed_telemetry::trace_export::ClaudeHookCommand::ClaudeModelState,
    );
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => {
            log_fail_open(logger, ERROR_EVENT, &error, None);
            return String::new();
        }
    };
    crate::services::managed_telemetry::trace_export::record_claude_hook_payload(&stdin_payload);
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
        |name| std::env::var(name).ok(),
    )
    .await
}

async fn run_claude_model_state_from_payload_with<
    L: crate::services::observability::traits::Logger,
    F,
    O,
    E,
>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
    observed_at_ms: F,
    open_db: O,
    env_lookup: E,
) -> String
where
    F: FnOnce() -> Result<i64>,
    O: std::ops::AsyncFnOnce(&Path, &'static str) -> Result<RepositoryAgentTraceDb>,
    E: Fn(&str) -> Option<String>,
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

    let ClaudeModelStatePayload {
        observation,
        session_start_session_id,
    } = match parse_claude_model_state_payload(stdin_payload, observed_at_ms) {
        Ok(payload) => payload,
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
    let auth_state = session_start_session_id
        .map(|session_id| claude_session_auth_state(session_id, env_lookup, observed_at_ms));
    if observation.is_none() && bridge_candidate.is_none() && auth_state.is_none() {
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

    let observation = match (observation, bridge_candidate) {
        (Some(observation), _) => Some(observation),
        (None, Some(candidate)) => newest_bridge_chain_model(&db, &candidate.transcript_path)
            .await
            .map(|model_id| ClaudeModelStateObservation {
                session_id: candidate.session,
                agent_id: candidate.agent,
                model_id,
                observation_kind: ObservationKind::SessionStart,
                source: String::from("bridge_inherited"),
                observed_at_ms,
            }),
        (None, None) => None,
    };

    if let Some(observation) = observation {
        if let Err(error) = persist_claude_model_state(&db, observation).await {
            log_fail_open(logger, DB_WRITE_FAILED_EVENT, &error, session_id.as_deref());
        }
    }

    if let Some(auth_state) = auth_state {
        persist_claude_session_auth_state(&db, auth_state, logger).await;
    }

    String::new()
}

fn claude_session_auth_state<E>(
    session_id: String,
    env_lookup: E,
    observed_at_ms: i64,
) -> ClaudeSessionAuthState
where
    E: Fn(&str) -> Option<String>,
{
    let detection = detect_claude_auth(ClaudeAuthSignals::from_env_lookup(env_lookup));
    ClaudeSessionAuthState {
        session_id,
        provider: detection.provider,
        billing_mode: detection.billing_mode,
        auth_env_key: detection.auth_env_key,
        confidence: detection.confidence,
        observed_at_ms,
    }
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

async fn persist_claude_session_auth_state<L: crate::services::observability::traits::Logger>(
    db: &RepositoryAgentTraceDb,
    auth_state: ClaudeSessionAuthState,
    logger: Option<&L>,
) {
    let session_id = auth_state.session_id.clone();
    if let Err(error) = db
        .upsert_claude_session_auth_state(auth_state)
        .await
        .context("Failed to persist Claude session auth state.")
    {
        log_fail_open(
            logger,
            AUTH_DB_WRITE_FAILED_EVENT,
            &error,
            Some(&session_id),
        );
    }
}

fn parse_claude_model_state_payload(
    stdin_payload: &str,
    observed_at_ms: i64,
) -> Result<ClaudeModelStatePayload> {
    let parsed: Value = serde_json::from_str(stdin_payload)
        .context("Invalid Claude model-state payload from STDIN: expected valid JSON.")?;
    let payload = parsed.as_object().ok_or_else(|| {
        anyhow!("Invalid Claude model-state payload from STDIN: expected a JSON object.")
    })?;

    let event_name = required_non_empty_string(payload, "hook_event_name")?;
    let observation_kind = match event_name.as_str() {
        SESSION_START_EVENT => ObservationKind::SessionStart,
        POST_MODEL_SWITCH_EVENT => ObservationKind::PostModelSwitch,
        _ => return Ok(ClaudeModelStatePayload::default()),
    };

    let session_id = prefixed_diff_trace_session_id(
        CLAUDE_TOOL_NAME,
        required_non_empty_string(payload, "session_id")?.as_str(),
    );
    let agent_id = optional_agent_id(payload)?;

    match observation_kind {
        ObservationKind::SessionStart => {
            let model_id = optional_model_id(payload, "model")?;
            let source = required_non_empty_string(payload, "source")?;
            let observation = model_id.map(|model_id| ClaudeModelStateObservation {
                session_id: session_id.clone(),
                agent_id,
                model_id,
                observation_kind,
                source,
                observed_at_ms,
            });

            Ok(ClaudeModelStatePayload {
                observation,
                session_start_session_id: Some(session_id),
            })
        }
        ObservationKind::PostModelSwitch => {
            let _from_model = required_model_id(payload, "from_model")?;
            let to_model = required_model_id(payload, "to_model")?;
            let source = required_non_empty_string(payload, "source")?;

            Ok(ClaudeModelStatePayload {
                observation: Some(ClaudeModelStateObservation {
                    session_id,
                    agent_id,
                    model_id: to_model,
                    observation_kind,
                    source,
                    observed_at_ms,
                }),
                session_start_session_id: None,
            })
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use anyhow::anyhow;

    use super::{
        run_claude_model_state_from_payload_with, AUTH_DB_WRITE_FAILED_EVENT, DB_OPEN_FAILED_EVENT,
    };
    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_db::{
        ClaudeModelStateObservation, ClaudeSessionAuthState, ObservationKind,
    };
    use crate::services::error::CliError;
    use crate::services::hooks::claude_auth_detection::{
        AuthEnvKey, AuthProvider, BillingMode, DetectionConfidence,
    };
    use crate::services::observability::traits::Logger;

    const SESSION_ID: &str = "cc_session-1";

    #[derive(Default)]
    struct RecordingLogger {
        entries: Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl RecordingLogger {
        fn record(&self, event_id: &str, message: &str, session_id: Option<&str>) {
            self.entries.lock().unwrap().push((
                event_id.to_string(),
                message.to_string(),
                session_id.map(str::to_string),
            ));
        }

        fn entries(&self) -> Vec<(String, String, Option<String>)> {
            self.entries.lock().unwrap().clone()
        }

        fn event_ids(&self) -> Vec<String> {
            self.entries()
                .into_iter()
                .map(|(event_id, _, _)| event_id)
                .collect()
        }
    }

    impl Logger for RecordingLogger {
        fn info(&self, event_id: &str, message: &str, _: &[(&str, &str)], session: Option<&str>) {
            self.record(event_id, message, session);
        }
        fn debug(&self, event_id: &str, message: &str, _: &[(&str, &str)], session: Option<&str>) {
            self.record(event_id, message, session);
        }
        fn warn(&self, event_id: &str, message: &str, _: &[(&str, &str)], session: Option<&str>) {
            self.record(event_id, message, session);
        }
        fn error(&self, event_id: &str, message: &str, _: &[(&str, &str)], session: Option<&str>) {
            self.record(event_id, message, session);
        }
        fn log_cli_error(&self, error: &CliError, session_id: Option<&str>) {
            self.record("cli_error", &error.to_string(), session_id);
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        db_path: PathBuf,
        logger: RecordingLogger,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let db_path = dir.path().join("agent-trace.db");
            drop(
                RepositoryAgentTraceDb::new_at(&db_path)
                    .await
                    .expect("current database"),
            );
            Self {
                _dir: dir,
                db_path,
                logger: RecordingLogger::default(),
            }
        }

        async fn run(&self, payload: &str, observed_at_ms: i64, env: &[(&str, &str)]) -> String {
            let db_path = self.db_path.clone();
            run_with(
                &self.db_path,
                payload,
                observed_at_ms,
                env,
                &self.logger,
                async move |_: &Path, _: &'static str| {
                    RepositoryAgentTraceDb::new_at(&db_path).await
                },
            )
            .await
        }

        async fn db(&self) -> RepositoryAgentTraceDb {
            RepositoryAgentTraceDb::new_at(&self.db_path)
                .await
                .expect("reopen database")
        }

        async fn auth_state(&self) -> Option<ClaudeSessionAuthState> {
            self.db()
                .await
                .claude_session_auth_state_by_session(SESSION_ID)
                .await
                .expect("auth state read")
        }

        async fn model_state(&self) -> Option<ClaudeModelStateObservation> {
            self.db()
                .await
                .claude_model_state_by_session_and_agent(SESSION_ID, "")
                .await
                .expect("model state read")
        }

        async fn dump_rows(&self, sql: &str) -> Vec<String> {
            self.db()
                .await
                .query_map(sql, (), |row| row.get::<String>(0).map_err(Into::into))
                .await
                .expect("row dump")
        }
    }

    async fn run_with<O>(
        db_path: &Path,
        payload: &str,
        observed_at_ms: i64,
        env: &[(&str, &str)],
        logger: &RecordingLogger,
        open_db: O,
    ) -> String
    where
        O: std::ops::AsyncFnOnce(&Path, &'static str) -> anyhow::Result<RepositoryAgentTraceDb>,
    {
        let vars: HashMap<String, String> = env
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        run_claude_model_state_from_payload_with(
            db_path.parent().expect("db parent"),
            payload,
            Some(logger),
            || Ok(observed_at_ms),
            open_db,
            |name| vars.get(name).cloned(),
        )
        .await
    }

    fn session_start(source: &str, model: Option<&str>) -> String {
        let mut payload = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "session-1",
            "source": source,
        });
        if let Some(model) = model {
            payload["model"] = serde_json::Value::from(model);
        }
        payload.to_string()
    }

    fn post_model_switch() -> String {
        serde_json::json!({
            "hook_event_name": "PostModelSwitch",
            "session_id": "session-1",
            "source": "user",
            "from_model": "claude-sonnet-5-5",
            "to_model": "claude-opus-5-5",
        })
        .to_string()
    }

    fn auth_state(
        provider: AuthProvider,
        billing_mode: BillingMode,
        auth_env_key: Option<AuthEnvKey>,
        confidence: DetectionConfidence,
        observed_at_ms: i64,
    ) -> ClaudeSessionAuthState {
        ClaudeSessionAuthState {
            session_id: SESSION_ID.to_string(),
            provider,
            billing_mode,
            auth_env_key,
            confidence,
            observed_at_ms,
        }
    }

    fn api_key_auth(observed_at_ms: i64) -> ClaudeSessionAuthState {
        auth_state(
            AuthProvider::Anthropic,
            BillingMode::ApiKey,
            Some(AuthEnvKey::AnthropicApiKey),
            DetectionConfidence::Inferred,
            observed_at_ms,
        )
    }

    const API_KEY_ENV: &[(&str, &str)] = &[("ANTHROPIC_API_KEY", "key")];

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_model_state_startup_stores_auth_and_unchanged_model_state() {
        let fixture = Fixture::new().await;

        let stdout = fixture
            .run(
                &session_start("startup", Some("claude-opus-5-5")),
                10,
                API_KEY_ENV,
            )
            .await;

        assert_eq!(stdout, "");
        assert_eq!(fixture.auth_state().await, Some(api_key_auth(10)));
        assert_eq!(
            fixture.model_state().await,
            Some(ClaudeModelStateObservation {
                session_id: SESSION_ID.to_string(),
                agent_id: String::new(),
                model_id: String::from("claude/claude-opus-5-5"),
                observation_kind: ObservationKind::SessionStart,
                source: String::from("startup"),
                observed_at_ms: 10,
            })
        );
        assert!(fixture.logger.entries().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_model_state_resume_with_changed_auth_updates_row() {
        let fixture = Fixture::new().await;
        fixture
            .run(
                &session_start("startup", Some("claude-opus-5-5")),
                10,
                API_KEY_ENV,
            )
            .await;

        let stdout = fixture
            .run(
                &session_start("resume", Some("claude-opus-5-5")),
                20,
                &[
                    ("CLAUDE_CODE_USE_BEDROCK", "1"),
                    ("ANTHROPIC_API_KEY", "key"),
                ],
            )
            .await;

        assert_eq!(stdout, "");
        assert_eq!(
            fixture.auth_state().await,
            Some(auth_state(
                AuthProvider::AwsBedrock,
                BillingMode::CloudProvider,
                Some(AuthEnvKey::ClaudeCodeUseBedrock),
                DetectionConfidence::Inferred,
                20,
            ))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_model_state_resume_without_auth_change_leaves_row_unchanged() {
        let fixture = Fixture::new().await;
        fixture
            .run(
                &session_start("startup", Some("claude-opus-5-5")),
                10,
                API_KEY_ENV,
            )
            .await;
        let model_state = fixture.model_state().await;

        let stdout = fixture
            .run(&session_start("resume", None), 20, API_KEY_ENV)
            .await;

        assert_eq!(stdout, "");
        assert_eq!(fixture.auth_state().await, Some(api_key_auth(10)));
        assert_eq!(fixture.model_state().await, model_state);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_model_state_duplicate_session_start_keeps_single_unchanged_auth_row() {
        let fixture = Fixture::new().await;
        let payload = session_start("startup", Some("claude-opus-5-5"));

        assert_eq!(fixture.run(&payload, 10, API_KEY_ENV).await, "");
        assert_eq!(fixture.run(&payload, 10, API_KEY_ENV).await, "");
        assert_eq!(fixture.run(&payload, 15, API_KEY_ENV).await, "");

        assert_eq!(fixture.auth_state().await, Some(api_key_auth(10)));
        assert_eq!(
            fixture
                .dump_rows("SELECT session_id FROM claude_session_auth_state")
                .await,
            vec![SESSION_ID.to_string()]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_model_state_model_less_session_start_stores_auth_only() {
        let fixture = Fixture::new().await;

        let stdout = fixture.run(&session_start("clear", None), 10, &[]).await;

        assert_eq!(stdout, "");
        assert_eq!(
            fixture.auth_state().await,
            Some(auth_state(
                AuthProvider::Unknown,
                BillingMode::Unknown,
                None,
                DetectionConfidence::Unknown,
                10,
            ))
        );
        assert_eq!(fixture.model_state().await, None);
        assert!(fixture.logger.entries().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_model_state_post_model_switch_ignores_auth() {
        let fixture = Fixture::new().await;

        let stdout = fixture.run(&post_model_switch(), 10, API_KEY_ENV).await;

        assert_eq!(stdout, "");
        assert_eq!(fixture.auth_state().await, None);
        assert_eq!(
            fixture.model_state().await,
            Some(ClaudeModelStateObservation {
                session_id: SESSION_ID.to_string(),
                agent_id: String::new(),
                model_id: String::from("claude/claude-opus-5-5"),
                observation_kind: ObservationKind::PostModelSwitch,
                source: String::from("user"),
                observed_at_ms: 10,
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_model_state_db_open_failure_fails_open_with_empty_stdout() {
        let fixture = Fixture::new().await;

        let stdout = run_with(
            &fixture.db_path,
            &session_start("startup", None),
            10,
            API_KEY_ENV,
            &fixture.logger,
            async |_: &Path, _: &'static str| -> anyhow::Result<RepositoryAgentTraceDb> {
                Err(anyhow!("open failed"))
            },
        )
        .await;

        assert_eq!(stdout, "");
        assert_eq!(fixture.logger.event_ids(), vec![DB_OPEN_FAILED_EVENT]);
        assert_eq!(fixture.auth_state().await, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_model_state_auth_write_failure_does_not_block_model_state() {
        let fixture = Fixture::new().await;
        fixture
            .db()
            .await
            .execute("DROP TABLE claude_session_auth_state", ())
            .await
            .expect("drop auth table");

        let stdout = fixture
            .run(
                &session_start("startup", Some("claude-opus-5-5")),
                10,
                API_KEY_ENV,
            )
            .await;

        assert_eq!(stdout, "");
        assert!(fixture.model_state().await.is_some());
        let entries = fixture.logger.entries();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].0, AUTH_DB_WRITE_FAILED_EVENT);
        assert_eq!(entries[0].2.as_deref(), Some(SESSION_ID));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_auth_state_no_secret_values_reach_rows_logs_or_stdout() {
        const SENTINELS: &[&str] = &[
            "sentinel-auth-token-7f3a",
            "sentinel-api-key-91c2",
            "sentinel-oauth-token-4d8e",
        ];
        let env = [
            ("ANTHROPIC_AUTH_TOKEN", SENTINELS[0]),
            ("ANTHROPIC_API_KEY", SENTINELS[1]),
            ("CLAUDE_CODE_OAUTH_TOKEN", SENTINELS[2]),
        ];
        let fixture = Fixture::new().await;

        let mut stdout = fixture
            .run(&session_start("startup", Some("claude-opus-5-5")), 10, &env)
            .await;
        stdout.push_str(&fixture.run(&session_start("resume", None), 20, &env).await);
        fixture
            .db()
            .await
            .execute("DROP TABLE claude_session_auth_state", ())
            .await
            .expect("drop auth table");
        stdout.push_str(&fixture.run(&session_start("resume", None), 30, &env).await);

        let mut haystack = fixture
            .dump_rows(
                "SELECT session_id || '|' || agent_id || '|' || model_id || '|' || observation_kind || '|' || source FROM claude_model_state",
            )
            .await;
        haystack.extend(fixture.logger.entries().into_iter().flat_map(
            |(event_id, message, session_id)| [event_id, message, session_id.unwrap_or_default()],
        ));
        assert!(
            fixture
                .logger
                .event_ids()
                .iter()
                .any(|event_id| event_id == AUTH_DB_WRITE_FAILED_EVENT),
            "auth write failure must produce log output to inspect"
        );
        haystack.push(stdout.clone());

        assert_eq!(stdout, "");
        for sentinel in SENTINELS {
            for text in &haystack {
                assert!(!text.contains(sentinel), "{sentinel} leaked into {text}");
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_auth_state_no_secret_values_in_stored_auth_row() {
        const SENTINEL: &str = "sentinel-api-key-91c2";
        let fixture = Fixture::new().await;

        fixture
            .run(
                &session_start("startup", None),
                10,
                &[("ANTHROPIC_API_KEY", SENTINEL)],
            )
            .await;

        let rows = fixture
            .dump_rows(
                "SELECT session_id || '|' || provider || '|' || billing_mode || '|' || COALESCE(auth_env_key, '') || '|' || confidence || '|' || observed_at_ms FROM claude_session_auth_state",
            )
            .await;
        assert_eq!(
            rows,
            vec![format!(
                "{SESSION_ID}|anthropic|api_key|anthropic_api_key|inferred|10"
            )]
        );
        assert!(rows.iter().all(|row| !row.contains(SENTINEL)));
    }
}
