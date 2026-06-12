use std::path::Path;

use crate::services::config::policy::{
    runtime_bash_policy_presets, BashPolicyConfig, CustomBashPolicyEntry, RuntimeBashPolicyPreset,
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
