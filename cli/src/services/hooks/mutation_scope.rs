use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

use crate::services::mutation_trace::types::ActorKind;

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
}
