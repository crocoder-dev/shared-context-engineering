use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::runtime::{
    abandon_scope, arm_external_mutation_guard, coordinate, AbandonScopeError, AbandonScopeOutcome,
    CoordinateError, CoordinateOutcome, GuardEvent, GuardRequest, RuntimeBoundary, StartProvenance,
};
use crate::services::mutation_trace::types::{ActorKind, EventId, ScopeId};

const MUTATION_SCOPE_DB_CONTEXT: &str = "Failed to open Agent Trace DB for mutation-scope runtime.";

const OPERATION_FIELD: &str = "operation";
const SCOPE_ID_FIELD: &str = "scope_id";
const EVENT_ID_FIELD: &str = "event_id";
const ACTOR_KIND_FIELD: &str = "actor_kind";
const WORKTREE_ID_FIELD: &str = "worktree_id";
const PROVENANCE_FIELD: &str = "provenance";
const SESSION_ID_FIELD: &str = "session_id";
const MODEL_ID_FIELD: &str = "model_id";

const ACTOR_KIND_CLAUDE_CODE: &str = "claude_code";
const ACTOR_KIND_CODEX: &str = "codex";
const ACTOR_KIND_OPENCODE: &str = "opencode";
const ACTOR_KIND_PI: &str = "pi";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MutationScopePayload {
    Start {
        scope_id: String,
        event_id: String,
        actor_kind: ActorKind,
        provenance: Option<StartProvenance>,
    },
    Advance {
        scope_id: String,
        event_id: String,
        actor_kind: ActorKind,
    },
    Close {
        scope_id: String,
        event_id: String,
        actor_kind: ActorKind,
    },
    Flush,
    Abandon {
        scope_id: String,
    },
}

pub(crate) fn parse_mutation_scope_payload(stdin_payload: &str) -> Result<MutationScopePayload> {
    if stdin_payload.trim().is_empty() {
        bail!(validation_error(
            "expected a JSON object, got an empty payload"
        ));
    }

    let parsed: Value = serde_json::from_str(stdin_payload)
        .with_context(|| validation_error("expected valid JSON"))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| anyhow!(validation_error("expected a JSON object")))?;

    let operation = required_str(object, OPERATION_FIELD)?;

    match operation.as_str() {
        "start" => parse_start(object),
        "advance" => parse_scope_boundary(object, |scope_id, event_id, actor_kind| {
            MutationScopePayload::Advance {
                scope_id,
                event_id,
                actor_kind,
            }
        }),
        "close" => parse_scope_boundary(object, |scope_id, event_id, actor_kind| {
            MutationScopePayload::Close {
                scope_id,
                event_id,
                actor_kind,
            }
        }),
        "flush" => parse_flush(object),
        "abandon" => parse_abandon(object),
        other => bail!(validation_error(&format!(
            "field 'operation' must be one of 'start', 'advance', 'close', 'flush' or 'abandon', got '{other}'"
        ))),
    }
}

fn parse_start(object: &Map<String, Value>) -> Result<MutationScopePayload> {
    let (scope_id, event_id, actor_kind) = parse_scope_boundary_identity(
        object,
        &[
            OPERATION_FIELD,
            SCOPE_ID_FIELD,
            EVENT_ID_FIELD,
            ACTOR_KIND_FIELD,
            PROVENANCE_FIELD,
        ],
    )?;

    Ok(MutationScopePayload::Start {
        scope_id,
        event_id,
        actor_kind,
        provenance: parse_provenance(object)?,
    })
}

fn parse_scope_boundary(
    object: &Map<String, Value>,
    build: impl FnOnce(String, String, ActorKind) -> MutationScopePayload,
) -> Result<MutationScopePayload> {
    let (scope_id, event_id, actor_kind) = parse_scope_boundary_identity(
        object,
        &[
            OPERATION_FIELD,
            SCOPE_ID_FIELD,
            EVENT_ID_FIELD,
            ACTOR_KIND_FIELD,
        ],
    )?;

    Ok(build(scope_id, event_id, actor_kind))
}

fn parse_scope_boundary_identity(
    object: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(String, String, ActorKind)> {
    reject_unexpected_keys(object, allowed)?;

    let scope_id = required_non_blank_str(object, SCOPE_ID_FIELD)?;
    let event_id = required_non_blank_str(object, EVENT_ID_FIELD)?;
    let actor_kind = parse_actor_kind(&required_str(object, ACTOR_KIND_FIELD)?)?;

    Ok((scope_id, event_id, actor_kind))
}

fn parse_provenance(object: &Map<String, Value>) -> Result<Option<StartProvenance>> {
    let Some(value) = object.get(PROVENANCE_FIELD) else {
        return Ok(None);
    };

    let provenance = value
        .as_object()
        .ok_or_else(|| anyhow!(validation_error("field 'provenance' must be a JSON object")))?;

    for key in provenance.keys() {
        if key != SESSION_ID_FIELD && key != MODEL_ID_FIELD {
            bail!(validation_error(&format!(
                "unexpected field 'provenance.{key}'"
            )));
        }
    }

    Ok(Some(StartProvenance {
        session_id: required_non_blank_str(provenance, SESSION_ID_FIELD)?,
        model_id: optional_non_blank_str(provenance, MODEL_ID_FIELD)?,
    }))
}

fn parse_flush(object: &Map<String, Value>) -> Result<MutationScopePayload> {
    reject_unexpected_keys(object, &[OPERATION_FIELD])?;
    Ok(MutationScopePayload::Flush)
}

fn parse_abandon(object: &Map<String, Value>) -> Result<MutationScopePayload> {
    reject_unexpected_keys(object, &[OPERATION_FIELD, SCOPE_ID_FIELD])?;
    let scope_id = required_non_blank_str(object, SCOPE_ID_FIELD)?;
    Ok(MutationScopePayload::Abandon { scope_id })
}

fn parse_actor_kind(wire: &str) -> Result<ActorKind> {
    match wire {
        ACTOR_KIND_CLAUDE_CODE => Ok(ActorKind::ClaudeCode),
        ACTOR_KIND_CODEX => Ok(ActorKind::Codex),
        ACTOR_KIND_OPENCODE => Ok(ActorKind::OpenCode),
        ACTOR_KIND_PI => Ok(ActorKind::Pi),
        other => bail!(validation_error(&format!(
            "field 'actor_kind' must be one of 'claude_code', 'codex', 'opencode' or 'pi', got '{other}'"
        ))),
    }
}

fn reject_unexpected_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    for key in object.keys() {
        if key == WORKTREE_ID_FIELD {
            bail!(validation_error(
                "field 'worktree_id' is not accepted; worktree identity is derived from the invoking checkout"
            ));
        }
        if !allowed.contains(&key.as_str()) {
            bail!(validation_error(&format!("unexpected field '{key}'")));
        }
    }
    Ok(())
}

fn required_field<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a Value> {
    object.get(field).ok_or_else(|| {
        anyhow!(validation_error(&format!(
            "missing required field '{field}'"
        )))
    })
}

fn required_str(object: &Map<String, Value>, field: &str) -> Result<String> {
    required_field(object, field)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow!(validation_error(&format!(
                "field '{field}' must be a string"
            )))
        })
}

fn required_non_blank_str(object: &Map<String, Value>, field: &str) -> Result<String> {
    let value = required_str(object, field)?;
    if value.trim().is_empty() {
        bail!(validation_error(&format!(
            "field '{field}' must be a non-blank string"
        )));
    }
    Ok(value)
}

fn optional_non_blank_str(object: &Map<String, Value>, field: &str) -> Result<Option<String>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => Ok(Some(required_non_blank_str(object, field)?)),
    }
}

fn validation_error(detail: &str) -> String {
    format!("Invalid mutation-scope payload from STDIN: {detail}.")
}

pub(crate) async fn run_mutation_scope_subcommand<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    logger: Option<&L>,
) -> Result<String> {
    let stdin_payload = super::read_hook_stdin()?;
    run_mutation_scope_from_payload(repository_root, &stdin_payload, logger).await
}

pub(crate) async fn run_mutation_scope_from_payload<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
) -> Result<String> {
    run_mutation_scope_from_payload_with(
        repository_root,
        stdin_payload,
        logger,
        super::open_agent_trace_db_for_hook_runtime,
    )
    .await
}

#[cfg(any())]
pub(super) async fn run_mutation_scope_from_payload_at_state_root<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    state_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
) -> Result<String> {
    run_mutation_scope_from_payload_with(
        repository_root,
        stdin_payload,
        logger,
        async |root, context_message| {
            super::open_agent_trace_db_for_hook_runtime_at_state_root(
                root,
                state_root,
                context_message,
            )
            .await
        },
    )
    .await
}

async fn run_mutation_scope_from_payload_with<
    L: crate::services::observability::traits::Logger,
    O,
