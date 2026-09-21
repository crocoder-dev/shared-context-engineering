#![allow(dead_code)]

mod boundary_lock;
mod events;
pub(crate) mod health;
mod lifecycle;
mod os_lock;
mod payload;
pub(crate) mod state;

#[allow(unused_imports)]
use crate::services::mutation_trace::runtime::resolve_git_dir;
#[allow(unused_imports)]
use crate::services::observability::traits::Logger;
#[allow(unused_imports)]
use anyhow::{anyhow, bail, Context, Result};
#[allow(unused_imports)]
use std::path::{Path, PathBuf};

#[allow(unused_imports)]
use serde_json::{json, Map, Value};

#[allow(unused_imports)]
pub(crate) use events::{
    classify_tool, format_pi_scope_id, parse_pi_hook_event, pi_scope_close_event_id,
    pi_scope_provenance, pi_scope_start_event_id, AttemptKey, PiHookEvent, PiScopeProvenance,
    PiToolCall, PiToolIdentity, ToolClassification,
};
#[allow(unused_imports)]
use events::{
    ACTOR_KIND_PI, CWD_FIELD, HOOK_EVENT_NAME_FIELD, HOOK_EVENT_TOOL_CALL,
    HOOK_EVENT_TOOL_EXECUTION_ABANDON, HOOK_EVENT_TOOL_EXECUTION_END,
    HOOK_EVENT_TOOL_EXECUTION_START, HOOK_EVENT_TOOL_RESULT, MODEL_FIELD, SESSION_ID_FIELD,
    TOOL_CALL_ID_FIELD, TOOL_NAME_FIELD,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use lifecycle::force_attempt_owner_dead_for_tests;
#[cfg(test)]
pub(crate) use lifecycle::run_pi_mutation_scope_from_payload_at_state_root;
#[allow(unused_imports)]
pub(crate) use lifecycle::{run_pi_mutation_scope_from_payload, run_pi_mutation_scope_subcommand};
#[allow(unused_imports)]
use lifecycle::{
    run_pi_mutation_scope_from_payload_with_seams, FAIL_CLOSED_EVENT, FAIL_CLOSED_MESSAGE,
};
#[allow(unused_imports)]
use payload::{abandon_payload, flush_payload, scope_boundary_payload, scope_start_payload};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod runtime_seam_tests;

#[cfg(all(unix, test))]
mod guard_reconciliation_tests;
