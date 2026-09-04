use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::mutation_trace::runtime::{
    abandon_scope, coordinate, AbandonScopeError, AbandonScopeOutcome, CoordinateError,
    CoordinateOutcome, RuntimeBoundary,
};
use crate::services::mutation_trace::types::{ActorKind, EventId, ScopeId};
use crate::services::observability::traits::Logger;

const MUTATION_SCOPE_DB_CONTEXT: &str = "Failed to open Agent Trace DB for mutation-scope runtime.";

const OPERATION_FIELD: &str = "operation";
const SCOPE_ID_FIELD: &str = "scope_id";
const EVENT_ID_FIELD: &str = "event_id";
const ACTOR_KIND_FIELD: &str = "actor_kind";
const WORKTREE_ID_FIELD: &str = "worktree_id";

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
        "start" => parse_scope_boundary(object, |scope_id, event_id, actor_kind| {
            MutationScopePayload::Start {
                scope_id,
                event_id,
                actor_kind,
            }
        }),
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

fn parse_scope_boundary(
    object: &Map<String, Value>,
    build: impl FnOnce(String, String, ActorKind) -> MutationScopePayload,
) -> Result<MutationScopePayload> {
    reject_unexpected_keys(
        object,
        &[
            OPERATION_FIELD,
            SCOPE_ID_FIELD,
            EVENT_ID_FIELD,
            ACTOR_KIND_FIELD,
        ],
    )?;

    let scope_id = required_non_blank_str(object, SCOPE_ID_FIELD)?;
    let event_id = required_non_blank_str(object, EVENT_ID_FIELD)?;
    let actor_kind = parse_actor_kind(&required_str(object, ACTOR_KIND_FIELD)?)?;

    Ok(build(scope_id, event_id, actor_kind))
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

fn validation_error(detail: &str) -> String {
    format!("Invalid mutation-scope payload from STDIN: {detail}.")
}

pub(crate) fn run_mutation_scope_subcommand(
    repository_root: &Path,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    let stdin_payload = super::read_hook_stdin()?;
    run_mutation_scope_from_payload(repository_root, &stdin_payload, logger)
}

fn run_mutation_scope_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_mutation_scope_from_payload_with(
        repository_root,
        stdin_payload,
        logger,
        super::open_agent_trace_db_for_hook_runtime,
    )
}

#[cfg(test)]
pub(super) fn run_mutation_scope_from_payload_at_state_root(
    repository_root: &Path,
    state_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_mutation_scope_from_payload_with(
        repository_root,
        stdin_payload,
        logger,
        |root, context_message| {
            super::open_agent_trace_db_for_hook_runtime_at_state_root(
                root,
                state_root,
                context_message,
            )
        },
    )
}

