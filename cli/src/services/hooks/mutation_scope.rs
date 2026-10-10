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
    )
    .await?;

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
