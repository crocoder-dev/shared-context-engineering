use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::services::observability::traits::Logger;

use super::read_hook_stdin;

mod apply_patch;
mod bash_policy;
mod stop;
mod user_prompt_submit;

const CODEX_HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
const CODEX_HOOK_EVENT_STOP: &str = "Stop";
const CODEX_HOOK_EVENT_PRE_TOOL_USE: &str = "PreToolUse";
const CODEX_HOOK_EVENT_POST_TOOL_USE: &str = "PostToolUse";
const CODEX_HOOK_TOOL_BASH: &str = "Bash";
const CODEX_HOOK_TOOL_APPLY_PATCH: &str = "apply_patch";

/// Distinguishes a JSON field that is absent from the payload entirely
/// (`Missing`) from one that is present with an explicit `null` (`Null`)
/// from one that is present with a value (`Value`). A plain
/// `#[serde(default)] Option<T>` cannot make this distinction: Serde's
/// `Option<T>` deserializer maps JSON `null` to `None` at the *same* layer
/// it uses for "value absent", so both missing-field and explicit-null
/// collapse to `None`. `#[serde(default, deserialize_with = "...")]` on a
/// field of this type keeps `Default` (→ `Missing`) for the no-field case
/// and routes every present field (including `null`) through
/// [`deserialize_nullable_field`], which is the only path that can produce
/// `Null` or `Value`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum NullableField<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> NullableField<T> {
    #[cfg(test)]
    pub(crate) fn is_missing(&self) -> bool {
        matches!(self, NullableField::Missing)
    }

    #[cfg(test)]
    pub(crate) fn is_null(&self) -> bool {
        matches!(self, NullableField::Null)
    }

    #[cfg(test)]
    pub(crate) fn as_value(&self) -> Option<&T> {
        match self {
            NullableField::Value(value) => Some(value),
            NullableField::Missing | NullableField::Null => None,
        }
    }
}

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

/// A single Codex hook lifecycle event, deserialized from the raw STDIN JSON
/// payload `sce hooks codex` receives via
/// `.codex/hooks/run-sce-or-show-install-guidance.sh`.
///
/// Working contract (see plan `context/plans/codex-cli-integration.md`
/// Assumptions): `hook_event_name` is present on every event; `session_id`,
/// `turn_id`, `cwd`, and `model` vary by event; `tool_name`/`tool_use_id`/
/// `tool_input`/`tool_response` are present only on `PreToolUse`/`PostToolUse`;
/// `prompt` is present only on `UserPromptSubmit`, matching Claude's own
/// `UserPromptSubmit` payload shape (see `transform_claude_user_prompt_submit_with`);
/// `last_assistant_message` is present (per current upstream Codex `Stop`
/// schema, required and typed `string | null`) only on `Stop`, matching
/// Claude's own `Stop` payload shape (see `transform_claude_stop_with`)
/// except that Codex allows an explicit `null` where Claude does not — see
/// [`NullableField`].
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
    #[serde(default, deserialize_with = "deserialize_nullable_field")]
    pub(crate) last_assistant_message: NullableField<String>,
}

/// The set of Codex hook-event/tool combinations `sce hooks codex` gives
/// distinct behavior. Every other combination classifies as `NoOp`.
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

pub(super) fn run_codex_subcommand(repository_root: &Path, logger: Option<&dyn Logger>) -> String {
    let stdin_payload = match read_hook_stdin() {
        Ok(payload) => payload,
        Err(error) => return log_codex_fail_open(&error, logger),
    };

    match run_codex_subcommand_from_payload(repository_root, &stdin_payload, logger) {
        Ok(output) => output,
        Err(error) => log_codex_fail_open(&error, logger),
    }
}

fn run_codex_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
) -> Result<String> {
    run_codex_subcommand_from_payload_with_state_root(repository_root, stdin_payload, logger, None)
}

#[cfg(test)]
fn run_codex_subcommand_from_payload_at_state_root(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    state_root: &Path,
) -> Result<String> {
    run_codex_subcommand_from_payload_with_state_root(
        repository_root,
        stdin_payload,
        logger,
        Some(state_root),
    )
}

