use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::read_hook_stdin;

mod apply_patch;
pub(crate) mod bash_policy;
mod stop;
mod user_prompt_submit;

const CODEX_HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
const CODEX_HOOK_EVENT_STOP: &str = "Stop";
const CODEX_HOOK_EVENT_PRE_TOOL_USE: &str = "PreToolUse";
const CODEX_HOOK_EVENT_POST_TOOL_USE: &str = "PostToolUse";
const CODEX_HOOK_TOOL_BASH: &str = "Bash";
const CODEX_HOOK_TOOL_APPLY_PATCH: &str = "apply_patch";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum NullableField<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> NullableField<T> {}

fn deserialize_nullable_field<'de, T, D>(
    deserializer: D,
) -> std::result::Result<NullableField<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(match Option::<T>::deserialize(deserializer)? {
        Some(value) => NullableField::Value(value),
        None => NullableField::Null,
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexHookEvent {
    pub(crate) hook_event_name: String,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) turn_id: Option<String>,
    #[serde(default)]
    pub(crate) cwd: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) tool_name: Option<String>,
    #[serde(default)]
    pub(crate) tool_use_id: Option<String>,
    #[serde(default)]
    pub(crate) tool_input: Option<Value>,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_field")]
    pub(crate) last_assistant_message: NullableField<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexDispatchArm {
    UserPromptSubmit,
    Stop,
    PreToolUseBash,
    PostToolUseApplyPatch,
    NoOp,
}

pub(crate) fn classify_codex_event(event: &CodexHookEvent) -> CodexDispatchArm {
    match (event.hook_event_name.as_str(), event.tool_name.as_deref()) {
        (CODEX_HOOK_EVENT_USER_PROMPT_SUBMIT, _) => CodexDispatchArm::UserPromptSubmit,
        (CODEX_HOOK_EVENT_STOP, _) => CodexDispatchArm::Stop,
        (CODEX_HOOK_EVENT_PRE_TOOL_USE, Some(CODEX_HOOK_TOOL_BASH)) => {
            CodexDispatchArm::PreToolUseBash
        }
        (CODEX_HOOK_EVENT_POST_TOOL_USE, Some(CODEX_HOOK_TOOL_APPLY_PATCH)) => {
            CodexDispatchArm::PostToolUseApplyPatch
        }
        _ => CodexDispatchArm::NoOp,
    }
}

pub(super) async fn run_codex_subcommand<L: crate::services::observability::traits::Logger>(
    repository_root: &Path,
    logger: Option<&L>,
) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_codex_fail_open(&error, logger),
    };

    match run_codex_subcommand_from_payload(repository_root, &stdin_payload, logger).await {
        Ok(output) => output,
        Err(error) => log_codex_fail_open(&error, logger),
    }
}

async fn run_codex_subcommand_from_payload<L: crate::services::observability::traits::Logger>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
) -> Result<String> {
    run_codex_subcommand_from_payload_with_state_root(repository_root, stdin_payload, logger, None)
        .await
}

async fn run_codex_subcommand_from_payload_with_state_root<
    L: crate::services::observability::traits::Logger,
>(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&L>,
    state_root: Option<&Path>,
) -> Result<String> {
    let event: CodexHookEvent = serde_json::from_str(stdin_payload)
        .context("Invalid Codex hook payload from STDIN: expected valid JSON.")?;

    Ok(match classify_codex_event(&event) {
        CodexDispatchArm::UserPromptSubmit => {
            user_prompt_submit::handle(repository_root, &event).await?
        }
        CodexDispatchArm::Stop => stop::handle(repository_root, &event).await?,
        CodexDispatchArm::PreToolUseBash => bash_policy::handle(repository_root, &event)?,
        CodexDispatchArm::PostToolUseApplyPatch => match state_root {
            Some(state_root) => {
                apply_patch::handle_with_state_root(
                    repository_root,
                    &event,
                    Some(state_root),
                    logger,
                )
                .await?
            }
            None => apply_patch::handle(repository_root, &event, logger).await?,
        },
        CodexDispatchArm::NoOp => String::new(),
    })
}

fn log_codex_fail_open<L: crate::services::observability::traits::Logger>(
    error: &anyhow::Error,
    logger: Option<&L>,
) -> String {
    if let Some(log) = logger {
        log.error("sce.hooks.codex.error", &error.to_string(), &[], None);
    }

    String::new()
}