>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
    open_db: O,
) -> Result<String>
where
    O: std::ops::AsyncFn(&Path, &'static str) -> Result<RepositoryAgentTraceDb> + Copy,
{
    let payload = parse_mutation_scope_payload(stdin_payload)?;

    drive_mutation_scope(
        repository_root,
        payload,
        logger,
        async |root, boundary| {
            coordinate(root, boundary, async || {
                open_db(root, MUTATION_SCOPE_DB_CONTEXT).await
            })
            .await
        },
        async |root, scope| {
            abandon_scope(root, scope, async || {
                open_db(root, MUTATION_SCOPE_DB_CONTEXT).await
            })
            .await
        },
    )
    .await
}

async fn drive_mutation_scope<L: crate::services::observability::traits::Logger, C, A>(
    repository_root: &Path,
    payload: MutationScopePayload,
    logger: Option<&L>,
    coordinate_boundary: C,
    abandon: A,
) -> Result<String>
where
    C: std::ops::AsyncFnOnce(
        &Path,
        &RuntimeBoundary,
    ) -> std::result::Result<CoordinateOutcome, CoordinateError>,
    A: std::ops::AsyncFnOnce(
        &Path,
        &ScopeId,
    ) -> std::result::Result<AbandonScopeOutcome, AbandonScopeError>,
{
    let boundary = match payload {
        MutationScopePayload::Start {
            scope_id,
            event_id,
            actor_kind,
            provenance,
        } => RuntimeBoundary::Start {
            scope: ScopeId(scope_id),
            event: EventId(event_id),
            actor_kind,
            provenance,
        },
        MutationScopePayload::Advance {
            scope_id,
            event_id,
            actor_kind,
        } => RuntimeBoundary::Advance {
            scope: ScopeId(scope_id),
            event: EventId(event_id),
            actor_kind,
        },
        MutationScopePayload::Close {
            scope_id,
            event_id,
            actor_kind,
        } => RuntimeBoundary::Close {
            scope: ScopeId(scope_id),
            event: EventId(event_id),
            actor_kind,
        },
        MutationScopePayload::Flush => RuntimeBoundary::Flush,
        MutationScopePayload::Abandon { scope_id } => {
            return classify_abandon(abandon(repository_root, &ScopeId(scope_id)).await, logger);
        }
    };

    classify_coordinate(
        coordinate_boundary(repository_root, &boundary).await,
        logger,
    )
}

fn classify_coordinate<L: crate::services::observability::traits::Logger>(
    result: std::result::Result<CoordinateOutcome, CoordinateError>,
    logger: Option<&L>,
) -> Result<String> {
    match result {
        Ok(_) => Ok(String::new()),
        Err(CoordinateError::MarkerClearAfterCommit { source, .. }) => {
            log_marker_clear_after_durable_completion(logger, "coordinate", &source);
            Ok(String::new())
        }
        Err(error) => Err(anyhow!(
            "mutation-scope runtime boundary failed before durable completion: {error}"
        )),
    }
}

fn classify_abandon<L: crate::services::observability::traits::Logger>(
    result: std::result::Result<AbandonScopeOutcome, AbandonScopeError>,
    logger: Option<&L>,
) -> Result<String> {
    match result {
        Ok(_) => Ok(String::new()),
        Err(AbandonScopeError::MarkerClearAfterCompletion { source, .. }) => {
            log_marker_clear_after_durable_completion(logger, "abandon_scope", &source);
            Ok(String::new())
        }
        Err(error) => Err(anyhow!(
            "mutation-scope runtime abandonment failed before durable completion: {error}"
        )),
    }
}

fn log_marker_clear_after_durable_completion<L: crate::services::observability::traits::Logger>(
    logger: Option<&L>,
    entrypoint: &str,
    source: &anyhow::Error,
) {
    if let Some(log) = logger {
        log.warn(
            "sce.hooks.mutation_scope.marker_clear_after_durable_completion",
            &source.to_string(),
            &[("entrypoint", entrypoint)],
            None,
        );
    }
}

const GUARD_OPERATION_FIELD: &str = "operation";
const GUARD_OPERATION_ARM: &str = "arm";
const GUARD_OPERATION_EXEC: &str = "exec";
const GUARD_OPERATION_CANCEL: &str = "cancel";
const GUARD_COMMAND_FIELD: &str = "command";
const GUARD_CWD_FIELD: &str = "cwd";
const GUARD_ENV_FIELD: &str = "env";

fn parse_guard_object(line: &str) -> Result<Map<String, Value>> {
    if line.trim().is_empty() {
        bail!(validation_error(
            "expected a JSON object guard request, got an empty line"
        ));
    }
    let parsed: Value = serde_json::from_str(line)
        .with_context(|| validation_error("expected a valid JSON guard request"))?;
    parsed
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!(validation_error("expected a JSON object guard request")))
}

fn parse_guard_arm(line: &str) -> Result<()> {
    let object = parse_guard_object(line)?;
    reject_unexpected_keys(&object, &[GUARD_OPERATION_FIELD])?;
    let operation = required_str(&object, GUARD_OPERATION_FIELD)?;
    if operation != GUARD_OPERATION_ARM {
        bail!(validation_error(&format!(
            "field 'operation' must be 'arm', got '{operation}'"
        )));
    }
    Ok(())
}

fn parse_guard_exec(line: &str) -> Result<GuardRequest> {
    let object = parse_guard_object(line)?;
    reject_unexpected_keys(
        &object,
        &[
            GUARD_OPERATION_FIELD,
            GUARD_COMMAND_FIELD,
            GUARD_CWD_FIELD,
            GUARD_ENV_FIELD,
        ],
    )?;
    let operation = required_str(&object, GUARD_OPERATION_FIELD)?;
    if operation != GUARD_OPERATION_EXEC {
        bail!(validation_error(&format!(
            "field 'operation' must be 'exec', got '{operation}'"
        )));
    }

    let command = required_non_blank_str(&object, GUARD_COMMAND_FIELD)?;
    let cwd = optional_non_blank_str(&object, GUARD_CWD_FIELD)?;
    let env = match object.get(GUARD_ENV_FIELD) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(entries)) => entries
            .iter()
            .map(|(key, value)| {
                let value = value.as_str().ok_or_else(|| {
                    anyhow!(validation_error(&format!(
                        "field 'env.{key}' must be a string"
                    )))
                })?;
                Ok((key.clone(), value.to_string()))
            })
            .collect::<Result<Vec<_>>>()?,
        Some(_) => bail!(validation_error("field 'env' must be a JSON object")),
    };

    Ok(GuardRequest { command, cwd, env })
}

fn parse_guard_cancel(line: &str) -> Result<()> {
    let object = parse_guard_object(line)?;
    reject_unexpected_keys(&object, &[GUARD_OPERATION_FIELD])?;
    let operation = required_str(&object, GUARD_OPERATION_FIELD)?;
    if operation != GUARD_OPERATION_CANCEL {
        bail!(validation_error(&format!(
            "field 'operation' must be 'cancel', got '{operation}'"
        )));
    }
    Ok(())
}

fn guard_operation(line: &str) -> Option<String> {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .and_then(|object| {
            object
                .get(GUARD_OPERATION_FIELD)
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

fn guard_event_json_line(event: &GuardEvent) -> String {
    match event {
        GuardEvent::Armed => json!({ "status": "armed" }).to_string(),
        GuardEvent::Stdout(chunk) => json!({
            "stream": "stdout",
            "data": String::from_utf8_lossy(chunk),
        })
        .to_string(),
        GuardEvent::Stderr(chunk) => json!({
            "stream": "stderr",
            "data": String::from_utf8_lossy(chunk),
        })
        .to_string(),
    }
}

pub(crate) async fn run_external_mutation_guard_subcommand<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    logger: Option<&L>,
) -> Result<String> {
    let reader = std::io::BufReader::new(std::io::stdin());
    let stdout = std::io::stdout();
    run_external_mutation_guard_protocol_with(
        repository_root,
        logger,
        reader,
        stdout.lock(),
        async |root: &Path| {
            super::open_agent_trace_db_for_hook_runtime(root, MUTATION_SCOPE_DB_CONTEXT).await
        },
    )
    .await
}

async fn run_external_mutation_guard_protocol_with<
    L: crate::services::observability::traits::Logger,
    R,
    W,
    O,
>(
    repository_root: &Path,
    logger: Option<&L>,
    mut reader: R,
    mut writer: W,
    open_db: O,
) -> Result<String>
where
    R: std::io::BufRead + Send + 'static,
    W: Write,
    O: std::ops::AsyncFn(&Path) -> Result<RepositoryAgentTraceDb>,
{
    let mut first_line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut first_line)
        .context("Failed to read the external-mutation guard arm request from STDIN.")?;
    parse_guard_arm(&first_line)?;

    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel();
    let repository_root = repository_root.to_path_buf();
    let armed_guard = arm_external_mutation_guard(
        &repository_root,
        async || open_db(&repository_root).await,
        || write_guard_event(&mut writer, &GuardEvent::Armed),
        cancel_rx,
    )?;

    let mut exec_line = String::new();
    match std::io::BufRead::read_line(&mut reader, &mut exec_line)
        .context("Failed to read the external-mutation guard exec request from STDIN.")?
    {
        0 => return Ok(String::new()),
        _ if guard_operation(&exec_line).as_deref() == Some(GUARD_OPERATION_CANCEL) => {
            parse_guard_cancel(&exec_line)?;
            return Ok(String::new());
        }
        _ => {}
    }
    let request = parse_guard_exec(&exec_line)?;

    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match std::io::BufRead::read_line(&mut reader, &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if guard_operation(&line).as_deref() == Some(GUARD_OPERATION_CANCEL) => {
                    if parse_guard_cancel(&line).is_ok() {
                        let _ = cancel_tx.send(());
                    }
                }
                Ok(_) => {}
            }
        }
    });

    let outcome = armed_guard
        .exec(&request, |event| {
            let _ = write_guard_event(&mut writer, &event);
        })
        .await?;

    if outcome.marker_clear_failed {
        log_marker_clear_after_durable_completion(
            logger,
            "external_mutation_guard",
            &anyhow!("external-taint marker clear failed after a durable guard finish"),
        );
    }

    let _ = write_guard_line(
        &mut writer,
        &json!({
            "status": "result",
            "exit_code": outcome.exit_code,
        })
        .to_string(),
    );

    Ok(String::new())
}

