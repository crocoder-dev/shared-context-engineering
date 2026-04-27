use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Number, Value};

use crate::services::config;

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";

static TRACE_FILE_NAME_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HookSubcommand {
    PreCommit,
    CommitMsg { message_file: PathBuf },
    PostCommit,
    PostRewrite { rewrite_method: String },
    DiffTrace,
}

pub fn run_hooks_subcommand(subcommand: &HookSubcommand) -> Result<String> {
    let repository_root = std::env::current_dir().with_context(|| {
        format!(
            "Failed to determine current directory for {}.",
            hook_runtime_invocation_name(subcommand)
        )
    })?;

    run_hooks_subcommand_in_repo(&repository_root, subcommand)
}

fn run_hooks_subcommand_in_repo(
    repository_root: &Path,
    subcommand: &HookSubcommand,
) -> Result<String> {
    match subcommand {
        HookSubcommand::PreCommit => run_pre_commit_subcommand_with_trace(repository_root),
        HookSubcommand::CommitMsg { message_file } => {
            run_commit_msg_subcommand_with_trace(repository_root, subcommand, message_file)
        }
        HookSubcommand::PostCommit => run_post_commit_subcommand_with_trace(repository_root),
        HookSubcommand::PostRewrite { rewrite_method } => {
            run_post_rewrite_subcommand_with_trace(repository_root, subcommand, rewrite_method)
        }
        HookSubcommand::DiffTrace => run_diff_trace_subcommand(repository_root),
    }
}

fn run_diff_trace_subcommand(repository_root: &Path) -> Result<String> {
    let stdin_payload = read_hook_stdin()?;
    run_diff_trace_subcommand_from_payload(repository_root, &stdin_payload)
}

fn run_diff_trace_subcommand_from_payload(
    repository_root: &Path,
    stdin_payload: &str,
) -> Result<String> {
    let payload = parse_diff_trace_payload(stdin_payload)?;
    persist_diff_trace_payload(repository_root, &payload)?;

    Ok(String::from(
        "diff-trace hook intake persisted payload to context/tmp.",
    ))
}

fn parse_diff_trace_payload(stdin_payload: &str) -> Result<Value> {
    let parsed: Value = serde_json::from_str(stdin_payload)
        .context("Invalid diff-trace payload from STDIN: expected valid JSON.")?;

    let session_id = required_non_empty_string_field(&parsed, "sessionID")?;
    let diff = required_non_empty_string_field(&parsed, "diff")?;
    let time = required_number_field(&parsed, "time")?;

    Ok(json!({
        "sessionID": session_id,
        "diff": diff,
        "time": time,
    }))
}

fn required_non_empty_string_field(payload: &Value, field_name: &str) -> Result<String> {
    let object = payload
        .as_object()
        .ok_or_else(|| anyhow!("Invalid diff-trace payload from STDIN: expected a JSON object."))?;

    let raw = object.get(field_name).ok_or_else(|| {
        anyhow!("Invalid diff-trace payload from STDIN: missing required field '{field_name}'.")
    })?;

    let value = raw.as_str().ok_or_else(|| {
        anyhow!(
            "Invalid diff-trace payload from STDIN: field '{field_name}' must be a non-empty string."
        )
    })?;

    if value.trim().is_empty() {
        bail!("Invalid diff-trace payload from STDIN: field '{field_name}' must be a non-empty string.");
    }

    Ok(value.to_string())
}

fn required_number_field(payload: &Value, field_name: &str) -> Result<Number> {
    let object = payload
        .as_object()
        .ok_or_else(|| anyhow!("Invalid diff-trace payload from STDIN: expected a JSON object."))?;

    let raw = object.get(field_name).ok_or_else(|| {
        anyhow!("Invalid diff-trace payload from STDIN: missing required field '{field_name}'.")
    })?;

    raw.as_number().cloned().ok_or_else(|| {
        anyhow!("Invalid diff-trace payload from STDIN: field '{field_name}' must be a number.")
    })
}

fn persist_diff_trace_payload(repository_root: &Path, payload: &Value) -> Result<PathBuf> {
    let trace_directory = repository_root.join("context").join("tmp");
    let file_path = trace_directory.join(build_diff_trace_file_name(Utc::now()));

    persist_diff_trace_payload_to_file(&trace_directory, &file_path, payload)?;

    Ok(file_path)
}

fn persist_diff_trace_payload_to_file(
    trace_directory: &Path,
    file_path: &Path,
    payload: &Value,
) -> Result<()> {
    fs::create_dir_all(trace_directory).with_context(|| {
        format!(
            "Failed to create diff-trace directory '{}'.",
            trace_directory.display()
        )
    })?;

    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(payload)
            .context("Failed to serialize diff-trace payload for persistence.")?
    );
    fs::write(file_path, serialized).with_context(|| {
        format!(
            "Failed to write diff-trace payload file '{}'.",
            file_path.display()
        )
    })?;

    Ok(())
}

fn build_diff_trace_file_name(timestamp: DateTime<Utc>) -> String {
    build_trace_file_name("diff-trace", timestamp)
}

fn format_trace_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%dT%H-%M-%S-%3fZ").to_string()
}