fn run_codex_subcommand_from_payload_with_state_root(
    repository_root: &Path,
    stdin_payload: &str,
    logger: Option<&dyn Logger>,
    state_root: Option<&Path>,
) -> Result<String> {
    let event: CodexHookEvent = serde_json::from_str(stdin_payload)
        .context("Invalid Codex hook payload from STDIN: expected valid JSON.")?;

    Ok(match classify_codex_event(&event) {
        CodexDispatchArm::UserPromptSubmit => user_prompt_submit::handle(repository_root, &event)?,
        CodexDispatchArm::Stop => stop::handle(repository_root, &event)?,
        CodexDispatchArm::PreToolUseBash => bash_policy::handle(repository_root, &event)?,
        CodexDispatchArm::PostToolUseApplyPatch => match state_root {
            Some(state_root) => apply_patch::handle_with_state_root(
                repository_root,
                &event,
                Some(state_root),
                logger,
            )?,
            None => apply_patch::handle(repository_root, &event, logger)?,
        },
        CodexDispatchArm::NoOp => String::new(),
    })
}

fn log_codex_fail_open(error: &anyhow::Error, logger: Option<&dyn Logger>) -> String {
    if let Some(log) = logger {
        log.error("sce.hooks.codex.error", &error.to_string(), &[], None);
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use crate::services::agent_trace_storage::{
        resolve_agent_trace_storage_at_state_root,
        resolve_agent_trace_storage_for_hook_runtime_at_state_root, AgentTraceStorageContext,
    };
    use crate::services::patch::FileChangeKind;

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
            last_assistant_message: NullableField::Missing,
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
    fn classify_codex_event_routes_pre_tool_use_apply_patch_to_no_op() {
        assert_eq!(
            classify_codex_event(&event("PreToolUse", Some("apply_patch"))),
            CodexDispatchArm::NoOp
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
    fn run_codex_subcommand_from_payload_no_ops_unsupported_combination_without_error() {
        let payload = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Read"}"#;

        let output = run_codex_subcommand_from_payload(Path::new("/tmp"), payload, None)
            .expect("no-op dispatch should succeed");

        assert_eq!(output, "");
    }

    #[test]
    fn run_codex_subcommand_from_payload_no_ops_unrecognized_hook_event_name_without_error() {
        let payload = r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#;

        let output = run_codex_subcommand_from_payload(Path::new("/tmp"), payload, None)
            .expect("no-op dispatch should succeed");

        assert_eq!(output, "");
    }

    #[test]
    fn run_codex_subcommand_from_payload_rejects_non_json_stdin() {
        let error = run_codex_subcommand_from_payload(Path::new("/tmp"), "not json", None)
            .expect_err("malformed payload should fail parsing");

        assert!(error.to_string().contains("Invalid Codex hook payload"));
    }

    #[test]
    fn run_codex_subcommand_fails_open_on_malformed_stdin_payload() {
        let error = anyhow::anyhow!("Invalid Codex hook payload from STDIN: expected valid JSON.");

        let output = log_codex_fail_open(&error, None);

        assert_eq!(output, "");
    }

    #[derive(Clone, Default)]
    struct RecordingLogger {
        errors: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Logger for RecordingLogger {
        fn info(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn debug(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}
        fn warn(&self, _: &str, _: &str, _: &[(&str, &str)], _: Option<&str>) {}

        fn error(&self, _event_id: &str, message: &str, _: &[(&str, &str)], _: Option<&str>) {
            self.errors
                .lock()
                .expect("recording logger mutex must not be poisoned")
                .push(message.to_string());
        }

        fn log_cli_error(&self, _error: &crate::services::error::CliError, _: Option<&str>) {}
    }

    #[test]
    fn log_codex_fail_open_logs_a_propagated_timestamp_failure_and_returns_empty_stdout() {
        let logger = RecordingLogger::default();
        let error = anyhow::anyhow!("clock failed");

        let output = log_codex_fail_open(&error, Some(&logger));

        assert_eq!(output, "");
        let errors = logger.errors.lock().expect("mutex must not be poisoned");
        assert_eq!(errors.as_slice(), ["clock failed"]);
    }

    #[derive(Debug, Clone, Copy)]
    struct StopRowCounts {
        messages: i64,
        parts: i64,
    }

    fn stop_row_counts(
        storage: &crate::services::agent_trace_storage::ResolvedAgentTraceStorage,
    ) -> StopRowCounts {
        let messages = storage
            .db
            .query_map("SELECT COUNT(*) FROM messages", (), |row| {
                row.get::<i64>(0).map_err(anyhow::Error::from)
            })
            .expect("messages count query should succeed")[0];
        let parts = storage
            .db
            .query_map("SELECT COUNT(*) FROM parts", (), |row| {
                row.get::<i64>(0).map_err(anyhow::Error::from)
            })
            .expect("parts count query should succeed")[0];
        StopRowCounts { messages, parts }
    }

    fn reopen_storage_for_counts(
        repository_root: &Path,
        state_root: &Path,
    ) -> crate::services::agent_trace_storage::ResolvedAgentTraceStorage {
        resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            state_root,
        )
        .expect("repository Agent Trace DB should reopen")
    }

    #[test]
    fn stop_dispatch_propagates_a_missing_last_assistant_message_field_for_the_outer_fail_open_boundary(
    ) {
        let (repository_root, state_root) = initialize_repository("stop-dispatch-missing-field");
        let payload = json!({
            "hook_event_name": "Stop",
            "session_id": "s1",
            "turn_id": "t1"
        })
        .to_string();

        let error = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect_err(
            "a Stop payload missing last_assistant_message must error so the outer boundary can fail open",
        );
        assert!(error.to_string().contains("last_assistant_message"));
        assert_eq!(log_codex_fail_open(&error, None), "");

        let storage = reopen_storage_for_counts(&repository_root, &state_root);
        let counts = stop_row_counts(&storage);
        assert_eq!(counts.messages, 0);
        assert_eq!(counts.parts, 0);

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    #[test]
    fn stop_dispatch_is_a_silent_no_op_for_an_explicit_null_last_assistant_message() {
        let (repository_root, state_root) = initialize_repository("stop-dispatch-null");
        let payload = json!({
            "hook_event_name": "Stop",
            "session_id": "s1",
            "turn_id": "t1",
            "last_assistant_message": null
        })
        .to_string();

        let output = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect("explicit null last_assistant_message should be a successful no-op");
        assert_eq!(output, "");

        let storage = reopen_storage_for_counts(&repository_root, &state_root);
        let counts = stop_row_counts(&storage);
        assert_eq!(counts.messages, 0);
        assert_eq!(counts.parts, 0);

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    // Explicit-empty-string and normal-text persistence *through raw JSON
    // deserialization* are covered in `stop::tests` (e.g.
    // `capture_with_persists_deserialized_raw_json_with_empty_string_last_assistant_message`),
    // not here: `open_agent_trace_db_for_hook_runtime` (used by `stop::handle`
    // for every persisting case) resolves the real default Agent Trace
    // storage path and has no `state_root` injection seam — unlike
    // `apply_patch`, which added one specifically for its own dispatcher
    // tests. Missing/null above need no DB at all (they short-circuit before
    // DB open), so they remain safe to exercise through the full
    // `run_codex_subcommand_from_payload_at_state_root` dispatcher path.

    #[test]
    fn codex_hook_event_deserializes_missing_last_assistant_message_as_missing() {
        let event: CodexHookEvent =
            serde_json::from_str(r#"{"hook_event_name":"Stop","session_id":"s1","turn_id":"t1"}"#)
                .expect("payload without last_assistant_message should still deserialize");

        assert!(event.last_assistant_message.is_missing());
    }

    #[test]
    fn codex_hook_event_deserializes_explicit_null_last_assistant_message_as_null() {
        let event: CodexHookEvent = serde_json::from_str(
            r#"{"hook_event_name":"Stop","session_id":"s1","turn_id":"t1","last_assistant_message":null}"#,
        )
        .expect("payload with explicit null last_assistant_message should deserialize");

        assert!(event.last_assistant_message.is_null());
    }

    #[test]
    fn codex_hook_event_deserializes_empty_string_last_assistant_message_as_value() {
        let event: CodexHookEvent = serde_json::from_str(
            r#"{"hook_event_name":"Stop","session_id":"s1","turn_id":"t1","last_assistant_message":""}"#,
        )
        .expect("payload with empty string last_assistant_message should deserialize");

        assert_eq!(
            event.last_assistant_message.as_value().map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn codex_hook_event_deserializes_present_text_last_assistant_message_as_value() {
        let event: CodexHookEvent = serde_json::from_str(
            r#"{"hook_event_name":"Stop","session_id":"s1","turn_id":"t1","last_assistant_message":"hello"}"#,
        )
        .expect("payload with text last_assistant_message should deserialize");

        assert_eq!(
            event.last_assistant_message.as_value().map(String::as_str),
            Some("hello")
        );
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sce-codex-pipeline-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    fn git(repository_root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repository_root)
            .output()
            .unwrap_or_else(|error| panic!("git {args:?} failed to spawn: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn initialize_repository(label: &str) -> (PathBuf, PathBuf) {
        let repository_root = unique_temp_dir(&format!("{label}-repo"));
        git(&repository_root, &["init", "-q"]);
        git(
            &repository_root,
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/codex-t19.git",
            ],
        );
        let state_root = unique_temp_dir(&format!("{label}-state"));
        let context = AgentTraceStorageContext {
            repository_root: &repository_root,
            explicit_repository_id: None,
            repository_remote: "origin",
        };
        let storage = resolve_agent_trace_storage_at_state_root(&context, &state_root)
            .expect("repository Agent Trace DB should initialize");
        drop(storage);
        (repository_root, state_root)
    }

    fn codex_apply_patch_payload(
        cwd: &Path,
        session_id: &str,
        model: &str,
        tool_use_id: &str,
        command: &str,
    ) -> String {
        json!({
            "hook_event_name": "PostToolUse",
            "session_id": session_id,
            "turn_id": "turn-realistic",
            "cwd": cwd,
            "model": model,
            "tool_name": "apply_patch",
            "tool_use_id": tool_use_id,
            "tool_input": {"command": command},
            "tool_response": {"success": true}
        })
        .to_string()
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn realistic_post_tool_use_patch_flows_through_repository_db_and_post_commit_attribution() {
        let (repository_root, state_root) = initialize_repository("end-to-end");
        let source_dir = repository_root.join("src");
        fs::create_dir_all(&source_dir).expect("source directory should be created");
        fs::write(source_dir.join("lib.rs"), "prefix\nold_line\nsuffix\n")
            .expect("initial source should be written");
        git(&repository_root, &["add", "."]);
        git(
            &repository_root,
            &[
                "-c",
                "user.name=SCE Test",
                "-c",
                "user.email=sce@example.invalid",
                "commit",
                "-qm",
                "initial",
            ],
        );

        let command = "<<\"EOF\"\n*** Begin Patch\n*** Update File: lib.rs\n@@\n-old_line\n+new_line\n*** End Patch\nEOF";
        let payload = codex_apply_patch_payload(
            &source_dir,
            " session-realistic ",
            "custom/codex-model",
            "tool-realistic-1",
            command,
        );
        let output = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect("realistic Codex PostToolUse dispatch should succeed");
        assert_eq!(output, "", "successful apply_patch hooks are silent");

        fs::write(source_dir.join("lib.rs"), "prefix\nnew_line\nsuffix\n")
            .expect("updated source should be written");
        git(&repository_root, &["add", "."]);
        git(
            &repository_root,
            &[
                "-c",
                "user.name=SCE Test",
                "-c",
                "user.email=sce@example.invalid",
                "commit",
                "-qm",
                "apply patch",
            ],
        );

        let context = AgentTraceStorageContext {
            repository_root: &repository_root,
            explicit_repository_id: None,
            repository_remote: "origin",
        };
        let storage =
            resolve_agent_trace_storage_for_hook_runtime_at_state_root(&context, &state_root)
                .expect("repository Agent Trace DB should reopen");
        let recent = storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("stored Codex patch should be queryable");
        assert_eq!(recent.loaded_count(), 1);
        let stored_file = &recent.patches[0].patch.files[0];
        assert_eq!(recent.patches[0].session_id, "cx_session-realistic");
        assert_eq!(recent.patches[0].tool_name.as_deref(), Some("codex"));
        assert_eq!(recent.patches[0].tool_version, None);
        assert_eq!(recent.patches[0].payload_type, "patch");
        assert_eq!(recent.patches[0].patch.files.len(), 1);
        assert_eq!(stored_file.old_path, "src/lib.rs");
        assert_eq!(stored_file.new_path, "src/lib.rs");
        assert_eq!(
            stored_file.hunks[0].model_id.as_deref(),
            Some("custom/codex-model")
        );

        let flow = super::super::run_post_commit_intersection_flow_with(
            &repository_root,
            super::super::capture_post_commit_patch_from_git,
            super::super::current_unix_time_ms,
            |cutoff_ms, end_ms| storage.db.recent_diff_trace_patches(cutoff_ms, end_ms),
            |insert| {
                storage
                    .db
                    .insert_post_commit_patch_intersection(insert)
                    .map(|_| ())
            },
        )
        .expect("post-commit intersection should use the stored Codex evidence");
        let trace = super::super::run_post_commit_agent_trace_flow_with(
            &flow,
            None,
            "https://example.invalid/codex-t19.git",
            |value| {
                crate::services::agent_trace::validate_agent_trace_value(value)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
            },
            |insert| storage.db.insert_agent_trace(insert).map(|_| ()),
        )
        .expect("post-commit Agent Trace should persist");
        assert_eq!(
            trace.tool.as_ref().and_then(|tool| tool.name.as_deref()),
            Some("codex")
        );

        let intersections = storage
            .db
            .query_map(
                "SELECT intersection_patch FROM post_commit_patch_intersections ORDER BY id",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("intersection row should be queryable");
        assert_eq!(intersections.len(), 1);
        let intersection: serde_json::Value =
            serde_json::from_str(&intersections[0]).expect("intersection JSON should parse");
        assert_eq!(
            intersection["files"][0]["hunks"][0]["lines"][1]["session_id"],
            "cx_session-realistic"
        );

        let traces = storage
            .db
            .query_map(
                "SELECT trace_json FROM agent_traces ORDER BY id",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("Agent Trace row should be queryable");
        assert_eq!(traces.len(), 1);
        let trace_json: serde_json::Value =
            serde_json::from_str(&traces[0]).expect("stored Agent Trace JSON should parse");
        assert_eq!(trace_json["tool"]["name"], "codex");
        assert_eq!(
            trace_json["files"][0]["conversations"][0]["contributor"]["model_id"],
            "custom/codex-model"
        );

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    #[test]
    fn delete_only_and_pure_rename_apply_patch_events_persist_no_rows() {
        let (repository_root, state_root) = initialize_repository("no-row-boundaries");
        let delete_payload = codex_apply_patch_payload(
            &repository_root,
            "session-delete",
            "custom/model",
            "tool-delete",
            "*** Begin Patch\n*** Delete File: obsolete.txt\n*** End Patch",
        );
        let rename_payload = codex_apply_patch_payload(
            &repository_root,
            "session-rename",
            "custom/model",
            "tool-rename",
            "*** Begin Patch\n*** Update File: old.txt\n*** Move to: new.txt\n*** End Patch",
        );

        for payload in [delete_payload, rename_payload] {
            let output = run_codex_subcommand_from_payload_at_state_root(
                &repository_root,
                &payload,
                None,
                &state_root,
            )
            .expect("delete and pure-rename hooks should fail open successfully");
            assert_eq!(output, "");
        }

        let storage = resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root: &repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            &state_root,
        )
        .expect("repository Agent Trace DB should reopen");
        let recent = storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 0);
        assert_eq!(recent.skipped_count(), 0);

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    // --- T20/AC26 ownership boundary: the parser accepts absolute and `..`
    // path syntax unresolved (see apply_patch/parser.rs), and
    // `resolve_codex_patch_paths` (apply_patch/path.rs) is the sole
    // authority deciding whether a parsed path is safe and stays inside the
    // canonical Git worktree. These end-to-end tests exercise the real
    // `PostToolUse apply_patch -> parse -> cwd-aware path resolution ->
    // normalize -> diff_traces` pipeline, not `path.rs` in isolation. ---

    fn diff_trace_count(repository_root: &Path, state_root: &Path) -> usize {
        let storage = resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            state_root,
        )
        .expect("repository Agent Trace DB should reopen");
        storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed")
            .loaded_count()
    }

    #[test]
    fn nested_cwd_parent_traversal_path_is_accepted_and_persisted_repo_relative() {
        let (repository_root, state_root) = initialize_repository("nested-cwd-traversal");
        let cwd = repository_root.join("src").join("lib");
        fs::create_dir_all(&cwd).expect("nested cwd should be created");

        let payload = codex_apply_patch_payload(
            &cwd,
            "session-nested-traversal",
            "custom/model",
            "tool-nested-traversal",
            "*** Begin Patch\n*** Add File: ../inside.rs\n+content\n*** End Patch",
        );
        let output = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect("a `..` path that stays inside the repo should be accepted");
        assert_eq!(output, "");

        let storage = resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root: &repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            &state_root,
        )
        .expect("repository Agent Trace DB should reopen");
        let recent = storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 1);
        let file = &recent.patches[0].patch.files[0];
        assert_eq!(file.kind, FileChangeKind::Added);
        assert_eq!(file.new_path, "src/inside.rs");

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    #[test]
    fn absolute_path_inside_worktree_is_accepted_and_persisted_repo_relative() {
        let (repository_root, state_root) = initialize_repository("absolute-inside");
        let absolute_target = repository_root.join("lib.rs");

        let payload = codex_apply_patch_payload(
            &repository_root,
            "session-absolute-inside",
            "custom/model",
            "tool-absolute-inside",
            &format!(
                "*** Begin Patch\n*** Add File: {}\n+content\n*** End Patch",
                absolute_target.display()
            ),
        );
        let output = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect("an absolute path inside the worktree should be accepted");
        assert_eq!(output, "");

        let storage = resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root: &repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            &state_root,
        )
        .expect("repository Agent Trace DB should reopen");
        let recent = storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 1);
        let file = &recent.patches[0].patch.files[0];
        assert_eq!(file.kind, FileChangeKind::Added);
        assert_eq!(file.new_path, "lib.rs");

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    #[test]
    fn parent_traversal_path_escaping_repository_is_rejected_with_no_diff_trace() {
        let (repository_root, state_root) = initialize_repository("traversal-escape");

        let payload = codex_apply_patch_payload(
            &repository_root,
            "session-traversal-escape",
            "custom/model",
            "tool-traversal-escape",
            "*** Begin Patch\n*** Add File: ../outside.rs\n+content\n*** End Patch",
        );
        let output = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect("a `..` path escaping the repo should fail open, not error");
        assert_eq!(output, "");
        assert_eq!(diff_trace_count(&repository_root, &state_root), 0);

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    #[test]
    fn absolute_path_outside_repository_is_rejected_with_no_diff_trace() {
        let (repository_root, state_root) = initialize_repository("absolute-outside");
        let outside_target = std::env::temp_dir().join(format!(
            "sce-codex-outside-target-{}-{}.rs",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after Unix epoch")
                .as_nanos()
        ));

        let payload = codex_apply_patch_payload(
            &repository_root,
            "session-absolute-outside",
            "custom/model",
            "tool-absolute-outside",
            &format!(
                "*** Begin Patch\n*** Add File: {}\n+content\n*** End Patch",
                outside_target.display()
            ),
        );
        let output = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect("an absolute path outside the repo should fail open, not error");
        assert_eq!(output, "");
        assert_eq!(diff_trace_count(&repository_root, &state_root), 0);

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }

    #[test]
    fn move_to_destination_with_valid_parent_traversal_resolves_source_and_destination_independently(
    ) {
        let (repository_root, state_root) = initialize_repository("move-traversal");
        let cwd = repository_root.join("src");
        fs::create_dir_all(&cwd).expect("src directory should be created");

        let payload = codex_apply_patch_payload(
            &cwd,
            "session-move-traversal",
            "custom/model",
            "tool-move-traversal",
            "*** Begin Patch\n*** Update File: old.rs\n*** Move to: ../moved.rs\n@@\n-old\n+new\n*** End Patch",
        );
        let output = run_codex_subcommand_from_payload_at_state_root(
            &repository_root,
            &payload,
            None,
            &state_root,
        )
        .expect("a move whose destination traverses `..` inside the repo should be accepted");
        assert_eq!(output, "");

        let storage = resolve_agent_trace_storage_for_hook_runtime_at_state_root(
            &AgentTraceStorageContext {
                repository_root: &repository_root,
                explicit_repository_id: None,
                repository_remote: "origin",
            },
            &state_root,
        )
        .expect("repository Agent Trace DB should reopen");
        let recent = storage
            .db
            .recent_diff_trace_patches(0, i64::MAX)
            .expect("diff trace query should succeed");
        assert_eq!(recent.loaded_count(), 1);
        let file = &recent.patches[0].patch.files[0];
        assert_eq!(file.old_path, "src/old.rs");
        assert_eq!(file.new_path, "moved.rs");

        fs::remove_dir_all(&repository_root).ok();
        fs::remove_dir_all(&state_root).ok();
    }
}
