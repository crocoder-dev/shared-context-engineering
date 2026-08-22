use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::services::observability::traits::Logger;

use super::read_hook_stdin;

mod user_prompt_submit;

const CODEX_HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
const CODEX_HOOK_EVENT_STOP: &str = "Stop";
const CODEX_HOOK_EVENT_PRE_TOOL_USE: &str = "PreToolUse";
const CODEX_HOOK_EVENT_POST_TOOL_USE: &str = "PostToolUse";
const CODEX_HOOK_TOOL_BASH: &str = "Bash";
const CODEX_HOOK_TOOL_APPLY_PATCH: &str = "apply_patch";

/// A single Codex hook lifecycle event, deserialized from the raw STDIN JSON
/// payload `sce hooks codex` receives via
/// `.codex/hooks/run-sce-or-show-install-guidance.sh`.
///
/// Working contract (see plan `context/plans/codex-cli-integration.md`
/// Assumptions): `hook_event_name` is present on every event; `session_id`,
/// `turn_id`, `cwd`, and `model` vary by event; `tool_name`/`tool_use_id`/
/// `tool_input`/`tool_response` are present only on `PreToolUse`/`PostToolUse`;
/// `prompt` is present only on `UserPromptSubmit`, matching Claude's own
/// `UserPromptSubmit` payload shape (see `transform_claude_user_prompt_submit_with`).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
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
    pub(crate) tool_response: Option<Value>,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
}

/// The set of Codex hook-event/tool combinations `sce hooks codex` gives
/// distinct behavior. Every other combination classifies as `NoOp`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexDispatchArm {
    UserPromptSubmit,
    Stop,
    PreToolUseBash,
    PreToolUseApplyPatch,
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
        (CODEX_HOOK_EVENT_PRE_TOOL_USE, Some(CODEX_HOOK_TOOL_APPLY_PATCH)) => {
            CodexDispatchArm::PreToolUseApplyPatch
        }
        (CODEX_HOOK_EVENT_POST_TOOL_USE, Some(CODEX_HOOK_TOOL_APPLY_PATCH)) => {
            CodexDispatchArm::PostToolUseApplyPatch
        }
        _ => CodexDispatchArm::NoOp,
    }
}

pub(super) fn run_codex_subcommand(repository_root: &Path, logger: Option<&dyn Logger>) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_codex_fail_open(&error, logger),
    };

    match run_codex_subcommand_from_payload(repository_root, &stdin_payload) {
        Ok(output) => output,
        Err(error) => log_codex_fail_open(&error, logger),
    }
}

fn run_codex_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
) -> Result<String> {
    let event: CodexHookEvent = serde_json::from_str(stdin_payload)
        .context("Invalid Codex hook payload from STDIN: expected valid JSON.")?;

    Ok(match classify_codex_event(&event) {
        CodexDispatchArm::UserPromptSubmit => {
            user_prompt_submit::handle(repository_root, &event)?
        }
        CodexDispatchArm::Stop => {
            "codex hooks: Stop dispatch (stub; capture lands in T08).".to_string()
        }
        CodexDispatchArm::PreToolUseBash => {
            "codex hooks: PreToolUse Bash dispatch (stub; policy routing lands in T09)."
                .to_string()
        }
        CodexDispatchArm::PreToolUseApplyPatch => {
            "codex hooks: PreToolUse apply_patch dispatch (stub; before-state snapshot lands in T10)."
                .to_string()
        }
        CodexDispatchArm::PostToolUseApplyPatch => {
            "codex hooks: PostToolUse apply_patch dispatch (stub; finalize lands in T11).".to_string()
        }
        CodexDispatchArm::NoOp => format!(
            "codex hooks: no-op for unsupported event/tool combination (hook_event_name='{}', tool_name={:?}).",
            event.hook_event_name, event.tool_name
        ),
    })
}