fn build_trace_file_name(trace_name: &str, timestamp: DateTime<Utc>) -> String {
    let safe_name = sanitize_trace_name(trace_name);
    let sequence = TRACE_FILE_NAME_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;

    format!(
        "{}-{}-{}.json",
        format_trace_timestamp(timestamp),
        sequence,
        safe_name
    )
}

fn sanitize_trace_name(trace_name: &str) -> String {
    trace_name
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => character,
            _ => '_',
        })
        .collect()
}

fn run_pre_commit_subcommand_with_trace(repository_root: &Path) -> Result<String> {
    run_pre_commit_subcommand(repository_root)
}

fn run_pre_commit_subcommand(repository_root: &Path) -> Result<String> {
    let runtime = resolve_runtime_state(repository_root)?;

    Ok(format!(
        "pre-commit hook executed with no-op runtime state: {:?}",
        pre_commit_no_op_reason(&runtime)
    ))
}

fn run_commit_msg_subcommand_in_repo(
    repository_root: &Path,
    message_file: &Path,
) -> Result<String> {
    let metadata = fs::metadata(message_file).with_context(|| {
        format!(
            "Invalid commit message file '{}': file does not exist or is not readable.",
            message_file.display()
        )
    })?;

    if !metadata.is_file() {
        bail!(
            "Invalid commit message file '{}': expected a regular file path.",
            message_file.display()
        );
    }

    let runtime = resolve_runtime_state(repository_root)?;
    let original = fs::read_to_string(message_file).with_context(|| {
        format!(
            "Invalid commit message file '{}': failed to read UTF-8 content.",
            message_file.display()
        )
    })?;

    let gate_passed = commit_msg_policy_gate_passed(&runtime);
    let transformed = apply_commit_msg_coauthor_policy(&runtime, &original);
    let trailer_applied = gate_passed && transformed != original;

    if trailer_applied {
        fs::write(message_file, transformed.as_bytes()).with_context(|| {
            format!(
                "Failed to update commit message file '{}' with canonical co-author trailer.",
                message_file.display()
            )
        })?;
    }

    Ok(format!(
        "commit-msg hook processed message file '{}' (policy_gate_passed={}, trailer_applied={}).",
        message_file.display(),
        gate_passed,
        trailer_applied
    ))
}

fn run_commit_msg_subcommand_with_trace(
    repository_root: &Path,
    _: &HookSubcommand,
    message_file: &Path,
) -> Result<String> {
    run_commit_msg_subcommand_in_repo(repository_root, message_file)
}

fn run_post_commit_subcommand(repository_root: &Path) -> Result<String> {
    let runtime = resolve_runtime_state(repository_root)?;

    Ok(format!(
        "post-commit hook executed with no-op runtime state: {:?}",
        post_commit_no_op_reason(&runtime)
    ))
}

fn run_post_commit_subcommand_with_trace(repository_root: &Path) -> Result<String> {
    let subcommand = HookSubcommand::PostCommit;
    let input = build_hook_trace_input_for_post_commit(repository_root);
    let outcome = run_post_commit_subcommand(repository_root);

    let _ = persist_hook_trace(repository_root, &subcommand, &input, &outcome);

    outcome
}

fn run_post_rewrite_subcommand(repository_root: &Path, rewrite_method: &str) -> Result<String> {
    let runtime = resolve_runtime_state(repository_root)?;

    Ok(format!(
        "post-rewrite hook executed with no-op runtime state: {:?} (rewrite_method='{}')",
        post_rewrite_no_op_reason(&runtime),
        rewrite_method.trim()
    ))
}

fn run_post_rewrite_subcommand_with_trace(
    repository_root: &Path,
    _: &HookSubcommand,
    rewrite_method: &str,
) -> Result<String> {
    let stdin_payload = read_hook_stdin();
    stdin_payload.and_then(|_| run_post_rewrite_subcommand(repository_root, rewrite_method))
}

fn hook_runtime_invocation_name(subcommand: &HookSubcommand) -> &'static str {
    match subcommand {
        HookSubcommand::PreCommit => "pre-commit runtime invocation",
        HookSubcommand::CommitMsg { .. } => "commit-msg runtime invocation",
        HookSubcommand::PostCommit => "post-commit runtime invocation",
        HookSubcommand::PostRewrite { .. } => "post-rewrite runtime invocation",
        HookSubcommand::DiffTrace => "diff-trace runtime invocation",
    }
}

fn persist_hook_trace(
    repository_root: &Path,
    subcommand: &HookSubcommand,
    input: &Value,
    outcome: &Result<String>,
) -> Result<()> {
    let trace_directory = repository_root.join("context").join("tmp");
    let file_path = trace_directory.join(build_hook_trace_file_name(subcommand));
    let body = match outcome {
        Ok(output) => json!({
            "input": input,
            "output": output,
        }),
        Err(error) => json!({
            "input": input,
            "error": error.to_string(),
        }),
    };

    fs::create_dir_all(&trace_directory).with_context(|| {
        format!(
            "Failed to create hook trace directory '{}'.",
            trace_directory.display()
        )
    })?;

    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&body).context("Failed to serialize hook trace.")?
    );
    fs::write(&file_path, serialized)
        .with_context(|| format!("Failed to write hook trace file '{}'.", file_path.display()))?;

    Ok(())
}

