#![allow(dead_code)]

mod events;
pub(crate) mod health;
mod lifecycle;
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
    classify_tool, claude_scope_close_event_id, claude_scope_start_event_id,
    format_claude_scope_id, is_explicit_background_shell, parse_claude_hook_event, AttemptKey,
    ClaudeAgentIdentity, ClaudeHookEvent, ClaudeSessionIdentity, ClaudeToolExecution,
    ClaudeToolIdentity, ClaudeWorktreeRemove, ToolClassification,
};
#[allow(unused_imports)]
use events::{
    AGENT_ID_FIELD, AGENT_TYPE_FIELD, CWD_FIELD, HOOK_EVENT_NAME_FIELD,
    HOOK_EVENT_PERMISSION_DENIED, HOOK_EVENT_POST_TOOL_USE, HOOK_EVENT_POST_TOOL_USE_FAILURE,
    HOOK_EVENT_PRE_TOOL_USE, HOOK_EVENT_SESSION_END, HOOK_EVENT_SESSION_START, HOOK_EVENT_STOP,
    HOOK_EVENT_STOP_FAILURE, HOOK_EVENT_SUBAGENT_START, HOOK_EVENT_SUBAGENT_STOP,
    HOOK_EVENT_USER_PROMPT_SUBMIT, HOOK_EVENT_WORKTREE_REMOVE, PROMPT_ID_FIELD,
    RUN_IN_BACKGROUND_FIELD, SESSION_ID_FIELD, TOOL_INPUT_FIELD, TOOL_NAME_FIELD,
    TOOL_USE_ID_FIELD, WORKTREE_PATH_FIELD,
};
#[cfg(test)]
pub(crate) use lifecycle::run_claude_mutation_scope_from_payload_at_state_root;
#[cfg(test)]
#[allow(unused_imports)]
use lifecycle::ClaudeModelStateResolver;
#[allow(unused_imports)]
use lifecycle::{
    abandon_attempt, apply_recovery_barrier, BarrierOutcome, GitDirResolver, IngressSeam,
    EXPLICIT_BACKGROUND_SHELL_DENY_REASON, FAIL_CLOSED_DENY_REASON,
};
#[allow(unused_imports)]
pub(crate) use lifecycle::{
    run_claude_mutation_scope_from_payload, run_claude_mutation_scope_subcommand,
};
#[cfg(test)]
#[allow(unused_imports)]
use lifecycle::{
    run_claude_mutation_scope_from_payload_with,
    run_claude_mutation_scope_from_payload_with_resolver,
};
#[allow(unused_imports)]
use payload::{
    abandon_payload, flush_payload, pre_tool_use_deny_json, scope_boundary_payload,
    scope_start_payload,
};

#[cfg(test)]
mod tests;
