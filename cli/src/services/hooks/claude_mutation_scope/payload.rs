use serde_json::json;

use super::lifecycle::ACTOR_KIND_CLAUDE_CODE;

pub(super) fn scope_boundary_payload(operation: &str, scope_id: &str, event_id: &str) -> String {
    json!({
        "operation": operation,
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_CLAUDE_CODE,
    })
    .to_string()
}

pub(super) fn scope_start_payload(
    scope_id: &str,
    event_id: &str,
    session_id: &str,
    model_id: Option<&str>,
) -> String {
    json!({
        "operation": "start",
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_CLAUDE_CODE,
        "provenance": {
            "session_id": session_id,
            "model_id": model_id,
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
