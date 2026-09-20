use serde_json::json;

use super::events::ACTOR_KIND_PI;
use super::PiScopeProvenance;

pub(super) fn scope_boundary_payload(operation: &str, scope_id: &str, event_id: &str) -> String {
    json!({
        "operation": operation,
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_PI,
    })
    .to_string()
}

pub(super) fn scope_start_payload(
    scope_id: &str,
    event_id: &str,
    provenance: &PiScopeProvenance,
) -> String {
    json!({
        "operation": "start",
        "scope_id": scope_id,
        "event_id": event_id,
        "actor_kind": ACTOR_KIND_PI,
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
