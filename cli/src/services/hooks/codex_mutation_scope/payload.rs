use serde_json::json;

use super::events::PROVENANCE_FIELD;
use super::lifecycle::{CodexScopeProvenance, ACTOR_KIND_CODEX};

pub(super) fn scope_boundary_payload(operation: &str, scope_id: &str, event_id: &str) -> String {
    json!({
        "operation": operation,
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_CODEX,
    })
    .to_string()
}

pub(super) fn scope_start_payload(
    scope_id: &str,
    event_id: &str,
    provenance: &CodexScopeProvenance,
) -> String {
    json!({
        "operation": "start",
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_CODEX,
        PROVENANCE_FIELD: {
            "session_id": provenance.session_id,
            "model_id": provenance.model_id,
        },
    })
    .to_string()
}

pub(super) fn abandon_payload(scope_id: &str) -> String {
    json!({
        "operation": "abandon",
        "scope_id": scope_id,
    })
    .to_string()
}

pub(super) fn flush_payload() -> String {
    json!({ "operation": "flush" }).to_string()
}

pub(super) fn pre_tool_use_deny_json(reason: &str) -> String {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    })
    .to_string()
}