fn write_guard_event<W: Write>(writer: &mut W, event: &GuardEvent) -> std::io::Result<()> {
    write_guard_line(writer, &guard_event_json_line(event))
}

fn write_guard_line<W: Write>(writer: &mut W, line: &str) -> std::io::Result<()> {
    writeln!(writer, "{line}")?;
    writer.flush()
}

#[cfg(any())]
mod tests {
    use super::*;

    fn parse(payload: &str) -> Result<MutationScopePayload> {
        parse_mutation_scope_payload(payload)
    }

    struct ParseCase {
        label: &'static str,
        payload: &'static str,
        expected: MutationScopePayload,
    }

    struct InvalidCase {
        label: &'static str,
        payload: &'static str,
        expected_fragment: &'static str,
    }

    fn assert_all_rejected(cases: &[InvalidCase]) {
        for case in cases {
            let error = parse(case.payload)
                .expect_err(&format!(
                    "{}: expected the payload to be rejected",
                    case.label
                ))
                .to_string();
            assert!(
                error.contains(case.expected_fragment),
                "{}: expected error containing {:?}, got: {error}",
                case.label,
                case.expected_fragment
            );
        }
    }

    #[test]
    fn valid_operation_payloads_parse_exactly() {
        let cases = [
            ParseCase {
                label: "start preserves identity values verbatim",
                payload: r#"{"operation":"start","scope_id":"  scope-A  ","event_id":"event-start","actor_kind":"claude_code"}"#,
                expected: MutationScopePayload::Start {
                    scope_id: "  scope-A  ".to_string(),
                    event_id: "event-start".to_string(),
                    actor_kind: ActorKind::ClaudeCode,
                    provenance: None,
                },
            },
            ParseCase {
                label: "advance",
                payload: r#"{"operation":"advance","scope_id":"scope-B","event_id":"event-advance","actor_kind":"codex"}"#,
                expected: MutationScopePayload::Advance {
                    scope_id: "scope-B".to_string(),
                    event_id: "event-advance".to_string(),
                    actor_kind: ActorKind::Codex,
                },
            },
            ParseCase {
                label: "close",
                payload: r#"{"operation":"close","scope_id":"scope-C","event_id":"event-close","actor_kind":"opencode"}"#,
                expected: MutationScopePayload::Close {
                    scope_id: "scope-C".to_string(),
                    event_id: "event-close".to_string(),
                    actor_kind: ActorKind::OpenCode,
                },
            },
            ParseCase {
                label: "flush takes no identity",
                payload: r#"{"operation":"flush"}"#,
                expected: MutationScopePayload::Flush,
            },
            ParseCase {
                label: "abandon takes only scope_id",
                payload: r#"{"operation":"abandon","scope_id":"scope-D"}"#,
                expected: MutationScopePayload::Abandon {
                    scope_id: "scope-D".to_string(),
                },
            },
        ];

        for case in cases {
            let parsed = parse(case.payload)
                .unwrap_or_else(|error| panic!("{}: expected valid payload: {error}", case.label));
            assert_eq!(parsed, case.expected, "{}", case.label);
        }
    }

