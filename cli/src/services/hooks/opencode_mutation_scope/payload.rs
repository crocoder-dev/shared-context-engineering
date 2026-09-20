use serde_json::json;

use super::events::{OpenCodeScopeProvenance, ACTOR_KIND_OPENCODE};

pub(super) fn scope_boundary_payload(operation: &str, scope_id: &str, event_id: &str) -> String {
    json!({
        "operation": operation,
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_OPENCODE,
    })
    .to_string()
}

pub(super) fn scope_start_payload(
    scope_id: &str,
    event_id: &str,
    provenance: &OpenCodeScopeProvenance,
) -> String {
    json!({
        "operation": "start",
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_OPENCODE,
        "provenance": {
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
