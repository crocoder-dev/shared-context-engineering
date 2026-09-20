#![allow(dead_code)]

mod boundary_lock;
mod events;
pub(crate) mod health;
mod lifecycle;
mod os_lock;
mod payload;
pub(crate) mod state;

#[allow(unused_imports)]
use crate::services::hooks::codex::bash_policy::{
    evaluate_codex_bash_policy, CodexBashPolicyDecision,
};
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
    classify_tool, codex_scope_close_event_id, codex_scope_start_event_id, format_codex_scope_id,
    is_mcp_tool_name, parse_codex_hook_event, AttemptKey, CodexAgentIdentity, CodexHookEvent,
    CodexSessionIdentity, CodexToolExecution, CodexToolIdentity, CodexTurnIdentity,
    ToolClassification,
};
#[allow(unused_imports)]
use events::{
    AGENT_ID_FIELD, AGENT_TYPE_FIELD, CODEX_TRACKED_TOOL_BASH, CWD_FIELD, HOOK_EVENT_INTERRUPT,
    HOOK_EVENT_NAME_FIELD, HOOK_EVENT_POST_TOOL_USE, HOOK_EVENT_PRE_TOOL_USE,
    HOOK_EVENT_SESSION_END, HOOK_EVENT_STOP, HOOK_EVENT_SUBAGENT_STOP, MODEL_FIELD,
    PROVENANCE_FIELD, SESSION_ID_FIELD, TOOL_INPUT_FIELD, TOOL_NAME_FIELD, TOOL_USE_ID_FIELD,
    TRACKED_MUTATION_TOOL_NAMES, TURN_ID_FIELD,
};
#[cfg(test)]
pub(crate) use lifecycle::run_codex_mutation_scope_from_payload_at_state_root;
#[allow(unused_imports)]
use lifecycle::{
    abandon_attempt, run_codex_mutation_scope_from_payload_with_seams, FAIL_CLOSED_DENY_REASON,
    PRE_TOOL_USE_FAIL_CLOSED_EVENT,
};
#[cfg(test)]
#[allow(unused_imports)]
use lifecycle::{
    codex_scope_provenance, run_codex_mutation_scope_from_payload_with,
    run_codex_mutation_scope_from_payload_with_bash_policy, GitDirResolver, IngressSeam,
};
#[allow(unused_imports)]
pub(crate) use lifecycle::{
    run_codex_mutation_scope_from_payload, run_codex_mutation_scope_subcommand,
};
#[allow(unused_imports)]
use payload::{
    abandon_payload, flush_payload, pre_tool_use_deny_json, scope_boundary_payload,
    scope_start_payload,
};

#[cfg(test)]
mod tests;