fn build_hook_trace_file_name(subcommand: &HookSubcommand) -> String {
    build_trace_file_name(hook_trace_name(subcommand), Utc::now())
}

fn hook_trace_name(subcommand: &HookSubcommand) -> &'static str {
    match subcommand {
        HookSubcommand::PreCommit => "pre-commit",
        HookSubcommand::CommitMsg { .. } => "commit-msg",
        HookSubcommand::PostCommit => "post-commit",
        HookSubcommand::PostRewrite { .. } => "post-rewrite",
        HookSubcommand::DiffTrace => "diff-trace",
    }
}

fn build_hook_trace_input_for_post_commit(repository_root: &Path) -> Value {
    let mut input = build_base_hook_trace_input("post-commit");
    insert_head_commit_from_git(repository_root, &mut input);
    Value::Object(input)
}

fn build_base_hook_trace_input(hook_name: &str) -> serde_json::Map<String, Value> {
    let mut input = serde_json::Map::new();
    input.insert("hook".to_string(), Value::String(hook_name.to_string()));
    input.insert(
        "git_env".to_string(),
        Value::Object(
            collect_git_environment()
                .into_iter()
                .map(|(key, value)| (key, Value::String(value)))
                .collect(),
        ),
    );
    input
}

fn collect_git_environment() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(key, _)| key.starts_with("GIT_"))
        .collect()
}

fn read_hook_stdin() -> Result<String> {
    let mut stdin_payload = String::new();
    io::stdin()
        .read_to_string(&mut stdin_payload)
        .context("Failed to read hook input from STDIN.")?;
    Ok(stdin_payload)
}

fn insert_head_commit_from_git(repository_root: &Path, input: &mut serde_json::Map<String, Value>) {
    insert_git_output(
        repository_root,
        &["rev-parse", "HEAD"],
        "Failed to capture HEAD revision from git.",
        input,
        "head_oid_from_git",
        "head_oid_from_git_read_error",
    );
    insert_git_output(
        repository_root,
        &["show", "--format=", "--patch", "--no-ext-diff", "HEAD"],
        "Failed to capture HEAD patch from git.",
        input,
        "head_patch_from_git",
        "head_patch_from_git_read_error",
    );
}

fn insert_git_output(
    repository_root: &Path,
    args: &[&str],
    context_message: &str,
    input: &mut serde_json::Map<String, Value>,
    output_key: &str,
    error_key: &str,
) {
    match run_git_command_capture_stdout(repository_root, args, context_message) {
        Ok(stdout) => {
            input.insert(output_key.to_string(), Value::String(stdout));
        }
        Err(error) => {
            input.insert(error_key.to_string(), Value::String(error.to_string()));
        }
    }
}

fn run_git_command_capture_stdout(
    repository_root: &Path,
    args: &[&str],
    context_message: &str,
) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "{} (directory: '{}')",
                context_message,
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let diagnostic = if stderr.is_empty() {
            String::from("git command exited with a non-zero status")
        } else {
            stderr
        };
        bail!("{context_message} {diagnostic}");
    }

    String::from_utf8(output.stdout).context("git command output contained invalid UTF-8")
}

fn resolve_runtime_state(repository_root: &Path) -> Result<HookRuntimeState> {
    Ok(HookRuntimeState {
        sce_disabled: env_flag_is_truthy("SCE_DISABLED"),
        attribution_hooks_enabled: config::resolve_hook_runtime_config(repository_root)?
            .attribution_hooks_enabled,
    })
}

fn env_flag_is_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| env_value_is_truthy(&value))
}

fn env_value_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn commit_msg_policy_gate_passed(runtime: &HookRuntimeState) -> bool {
    !runtime.sce_disabled && runtime.attribution_hooks_enabled
}

fn pre_commit_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

fn post_commit_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

fn post_rewrite_no_op_reason(runtime: &HookRuntimeState) -> HookNoOpReason {
    if runtime.sce_disabled {
        HookNoOpReason::Disabled
    } else {
        HookNoOpReason::AttributionOnlyCommitMsgMode
    }
}

pub fn apply_commit_msg_coauthor_policy(
    runtime: &HookRuntimeState,
    commit_message: &str,
) -> String {
    if !commit_msg_policy_gate_passed(runtime) {
        return commit_message.to_string();
    }

    let mut lines: Vec<&str> = commit_message.lines().collect();
    lines.retain(|line| *line != CANONICAL_SCE_COAUTHOR_TRAILER);

    if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
        lines.push("");
    }
    lines.push(CANONICAL_SCE_COAUTHOR_TRAILER);

    let mut normalized = lines.join("\n");
    if commit_message.ends_with('\n') {
        normalized.push('\n');
    }

    normalized
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookRuntimeState {
    pub sce_disabled: bool,
    pub attribution_hooks_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookNoOpReason {
    Disabled,
    AttributionOnlyCommitMsgMode,
}