fn log_codex_fail_open(error: &anyhow::Error, logger: Option<&dyn Logger>) -> String {
    if let Some(log) = logger {
        log.error("sce.hooks.codex.error", &error.to_string(), &[], None);
    }

    String::from("codex hook intake failed open; error logged.")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn event(hook_event_name: &str, tool_name: Option<&str>) -> CodexHookEvent {
        CodexHookEvent {
            hook_event_name: hook_event_name.to_string(),
            session_id: Some("abc123".to_string()),
            turn_id: Some("turn-1".to_string()),
            cwd: None,
            model: None,
            tool_name: tool_name.map(str::to_string),
            tool_use_id: None,
            tool_input: None,
            tool_response: None,
            prompt: None,
        }
    }

    #[test]
    fn classify_codex_event_routes_user_prompt_submit() {
        assert_eq!(
            classify_codex_event(&event("UserPromptSubmit", None)),
            CodexDispatchArm::UserPromptSubmit
        );
    }

    #[test]
    fn classify_codex_event_routes_stop() {
        assert_eq!(
            classify_codex_event(&event("Stop", None)),
            CodexDispatchArm::Stop
        );
    }

    #[test]
    fn classify_codex_event_routes_pre_tool_use_bash() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", Some("Bash"))),
            CodexDispatchArm::PreToolUseBash
        );
    }

    #[test]
    fn classify_codex_event_routes_pre_tool_use_apply_patch() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", Some("apply_patch"))),
            CodexDispatchArm::PreToolUseApplyPatch
        );
    }

    #[test]
    fn classify_codex_event_routes_post_tool_use_apply_patch() {
        assert_eq!(
            classify_codex_event(&event("PostToolUse", Some("apply_patch"))),
            CodexDispatchArm::PostToolUseApplyPatch
        );
    }

    #[test]
    fn classify_codex_event_routes_unknown_pre_tool_use_tool_name_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", Some("Edit"))),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn classify_codex_event_routes_pre_tool_use_with_no_tool_name_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", None)),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn classify_codex_event_routes_post_tool_use_bash_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("PostToolUse", Some("Bash"))),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn classify_codex_event_routes_unrecognized_hook_event_name_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("SessionStart", None)),
            CodexDispatchArm::NoOp
        );
    }

    #[test]
    fn run_codex_subcommand_from_payload_dispatches_each_still_stubbed_combination() {
        let cases = [
            (
                r#"{"hook_event_name":"Stop","session_id":"s1","turn_id":"t1"}"#,
                "Stop",
            ),
            (
                r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Bash"}"#,
                "PreToolUse Bash",
            ),
            (
                r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"apply_patch"}"#,
                "PreToolUse apply_patch",
            ),
            (
                r#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_name":"apply_patch"}"#,
                "PostToolUse apply_patch",
            ),
        ];

        for (payload, expected_substring) in cases {
            let output = run_codex_subcommand_from_payload(Path::new("/tmp"), payload)
                .expect("stub dispatch should succeed");
            assert!(
                output.contains(expected_substring),
                "expected output '{output}' to mention '{expected_substring}'"
            );
        }
    }

    #[test]
    fn run_codex_subcommand_from_payload_no_ops_unsupported_combination_without_error() {
        let payload = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Read"}"#;

        let output = run_codex_subcommand_from_payload(Path::new("/tmp"), payload)
            .expect("no-op dispatch should succeed");

        assert!(output.contains("no-op"));
    }

    #[test]
    fn run_codex_subcommand_from_payload_no_ops_unrecognized_hook_event_name_without_error() {
        let payload = r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#;

        let output = run_codex_subcommand_from_payload(Path::new("/tmp"), payload)
            .expect("no-op dispatch should succeed");

        assert!(output.contains("no-op"));
    }

    #[test]
    fn run_codex_subcommand_from_payload_rejects_non_json_stdin() {
        let error = run_codex_subcommand_from_payload(Path::new("/tmp"), "not json")
            .expect_err("malformed payload should fail parsing");

        assert!(error.to_string().contains("Invalid Codex hook payload"));
    }

    #[test]
    fn run_codex_subcommand_fails_open_on_malformed_stdin_payload() {
        let error = anyhow::anyhow!("Invalid Codex hook payload from STDIN: expected valid JSON.");

        let output = log_codex_fail_open(&error, None);

        assert_eq!(output, "codex hook intake failed open; error logged.");
    }
}
