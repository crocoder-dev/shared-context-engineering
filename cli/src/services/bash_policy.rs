use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::services::config;
use crate::services::config::policy::{
    runtime_bash_policy_presets, BashPolicyConfig, CustomBashPolicyEntry, RuntimeBashPolicyPreset,
};
use crate::services::error::ClassifiedError;

pub mod command {
    use crate::services::bash_policy;
    use crate::services::error::ClassifiedError;

    pub struct PolicyCommand {
        pub request: bash_policy::BashPolicyRequest,
    }

    impl PolicyCommand {
        pub fn execute(&self) -> Result<String, ClassifiedError> {
            bash_policy::run_bash_policy_request(&self.request)
        }
    }
}

pub const NAME: &str = "policy";

const ENV_ASSIGNMENT_PREFIX_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_";
const WRAPPER_BINARIES: &[&str] = &["env", "/usr/bin/env", "command", "nohup", "sudo"];
const SHELL_BINARIES: &[&str] = &["sh", "bash"];
const SHELL_OPERATORS: &[&str] = &["|", "&&", "||", ";", "&"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PolicySource {
    Preset,
    Custom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyMatch {
    pub(crate) id: String,
    pub(crate) message: String,
    pub(crate) argv_prefix: Vec<String>,
    pub(crate) source: PolicySource,
    pub(crate) order: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PolicyEvaluation {
    Allowed {
        normalized_argv: Option<Vec<String>>,
    },
    Blocked {
        normalized_argv: Vec<String>,
        policy: PolicyMatch,
    },
}

pub(crate) fn evaluate_bash_command_policy(
    command: &str,
    policy_config: Option<&BashPolicyConfig>,
) -> PolicyEvaluation {
    let Some(segments) = parse_command_segments(command) else {
        return PolicyEvaluation::Allowed {
            normalized_argv: None,
        };
    };
    if segments.is_empty() {
        return PolicyEvaluation::Allowed {
            normalized_argv: None,
        };
    }

    let active_policies = policy_config.map(build_active_policies).unwrap_or_default();
    let mut first_normalized = None;

    for segment in segments {
        for normalized_argv in normalize_segment(&segment) {
            if normalized_argv.is_empty() {
                continue;
            }
            if first_normalized.is_none() {
                first_normalized = Some(normalized_argv.clone());
            }
            if let Some(policy) = select_matching_policy(&active_policies, &normalized_argv) {
                return PolicyEvaluation::Blocked {
                    normalized_argv,
                    policy,
                };
            }
        }
    }

    PolicyEvaluation::Allowed {
        normalized_argv: first_normalized,
    }
}

pub(crate) fn format_policy_block_message(policy: &PolicyMatch) -> String {
    format!(
        "Blocked by SCE bash-tool policy '{}': {}",
        policy.id, policy.message
    )
}

pub(crate) fn parse_command_segments(command: &str) -> Option<Vec<Vec<String>>> {
    let tokens = tokenize_shell_command(command)?;
    if tokens.is_empty() {
        return None;
    }

    let mut segments = Vec::new();
    let mut current_segment = Vec::new();
    for token in tokens {
        if is_shell_operator(&token) {
            if !current_segment.is_empty() {
                segments.push(current_segment);
                current_segment = Vec::new();
            }
        } else {
            current_segment.push(token);
        }
    }
    if !current_segment.is_empty() {
        segments.push(current_segment);
    }

    Some(segments)
}

fn normalize_segment(segment: &[String]) -> Vec<Vec<String>> {
    if segment.is_empty() {
        return Vec::new();
    }

    let mut normalized = segment.to_vec();
    drop_leading_env_assignments(&mut normalized);

    while let Some(executable) = normalized.first() {
        if !WRAPPER_BINARIES.contains(&executable.as_str()) {
            break;
        }
        normalized.remove(0);
        drop_leading_env_assignments(&mut normalized);
    }

    if normalized.is_empty() {
        return Vec::new();
    }

    normalized[0] = Path::new(&normalized[0])
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string();

    let Some(nested_segments) = unwrap_nested_command_segments(&normalized) else {
        return vec![normalized];
    };

    let mut nested_normalized = Vec::new();
    for nested_segment in nested_segments {
        nested_normalized.extend(normalize_segment(&nested_segment));
    }

    if nested_normalized.is_empty() {
        vec![normalized]
    } else {
        nested_normalized
    }
}

fn unwrap_nested_command_segments(segment: &[String]) -> Option<Vec<Vec<String>>> {
    let executable = segment.first()?;
    if executable == "nix" {
        return extract_nix_command_argv(segment).map(|argv| vec![argv]);
    }
    if SHELL_BINARIES.contains(&executable.as_str()) {
        return extract_shell_command_payload(segment)
            .and_then(|payload| parse_command_segments(&payload));
    }
    None
}

fn extract_nix_command_argv(segment: &[String]) -> Option<Vec<String>> {
    for (index, token) in segment.iter().enumerate().skip(1) {
        if token == "-c" || token == "--command" {
            let nested_argv = segment[index + 1..].to_vec();
            return (!nested_argv.is_empty()).then_some(nested_argv);
        }
    }
    None
}

fn extract_shell_command_payload(segment: &[String]) -> Option<String> {
    for (index, token) in segment.iter().enumerate().skip(1) {
        if token.is_empty() || token == "--" {
            continue;
        }
        if token == "-c" || (token.starts_with('-') && token.contains('c')) {
            return segment.get(index + 1).cloned();
        }
    }
    None
}

fn build_active_policies(policy_config: &BashPolicyConfig) -> Vec<PolicyMatch> {
    let preset_catalog = runtime_bash_policy_presets();
    build_active_policies_from_catalog(policy_config, &preset_catalog)
}

fn build_active_policies_from_catalog(
    policy_config: &BashPolicyConfig,
    preset_catalog: &[RuntimeBashPolicyPreset],
) -> Vec<PolicyMatch> {
    let mut policies = Vec::new();

    for preset_id in &policy_config.presets {
        let Some(preset) = preset_catalog.iter().find(|preset| &preset.id == preset_id) else {
            continue;
        };
        for argv_prefix in &preset.argv_prefixes {
            if argv_prefix.is_empty() || argv_prefix.iter().any(String::is_empty) {
                continue;
            }
            policies.push(PolicyMatch {
                id: preset.id.clone(),
                message: preset.message.clone(),
                argv_prefix: argv_prefix.clone(),
                source: PolicySource::Preset,
                order: preset.order,
            });
        }
    }

    policies.extend(
        policy_config
            .custom
            .iter()
            .enumerate()
            .filter_map(|(order, policy)| custom_policy_match(policy, order)),
    );

    policies
}

fn custom_policy_match(policy: &CustomBashPolicyEntry, order: usize) -> Option<PolicyMatch> {
    if policy.id.is_empty()
        || policy.message.is_empty()
        || policy.argv_prefix.is_empty()
        || policy.argv_prefix.iter().any(String::is_empty)
    {
        return None;
    }
    Some(PolicyMatch {
        id: policy.id.clone(),
        message: policy.message.clone(),
        argv_prefix: policy.argv_prefix.clone(),
        source: PolicySource::Custom,
        order,
    })
}

fn select_matching_policy(
    active_policies: &[PolicyMatch],
    normalized_argv: &[String],
) -> Option<PolicyMatch> {
    active_policies
        .iter()
        .filter(|policy| argv_starts_with(normalized_argv, &policy.argv_prefix))
        .min_by(|left, right| compare_policy_priority(left, right))
        .cloned()
}

fn compare_policy_priority(left: &PolicyMatch, right: &PolicyMatch) -> std::cmp::Ordering {
    right
        .argv_prefix
        .len()
        .cmp(&left.argv_prefix.len())
        .then_with(|| match (left.source, right.source) {
            (PolicySource::Custom, PolicySource::Preset) => std::cmp::Ordering::Less,
            (PolicySource::Preset, PolicySource::Custom) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        })
        .then_with(|| left.order.cmp(&right.order))
}

fn argv_starts_with(argv: &[String], prefix: &[String]) -> bool {
    prefix.len() <= argv.len()
        && prefix
            .iter()
            .enumerate()
            .all(|(index, token)| argv.get(index) == Some(token))
}

fn drop_leading_env_assignments(argv: &mut Vec<String>) {
    while argv.first().is_some_and(|token| is_env_assignment(token)) {
        argv.remove(0);
    }
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    ENV_ASSIGNMENT_PREFIX_CHARS.contains(first)
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn tokenize_shell_command(command: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaping = false;
    let mut chars = command.chars().peekable();

    while let Some(character) = chars.next() {
        if escaping {
            current.push(character);
            escaping = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaping = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if character == '"' || character == '\'' {
            quote = Some(character);
            continue;
        }
        if character.is_whitespace() {
            push_current_token(&mut tokens, &mut current);
            continue;
        }
        if matches!(character, '&' | '|' | ';') {
            push_current_token(&mut tokens, &mut current);
            if matches!(character, '&' | '|') && chars.peek() == Some(&character) {
                chars.next();
                tokens.push(format!("{character}{character}"));
            } else {
                tokens.push(character.to_string());
            }
            continue;
        }
        current.push(character);
    }

    if escaping || quote.is_some() {
        return None;
    }
    push_current_token(&mut tokens, &mut current);
    Some(tokens)
}

fn push_current_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn is_shell_operator(token: &str) -> bool {
    SHELL_OPERATORS.contains(&token)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyInputMode {
    ClaudePreToolUse,
    Normalized,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyOutputMode {
    ClaudeHook,
    Json,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BashPolicyRequest {
    pub input: PolicyInputMode,
    pub output: PolicyOutputMode,
}

#[derive(Debug, Deserialize)]
struct NormalizedBashPolicyRequest {
    command: String,
}

#[derive(Debug, Deserialize)]
struct ClaudePreToolUseEvent {
    #[serde(default)]
    tool_name: Option<String>,
    tool_input: ClaudeBashToolInput,
}

#[derive(Debug, Deserialize)]
struct ClaudeBashToolInput {
    command: String,
}

#[derive(Serialize)]
struct JsonPolicyResult<'a> {
    status: &'static str,
    decision: &'static str,
    command: &'a str,
    normalized_argv: Option<&'a [String]>,
    reason: Option<&'a str>,
    policy_id: Option<&'a str>,
}

pub fn run_bash_policy_request(request: &BashPolicyRequest) -> Result<String, ClassifiedError> {
    let stdin_payload = read_stdin_payload()?;
    run_bash_policy_request_from_payload(request, &stdin_payload)
}

fn run_bash_policy_request_from_payload(
    request: &BashPolicyRequest,
    stdin_payload: &str,
) -> Result<String, ClassifiedError> {
    let command = parse_command_from_stdin(request.input, stdin_payload)?;
    let cwd = resolved_policy_project_root()?;
    let policy_config = config::resolve_bash_policy_runtime_config(&cwd).map_err(|error| {
        ClassifiedError::runtime(format!(
            "Failed to resolve bash policy configuration for '{}': {error}",
            cwd.display()
        ))
    })?;
    let evaluation = evaluate_bash_command_policy(&command, policy_config.as_ref());

    render_policy_result(request.output, &command, &evaluation)
}

fn read_stdin_payload() -> Result<String, ClassifiedError> {
    let mut payload = String::new();
    io::stdin().read_to_string(&mut payload).map_err(|error| {
        ClassifiedError::validation(format!(
            "Failed to read bash policy request from STDIN: {error}. Try: pipe a JSON payload to 'sce policy bash'."
        ))
    })?;
    if payload.trim().is_empty() {
        return Err(ClassifiedError::validation(
            "Missing bash policy request on STDIN. Try: pipe Claude PreToolUse JSON or normalized {\"command\":...} JSON to 'sce policy bash'.",
        ));
    }
    Ok(payload)
}

fn parse_command_from_stdin(
    input: PolicyInputMode,
    stdin_payload: &str,
) -> Result<String, ClassifiedError> {
    match input {
        PolicyInputMode::ClaudePreToolUse => parse_claude_pre_tool_use_command(stdin_payload),
        PolicyInputMode::Normalized => parse_normalized_command(stdin_payload),
    }
}

fn parse_claude_pre_tool_use_command(stdin_payload: &str) -> Result<String, ClassifiedError> {
    let event: ClaudePreToolUseEvent = parse_json_payload(stdin_payload, "Claude PreToolUse")?;
    if let Some(tool_name) = event.tool_name.as_deref() {
        if tool_name != "Bash" {
            return Err(ClassifiedError::validation(format!(
                "Invalid Claude PreToolUse payload: expected tool_name 'Bash' but received '{tool_name}'."
            )));
        }
    }
    validate_non_empty_command(event.tool_input.command, "Claude PreToolUse")
}

fn parse_normalized_command(stdin_payload: &str) -> Result<String, ClassifiedError> {
    let request: NormalizedBashPolicyRequest =
        parse_json_payload(stdin_payload, "normalized bash policy")?;
    validate_non_empty_command(request.command, "normalized bash policy")
}

fn parse_json_payload<T>(stdin_payload: &str, label: &str) -> Result<T, ClassifiedError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(stdin_payload).map_err(|error| {
        ClassifiedError::validation(format!(
            "Invalid {label} JSON from STDIN: {error}. Try: pipe a valid JSON object to 'sce policy bash'."
        ))
    })
}

fn validate_non_empty_command(command: String, label: &str) -> Result<String, ClassifiedError> {
    if command.trim().is_empty() {
        return Err(ClassifiedError::validation(format!(
            "Invalid {label} payload: command must be a non-empty string."
        )));
    }
    Ok(command)
}

fn resolved_policy_project_root() -> Result<PathBuf, ClassifiedError> {
    let cwd = std::env::current_dir().map_err(|error| {
        ClassifiedError::runtime(format!(
            "Failed to determine current directory for bash policy configuration: {error}"
        ))
    })?;
    Ok(resolve_git_root(&cwd).unwrap_or(cwd))
}

fn resolve_git_root(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn render_policy_result(
    output: PolicyOutputMode,
    command: &str,
    evaluation: &PolicyEvaluation,
) -> Result<String, ClassifiedError> {
    match output {
        PolicyOutputMode::ClaudeHook => render_claude_hook_result(evaluation),
        PolicyOutputMode::Json => render_json_result(command, evaluation),
    }
}

fn render_claude_hook_result(evaluation: &PolicyEvaluation) -> Result<String, ClassifiedError> {
    match evaluation {
        PolicyEvaluation::Allowed { .. } => Ok(String::new()),
        PolicyEvaluation::Blocked { policy, .. } => serialize_json(&json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": format_policy_block_message(policy)
            }
        })),
    }
}

fn render_json_result(
    command: &str,
    evaluation: &PolicyEvaluation,
) -> Result<String, ClassifiedError> {
    match evaluation {
        PolicyEvaluation::Allowed { normalized_argv } => serialize_json(&JsonPolicyResult {
            status: "ok",
            decision: "allow",
            command,
            normalized_argv: normalized_argv.as_deref(),
            reason: None,
            policy_id: None,
        }),
        PolicyEvaluation::Blocked {
            normalized_argv,
            policy,
        } => {
            let reason = format_policy_block_message(policy);
            serialize_json(&JsonPolicyResult {
                status: "ok",
                decision: "deny",
                command,
                normalized_argv: Some(normalized_argv.as_slice()),
                reason: Some(&reason),
                policy_id: Some(&policy.id),
            })
        }
    }
}

fn serialize_json<T: ?Sized + Serialize>(value: &T) -> Result<String, ClassifiedError> {
    serde_json::to_string(value).map_err(|error| {
        ClassifiedError::runtime(format!(
            "Failed to serialize bash policy result JSON: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::Value;

    fn config(presets: &[&str], custom: Vec<CustomBashPolicyEntry>) -> BashPolicyConfig {
        BashPolicyConfig {
            presets: presets.iter().map(|preset| (*preset).to_string()).collect(),
            custom,
        }
    }

    fn custom(id: &str, argv_prefix: &[&str], message: &str) -> CustomBashPolicyEntry {
        CustomBashPolicyEntry {
            id: id.to_string(),
            argv_prefix: argv_prefix
                .iter()
                .map(|token| (*token).to_string())
                .collect(),
            message: message.to_string(),
        }
    }

    fn blocked_policy_id(command: &str, config: &BashPolicyConfig) -> Option<String> {
        match evaluate_bash_command_policy(command, Some(config)) {
            PolicyEvaluation::Blocked { policy, .. } => Some(policy.id),
            PolicyEvaluation::Allowed { .. } => None,
        }
    }

    #[test]
    fn bash_policy_blocks_and_allows_preset_commands() {
        let config = config(&["forbid-git-all"], Vec::new());

        assert_eq!(
            blocked_policy_id("git status", &config).as_deref(),
            Some("forbid-git-all")
        );
        assert_eq!(blocked_policy_id("echo git", &config), None);
        assert_eq!(blocked_policy_id("npm install", &config), None);
    }

    #[test]
    fn bash_policy_normalizes_wrappers_env_assignments_and_basename() {
        let config = config(&["forbid-git-all"], Vec::new());

        let evaluation =
            evaluate_bash_command_policy("env FOO=bar sudo /usr/bin/git commit", Some(&config));

        assert!(matches!(
            evaluation,
            PolicyEvaluation::Blocked { normalized_argv, .. }
                if normalized_argv == ["git", "commit"]
        ));
    }

    #[test]
    fn bash_policy_splits_shell_operator_segments() {
        assert_eq!(
            parse_command_segments("cat abc | git diff && npm run build; ls &"),
            Some(vec![
                vec!["cat".into(), "abc".into()],
                vec!["git".into(), "diff".into()],
                vec!["npm".into(), "run".into(), "build".into()],
                vec!["ls".into()],
            ])
        );
        assert_eq!(parse_command_segments("echo 'unclosed"), None);
    }

    #[test]
    fn bash_policy_blocks_shell_operator_segments() {
        let config = config(&["forbid-git-commit"], Vec::new());

        assert_eq!(
            blocked_policy_id("ls; git push", &config).as_deref(),
            Some("forbid-git-commit")
        );
        assert_eq!(blocked_policy_id("cat file | ls", &config), None);
    }

    #[test]
    fn bash_policy_recurses_into_shell_and_nix_payloads() {
        let config = config(
            &[],
            vec![custom(
                "use-nix-flake-check-over-cargo-fmt-check",
                &["cargo", "fmt", "--check"],
                "Use nix flake check.",
            )],
        );

        let evaluation = evaluate_bash_command_policy(
            "nix develop -c sh -c 'cd cli && cargo fmt --check'",
            Some(&config),
        );

        assert!(matches!(
            evaluation,
            PolicyEvaluation::Blocked { normalized_argv, policy }
                if normalized_argv == ["cargo", "fmt", "--check"]
                    && policy.id == "use-nix-flake-check-over-cargo-fmt-check"
        ));
    }

    #[test]
    fn bash_policy_uses_longest_prefix_then_custom_then_order_priority() {
        let config = config(
            &["forbid-git-all", "forbid-git-commit"],
            vec![custom("custom-git", &["git"], "Custom git block")],
        );

        assert_eq!(
            blocked_policy_id("git commit -m test", &config).as_deref(),
            Some("forbid-git-commit")
        );
        assert_eq!(
            blocked_policy_id("git status", &config).as_deref(),
            Some("custom-git")
        );
    }

    #[test]
    fn bash_policy_fail_opens_without_config_or_matching_catalog_entries() {
        let unknown_only = config(&["missing-preset"], Vec::new());

        assert!(matches!(
            evaluate_bash_command_policy("git status", None),
            PolicyEvaluation::Allowed { .. }
        ));
        assert_eq!(blocked_policy_id("git status", &unknown_only), None);
        assert!(matches!(
            evaluate_bash_command_policy("echo 'unterminated", Some(&unknown_only)),
            PolicyEvaluation::Allowed { .. }
        ));
    }

    #[test]
    fn bash_policy_formats_canonical_block_message() {
        let policy = custom_policy_match(&custom("test-policy", &["test"], "Test message"), 0)
            .expect("valid custom policy");

        assert_eq!(
            format_policy_block_message(&policy),
            "Blocked by SCE bash-tool policy 'test-policy': Test message"
        );
    }

    #[test]
    fn policy_adapter_parses_claude_pre_tool_use_bash_command() {
        let command = parse_command_from_stdin(
            PolicyInputMode::ClaudePreToolUse,
            r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git status"}}"#,
        )
        .expect("payload should parse");

        assert_eq!(command, "git status");
    }

    #[test]
    fn policy_adapter_rejects_claude_pre_tool_use_non_bash_tool() {
        let error = parse_command_from_stdin(
            PolicyInputMode::ClaudePreToolUse,
            r#"{"tool_name":"Read","tool_input":{"command":"git status"}}"#,
        )
        .expect_err("payload should fail");

        assert!(error.message().contains("expected tool_name 'Bash'"));
    }

    #[test]
    fn policy_adapter_parses_normalized_policy_request() {
        let command = parse_command_from_stdin(
            PolicyInputMode::Normalized,
            r#"{"command":"nix develop -c git commit"}"#,
        )
        .expect("payload should parse");

        assert_eq!(command, "nix develop -c git commit");
    }

    #[test]
    fn policy_adapter_renders_claude_deny_json_for_blocked_policy() {
        let config = config(&["forbid-git-commit"], Vec::new());
        let evaluation = evaluate_bash_command_policy("git commit -m test", Some(&config));
        let rendered =
            render_policy_result(PolicyOutputMode::ClaudeHook, "git commit", &evaluation)
                .expect("result should render");
        let value: Value = serde_json::from_str(&rendered).expect("result should be JSON");

        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            Value::String("PreToolUse".to_string())
        );
        assert_eq!(
            value["hookSpecificOutput"]["permissionDecision"],
            Value::String("deny".to_string())
        );
        assert!(value["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .expect("reason should be a string")
            .starts_with("Blocked by SCE bash-tool policy 'forbid-git-commit':"));
    }

    #[test]
    fn policy_adapter_renders_empty_claude_output_for_allowed_policy() {
        let evaluation = evaluate_bash_command_policy("git status", None);
        let rendered =
            render_policy_result(PolicyOutputMode::ClaudeHook, "git status", &evaluation)
                .expect("result should render");

        assert_eq!(rendered, "");
    }

    #[test]
    fn policy_adapter_renders_normalized_json_result_for_blocked_policy() {
        let config = config(&["forbid-git-commit"], Vec::new());
        let evaluation = evaluate_bash_command_policy("git commit -m test", Some(&config));
        let rendered =
            render_policy_result(PolicyOutputMode::Json, "git commit -m test", &evaluation)
                .expect("result should render");
        let value: Value = serde_json::from_str(&rendered).expect("result should be JSON");

        assert_eq!(value["status"], Value::String("ok".to_string()));
        assert_eq!(value["decision"], Value::String("deny".to_string()));
        assert_eq!(
            value["policy_id"],
            Value::String("forbid-git-commit".to_string())
        );
    }

    // --- Malformed custom policy tests (TS parity) ---

    #[test]
    fn bash_policy_ignores_custom_policy_with_empty_id() {
        let config = config(
            &[],
            vec![CustomBashPolicyEntry {
                id: String::new(),
                argv_prefix: vec!["rm".to_string()],
                message: "Empty id".to_string(),
            }],
        );

        assert!(matches!(
            evaluate_bash_command_policy("rm -rf /tmp", Some(&config)),
            PolicyEvaluation::Allowed { .. }
        ));
    }

    #[test]
    fn bash_policy_ignores_custom_policy_with_empty_message() {
        let config = config(
            &[],
            vec![CustomBashPolicyEntry {
                id: "missing-message".to_string(),
                argv_prefix: vec!["rm".to_string()],
                message: String::new(),
            }],
        );

        assert!(matches!(
            evaluate_bash_command_policy("rm -rf /tmp", Some(&config)),
            PolicyEvaluation::Allowed { .. }
        ));
    }

    #[test]
    fn bash_policy_ignores_custom_policy_with_empty_argv_prefix() {
        let config = config(
            &[],
            vec![CustomBashPolicyEntry {
                id: "empty-prefix".to_string(),
                argv_prefix: vec![],
                message: "Empty prefix".to_string(),
            }],
        );

        assert!(matches!(
            evaluate_bash_command_policy("rm -rf /tmp", Some(&config)),
            PolicyEvaluation::Allowed { .. }
        ));
    }

    #[test]
    fn bash_policy_ignores_custom_policy_with_empty_string_in_argv_prefix() {
        let config = config(
            &[],
            vec![CustomBashPolicyEntry {
                id: "empty-string-prefix".to_string(),
                argv_prefix: vec!["rm".to_string(), String::new()],
                message: "Empty string in prefix".to_string(),
            }],
        );

        assert!(matches!(
            evaluate_bash_command_policy("rm -rf /tmp", Some(&config)),
            PolicyEvaluation::Allowed { .. }
        ));
    }

    #[test]
    fn bash_policy_custom_policy_blocks_matching_command() {
        let config = config(
            &[],
            vec![custom(
                "custom-block-rm",
                &["rm"],
                "rm is blocked by custom policy",
            )],
        );

        let evaluation = evaluate_bash_command_policy("rm -rf /tmp", Some(&config));
        assert!(matches!(
            evaluation,
            PolicyEvaluation::Blocked { policy, .. } if policy.id == "custom-block-rm"
        ));
    }

    // --- parseCommandSegments edge cases (TS parity) ---

    #[test]
    fn parse_command_segments_returns_none_for_empty_string() {
        assert_eq!(parse_command_segments(""), None);
    }

    #[test]
    fn parse_command_segments_handles_single_token() {
        assert_eq!(
            parse_command_segments("ls"),
            Some(vec![vec!["ls".to_string()]])
        );
    }

    #[test]
    fn parse_command_segments_handles_only_operators() {
        assert_eq!(parse_command_segments("| | |"), Some(vec![]));
    }

    #[test]
    fn parse_command_segments_handles_trailing_operator() {
        assert_eq!(
            parse_command_segments("ls |"),
            Some(vec![vec!["ls".to_string()]])
        );
    }

    #[test]
    fn parse_command_segments_handles_consecutive_operators() {
        let result = parse_command_segments("cat abc || || git diff");
        assert_eq!(
            result,
            Some(vec![
                vec!["cat".to_string(), "abc".to_string()],
                vec!["git".to_string(), "diff".to_string()],
            ])
        );
    }

    #[test]
    fn parse_command_segments_preserves_single_quoted_arguments() {
        assert_eq!(
            parse_command_segments("echo 'hello world' | wc -l"),
            Some(vec![
                vec!["echo".to_string(), "hello world".to_string()],
                vec!["wc".to_string(), "-l".to_string()],
            ])
        );
    }

    #[test]
    fn parse_command_segments_preserves_double_quoted_arguments() {
        assert_eq!(
            parse_command_segments("echo \"hello world\" | wc -l"),
            Some(vec![
                vec!["echo".to_string(), "hello world".to_string()],
                vec!["wc".to_string(), "-l".to_string()],
            ])
        );
    }

    #[test]
    fn parse_command_segments_returns_none_for_unclosed_quotes() {
        assert_eq!(parse_command_segments("echo 'unclosed"), None);
    }

    #[test]
    fn parse_command_segments_does_not_split_operators_inside_quotes() {
        assert_eq!(
            parse_command_segments("nix develop -c sh -c 'cd cli && cargo fmt --check'"),
            Some(vec![vec![
                "nix".to_string(),
                "develop".to_string(),
                "-c".to_string(),
                "sh".to_string(),
                "-c".to_string(),
                "cd cli && cargo fmt --check".to_string(),
            ]])
        );
    }

    // --- Shell operator policy tests (TS parity) ---

    #[test]
    fn bash_policy_blocks_or_or_operator_segment() {
        let config = config(&["forbid-git-all"], Vec::new());

        assert_eq!(
            blocked_policy_id("git status || echo fail", &config).as_deref(),
            Some("forbid-git-all")
        );
    }

    #[test]
    fn bash_policy_blocks_background_operator_segment() {
        let config = config(&["forbid-git-commit"], Vec::new());

        assert_eq!(
            blocked_policy_id("npm start & git push", &config).as_deref(),
            Some("forbid-git-commit")
        );
    }

    #[test]
    fn bash_policy_blocks_sh_c_payload_with_forbid_git_commit() {
        let config = config(&["forbid-git-commit"], Vec::new());

        let evaluation = evaluate_bash_command_policy("sh -c 'git commit -m test'", Some(&config));
        assert!(matches!(
            evaluation,
            PolicyEvaluation::Blocked { normalized_argv, policy }
                if normalized_argv == ["git", "commit", "-m", "test"] && policy.id == "forbid-git-commit"
        ));
    }

    #[test]
    fn bash_policy_reports_first_matching_segment_argv() {
        let config = config(&["forbid-git-all"], Vec::new());

        let evaluation = evaluate_bash_command_policy("git diff | cat file", Some(&config));
        assert!(matches!(
            evaluation,
            PolicyEvaluation::Blocked { normalized_argv, .. }
                if normalized_argv == ["git", "diff"]
        ));
    }

    #[test]
    fn bash_policy_allows_pipe_with_no_matching_segments() {
        let config = config(&["forbid-git-all"], Vec::new());

        assert!(matches!(
            evaluate_bash_command_policy("cat file | ls", Some(&config)),
            PolicyEvaluation::Allowed { .. }
        ));
    }

    #[test]
    fn bash_policy_blocks_double_and_segment() {
        let config = config(&["forbid-git-all"], Vec::new());

        assert_eq!(
            blocked_policy_id("git status && npm install", &config).as_deref(),
            Some("forbid-git-all")
        );
    }
}