fn run_mutation_scope_from_payload_with<O>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    open_db: O,
) -> Result<String>
where
    O: Fn(&Path, &'static str) -> Result<RepositoryAgentTraceDb> + Copy,
{
    let payload = parse_mutation_scope_payload(stdin_payload)?;

    drive_mutation_scope(
        repository_root,
        payload,
        logger,
        |root, boundary| coordinate(root, boundary, || open_db(root, MUTATION_SCOPE_DB_CONTEXT)),
        |root, scope| abandon_scope(root, scope, || open_db(root, MUTATION_SCOPE_DB_CONTEXT)),
    )
}

fn drive_mutation_scope<C, A>(
    repository_root: &Path,
    payload: MutationScopePayload,
    logger: Option<&dyn Logger>,
    coordinate_boundary: C,
    abandon: A,
) -> Result<String>
where
    C: FnOnce(&Path, &RuntimeBoundary) -> std::result::Result<CoordinateOutcome, CoordinateError>,
    A: FnOnce(&Path, &ScopeId) -> std::result::Result<AbandonScopeOutcome, AbandonScopeError>,
{
    let boundary = match payload {
        MutationScopePayload::Start {
            scope_id,
            event_id,
            actor_kind,
        } => RuntimeBoundary::Start {
            scope: ScopeId(scope_id),
            event: EventId(event_id),
            actor_kind,
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
            return classify_abandon(abandon(repository_root, &ScopeId(scope_id)), logger);
        }
    };

    classify_coordinate(coordinate_boundary(repository_root, &boundary), logger)
}

fn classify_coordinate(
    result: std::result::Result<CoordinateOutcome, CoordinateError>,
    logger: Option<&dyn Logger>,
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

fn classify_abandon(
    result: std::result::Result<AbandonScopeOutcome, AbandonScopeError>,
    logger: Option<&dyn Logger>,
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

fn log_marker_clear_after_durable_completion(
    logger: Option<&dyn Logger>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(payload: &str) -> Result<MutationScopePayload> {
        parse_mutation_scope_payload(payload)
    }

    fn error_of(payload: &str) -> String {
        parse(payload)
            .expect_err("expected the payload to be rejected")
            .to_string()
    }

    #[test]
    fn start_maps_all_fields_verbatim() {
        let payload = parse(
            r#"{"operation":"start","scope_id":"  scope-A  ","event_id":"e1","actor_kind":"claude_code"}"#,
        )
        .expect("valid start payload");

        assert_eq!(
            payload,
            MutationScopePayload::Start {
                scope_id: "  scope-A  ".to_string(),
                event_id: "e1".to_string(),
                actor_kind: ActorKind::ClaudeCode,
            }
        );
    }

    #[test]
    fn advance_and_close_parse_to_their_variants() {
        assert_eq!(
            parse(r#"{"operation":"advance","scope_id":"A","event_id":"e2","actor_kind":"codex"}"#)
                .expect("valid advance payload"),
            MutationScopePayload::Advance {
                scope_id: "A".to_string(),
                event_id: "e2".to_string(),
                actor_kind: ActorKind::Codex,
            }
        );

        assert_eq!(
            parse(
                r#"{"operation":"close","scope_id":"A","event_id":"e3","actor_kind":"opencode"}"#
            )
            .expect("valid close payload"),
            MutationScopePayload::Close {
                scope_id: "A".to_string(),
                event_id: "e3".to_string(),
                actor_kind: ActorKind::OpenCode,
            }
        );
    }

    #[test]
    fn every_actor_kind_wire_string_maps() {
        for (wire, expected) in [
            ("claude_code", ActorKind::ClaudeCode),
            ("codex", ActorKind::Codex),
            ("opencode", ActorKind::OpenCode),
            ("pi", ActorKind::Pi),
        ] {
            let payload = parse(&format!(
                r#"{{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"{wire}"}}"#
            ))
            .expect("valid start payload");
            match payload {
                MutationScopePayload::Start { actor_kind, .. } => assert_eq!(actor_kind, expected),
                other => panic!("expected Start, got {other:?}"),
            }
        }
    }

    #[test]
    fn flush_takes_no_identity_fields() {
        assert_eq!(
            parse(r#"{"operation":"flush"}"#).expect("valid flush payload"),
            MutationScopePayload::Flush
        );
    }

    #[test]
    fn abandon_takes_only_scope_id() {
        assert_eq!(
            parse(r#"{"operation":"abandon","scope_id":"A"}"#).expect("valid abandon payload"),
            MutationScopePayload::Abandon {
                scope_id: "A".to_string(),
            }
        );
    }

    #[test]
    fn empty_or_blank_payload_is_rejected() {
        assert!(parse("").is_err());
        assert!(parse("   \n\t ").is_err());
    }

    #[test]
    fn malformed_json_is_rejected() {
        assert!(parse("{").is_err());
        assert!(parse(r#"{"operation":"start""#).is_err());
        assert!(parse("not json at all").is_err());
    }

    #[test]
    fn non_object_json_is_rejected() {
        assert!(parse("123").is_err());
        assert!(parse(r#""start""#).is_err());
        assert!(parse(r#"["start"]"#).is_err());
        assert!(parse("null").is_err());
    }

    #[test]
    fn missing_operation_is_rejected() {
        assert!(parse(r#"{"scope_id":"A","event_id":"e1","actor_kind":"pi"}"#).is_err());
    }

    #[test]
    fn unknown_operation_is_rejected() {
        let error =
            error_of(r#"{"operation":"reopen","scope_id":"A","event_id":"e1","actor_kind":"pi"}"#);
        assert!(error.contains("'operation'"), "unexpected error: {error}");
    }

    #[test]
    fn operation_wrong_type_is_rejected() {
        assert!(parse(r#"{"operation":5}"#).is_err());
    }

    #[test]
    fn unknown_actor_kind_is_rejected() {
        let error = error_of(
            r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"cursor"}"#,
        );
        assert!(error.contains("'actor_kind'"), "unexpected error: {error}");
    }

    #[test]
    fn missing_scope_or_event_or_actor_is_rejected() {
        assert!(parse(r#"{"operation":"start","event_id":"e1","actor_kind":"pi"}"#).is_err());
        assert!(parse(r#"{"operation":"start","scope_id":"A","actor_kind":"pi"}"#).is_err());
        assert!(parse(r#"{"operation":"start","scope_id":"A","event_id":"e1"}"#).is_err());
    }

    #[test]
    fn empty_or_blank_scope_id_or_event_id_is_rejected() {
        assert!(
            parse(r#"{"operation":"start","scope_id":"","event_id":"e1","actor_kind":"pi"}"#)
                .is_err()
        );
        assert!(parse(
            r#"{"operation":"start","scope_id":"   ","event_id":"e1","actor_kind":"pi"}"#
        )
        .is_err());
        assert!(
            parse(r#"{"operation":"start","scope_id":"A","event_id":"","actor_kind":"pi"}"#)
                .is_err()
        );
        assert!(
            parse(r#"{"operation":"start","scope_id":"A","event_id":"\t","actor_kind":"pi"}"#)
                .is_err()
        );
        assert!(parse(r#"{"operation":"abandon","scope_id":"  "}"#).is_err());
    }

    #[test]
    fn wrong_field_json_type_is_rejected() {
        assert!(
            parse(r#"{"operation":"start","scope_id":123,"event_id":"e1","actor_kind":"pi"}"#)
                .is_err()
        );
        assert!(
            parse(r#"{"operation":"start","scope_id":"A","event_id":true,"actor_kind":"pi"}"#)
                .is_err()
        );
        assert!(parse(
            r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":["pi"]}"#
        )
        .is_err());
    }

    #[test]
    fn unexpected_field_is_rejected() {
        let error = error_of(
            r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"pi","attempt_id":"x"}"#,
        );
        assert!(
            error.contains("unexpected field 'attempt_id'"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn any_worktree_id_key_is_rejected_with_a_dedicated_diagnostic() {
        for payload in [
            r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"pi","worktree_id":"wt"}"#,
            r#"{"operation":"flush","worktree_id":"wt"}"#,
            r#"{"operation":"abandon","scope_id":"A","worktree_id":"wt"}"#,
        ] {
            let error = error_of(payload);
            assert!(
                error.contains("'worktree_id'"),
                "unexpected error for {payload}: {error}"
            );
        }
    }

    #[test]
    fn flush_rejects_any_scope_event_or_actor_field() {
        assert!(parse(r#"{"operation":"flush","scope_id":"A"}"#).is_err());
        assert!(parse(r#"{"operation":"flush","event_id":"e1"}"#).is_err());
        assert!(parse(r#"{"operation":"flush","actor_kind":"pi"}"#).is_err());
    }

    #[test]
    fn abandon_rejects_event_and_actor_fields() {
        assert!(parse(r#"{"operation":"abandon","scope_id":"A","event_id":"e1"}"#).is_err());
        assert!(parse(r#"{"operation":"abandon","scope_id":"A","actor_kind":"pi"}"#).is_err());
    }

    #[test]
    fn abandon_missing_scope_id_is_rejected() {
        assert!(parse(r#"{"operation":"abandon"}"#).is_err());
    }

    mod runtime_dispatch {
        use std::cell::Cell;

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

        #[test]
        fn start_forwards_identities_verbatim_to_coordinate() {
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Start {
                    scope_id: "A".to_string(),
                    event_id: "e1".to_string(),
                    actor_kind: ActorKind::ClaudeCode,
                },
                None,
                |_root, boundary| {
                    match boundary {
                        RuntimeBoundary::Start {
                            scope,
                            event,
                            actor_kind,
                        } => {
                            assert_eq!(scope.0, "A");
                            assert_eq!(event.0, "e1");
                            assert_eq!(*actor_kind, ActorKind::ClaudeCode);
                        }
                        other => panic!("expected RuntimeBoundary::Start, got {other:?}"),
                    }
                    Ok(committed_outcome())
                },
                unreachable_abandon,
            );

            assert_eq!(result.expect("start should succeed"), "");
        }

        #[test]
        fn flush_maps_to_flush_boundary_without_identity() {
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Flush,
                None,
                |_root, boundary| {
                    assert!(matches!(boundary, RuntimeBoundary::Flush));
                    Ok(committed_outcome())
                },
                unreachable_abandon,
            );

            assert_eq!(result.expect("flush should succeed"), "");
        }

        #[test]
        fn abandon_calls_abandon_scope_only() {
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Abandon {
                    scope_id: "A".to_string(),
                },
                None,
                unreachable_coordinate,
                |_root, scope| {
                    assert_eq!(scope.0, "A");
                    Ok(abandoned_outcome())
                },
            );

            assert_eq!(result.expect("abandon should succeed"), "");
        }

        #[test]
        fn successful_boundary_produces_empty_stdout() {
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Advance {
                    scope_id: "A".to_string(),
                    event_id: "e2".to_string(),
                    actor_kind: ActorKind::Codex,
                },
                None,
                |_root, _boundary| Ok(committed_outcome()),
                unreachable_abandon,
            );

            assert_eq!(result.expect("advance should succeed"), "");
        }

        #[test]
        fn marker_clear_after_commit_is_durable_success_without_reexecution() {
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
            );

            assert_eq!(result.expect("carried outcome is durable success"), "");
            assert_eq!(calls.get(), 1);
        }

        #[test]
        fn marker_clear_after_completion_is_durable_success_without_reexecution() {
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
            );

            assert_eq!(result.expect("carried outcome is durable success"), "");
            assert_eq!(calls.get(), 1);
        }

        #[test]
        fn pre_completion_coordinate_error_propagates() {
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
            );

            assert!(result.is_err());
        }

        #[test]
        fn pre_completion_abandon_error_propagates() {
            let result = drive_mutation_scope(
                Path::new("/unused"),
                MutationScopePayload::Abandon {
                    scope_id: "A".to_string(),
                },
                None,
                unreachable_coordinate,
                |_root, _scope| Err(AbandonScopeError::Other(anyhow!("lock acquisition failed"))),
            );

            assert!(result.is_err());
        }

        #[test]
        fn malformed_payload_returns_err() {
            let result = run_mutation_scope_from_payload(Path::new("/unused"), "{", None);
            assert!(result.is_err());
        }
    }

    mod real_git_db_ingress {
        use std::cell::Cell;
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::process::Command;

        use super::*;
        use crate::services::agent_trace_storage::{
            resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
        };
        use crate::services::checkout::resolve_git_dir;
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
            fn new(label: &str) -> Self {
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
                .expect("state-root storage should initialize the repository DB");

                Self {
                    _temp: temp,
                    root,
                    state_root,
                }
            }

            fn drive(&self, payload: &str) -> Result<String> {
                run_mutation_scope_from_payload_at_state_root(
                    &self.root,
                    &self.state_root,
                    payload,
                    None,
                )
            }

            fn db(&self) -> RepositoryAgentTraceDb {
                crate::services::hooks::open_agent_trace_db_for_hook_runtime_at_state_root(
                    &self.root,
                    &self.state_root,
                    "mutation-scope ingress test assertions",
                )
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

        fn count(db: &RepositoryAgentTraceDb, table: &str) -> i64 {
            db.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| {
                row.get::<i64>(0).map_err(anyhow::Error::from)
            })
            .expect("count query should succeed")
            .into_iter()
            .next()
            .expect("a count row should exist")
        }

        fn worktree_revision(db: &RepositoryAgentTraceDb) -> u64 {
            db.query_map("SELECT revision FROM mutation_trace_worktrees", (), |row| {
                let blob: Vec<u8> = row.get(0).map_err(anyhow::Error::from)?;
                decode_revision(&blob)
            })
            .expect("worktree revision query should succeed")
            .into_iter()
            .next()
            .expect("a worktree row should exist")
        }

        fn cursor_tree(db: &RepositoryAgentTraceDb) -> String {
            db.query_map(
                "SELECT cursor_tree FROM mutation_trace_worktrees",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("cursor_tree query should succeed")
            .into_iter()
            .next()
            .expect("a worktree row should exist")
        }

        fn needs_rebaseline(db: &RepositoryAgentTraceDb) -> bool {
            db.query_map(
                "SELECT needs_rebaseline FROM mutation_trace_worktrees",
                (),
                |row| row.get::<i64>(0).map_err(anyhow::Error::from),
            )
            .expect("needs_rebaseline query should succeed")
            .into_iter()
            .next()
            .expect("a worktree row should exist")
                != 0
        }

        fn processed_events(db: &RepositoryAgentTraceDb) -> Vec<(String, String)> {
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
            .expect("processed-events query should succeed")
        }

        fn scope_status(db: &RepositoryAgentTraceDb, scope_id: &str) -> Option<(String, String)> {
            db.query_map(
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

        fn mutation_events(db: &RepositoryAgentTraceDb) -> Vec<(String, Option<String>, String)> {
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
            .expect("mutation-events query should succeed")
        }

        const START_A_E1: &str =
            r#"{"operation":"start","scope_id":"A","event_id":"e1","actor_kind":"claude_code"}"#;
        const ADVANCE_A_E2: &str =
            r#"{"operation":"advance","scope_id":"A","event_id":"e2","actor_kind":"claude_code"}"#;
        const CLOSE_A_E3: &str =
            r#"{"operation":"close","scope_id":"A","event_id":"e3","actor_kind":"claude_code"}"#;
        const FLUSH: &str = r#"{"operation":"flush"}"#;
        const ABANDON_A: &str = r#"{"operation":"abandon","scope_id":"A"}"#;

        #[test]
        #[allow(clippy::too_many_lines)]
        fn test1_observed_start_advance_close_lifecycle_persists_durable_rows() {
            let repo = IngressRepo::new("observed-lifecycle");

            assert_eq!(repo.drive(START_A_E1).expect("start should succeed"), "");
            fs::write(repo.root.join("file.txt"), "one\ntwo\n")
                .expect("the scoped edit should write");
            assert_eq!(
                repo.drive(ADVANCE_A_E2).expect("advance should succeed"),
                ""
            );
            assert_eq!(repo.drive(CLOSE_A_E3).expect("close should succeed"), "");

            let db = repo.db();
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

        #[test]
        fn test2_replayed_advance_is_fully_idempotent() {
            let repo = IngressRepo::new("replay-idempotent");

            repo.drive(START_A_E1).expect("start should succeed");
            fs::write(repo.root.join("file.txt"), "one\ntwo\n")
                .expect("the scoped edit should write");
            repo.drive(ADVANCE_A_E2)
                .expect("the first advance should succeed");

            let (revision_before, events_before, processed_before) = {
                let db = repo.db();
                (
                    worktree_revision(&db),
                    count(&db, "mutation_trace_events"),
                    count(&db, "mutation_trace_processed_events"),
                )
            };

            assert_eq!(
                repo.drive(ADVANCE_A_E2)
                    .expect("the replayed advance should succeed"),
                ""
            );

            let db = repo.db();
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

        #[test]
        #[allow(clippy::too_many_lines)]
        fn test3_conflicting_actor_kind_commits_no_second_boundary() {
            let repo = IngressRepo::new("actor-conflict");

            repo.drive(START_A_E1).expect("start should succeed");

            let (revision_before, processed_before, scope_before, events_before) = {
                let db = repo.db();
                (
                    worktree_revision(&db),
                    processed_events(&db),
                    scope_status(&db, "A"),
                    count(&db, "mutation_trace_events"),
                )
            };

            let error = repo
                .drive(
                    r#"{"operation":"advance","scope_id":"A","event_id":"e2","actor_kind":"codex"}"#,
                )
                .expect_err(
                    "a conflicting actor_kind must fail the ingress, not commit a boundary",
                );

            let rendered = format!("{error:#}");
            assert!(
                rendered.contains("is already registered to actor"),
                "the ingress error must carry the scope/actor identity mismatch diagnostic, \
                 got: {rendered}"
            );

            let db = repo.db();
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

        #[test]
        #[allow(clippy::too_many_lines)]
        fn test4_abandonment_keeps_no_snapshot_semantics_for_an_unobserved_edit() {
            let repo = IngressRepo::new("abandon-unobserved-edit");

            repo.drive(START_A_E1).expect("start should succeed");

            let (revision_after_start, cursor_after_start) = {
                let db = repo.db();
                (worktree_revision(&db), cursor_tree(&db))
            };

            fs::write(repo.root.join("file.txt"), "one\nunobserved\n")
                .expect("the unobserved edit should write");
            let edited_tree = repo.working_tree();
            assert_ne!(
                edited_tree, cursor_after_start,
                "the unobserved edit must move the Git tree"
            );

            assert_eq!(repo.drive(ABANDON_A).expect("abandon should succeed"), "");

            let db = repo.db();
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

        #[test]
        #[allow(clippy::too_many_lines)]
        fn test5_adversarial_flush_drives_real_observed_flush_behavior() {
            let repo = IngressRepo::new("adversarial-flush");

            assert_eq!(
                repo.drive(FLUSH)
                    .expect("the baseline flush should succeed"),
                ""
            );
            let revision_after_baseline = {
                let db = repo.db();
                worktree_revision(&db)
            };

            fs::write(repo.root.join("file.txt"), "one\nunscoped\n")
                .expect("the unscoped edit should write");
            let edited_tree = repo.working_tree();

            assert_eq!(repo.drive(FLUSH).expect("the flush should succeed"), "");

            let db = repo.db();
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

        #[test]
        #[allow(clippy::too_many_lines)]
        fn test6_marker_clear_after_commit_is_durable_success_through_the_ingress() {
            let repo = IngressRepo::new("marker-clear-after-commit");

            repo.drive(START_A_E1).expect("start should succeed");
            fs::write(repo.root.join("file.txt"), "one\nattributable\n")
                .expect("the scoped edit should write");

            let marker = repo.marker_path();
            let calls = Cell::new(0_u32);
            let resolver =
                |root: &Path, context_message: &'static str| -> Result<RepositoryAgentTraceDb> {
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
                };

            let result =
                run_mutation_scope_from_payload_with(&repo.root, ADVANCE_A_E2, None, resolver);

            assert_eq!(
                result.expect("a post-commit marker-clear failure is durable success"),
                ""
            );
            assert_eq!(
                calls.get(),
                1,
                "the runtime entrypoint must run exactly once, with no retried transition"
            );

            let db = repo.db();
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

        #[test]
        #[allow(clippy::too_many_lines)]
        fn test7_marker_clear_after_abandon_is_durable_success_through_the_ingress() {
            let repo = IngressRepo::new("marker-clear-after-abandon");

            repo.drive(START_A_E1).expect("start should succeed");
            let revision_after_start = {
                let db = repo.db();
                worktree_revision(&db)
            };

            fs::write(repo.root.join("file.txt"), "one\nunobserved\n")
                .expect("the unobserved edit should write");

            let marker = repo.marker_path();
            let calls = Cell::new(0_u32);
            let resolver =
                |root: &Path, context_message: &'static str| -> Result<RepositoryAgentTraceDb> {
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
                };

            let result =
                run_mutation_scope_from_payload_with(&repo.root, ABANDON_A, None, resolver);

            assert_eq!(
                result.expect("a post-completion marker-clear failure is durable success"),
                ""
            );
            assert_eq!(calls.get(), 1, "abandon_scope must run exactly once");

            let db = repo.db();
            assert_eq!(
                scope_status(&db, "A").map(|(_, status)| status),
                Some("abandoned".to_string())
            );
            assert_eq!(worktree_revision(&db), revision_after_start + 1);
            assert!(needs_rebaseline(&db));
            assert_eq!(count(&db, "mutation_trace_events"), 0);

            assert_raw_agent_trace_tables_untouched(&db);
        }
    }
}