    #[test]
    fn valid_start_provenance_parses_exactly() {
        let cases = [
            (
                "session and model",
                r#"{"session_id":"cx_session-1","model_id":"gpt-5-codex"}"#,
                StartProvenance {
                    session_id: "cx_session-1".to_string(),
                    model_id: Some("gpt-5-codex".to_string()),
                },
            ),
            (
                "session only",
                r#"{"session_id":"cc_session-1"}"#,
                StartProvenance {
                    session_id: "cc_session-1".to_string(),
                    model_id: None,
                },
            ),
            (
                "session with a null model",
                r#"{"session_id":"cc_session-1","model_id":null}"#,
                StartProvenance {
                    session_id: "cc_session-1".to_string(),
                    model_id: None,
                },
            ),
        ];

        for (label, provenance, expected) in cases {
            let payload = format!(
                r#"{{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{provenance}}}"#
            );
            let parsed = parse(&payload)
                .unwrap_or_else(|error| panic!("{label}: expected valid payload: {error}"));
            assert_eq!(
                parsed,
                MutationScopePayload::Start {
                    scope_id: "A".to_string(),
                    event_id: "e1".to_string(),
                    actor_kind: ActorKind::Codex,
                    provenance: Some(expected),
                },
                "{label}"
            );
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn invalid_provenance_shapes_are_rejected() {
        const NOT_ON_THIS_OPERATION: &str = "unexpected field 'provenance'.";
        const NOT_AN_OBJECT: &str = "field 'provenance' must be a JSON object";
        const SESSION_NOT_A_STRING: &str = "field 'session_id' must be a string";
        const SESSION_BLANK: &str = "field 'session_id' must be a non-blank string";
        const MODEL_NOT_A_STRING: &str = "field 'model_id' must be a string";
        const MODEL_BLANK: &str = "field 'model_id' must be a non-blank string";

        assert_all_rejected(&[
            InvalidCase {
                label: "advance rejects provenance",
                payload: r#"{"operation":"advance","scope_id":"A","event_id":"e2","actor_kind":"codex","provenance":{"session_id":"cx_session-1"}}"#,
                expected_fragment: NOT_ON_THIS_OPERATION,
            },
            InvalidCase {
                label: "close rejects provenance",
                payload: r#"{"operation":"close","scope_id":"A","event_id":"e3","actor_kind":"codex","provenance":{"session_id":"cx_session-1"}}"#,
                expected_fragment: NOT_ON_THIS_OPERATION,
            },
            InvalidCase {
                label: "flush rejects provenance",
                payload: r#"{"operation":"flush","provenance":{"session_id":"cx_session-1"}}"#,
                expected_fragment: NOT_ON_THIS_OPERATION,
            },
            InvalidCase {
                label: "abandon rejects provenance",
                payload: r#"{"operation":"abandon","scope_id":"A","provenance":{"session_id":"cx_session-1"}}"#,
                expected_fragment: NOT_ON_THIS_OPERATION,
            },
            InvalidCase {
                label: "null provenance",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":null}"#,
                expected_fragment: NOT_AN_OBJECT,
            },
            InvalidCase {
                label: "string provenance",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":"cx_session-1"}"#,
                expected_fragment: NOT_AN_OBJECT,
            },
            InvalidCase {
                label: "array provenance",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":[]}"#,
                expected_fragment: NOT_AN_OBJECT,
            },
            InvalidCase {
                label: "numeric provenance",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":5}"#,
                expected_fragment: NOT_AN_OBJECT,
            },
            InvalidCase {
                label: "boolean provenance",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":true}"#,
                expected_fragment: NOT_AN_OBJECT,
            },
            InvalidCase {
                label: "missing session_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"model_id":"gpt-5-codex"}}"#,
                expected_fragment: "missing required field 'session_id'",
            },
            InvalidCase {
                label: "empty session_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"session_id":""}}"#,
                expected_fragment: SESSION_BLANK,
            },
            InvalidCase {
                label: "whitespace session_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"session_id":"   "}}"#,
                expected_fragment: SESSION_BLANK,
            },
            InvalidCase {
                label: "null session_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"session_id":null}}"#,
                expected_fragment: SESSION_NOT_A_STRING,
            },
            InvalidCase {
                label: "numeric session_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"session_id":7}}"#,
                expected_fragment: SESSION_NOT_A_STRING,
            },
            InvalidCase {
                label: "empty model_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"session_id":"cx_session-1","model_id":""}}"#,
                expected_fragment: MODEL_BLANK,
            },
            InvalidCase {
                label: "whitespace model_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"session_id":"cx_session-1","model_id":"  "}}"#,
                expected_fragment: MODEL_BLANK,
            },
            InvalidCase {
                label: "numeric model_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"session_id":"cx_session-1","model_id":42}}"#,
                expected_fragment: MODEL_NOT_A_STRING,
            },
            InvalidCase {
                label: "unexpected provenance key",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"codex","provenance":{"session_id":"cx_session-1","agent_id":"sub"}}"#,
                expected_fragment: "unexpected field 'provenance.agent_id'",
            },
        ]);
    }

    #[test]
    fn every_actor_kind_wire_string_maps() {
        for (wire, expected) in [
            ("claude_code", ActorKind::ClaudeCode),
            ("codex", ActorKind::Codex),
            ("opencode", ActorKind::OpenCode),
            ("pi", ActorKind::Pi),
        ] {
            let payload = format!(
                r#"{{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"{wire}"}}"#
            );
            let parsed = parse(&payload)
                .unwrap_or_else(|error| panic!("{wire}: expected valid payload: {error}"));
            assert_eq!(
                parsed,
                MutationScopePayload::Start {
                    scope_id: "A".to_string(),
                    event_id: "e1".to_string(),
                    actor_kind: expected,
                    provenance: None,
                },
                "{wire}"
            );
        }
    }

    #[test]
    fn invalid_payload_envelopes_are_rejected() {
        assert_all_rejected(&[
            InvalidCase {
                label: "empty payload",
                payload: "",
                expected_fragment: "expected a JSON object, got an empty payload",
            },
            InvalidCase {
                label: "whitespace-only payload",
                payload: "   \n\t ",
                expected_fragment: "expected a JSON object, got an empty payload",
            },
            InvalidCase {
                label: "unterminated object",
                payload: "{",
                expected_fragment: "expected valid JSON",
            },
            InvalidCase {
                label: "incomplete object",
                payload: r#"{"operation":"start""#,
                expected_fragment: "expected valid JSON",
            },
            InvalidCase {
                label: "non-JSON text",
                payload: "not json at all",
                expected_fragment: "expected valid JSON",
            },
            InvalidCase {
                label: "JSON number",
                payload: "123",
                expected_fragment: "expected a JSON object",
            },
            InvalidCase {
                label: "JSON string",
                payload: r#""start""#,
                expected_fragment: "expected a JSON object",
            },
            InvalidCase {
                label: "JSON array",
                payload: r#"["start"]"#,
                expected_fragment: "expected a JSON object",
            },
            InvalidCase {
                label: "JSON null",
                payload: "null",
                expected_fragment: "expected a JSON object",
            },
            InvalidCase {
                label: "missing operation",
                payload: r#"{"scope_id":"A","event_id":"e1","actor_kind":"pi"}"#,
                expected_fragment: "missing required field 'operation'",
            },
            InvalidCase {
                label: "unknown operation",
                payload: r#"{"operation":"reopen","scope_id":"A","event_id":"e1","actor_kind":"pi"}"#,
                expected_fragment: "field 'operation' must be one of",
            },
            InvalidCase {
                label: "numeric operation",
                payload: r#"{"operation":5}"#,
                expected_fragment: "field 'operation' must be a string",
            },
        ]);
    }

    #[test]
    fn invalid_required_fields_are_rejected() {
        assert_all_rejected(&[
            InvalidCase {
                label: "start missing scope_id",
                payload: r#"{"operation":"start","event_id":"e1","actor_kind":"pi"}"#,
                expected_fragment: "missing required field 'scope_id'",
            },
            InvalidCase {
                label: "start missing event_id",
                payload: r#"{"operation":"start","scope_id":"A","actor_kind":"pi"}"#,
                expected_fragment: "missing required field 'event_id'",
            },
            InvalidCase {
                label: "start missing actor_kind",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1"}"#,
                expected_fragment: "missing required field 'actor_kind'",
            },
            InvalidCase {
                label: "abandon missing scope_id",
                payload: r#"{"operation":"abandon"}"#,
                expected_fragment: "missing required field 'scope_id'",
            },
            InvalidCase {
                label: "start empty scope_id",
                payload: r#"{"operation":"start","scope_id":"","event_id":"e1","actor_kind":"pi"}"#,
                expected_fragment: "field 'scope_id' must be a non-blank string",
            },
            InvalidCase {
                label: "start whitespace scope_id",
                payload: r#"{"operation":"start","scope_id":"   ","event_id":"e1","actor_kind":"pi"}"#,
                expected_fragment: "field 'scope_id' must be a non-blank string",
            },
            InvalidCase {
                label: "start empty event_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"","actor_kind":"pi"}"#,
                expected_fragment: "field 'event_id' must be a non-blank string",
            },
            InvalidCase {
                label: "start tab event_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"\t","actor_kind":"pi"}"#,
                expected_fragment: "field 'event_id' must be a non-blank string",
            },
            InvalidCase {
                label: "abandon whitespace scope_id",
                payload: r#"{"operation":"abandon","scope_id":"  "}"#,
                expected_fragment: "field 'scope_id' must be a non-blank string",
            },
            InvalidCase {
                label: "start numeric scope_id",
                payload: r#"{"operation":"start","scope_id":123,"event_id":"e1","actor_kind":"pi"}"#,
                expected_fragment: "field 'scope_id' must be a string",
            },
            InvalidCase {
                label: "start boolean event_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":true,"actor_kind":"pi"}"#,
                expected_fragment: "field 'event_id' must be a string",
            },
            InvalidCase {
                label: "start array actor_kind",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":["pi"]}"#,
                expected_fragment: "field 'actor_kind' must be a string",
            },
            InvalidCase {
                label: "start unknown actor_kind",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"cursor"}"#,
                expected_fragment: "field 'actor_kind' must be one of",
            },
        ]);
    }

    #[test]
    fn operation_schema_rejects_forbidden_fields() {
        const WORKTREE: &str = "worktree identity is derived from the invoking checkout";

        assert_all_rejected(&[
            InvalidCase {
                label: "start rejects attempt_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"pi","attempt_id":"x"}"#,
                expected_fragment: "unexpected field 'attempt_id'",
            },
            InvalidCase {
                label: "advance rejects attempt_id",
                payload: r#"{"operation":"advance","scope_id":"A","event_id":"e1","actor_kind":"pi","attempt_id":"x"}"#,
                expected_fragment: "unexpected field 'attempt_id'",
            },
            InvalidCase {
                label: "close rejects attempt_id",
                payload: r#"{"operation":"close","scope_id":"A","event_id":"e1","actor_kind":"pi","attempt_id":"x"}"#,
                expected_fragment: "unexpected field 'attempt_id'",
            },
            InvalidCase {
                label: "flush rejects scope_id",
                payload: r#"{"operation":"flush","scope_id":"A"}"#,
                expected_fragment: "unexpected field 'scope_id'",
            },
            InvalidCase {
                label: "flush rejects event_id",
                payload: r#"{"operation":"flush","event_id":"e1"}"#,
                expected_fragment: "unexpected field 'event_id'",
            },
            InvalidCase {
                label: "flush rejects actor_kind",
                payload: r#"{"operation":"flush","actor_kind":"pi"}"#,
                expected_fragment: "unexpected field 'actor_kind'",
            },
            InvalidCase {
                label: "abandon rejects event_id",
                payload: r#"{"operation":"abandon","scope_id":"A","event_id":"e1"}"#,
                expected_fragment: "unexpected field 'event_id'",
            },
            InvalidCase {
                label: "abandon rejects actor_kind",
                payload: r#"{"operation":"abandon","scope_id":"A","actor_kind":"pi"}"#,
                expected_fragment: "unexpected field 'actor_kind'",
            },
            InvalidCase {
                label: "start rejects worktree_id",
                payload: r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"pi","worktree_id":"wt"}"#,
                expected_fragment: WORKTREE,
            },
            InvalidCase {
                label: "advance rejects worktree_id",
                payload: r#"{"operation":"advance","scope_id":"A","event_id":"e1","actor_kind":"pi","worktree_id":"wt"}"#,
                expected_fragment: WORKTREE,
            },
            InvalidCase {
                label: "close rejects worktree_id",
                payload: r#"{"operation":"close","scope_id":"A","event_id":"e1","actor_kind":"pi","worktree_id":"wt"}"#,
                expected_fragment: WORKTREE,
            },
            InvalidCase {
                label: "flush rejects worktree_id",
                payload: r#"{"operation":"flush","worktree_id":"wt"}"#,
                expected_fragment: WORKTREE,
            },
            InvalidCase {
                label: "abandon rejects worktree_id",
                payload: r#"{"operation":"abandon","scope_id":"A","worktree_id":"wt"}"#,
                expected_fragment: WORKTREE,
            },
        ]);
    }

    mod runtime_dispatch {
        use std::cell::{Cell, RefCell};

        use super::*;
        use crate::services::mutation_trace::protocol;
        use crate::services::mutation_trace::types::{TreeId, WorktreeId};

        fn committed_outcome() -> CoordinateOutcome {
            CoordinateOutcome {
                worktree_id: WorktreeId("wt-1".to_string()),
                observed_tree: TreeId("tree-1".to_string()),
                revision: 1,
                evaluation: protocol::CommitEvaluation::default(),
                mutation_event: None,
            }
        }

        fn abandoned_outcome() -> AbandonScopeOutcome {
            AbandonScopeOutcome::Abandoned {
                worktree_id: WorktreeId("wt-1".to_string()),
                scope: ScopeId("A".to_string()),
                revision: 1,
            }
        }

        fn unreachable_coordinate(
            _root: &Path,
            _boundary: &RuntimeBoundary,
        ) -> std::result::Result<CoordinateOutcome, CoordinateError> {
            panic!("coordinate must not be invoked for this payload");
        }

        fn unreachable_abandon(
            _root: &Path,
            _scope: &ScopeId,
        ) -> std::result::Result<AbandonScopeOutcome, AbandonScopeError> {
            panic!("abandon_scope must not be invoked for this payload");
        }

        fn ids(scope: &str, event: &str) -> (ScopeId, EventId) {
            (ScopeId(scope.to_string()), EventId(event.to_string()))
        }

        fn assert_runtime_boundary_eq(
            actual: &RuntimeBoundary,
            expected: &RuntimeBoundary,
            label: &str,
        ) {
            match (actual, expected) {
                (
                    RuntimeBoundary::Start {
                        scope: actual_scope,
                        event: actual_event,
                        actor_kind: actual_actor,
                        provenance: actual_provenance,
                    },
                    RuntimeBoundary::Start {
                        scope: expected_scope,
                        event: expected_event,
                        actor_kind: expected_actor,
                        provenance: expected_provenance,
                    },
                ) => {
                    assert_eq!(actual_scope, expected_scope, "{label}: scope");
                    assert_eq!(actual_event, expected_event, "{label}: event");
                    assert_eq!(actual_actor, expected_actor, "{label}: actor");
                    assert_eq!(
                        actual_provenance, expected_provenance,
                        "{label}: provenance"
                    );
                }
                (
                    RuntimeBoundary::Advance {
                        scope: actual_scope,
                        event: actual_event,
                        actor_kind: actual_actor,
                    },
                    RuntimeBoundary::Advance {
                        scope: expected_scope,
                        event: expected_event,
                        actor_kind: expected_actor,
                    },
                )
                | (
                    RuntimeBoundary::Close {
                        scope: actual_scope,
                        event: actual_event,
                        actor_kind: actual_actor,
                    },
                    RuntimeBoundary::Close {
                        scope: expected_scope,
                        event: expected_event,
                        actor_kind: expected_actor,
                    },
                ) => {
                    assert_eq!(actual_scope, expected_scope, "{label}: scope");
                    assert_eq!(actual_event, expected_event, "{label}: event");
                    assert_eq!(actual_actor, expected_actor, "{label}: actor");
                }
                (RuntimeBoundary::Flush, RuntimeBoundary::Flush) => {}
                _ => panic!("{label}: runtime boundary variant mismatch"),
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        #[allow(clippy::too_many_lines)]
        async fn coordinate_payloads_forward_exact_runtime_boundaries() {
            let (scope_a, event_a) = ids("  scope-A  ", "event-start");
            let (scope_b, event_b) = ids("scope-B", "event-advance");
            let (scope_c, event_c) = ids("scope-C", "event-close");
            let (scope_d, event_d) = ids("scope-D", "event-start-model");
            let (scope_e, event_e) = ids("scope-E", "event-start-session");
            let with_model = StartProvenance {
                session_id: "cx_session-1".to_string(),
                model_id: Some("gpt-5-codex".to_string()),
            };
            let without_model = StartProvenance {
                session_id: "cc_session-1".to_string(),
                model_id: None,
            };

            let cases = [
                (
                    "start without provenance",
                    MutationScopePayload::Start {
                        scope_id: scope_a.0.clone(),
                        event_id: event_a.0.clone(),
                        actor_kind: ActorKind::ClaudeCode,
                        provenance: None,
                    },
                    RuntimeBoundary::Start {
                        scope: scope_a,
                        event: event_a,
                        actor_kind: ActorKind::ClaudeCode,
                        provenance: None,
                    },
                ),
                (
                    "start with session and model provenance",
                    MutationScopePayload::Start {
                        scope_id: scope_d.0.clone(),
                        event_id: event_d.0.clone(),
                        actor_kind: ActorKind::Codex,
                        provenance: Some(with_model.clone()),
                    },
                    RuntimeBoundary::Start {
                        scope: scope_d,
                        event: event_d,
                        actor_kind: ActorKind::Codex,
                        provenance: Some(with_model),
                    },
                ),
                (
                    "start with session-only provenance",
                    MutationScopePayload::Start {
                        scope_id: scope_e.0.clone(),
                        event_id: event_e.0.clone(),
                        actor_kind: ActorKind::Pi,
                        provenance: Some(without_model.clone()),
                    },
                    RuntimeBoundary::Start {
                        scope: scope_e,
                        event: event_e,
                        actor_kind: ActorKind::Pi,
                        provenance: Some(without_model),
                    },
                ),
                (
                    "advance",
                    MutationScopePayload::Advance {
                        scope_id: scope_b.0.clone(),
                        event_id: event_b.0.clone(),
                        actor_kind: ActorKind::Codex,
                    },
                    RuntimeBoundary::Advance {
                        scope: scope_b,
                        event: event_b,
                        actor_kind: ActorKind::Codex,
                    },
                ),
                (
                    "close",
                    MutationScopePayload::Close {
                        scope_id: scope_c.0.clone(),
                        event_id: event_c.0.clone(),
                        actor_kind: ActorKind::OpenCode,
                    },
                    RuntimeBoundary::Close {
                        scope: scope_c,
                        event: event_c,
                        actor_kind: ActorKind::OpenCode,
                    },
                ),
                ("flush", MutationScopePayload::Flush, RuntimeBoundary::Flush),
            ];

            for (label, payload, expected) in cases {
                let calls = Cell::new(0_u32);
                let result = drive_mutation_scope(
                    Path::new("/unused"),
                    payload,
                    None,
                    |_root, boundary| {
                        assert_runtime_boundary_eq(boundary, &expected, label);
                        calls.set(calls.get() + 1);
                        Ok(committed_outcome())
                    },
                    unreachable_abandon,
                )
                .await;

                assert_eq!(
                    result.unwrap_or_else(|error| panic!("{label}: expected success: {error}")),
                    "",
                    "{label}"
                );
                assert_eq!(calls.get(), 1, "{label}: coordinate call count");
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn abandon_dispatches_only_to_abandon_scope() {
            let seen = RefCell::new(Vec::new());
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Abandon {
                    scope_id: "A".to_string(),
                },
                None,
                unreachable_coordinate,
                |_root, scope| {
                    seen.borrow_mut().push(scope.0.clone());
                    Ok(abandoned_outcome())
                },
            )
            .await;

            assert_eq!(result.expect("abandon should succeed"), "");
            assert_eq!(seen.into_inner(), vec!["A".to_string()]);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn marker_clear_after_commit_is_durable_success_without_reexecution() {
            let calls = Cell::new(0_u32);
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Advance {
                    scope_id: "A".to_string(),
                    event_id: "e2".to_string(),
                    actor_kind: ActorKind::ClaudeCode,
                },
                None,
                |_root, _boundary| {
                    calls.set(calls.get() + 1);
                    Err(CoordinateError::MarkerClearAfterCommit {
                        source: anyhow!("external-taint marker cleanup failed"),
                        committed: Box::new(committed_outcome()),
                    })
                },
                unreachable_abandon,
            )
            .await;

            assert_eq!(result.expect("carried outcome is durable success"), "");
            assert_eq!(calls.get(), 1);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn marker_clear_after_completion_is_durable_success_without_reexecution() {
            let calls = Cell::new(0_u32);
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Abandon {
                    scope_id: "A".to_string(),
                },
                None,
                unreachable_coordinate,
                |_root, _scope| {
                    calls.set(calls.get() + 1);
                    Err(AbandonScopeError::MarkerClearAfterCompletion {
                        source: anyhow!("external-taint marker cleanup failed"),
                        completed: Box::new(abandoned_outcome()),
                    })
                },
            )
            .await;

            assert_eq!(result.expect("carried outcome is durable success"), "");
            assert_eq!(calls.get(), 1);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn pre_completion_coordinate_error_propagates() {
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Close {
                    scope_id: "A".to_string(),
                    event_id: "e3".to_string(),
                    actor_kind: ActorKind::ClaudeCode,
                },
                None,
                |_root, _boundary| Err(CoordinateError::Other(anyhow!("snapshot capture failed"))),
                unreachable_abandon,
            )
            .await;

            assert!(result.is_err());
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn pre_completion_abandon_error_propagates() {
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Abandon {
                    scope_id: "A".to_string(),
                },
                None,
                unreachable_coordinate,
                |_root, _scope| Err(AbandonScopeError::Other(anyhow!("lock acquisition failed"))),
            )
            .await;

            assert!(result.is_err());
        }
    }

    mod real_git_db_ingress {
        use std::cell::Cell;
        use std::fs;
        use std::io::{Cursor, Error, ErrorKind, Write};
        use std::path::{Path, PathBuf};
        use std::process::Command;

        use super::*;
        use crate::services::agent_trace_storage::{
            resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
        };
        use crate::services::mutation_trace::runtime::resolve_git_dir;
        use crate::services::mutation_trace::store::decode_revision;

        fn git(dir: &Path, args: &[&str]) -> String {
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
            String::from_utf8(output.stdout).expect("git output should be UTF-8")
        }

        struct IngressRepo {
            _temp: tempfile::TempDir,
            root: PathBuf,
            state_root: PathBuf,
        }

        impl IngressRepo {
            async fn new(label: &str) -> Self {
                let temp = tempfile::Builder::new()
                    .prefix(&format!("sce-mutation-scope-ingress-{label}-"))
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
                .await
                .expect("state-root storage should initialize the repository DB");

                Self {
                    _temp: temp,
                    root,
                    state_root,
                }
            }

            async fn drive(&self, payload: &str) -> Result<String> {
                run_mutation_scope_from_payload_at_state_root(
                    &self.root,
                    &self.state_root,
                    payload,
                    None,
                )
                .await
            }

            async fn db(&self) -> RepositoryAgentTraceDb {
                crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                    &self.root,
                    &self.state_root,
                    "mutation-scope ingress test assertions",
                )
                .await
                .expect("assertion DB should open")
            }

            fn working_tree(&self) -> String {
                git(&self.root, &["add", "-A"]);
                git(&self.root, &["write-tree"]).trim().to_owned()
            }

            fn marker_path(&self) -> PathBuf {
                resolve_git_dir(&self.root)
                    .expect("git dir should resolve")
                    .join("sce")
                    .join("mutation-cursor-tainted")
            }
        }

        fn assert_raw_agent_trace_tables_untouched(db: &RepositoryAgentTraceDb) {
            assert_eq!(count(db, "diff_traces"), 0);
            assert_eq!(count(db, "post_commit_patch_intersections"), 0);
            assert_eq!(count(db, "agent_traces"), 0);
        }

        async fn count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
            db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
                row.get::<i64>(0).map_err(anyhow::Error::from)
            })
            .await
            .expect("count query should succeed")
            .into_iter()
            .next()
            .expect("a count row should exist")
        }

        async fn worktree_revision(db: &RepositoryAgentTraceDb) -> u64 {
            db.query_map("SELECT revision FROM mutation_trace_worktrees", (), |row| {
                let blob: Vec<u8> = row.get(0).map_err(anyhow::Error::from)?;
                decode_revision(&blob)
            })
            .await
            .expect("worktree revision query should succeed")
            .into_iter()
            .next()
            .expect("a worktree row should exist")
        }

        async fn cursor_tree(db: &RepositoryAgentTraceDb) -> String {
            db.query_map(
                "SELECT cursor_tree FROM mutation_trace_worktrees",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .await
            .expect("cursor_tree query should succeed")
            .into_iter()
            .next()
            .expect("a worktree row should exist")
        }

        async fn needs_rebaseline(db: &RepositoryAgentTraceDb) -> bool {
            db.query_map(
                "SELECT needs_rebaseline FROM mutation_trace_worktrees",
                (),
                |row| row.get::<i64>(0).map_err(anyhow::Error::from),
            )
            .await
            .expect("needs_rebaseline query should succeed")
            .into_iter()
            .next()
            .expect("a worktree row should exist")
                != 0
        }

        async fn processed_events(db: &RepositoryAgentTraceDb) -> Vec<(String, String)> {
            db.query_map(
                "SELECT scope_id, event_id FROM mutation_trace_processed_events \
                 ORDER BY scope_id, event_id",
                (),
                |row| {
                    let scope_id = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let event_id = row.get::<String>(1).map_err(anyhow::Error::from)?;
                    Ok((scope_id, event_id))
                },
            )
            .await
            .expect("processed-events query should succeed")
        }

        async fn scope_status(
            db: &RepositoryAgentTraceDb,
            scope_id: &str,
        ) -> Option<(String, String)> {
            db.query_map(
                "SELECT actor_kind, status FROM mutation_trace_scopes WHERE scope_id = ?1",
                (scope_id,),
                |row| {
                    let actor_kind = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let status = row.get::<String>(1).map_err(anyhow::Error::from)?;
                    Ok((actor_kind, status))
                },
            )
            .await
            .expect("scope query should succeed")
            .into_iter()
            .next()
        }

        async fn scope_provenance(
            db: &RepositoryAgentTraceDb,
            scope_id: &str,
        ) -> Option<(String, Option<String>)> {
            db.query_map(
                "SELECT session_id, model_id FROM mutation_trace_scope_provenance \
                 WHERE scope_id = ?1",
                (scope_id,),
                |row| {
                    let session_id = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let model_id = row.get::<Option<String>>(1).map_err(anyhow::Error::from)?;
                    Ok((session_id, model_id))
                },
            )
            .await
            .expect("scope-provenance query should succeed")
            .into_iter()
            .next()
        }

        async fn mutation_events(
            db: &RepositoryAgentTraceDb,
        ) -> Vec<(String, Option<String>, String)> {
            db.query_map(
                "SELECT attribution_kind, attribution_scope_id, boundary_kind \
                 FROM mutation_trace_events ORDER BY revision",
                (),
                |row| {
                    let attribution_kind = row.get::<String>(0).map_err(anyhow::Error::from)?;
                    let attribution_scope_id =
                        row.get::<Option<String>>(1).map_err(anyhow::Error::from)?;
                    let boundary_kind = row.get::<String>(2).map_err(anyhow::Error::from)?;
                    Ok((attribution_kind, attribution_scope_id, boundary_kind))
                },
            )
            .await
            .expect("mutation-events query should succeed")
        }

        struct LostArmedWriter;

        impl Write for LostArmedWriter {
            fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
                Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "injected lost Armed delivery",
                ))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "injected lost Armed delivery",
                ))
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn hidden_guard_lost_armed_ack_never_runs_the_exec_command() {
            let repo = IngressRepo::new("guard-lost-armed-ack");
            let target = repo.root.join("lost-armed-side-effect");
            let command = format!("touch '{}'", target.display());
            let input = format!(
                "{{\"operation\":\"arm\"}}\n{{\"operation\":\"exec\",\"command\":{command:?}}}\n"
            );

            let result = run_external_mutation_guard_protocol_with(
                &repo.root,
                None,
                Cursor::new(input.into_bytes()),
                LostArmedWriter,
                async |root| {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        root,
                        &repo.state_root,
                        "lost Armed guard transport test",
                    )
                    .await
                },
            )
            .await;

            assert!(result.is_err());
            assert!(!target.exists(), "lost Armed must not run the exec command");
            assert!(
                repo.marker_path().exists(),
                "ambiguous establishment remains conservatively tainted"
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn hidden_guard_cancel_after_armed_exits_without_running_a_shell() {
            let repo = IngressRepo::new("guard-cancel-before-exec");
            let target = repo.root.join("cancel-side-effect");
            let mut output = Vec::new();

            run_external_mutation_guard_protocol_with(
                &repo.root,
                None,
                Cursor::new(b"{\"operation\":\"arm\"}\n{\"operation\":\"cancel\"}\n".to_vec()),
                &mut output,
                async |root| {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        root,
                        &repo.state_root,
                        "cancel before exec transport test",
                    )
                    .await
                },
            )
            .await
            .expect("cancel before exec should terminate cleanly");

            assert!(!target.exists());
            assert!(repo.marker_path().exists());
            assert_eq!(
                String::from_utf8(output)
                    .expect("guard output should be UTF-8")
                    .lines()
                    .count(),
                1,
                "only Armed should be emitted"
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn hidden_guard_eof_after_armed_exits_without_running_a_shell() {
            let repo = IngressRepo::new("guard-arm-without-exec");
            let target = repo.root.join("eof-side-effect");
            let mut output = Vec::new();

            run_external_mutation_guard_protocol_with(
                &repo.root,
                None,
                Cursor::new(b"{\"operation\":\"arm\"}\n".to_vec()),
                &mut output,
                async |root| {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        root,
                        &repo.state_root,
                        "arm without exec transport test",
                    )
                    .await
                },
            )
            .await
            .expect("EOF before exec should terminate cleanly");

            assert!(!target.exists());
            assert!(repo.marker_path().exists());
            assert_eq!(
                String::from_utf8(output)
                    .expect("guard output should be UTF-8")
                    .lines()
                    .count(),
                1,
                "only Armed should be emitted"
            );
            repo.drive(FLUSH)
                .await
                .expect("next boundary should self-heal marker");
            assert!(!repo.marker_path().exists());
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn hidden_guard_transport_arms_then_executes_only_the_explicit_exec_command() {
            let repo = IngressRepo::new("guard-two-phase-transport");
            let target = repo.root.join("exec-side-effect");
            let duplicate_target = repo.root.join("duplicate-exec-side-effect");
            let command = format!("touch '{}'", target.display());
            let duplicate_command = format!("touch '{}'", duplicate_target.display());
            let input = format!(
                "{{\"operation\":\"arm\"}}\n{{\"operation\":\"exec\",\"command\":{command:?}}}\n{{\"operation\":\"exec\",\"command\":{duplicate_command:?}}}\n"
            );
            let mut output = Vec::new();

            run_external_mutation_guard_protocol_with(
                &repo.root,
                None,
                Cursor::new(input.into_bytes()),
                &mut output,
                async |root| {
                    crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                        root,
                        &repo.state_root,
                        "two-phase guard transport test",
                    )
                    .await
                },
            )
            .await
            .expect("the hidden guard transport should complete");

            assert!(target.exists(), "the side effect must occur after exec");
            assert!(
                !duplicate_target.exists(),
                "a duplicate exec must not launch a second shell"
            );
            let output = String::from_utf8(output).expect("guard output should be UTF-8");
            let lines: Vec<Value> = output
                .lines()
                .map(|line| serde_json::from_str(line).expect("guard output line should be JSON"))
                .collect();
            assert_eq!(
                lines.first().and_then(|line| line.get("status")),
                Some(&json!("armed"))
            );
            assert_eq!(
                lines.last().and_then(|line| line.get("status")),
                Some(&json!("result"))
            );
            assert_eq!(
                lines.last().and_then(|line| line.get("exit_code")),
                Some(&json!(0))
            );
        }

        const START_A_E1: &str =
            r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"claude_code"}"#;
        const ADVANCE_A_E2: &str =
            r#"{"operation":"advance","scope_id":"A","event_id":"e2","actor_kind":"claude_code"}"#;
        const CLOSE_A_E3: &str =
            r#"{"operation":"close","scope_id":"A","event_id":"e3","actor_kind":"claude_code"}"#;
        const START_A_E1_WITH_PROVENANCE: &str = r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"claude_code","provenance":{"session_id":"cc_session-1","model_id":"claude/opus"}}"#;
        const FLUSH: &str = r#"{"operation":"flush"}"#;
        const ABANDON_A: &str = r#"{"operation":"abandon","scope_id":"A"}"#;

        #[tokio::test(flavor = "multi_thread")]
        #[allow(clippy::too_many_lines)]
        async fn test1_observed_start_advance_close_lifecycle_persists_durable_rows() {
            let repo = IngressRepo::new("observed-lifecycle");

            assert_eq!(repo.drive(START_A_E1).expect("start should succeed"), "");
            fs::write(repo.root.join("file.txt"), "one\ntwo\n")
                .await
                .expect("the scoped edit should write");
            assert_eq!(
                repo.drive(ADVANCE_A_E2).expect("advance should succeed"),
                ""
            );
            assert_eq!(repo.drive(CLOSE_A_E3).expect("close should succeed"), "");

            let db = repo.db().await;
            assert_eq!(
                scope_status(&db, "A").map(|(_, status)| status),
                Some("closed".to_string())
            );
            assert_eq!(
                processed_events(&db),
                vec![
                    ("A".to_string(), "e1".to_string()),
                    ("A".to_string(), "e2".to_string()),
                    ("A".to_string(), "e3".to_string()),
                ]
            );
            assert_eq!(
                mutation_events(&db),
                vec![(
                    "ai_exclusive".to_string(),
                    Some("A".to_string()),
                    "advance".to_string(),
                )]
            );
            assert_eq!(cursor_tree(&db), repo.working_tree());

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test2_replayed_advance_is_fully_idempotent() {
            let repo = IngressRepo::new("replay-idempotent");

            repo.drive(START_A_E1).await.expect("start should succeed");
            fs::write(repo.root.join("file.txt"), "one\ntwo\n")
                .expect("the scoped edit should write");
            repo.drive(ADVANCE_A_E2)
                .await
                .expect("the first advance should succeed");

            let (revision_before, events_before, processed_before) = {
                let db = repo.db().await;
                (
                    worktree_revision(&db).await,
                    count(&db, "mutation_trace_events").await,
                    count(&db, "mutation_trace_processed_events").await,
                )
            };

            assert_eq!(
                repo.drive(ADVANCE_A_E2)
                    .expect("the replayed advance should succeed"),
                ""
            );

            let db = repo.db().await;
            assert_eq!(worktree_revision(&db), revision_before);
            assert_eq!(count(&db, "mutation_trace_events"), events_before);
            assert_eq!(
                count(&db, "mutation_trace_processed_events"),
                processed_before
            );
            assert_eq!(
                processed_events(&db)
                    .into_iter()
                    .filter(|(scope_id, event_id)| scope_id == "A" && event_id == "e2")
                    .count(),
                1
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        #[allow(clippy::too_many_lines)]
        async fn test3_conflicting_actor_kind_commits_no_second_boundary() {
            let repo = IngressRepo::new("actor-conflict");

            repo.drive(START_A_E1).await.expect("start should succeed");

            let (revision_before, processed_before, scope_before, events_before) = {
                let db = repo.db().await;
                (
                    worktree_revision(&db).await,
                    processed_events(&db).await,
                    scope_status(&db, "A").await,
                    count(&db, "mutation_trace_events").await,
                )
            };

            let error = repo
                .drive(
                    r#"{"operation":"advance","scope_id":"A","event_id":"e2","actor_kind":"codex"}"#,
                ).await
                .expect_err(
                    "a conflicting actor_kind must fail the ingress, not commit a boundary",
                );

            let rendered = format!("{error:#}");
            assert!(
                rendered.contains("is already registered to actor"),
                "the ingress error must carry the scope/actor identity mismatch diagnostic, \
                 got: {rendered}"
            );

            let db = repo.db().await;
            assert_eq!(worktree_revision(&db), revision_before);
            assert_eq!(processed_events(&db), processed_before);
            assert!(!processed_events(&db)
                .into_iter()
                .any(|(scope_id, event_id)| scope_id == "A" && event_id == "e2"));
            assert_eq!(scope_status(&db, "A"), scope_before);
            assert_eq!(
                scope_status(&db, "A").map(|(actor_kind, _)| actor_kind),
                Some("claude_code".to_string())
            );
            assert_eq!(count(&db, "mutation_trace_events"), events_before);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        #[allow(clippy::too_many_lines)]
        async fn test4_abandonment_keeps_no_snapshot_semantics_for_an_unobserved_edit() {
            let repo = IngressRepo::new("abandon-unobserved-edit");

            repo.drive(START_A_E1).await.expect("start should succeed");

            let (revision_after_start, cursor_after_start) = {
                let db = repo.db().await;
                (worktree_revision(&db).await, cursor_tree(&db).await)
            };

            fs::write(repo.root.join("file.txt"), "one\nunobserved\n")
                .await
                .expect("the unobserved edit should write");
            let edited_tree = repo.working_tree();
            assert_ne!(
                edited_tree, cursor_after_start,
                "the unobserved edit must move the Git tree"
            );

            assert_eq!(repo.drive(ABANDON_A).expect("abandon should succeed"), "");

            let db = repo.db().await;
            assert_eq!(
                scope_status(&db, "A").map(|(_, status)| status),
                Some("abandoned".to_string())
            );
            assert_eq!(worktree_revision(&db), revision_after_start + 1);
            assert!(needs_rebaseline(&db));
            assert_eq!(cursor_tree(&db), cursor_after_start);
            assert_ne!(cursor_tree(&db), edited_tree);
            assert_eq!(count(&db, "mutation_trace_events"), 0);
            assert_eq!(
                processed_events(&db),
                vec![("A".to_string(), "e1".to_string())]
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        #[allow(clippy::too_many_lines)]
        async fn test5_adversarial_flush_drives_real_observed_flush_behavior() {
            let repo = IngressRepo::new("adversarial-flush");

            assert_eq!(
                repo.drive(FLUSH)
                    .expect("the baseline flush should succeed"),
                ""
            );
            let revision_after_baseline = {
                let db = repo.db().await;
                worktree_revision(&db).await
            };

            fs::write(repo.root.join("file.txt"), "one\nunscoped\n")
                .await
                .expect("the unscoped edit should write");
            let edited_tree = repo.working_tree();

            assert_eq!(repo.drive(FLUSH).expect("the flush should succeed"), "");

            let db = repo.db().await;
            assert_eq!(cursor_tree(&db), edited_tree);
            assert_eq!(worktree_revision(&db), revision_after_baseline + 1);
            assert_eq!(
                mutation_events(&db),
                vec![("ineligible_unscoped".to_string(), None, "flush".to_string())]
            );
            assert_eq!(count(&db, "mutation_trace_scopes"), 0);
            assert_eq!(count(&db, "mutation_trace_processed_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        #[allow(clippy::too_many_lines)]
        async fn test6_marker_clear_after_commit_is_durable_success_through_the_ingress() {
            let repo = IngressRepo::new("marker-clear-after-commit");

            repo.drive(START_A_E1).await.expect("start should succeed");
            fs::write(repo.root.join("file.txt"), "one\nattributable\n")
                .await
                .expect("the scoped edit should write");

            let marker = repo.marker_path();
            let calls = Cell::new(0_u32);
            let resolver = async |root: &Path,
                                  context_message: &'static str|
                   -> Result<RepositoryAgentTraceDb> {
                calls.set(calls.get() + 1);
                fs::remove_file(&marker)
                    .expect("the armed marker file should be present mid-invocation");
                fs::create_dir_all(marker.join("nested"))
                    .expect("planting a non-empty directory at the marker path should succeed");
                crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                    root,
                    &repo.state_root,
                    context_message,
                )
                .await
            };

            let result =
                run_mutation_scope_from_payload_with(&repo.root, ADVANCE_A_E2, None, resolver)
                    .await;

            assert_eq!(
                result.expect("a post-commit marker-clear failure is durable success"),
                ""
            );
            assert_eq!(
                calls.get(),
                1,
                "the runtime entrypoint must run exactly once, with no retried transition"
            );

            let db = repo.db().await;
            assert_eq!(
                mutation_events(&db),
                vec![(
                    "ai_exclusive".to_string(),
                    Some("A".to_string()),
                    "advance".to_string(),
                )]
            );
            assert_eq!(
                processed_events(&db)
                    .into_iter()
                    .filter(|(scope_id, event_id)| scope_id == "A" && event_id == "e2")
                    .count(),
                1
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        #[allow(clippy::too_many_lines)]
        async fn test7_marker_clear_after_abandon_is_durable_success_through_the_ingress() {
            let repo = IngressRepo::new("marker-clear-after-abandon");

            repo.drive(START_A_E1).await.expect("start should succeed");
            let revision_after_start = {
                let db = repo.db().await;
                worktree_revision(&db).await
            };

            fs::write(repo.root.join("file.txt"), "one\nunobserved\n")
                .await
                .expect("the unobserved edit should write");

            let marker = repo.marker_path();
            let calls = Cell::new(0_u32);
            let resolver = async |root: &Path,
                                  context_message: &'static str|
                   -> Result<RepositoryAgentTraceDb> {
                calls.set(calls.get() + 1);
                fs::remove_file(&marker)
                    .expect("the armed marker file should be present mid-invocation");
                fs::create_dir_all(marker.join("nested"))
                    .expect("planting a non-empty directory at the marker path should succeed");
                crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                    root,
                    &repo.state_root,
                    context_message,
                )
                .await
            };

            let result =
                run_mutation_scope_from_payload_with(&repo.root, ABANDON_A, None, resolver).await;

            assert_eq!(
                result.expect("a post-completion marker-clear failure is durable success"),
                ""
            );
            assert_eq!(calls.get(), 1, "abandon_scope must run exactly once");

            let db = repo.db().await;
            assert_eq!(
                scope_status(&db, "A").map(|(_, status)| status),
                Some("abandoned".to_string())
            );
            assert_eq!(worktree_revision(&db), revision_after_start + 1);
            assert!(needs_rebaseline(&db));
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn start_registers_provenance_before_committing_protocol_start() {
            let repo = IngressRepo::new("provenance-start");

            assert_eq!(
                repo.drive(START_A_E1_WITH_PROVENANCE)
                    .expect("a start carrying provenance should succeed"),
                ""
            );

            let db = repo.db().await;
            assert_eq!(
                scope_status(&db, "A"),
                Some(("claude_code".to_string(), "active".to_string())),
                "the owning scope row must exist before provenance is registered"
            );
            assert_eq!(
                scope_provenance(&db, "A"),
                Some(("cc_session-1".to_string(), Some("claude/opus".to_string())))
            );
            assert_eq!(
                processed_events(&db),
                vec![("A".to_string(), "e1".to_string())],
                "the pure protocol Start must commit after provenance is durably registered"
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn start_without_provenance_creates_no_provenance() {
            let repo = IngressRepo::new("provenance-absent");

            repo.drive(START_A_E1)
                .await
                .expect("a start without provenance should succeed");

            let db = repo.db().await;
            assert_eq!(
                scope_status(&db, "A").map(|(_, status)| status),
                Some("active".to_string())
            );
            assert_eq!(
                count(&db, "mutation_trace_scope_provenance"),
                0,
                "a start that carried no provenance must not reach provenance persistence"
            );
            assert_eq!(
                processed_events(&db),
                vec![("A".to_string(), "e1".to_string())]
            );

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn replayed_start_with_provenance_commits_nothing_new() {
            let repo = IngressRepo::new("provenance-replay");

            repo.drive(START_A_E1_WITH_PROVENANCE)
                .await
                .expect("the first start should succeed");
            let (revision_before, processed_before, provenance_before) = {
                let db = repo.db().await;
                (
                    worktree_revision(&db).await,
                    processed_events(&db).await,
                    scope_provenance(&db, "A").await,
                )
            };

            assert_eq!(
                repo.drive(START_A_E1_WITH_PROVENANCE)
                    .expect("an identical replay should succeed"),
                ""
            );

            let db = repo.db().await;
            assert_eq!(scope_provenance(&db, "A"), provenance_before);
            assert_eq!(worktree_revision(&db), revision_before);
            assert_eq!(processed_events(&db), processed_before);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn provenance_conflict_prevents_start_commit() {
            let repo = IngressRepo::new("provenance-session-conflict");

            repo.drive(START_A_E1_WITH_PROVENANCE)
                .await
                .expect("the first start should succeed");

            let (revision_before, processed_before, provenance_before) = {
                let db = repo.db().await;
                (
                    worktree_revision(&db).await,
                    processed_events(&db).await,
                    scope_provenance(&db, "A").await,
                )
            };

            let error = repo
                .drive(
                    r#"{"operation":"start","scope_id":"A","event_id":"e9","actor_kind":"claude_code","provenance":{"session_id":"cc_session-2","model_id":"claude/opus"}}"#,
                ).await
                .expect_err("a conflicting provenance session must fail the start");

            let rendered = format!("{error:#}");
            assert!(
                rendered.contains("already has provenance for session"),
                "the ingress error must carry the provenance session conflict diagnostic, \
                 got: {rendered}"
            );

            let db = repo.db().await;
            assert_eq!(
                processed_events(&db),
                processed_before,
                "the fresh event e9 must not be processed once provenance registration fails"
            );
            assert_eq!(worktree_revision(&db), revision_before);
            assert_eq!(scope_provenance(&db, "A"), provenance_before);

            assert_raw_agent_trace_tables_untouched(&db);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn admitted_scope_is_never_backfilled_with_replay_provenance() {
            let repo = IngressRepo::new("provenance-no-late-backfill");

            repo.drive(START_A_E1)
                .await
                .expect("a start without provenance should succeed");

            let (revision_before, processed_before) = {
                let db = repo.db().await;
                assert_eq!(
                    scope_status(&db, "A").map(|(_, status)| status),
                    Some("active".to_string())
                );
                assert_eq!(scope_provenance(&db, "A"), None);
                (worktree_revision(&db).await, processed_events(&db).await)
            };

            assert_eq!(
                repo.drive(START_A_E1_WITH_PROVENANCE)
                    .expect("replaying the start with provenance should follow replay semantics"),
                ""
            );

            let db = repo.db().await;
            assert_eq!(
                scope_provenance(&db, "A"),
                None,
                "provenance may only be created while the scope is never_seen"
            );
            assert_eq!(count(&db, "mutation_trace_scope_provenance"), 0);
            assert_eq!(
                scope_status(&db, "A").map(|(_, status)| status),
                Some("active".to_string())
            );
            assert_eq!(worktree_revision(&db), revision_before);
            assert_eq!(processed_events(&db), processed_before);

            assert_raw_agent_trace_tables_untouched(&db);
        }
    }

    mod guard_protocol {
        use super::*;

        #[test]
        fn arm_has_no_execution_fields() {
            assert!(parse_guard_arm(r#"{"operation":"arm"}"#).is_ok());
            assert!(parse_guard_arm(r#"{"operation":"arm","command":"true"}"#).is_err());
            assert!(parse_guard_arm(r#"{"operation":"guard","command":"true"}"#).is_err());
        }

        #[test]
        fn malformed_arm_is_rejected() {
            assert!(parse_guard_arm("").is_err());
            assert!(parse_guard_arm("{").is_err());
            assert!(parse_guard_arm(r#"{"operation":"start"}"#).is_err());
        }

        #[test]
        fn exec_requires_a_non_blank_command() {
            assert!(parse_guard_exec(r#"{"operation":"exec","command":"pwd"}"#).is_ok());
            assert!(parse_guard_exec(r#"{"operation":"exec","command":"   "}"#).is_err());
            assert!(parse_guard_exec(r#"{"operation":"exec"}"#).is_err());
            assert!(parse_guard_exec(r#"{"operation":"guard","command":"pwd"}"#).is_err());
        }

        #[test]
        fn exec_parses_cwd_and_environment() {
            let request = parse_guard_exec(
                r#"{"operation":"exec","command":"pwd","cwd":"/repo/crates/foo","env":{"FOO":"bar","BAZ":"qux"}}"#,
            )
            .unwrap();
            assert_eq!(request.command, "pwd");
            assert_eq!(request.cwd, Some("/repo/crates/foo".to_string()));
            let mut env = request.env;
            env.sort();
            assert_eq!(
                env,
                vec![("BAZ".into(), "qux".into()), ("FOO".into(), "bar".into())]
            );
        }

        #[test]
        fn invalid_exec_fields_are_rejected() {
            assert!(
                parse_guard_exec(r#"{"operation":"exec","command":"pwd","cwd":"   "}"#).is_err()
            );
            assert!(
                parse_guard_exec(r#"{"operation":"exec","command":"pwd","env":{"FOO":1}}"#)
                    .is_err()
            );
            assert!(
                parse_guard_exec(r#"{"operation":"exec","command":"pwd","env":"nope"}"#).is_err()
            );
            assert!(
                parse_guard_exec(r#"{"operation":"exec","command":"pwd","extra":true}"#).is_err()
            );
        }

        #[test]
        fn cancel_is_a_strict_control_frame() {
            assert!(parse_guard_cancel(r#"{"operation":"cancel"}"#).is_ok());
            assert!(parse_guard_cancel(r#"{"operation":"cancel","command":"pwd"}"#).is_err());
            assert_eq!(
                guard_operation(r#"{"operation":"cancel"}"#).as_deref(),
                Some("cancel")
            );
            assert_eq!(guard_operation("not json"), None);
        }

        #[test]
        fn armed_event_serializes_without_a_stream_field() {
            let line = guard_event_json_line(&GuardEvent::Armed);
            let parsed: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(parsed["status"], "armed");
        }

        #[test]
        fn stdout_and_stderr_events_tag_their_stream() {
            let stdout_line = guard_event_json_line(&GuardEvent::Stdout(b"hello".to_vec()));
            let parsed: Value = serde_json::from_str(&stdout_line).unwrap();
            assert_eq!(parsed["stream"], "stdout");
            assert_eq!(parsed["data"], "hello");

            let stderr_line = guard_event_json_line(&GuardEvent::Stderr(b"oops".to_vec()));
            let parsed: Value = serde_json::from_str(&stderr_line).unwrap();
            assert_eq!(parsed["stream"], "stderr");
            assert_eq!(parsed["data"], "oops");
        }
    }
}
