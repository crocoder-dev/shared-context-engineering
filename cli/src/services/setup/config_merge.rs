use anyhow::{Context, Result};
use serde_json::Value;

use crate::services::managed_telemetry::{MANAGED_OTLP_ENDPOINT, MANAGED_OTLP_PROTOCOL};

pub(crate) const CLAUDE_SCE_HOOK_MARKER: &str = "run-sce-or-show-install-guidance.sh";
const LEGACY_CLAUDE_AGENT_TRACE_PLUGIN: &str = ".claude/plugins/sce-agent-trace.ts";

const OPENCODE_SCE_PLUGIN_PREFIX: &str = "./plugins/sce-";

pub fn merge_or_create_claude_settings(
    existing_bytes: Option<&[u8]>,
    generated_bytes: &[u8],
    source_path: &str,
) -> Result<Vec<u8>> {
    let Some(existing_bytes) = existing_bytes else {
        return Ok(generated_bytes.to_vec());
    };

    let existing: Value = serde_json::from_slice(existing_bytes).with_context(|| {
        format!("Existing config file '{source_path}' must contain valid JSON.")
    })?;
    let generated: Value = serde_json::from_slice(generated_bytes)
        .context("Generated settings payload must be valid JSON")?;

    let merged = merge_claude_settings(&existing, &generated, source_path)?;

    let mut serialized = serde_json::to_string_pretty(&merged)
        .context("Failed to serialize merged Claude settings")?;
    serialized.push('\n');
    Ok(serialized.into_bytes())
}

fn merge_claude_settings(existing: &Value, generated: &Value, source_path: &str) -> Result<Value> {
    let mut existing_obj = existing.as_object().cloned().with_context(|| {
        format!("Existing config file '{source_path}' must contain a top-level JSON object.")
    })?;
    let generated_obj = generated
        .as_object()
        .context("Generated settings payload must contain a top-level JSON object")?;

    if let Some(schema) = generated_obj.get("$schema") {
        existing_obj.insert("$schema".to_string(), schema.clone());
    }

    if let Some(generated_hooks) = generated_obj.get("hooks") {
        let generated_hooks = generated_hooks
            .as_object()
            .context("Generated settings 'hooks' must be a JSON object")?;

        let mut existing_hooks = match existing_obj.get("hooks") {
            Some(value) => value.as_object().cloned().with_context(|| {
                format!("Existing config file '{source_path}' key 'hooks' must be a JSON object.")
            })?,
            None => serde_json::Map::new(),
        };

        for (event, generated_entries) in generated_hooks {
            let generated_entries = generated_entries.as_array().with_context(|| {
                format!("Generated settings 'hooks.{event}' must be a JSON array")
            })?;

            let existing_entries = match existing_hooks.get(event) {
                Some(value) => value.as_array().with_context(|| {
                    format!("Existing config file '{source_path}' key 'hooks.{event}' must be a JSON array.")
                })?,
                None => &Vec::new(),
            };

            let mut merged_entries: Vec<Value> = existing_entries
                .iter()
                .filter(|entry| !hook_entry_is_sce_owned(entry))
                .cloned()
                .collect();
            merged_entries.extend(generated_entries.iter().cloned());

            existing_hooks.insert(event.clone(), Value::Array(merged_entries));
        }

        existing_obj.insert("hooks".to_string(), Value::Object(existing_hooks));
    }

    Ok(Value::Object(existing_obj))
}

fn hook_entry_is_sce_owned(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| hooks.iter().any(hook_is_sce_owned))
}

fn hook_is_sce_owned(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.contains(CLAUDE_SCE_HOOK_MARKER))
        || hook_is_legacy_claude_agent_trace(hook)
}

fn hook_is_legacy_claude_agent_trace(hook: &Value) -> bool {
    hook.get("type").and_then(Value::as_str) == Some("command")
        && hook.get("command").and_then(Value::as_str) == Some("bun")
        && hook
            .get("args")
            .and_then(Value::as_array)
            .and_then(|args| args.first())
            .and_then(Value::as_str)
            == Some(LEGACY_CLAUDE_AGENT_TRACE_PLUGIN)
}

pub fn merge_or_create_opencode_config(
    existing_bytes: Option<&[u8]>,
    generated_bytes: &[u8],
    source_path: &str,
) -> Result<Vec<u8>> {
    let Some(existing_bytes) = existing_bytes else {
        return Ok(generated_bytes.to_vec());
    };

    let existing: Value = serde_json::from_slice(existing_bytes).with_context(|| {
        format!("Existing config file '{source_path}' must contain valid JSON.")
    })?;
    let generated: Value = serde_json::from_slice(generated_bytes)
        .context("Generated OpenCode config payload must be valid JSON")?;

    let merged = merge_opencode_config(&existing, &generated, source_path)?;

    let mut serialized = serde_json::to_string_pretty(&merged)
        .context("Failed to serialize merged OpenCode config")?;
    serialized.push('\n');
    Ok(serialized.into_bytes())
}

fn merge_opencode_config(existing: &Value, generated: &Value, source_path: &str) -> Result<Value> {
    let mut existing_obj = existing.as_object().cloned().with_context(|| {
        format!("Existing config file '{source_path}' must contain a top-level JSON object.")
    })?;
    let generated_obj = generated
        .as_object()
        .context("Generated OpenCode config payload must contain a top-level JSON object")?;

    if let Some(schema) = generated_obj.get("$schema") {
        existing_obj.insert("$schema".to_string(), schema.clone());
    }

    if let Some(generated_plugin) = generated_obj.get("plugin") {
        let generated_plugin = generated_plugin
            .as_array()
            .context("Generated OpenCode config 'plugin' must be a JSON array")?;

        let existing_plugin = match existing_obj.get("plugin") {
            Some(value) => value.as_array().cloned().with_context(|| {
                format!("Existing config file '{source_path}' key 'plugin' must be a JSON array.")
            })?,
            None => Vec::new(),
        };

        let mut merged_plugin: Vec<Value> = existing_plugin
            .into_iter()
            .filter(|entry| !plugin_entry_is_sce_owned(entry))
            .collect();
        merged_plugin.extend(generated_plugin.iter().cloned());

        existing_obj.insert("plugin".to_string(), Value::Array(merged_plugin));
    }

    Ok(Value::Object(existing_obj))
}

fn plugin_entry_is_sce_owned(entry: &Value) -> bool {
    entry
        .as_str()
        .is_some_and(|path| path.starts_with(OPENCODE_SCE_PLUGIN_PREFIX))
}

pub(crate) fn claude_settings_fragment_is_current(
    existing_bytes: &[u8],
    generated_bytes: &[u8],
) -> Result<bool> {
    let existing: Value = serde_json::from_slice(existing_bytes)
        .context("Existing Claude settings file must contain valid JSON.")?;
    let generated: Value = serde_json::from_slice(generated_bytes)
        .context("Generated settings payload must be valid JSON")?;

    let merged = merge_claude_settings(&existing, &generated, "existing")?;
    Ok(merged == existing)
}

pub(crate) fn opencode_config_fragment_is_current(
    existing_bytes: &[u8],
    generated_bytes: &[u8],
) -> Result<bool> {
    let existing: Value = serde_json::from_slice(existing_bytes)
        .context("Existing OpenCode config file must contain valid JSON.")?;
    let generated: Value = serde_json::from_slice(generated_bytes)
        .context("Generated OpenCode config payload must be valid JSON")?;

    let merged = merge_opencode_config(&existing, &generated, "existing")?;
    Ok(merged == existing)
}

const CLAUDE_OTEL_HEADERS_HELPER_COMMAND: &str = "sce auth telemetry-headers";
const CLAUDE_OTEL_HEADERS_HELPER_KEY: &str = "otelHeadersHelper";
const CLAUDE_OTEL_HEADERS_DEBOUNCE_MS: &str = "300000";

const CLAUDE_OTEL_ENDPOINT_KEYS: [&str; 4] = [
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
];
const CLAUDE_OTEL_HEADERS_KEYS: [&str; 4] = [
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
];
const CLAUDE_OTEL_PROTOCOL_KEYS: [&str; 4] = [
    "OTEL_EXPORTER_OTLP_PROTOCOL",
    "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
    "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
    "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL",
];
const CLAUDE_OTEL_EXPORTER_KEYS: [&str; 3] = [
    "OTEL_TRACES_EXPORTER",
    "OTEL_METRICS_EXPORTER",
    "OTEL_LOGS_EXPORTER",
];
const CLAUDE_TELEMETRY_SWITCH_KEYS: [&str; 3] = [
    "CLAUDE_CODE_ENABLE_TELEMETRY",
    "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA",
    CLAUDE_ENHANCED_TELEMETRY_BETA_ALIAS,
];
const CLAUDE_ENHANCED_TELEMETRY_BETA_ALIAS: &str = "ENABLE_ENHANCED_TELEMETRY_BETA";
const CLAUDE_TELEMETRY_PRIVACY_KEYS: [&str; 1] = ["OTEL_METRICS_INCLUDE_REPOSITORY"];
const CLAUDE_TELEMETRY_HELPER_SETTING_KEYS: [&str; 1] =
    ["CLAUDE_CODE_OTEL_HEADERS_HELPER_DEBOUNCE_MS"];
const CLAUDE_TELEMETRY_ENABLED_VALUE: &str = "1";
const CLAUDE_OTEL_EXPORTER_DISABLED_VALUE: &str = "none";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClaudeTelemetrySkipReason {
    UserHelperPresent,
    TelemetryDisabled,
    PrivacySettingConflict,
    ConflictingEndpoint,
    ConflictingHeaders,
    ConflictingProtocol,
    ConflictingExporter,
    ConflictingHelperSetting,
    ManagedSettingsConflict,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ClaudeTelemetryMergeOutcome {
    Installed(Vec<u8>),
    AlreadyCurrent,
    Skipped(ClaudeTelemetrySkipReason),
}

fn claude_telemetry_env_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "CLAUDE_CODE_ENABLE_TELEMETRY",
            CLAUDE_TELEMETRY_ENABLED_VALUE,
        ),
        (
            "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA",
            CLAUDE_TELEMETRY_ENABLED_VALUE,
        ),
        ("OTEL_TRACES_EXPORTER", "otlp"),
        ("OTEL_METRICS_EXPORTER", "otlp"),
        ("OTEL_LOGS_EXPORTER", "otlp"),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", MANAGED_OTLP_PROTOCOL),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", MANAGED_OTLP_ENDPOINT),
        (
            "CLAUDE_CODE_OTEL_HEADERS_HELPER_DEBOUNCE_MS",
            CLAUDE_OTEL_HEADERS_DEBOUNCE_MS,
        ),
        ("OTEL_METRICS_INCLUDE_REPOSITORY", "true"),
    ]
}

fn expected_claude_telemetry_value(key: &str) -> Option<&'static str> {
    if key == CLAUDE_ENHANCED_TELEMETRY_BETA_ALIAS {
        return Some(CLAUDE_TELEMETRY_ENABLED_VALUE);
    }
    claude_telemetry_env_entries()
        .into_iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, expected)| expected)
}

fn claude_telemetry_conflict(settings: &Value) -> Option<ClaudeTelemetrySkipReason> {
    if let Some(helper) = settings.get(CLAUDE_OTEL_HEADERS_HELPER_KEY) {
        if helper.as_str() != Some(CLAUDE_OTEL_HEADERS_HELPER_COMMAND) {
            return Some(ClaudeTelemetrySkipReason::UserHelperPresent);
        }
    }

    let env = settings.get("env").and_then(Value::as_object)?;
    let differing = |keys: &[&str]| {
        keys.iter().any(|key| {
            env.get(*key)
                .is_some_and(|value| value.as_str() != expected_claude_telemetry_value(key))
        })
    };
    let exporter_disabled = CLAUDE_OTEL_EXPORTER_KEYS.iter().any(|key| {
        env.get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim() == CLAUDE_OTEL_EXPORTER_DISABLED_VALUE)
    });

    if differing(&CLAUDE_TELEMETRY_SWITCH_KEYS) || exporter_disabled {
        Some(ClaudeTelemetrySkipReason::TelemetryDisabled)
    } else if differing(&CLAUDE_TELEMETRY_PRIVACY_KEYS) {
        Some(ClaudeTelemetrySkipReason::PrivacySettingConflict)
    } else if differing(&CLAUDE_OTEL_ENDPOINT_KEYS) {
        Some(ClaudeTelemetrySkipReason::ConflictingEndpoint)
    } else if differing(&CLAUDE_OTEL_HEADERS_KEYS) {
        Some(ClaudeTelemetrySkipReason::ConflictingHeaders)
    } else if differing(&CLAUDE_OTEL_PROTOCOL_KEYS) {
        Some(ClaudeTelemetrySkipReason::ConflictingProtocol)
    } else if differing(&CLAUDE_OTEL_EXPORTER_KEYS) {
        Some(ClaudeTelemetrySkipReason::ConflictingExporter)
    } else if differing(&CLAUDE_TELEMETRY_HELPER_SETTING_KEYS) {
        Some(ClaudeTelemetrySkipReason::ConflictingHelperSetting)
    } else {
        None
    }
}

pub(crate) fn merge_claude_telemetry_settings(
    existing_bytes: Option<&[u8]>,
    managed_bytes: Option<&[u8]>,
    source_path: &str,
) -> Result<ClaudeTelemetryMergeOutcome> {
    if let Some(managed_bytes) = managed_bytes {
        let managed: Value = serde_json::from_slice(managed_bytes)
            .context("Managed Claude settings must contain valid JSON.")?;
        if claude_telemetry_conflict(&managed).is_some() {
            return Ok(ClaudeTelemetryMergeOutcome::Skipped(
                ClaudeTelemetrySkipReason::ManagedSettingsConflict,
            ));
        }
    }

    let existing: Value = match existing_bytes {
        Some(bytes) => serde_json::from_slice(bytes).with_context(|| {
            format!("Existing config file '{source_path}' must contain valid JSON.")
        })?,
        None => Value::Object(serde_json::Map::new()),
    };
    let mut merged_obj = existing.as_object().cloned().with_context(|| {
        format!("Existing config file '{source_path}' must contain a top-level JSON object.")
    })?;

    if let Some(reason) = claude_telemetry_conflict(&existing) {
        return Ok(ClaudeTelemetryMergeOutcome::Skipped(reason));
    }

    let mut env = match merged_obj.get("env") {
        Some(value) => value.as_object().cloned().with_context(|| {
            format!("Existing config file '{source_path}' key 'env' must be a JSON object.")
        })?,
        None => serde_json::Map::new(),
    };
    for (key, value) in claude_telemetry_env_entries() {
        env.insert(key.to_string(), Value::String(value.to_string()));
    }
    merged_obj.insert("env".to_string(), Value::Object(env));
    merged_obj.insert(
        CLAUDE_OTEL_HEADERS_HELPER_KEY.to_string(),
        Value::String(CLAUDE_OTEL_HEADERS_HELPER_COMMAND.to_string()),
    );

    let merged = Value::Object(merged_obj);
    if existing_bytes.is_some() && merged == existing {
        return Ok(ClaudeTelemetryMergeOutcome::AlreadyCurrent);
    }

    let mut serialized = serde_json::to_string_pretty(&merged)
        .context("Failed to serialize merged Claude telemetry settings")?;
    serialized.push('\n');
    Ok(ClaudeTelemetryMergeOutcome::Installed(
        serialized.into_bytes(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merge_telemetry(
        existing: Option<Value>,
        managed: Option<Value>,
    ) -> ClaudeTelemetryMergeOutcome {
        let existing = existing.map(|value| serde_json::to_vec(&value).unwrap());
        let managed = managed.map(|value| serde_json::to_vec(&value).unwrap());
        merge_claude_telemetry_settings(existing.as_deref(), managed.as_deref(), "settings.json")
            .unwrap()
    }

    fn installed_value(outcome: ClaudeTelemetryMergeOutcome) -> Value {
        match outcome {
            ClaudeTelemetryMergeOutcome::Installed(bytes) => {
                serde_json::from_slice(&bytes).unwrap()
            }
            other => panic!("expected Installed, got {other:?}"),
        }
    }

    #[test]
    fn telemetry_merge_installs_env_and_helper_preserving_unrelated_keys() {
        let existing = json!({
            "permissions": {"allow": ["Bash(git *)"]},
            "env": {"FOO": "bar"},
            "hooks": {"Stop": [user_hook_entry()]}
        });

        let merged = installed_value(merge_telemetry(Some(existing), None));

        assert_eq!(merged["permissions"]["allow"][0], "Bash(git *)");
        assert_eq!(merged["env"]["FOO"], "bar");
        assert_eq!(merged["hooks"]["Stop"][0], user_hook_entry());
        assert_eq!(merged["otelHeadersHelper"], "sce auth telemetry-headers");
        assert_eq!(merged["env"]["CLAUDE_CODE_ENABLE_TELEMETRY"], "1");
        assert_eq!(
            merged["env"]["OTEL_EXPORTER_OTLP_ENDPOINT"],
            MANAGED_OTLP_ENDPOINT
        );
        assert_eq!(
            merged["env"]["OTEL_EXPORTER_OTLP_PROTOCOL"],
            MANAGED_OTLP_PROTOCOL
        );
        assert_eq!(
            merged["env"]["CLAUDE_CODE_OTEL_HEADERS_HELPER_DEBOUNCE_MS"],
            "300000"
        );
        assert_eq!(merged["env"]["OTEL_METRICS_INCLUDE_REPOSITORY"], "true");
        assert!(merged["env"].get("OTEL_EXPORTER_OTLP_HEADERS").is_none());
    }

    #[test]
    fn telemetry_merge_is_byte_idempotent_and_reports_already_current() {
        let first = match merge_telemetry(None, None) {
            ClaudeTelemetryMergeOutcome::Installed(bytes) => bytes,
            other => panic!("expected Installed, got {other:?}"),
        };

        let second = merge_claude_telemetry_settings(Some(&first), None, "settings.json").unwrap();

        assert_eq!(second, ClaudeTelemetryMergeOutcome::AlreadyCurrent);
    }

    #[test]
    fn telemetry_merge_skips_user_owned_helper() {
        let existing = json!({"otelHeadersHelper": "/usr/local/bin/my-helper"});

        assert_eq!(
            merge_telemetry(Some(existing), None),
            ClaudeTelemetryMergeOutcome::Skipped(ClaudeTelemetrySkipReason::UserHelperPresent)
        );
    }

    #[test]
    fn telemetry_merge_skips_each_conflicting_env_class_in_user_settings() {
        let cases = [
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                "https://other.example",
                ClaudeTelemetrySkipReason::ConflictingEndpoint,
            ),
            (
                "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
                "https://other.example/v1/logs",
                ClaudeTelemetrySkipReason::ConflictingEndpoint,
            ),
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                "x=y",
                ClaudeTelemetrySkipReason::ConflictingHeaders,
            ),
            (
                "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
                "x=y",
                ClaudeTelemetrySkipReason::ConflictingHeaders,
            ),
            (
                "OTEL_EXPORTER_OTLP_PROTOCOL",
                "grpc",
                ClaudeTelemetrySkipReason::ConflictingProtocol,
            ),
            (
                "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
                "grpc",
                ClaudeTelemetrySkipReason::ConflictingProtocol,
            ),
            (
                "OTEL_METRICS_EXPORTER",
                "prometheus",
                ClaudeTelemetrySkipReason::ConflictingExporter,
            ),
        ];

        for (key, value, reason) in cases {
            let existing = json!({"env": {key: value}});
            assert_eq!(
                merge_telemetry(Some(existing), None),
                ClaudeTelemetryMergeOutcome::Skipped(reason),
                "{key}"
            );
        }
    }

    #[test]
    fn telemetry_merge_enables_native_tracing_beta() {
        let merged = installed_value(merge_telemetry(None, None));

        assert_eq!(merged["env"]["CLAUDE_CODE_ENHANCED_TELEMETRY_BETA"], "1");
        assert_eq!(merged["env"]["OTEL_TRACES_EXPORTER"], "otlp");

        let without_beta = json!({
            "otelHeadersHelper": "sce auth telemetry-headers",
            "env": {"CLAUDE_CODE_ENABLE_TELEMETRY": "1", "OTEL_TRACES_EXPORTER": "otlp"}
        });
        let upgraded = installed_value(merge_telemetry(Some(without_beta), None));
        assert_eq!(upgraded["env"]["CLAUDE_CODE_ENHANCED_TELEMETRY_BETA"], "1");
    }

    #[test]
    fn telemetry_merge_preserves_explicit_opt_out_and_privacy_settings() {
        let cases = [
            (
                "CLAUDE_CODE_ENABLE_TELEMETRY",
                "0",
                ClaudeTelemetrySkipReason::TelemetryDisabled,
            ),
            (
                "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA",
                "0",
                ClaudeTelemetrySkipReason::TelemetryDisabled,
            ),
            (
                "ENABLE_ENHANCED_TELEMETRY_BETA",
                "false",
                ClaudeTelemetrySkipReason::TelemetryDisabled,
            ),
            (
                "OTEL_TRACES_EXPORTER",
                "none",
                ClaudeTelemetrySkipReason::TelemetryDisabled,
            ),
            (
                "OTEL_LOGS_EXPORTER",
                "none",
                ClaudeTelemetrySkipReason::TelemetryDisabled,
            ),
            (
                "OTEL_METRICS_INCLUDE_REPOSITORY",
                "false",
                ClaudeTelemetrySkipReason::PrivacySettingConflict,
            ),
            (
                "CLAUDE_CODE_OTEL_HEADERS_HELPER_DEBOUNCE_MS",
                "60000",
                ClaudeTelemetrySkipReason::ConflictingHelperSetting,
            ),
        ];

        for (key, value, reason) in cases {
            let user = json!({"env": {key: value}});
            assert_eq!(
                merge_telemetry(Some(user), None),
                ClaudeTelemetryMergeOutcome::Skipped(reason),
                "{key}"
            );
            let managed = json!({"env": {key: value}});
            assert_eq!(
                merge_telemetry(Some(json!({})), Some(managed)),
                ClaudeTelemetryMergeOutcome::Skipped(
                    ClaudeTelemetrySkipReason::ManagedSettingsConflict
                ),
                "managed {key}"
            );
        }
    }

    #[test]
    fn telemetry_merge_skips_when_managed_settings_conflict() {
        let managed = json!({"env": {"OTEL_EXPORTER_OTLP_ENDPOINT": "https://corp.example"}});

        assert_eq!(
            merge_telemetry(Some(json!({})), Some(managed)),
            ClaudeTelemetryMergeOutcome::Skipped(
                ClaudeTelemetrySkipReason::ManagedSettingsConflict
            )
        );
    }

    #[test]
    fn telemetry_merge_rejects_malformed_existing_json_naming_the_path() {
        let error = merge_claude_telemetry_settings(
            Some(b"{ not valid json"),
            None,
            "~/.claude/settings.json",
        )
        .unwrap_err();

        assert!(error.to_string().contains("~/.claude/settings.json"));
    }

    #[test]
    fn telemetry_merge_output_contains_no_token_material() {
        let merged = installed_value(merge_telemetry(None, None)).to_string();

        assert!(!merged.contains("Bearer"));
        assert!(!merged.contains("Authorization"));
        assert!(!merged.contains("OTEL_EXPORTER_OTLP_HEADERS"));
    }

    fn sce_hook_entry(command: &str) -> Value {
        json!({
            "hooks": [
                {"type": "command", "command": format!("bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/{}\" {}", CLAUDE_SCE_HOOK_MARKER, command)}
            ]
        })
    }

    fn legacy_sce_hook_entry(event: &str) -> Value {
        json!({
            "hooks": [{
                "type": "command",
                "command": "bun",
                "args": [LEGACY_CLAUDE_AGENT_TRACE_PLUGIN, event]
            }]
        })
    }

    fn user_hook_entry() -> Value {
        json!({
            "matcher": "Bash",
            "hooks": [
                {"type": "command", "command": "echo user-hook"}
            ]
        })
    }

    fn user_bun_hook_entry() -> Value {
        json!({
            "hooks": [{
                "type": "command",
                "command": "bun",
                "args": [".claude/plugins/my-company-hook.ts"]
            }]
        })
    }

    fn generated_settings() -> Value {
        json!({
            "$schema": "https://json.schemastore.org/claude-code-settings.json",
            "hooks": {
                "SessionStart": [sce_hook_entry("sce hooks claude-model-state")],
                "PostModelSwitch": [sce_hook_entry("sce hooks claude-model-state")],
                "PreToolUse": [sce_hook_entry("sce policy bash")],
                "PostToolUse": [
                    sce_hook_entry("sce hooks diff-trace"),
                    sce_hook_entry("sce hooks conversation-trace")
                ],
                "UserPromptSubmit": [sce_hook_entry("sce hooks conversation-trace")],
                "Stop": [sce_hook_entry("sce hooks conversation-trace")]
            }
        })
    }

    #[test]
    fn preserves_user_keys_and_non_sce_hook_entries() {
        let existing = json!({
            "permissions": {"allow": ["Bash(git *)"]},
            "env": {"FOO": "bar"},
            "hooks": {
                "PreToolUse": [user_hook_entry()]
            }
        });

        let merged =
            merge_claude_settings(&existing, &generated_settings(), "settings.json").unwrap();

        assert_eq!(merged["permissions"]["allow"][0], "Bash(git *)");
        assert_eq!(merged["env"]["FOO"], "bar");

        let pre_tool_use = merged["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool_use.len(), 2);
        assert_eq!(pre_tool_use[0], user_hook_entry());
        assert!(hook_entry_is_sce_owned(&pre_tool_use[1]));
    }

    #[test]
    fn replaces_sce_entries_instead_of_duplicating_them_across_two_merges() {
        let existing = json!({});

        let once =
            merge_claude_settings(&existing, &generated_settings(), "settings.json").unwrap();
        let twice = merge_claude_settings(&once, &generated_settings(), "settings.json").unwrap();

        assert_eq!(once, twice);
        assert_eq!(twice["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
        assert_eq!(
            twice["hooks"]["PostModelSwitch"].as_array().unwrap().len(),
            1
        );
        assert_eq!(twice["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(twice["hooks"]["PostToolUse"].as_array().unwrap().len(), 2);
        assert_eq!(
            twice["hooks"]["UserPromptSubmit"].as_array().unwrap().len(),
            1
        );
        assert_eq!(twice["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn recognizes_only_the_exact_legacy_claude_agent_trace_shape() {
        assert!(hook_entry_is_sce_owned(&legacy_sce_hook_entry(
            "SessionStart"
        )));
        assert!(!hook_entry_is_sce_owned(&user_bun_hook_entry()));
        assert!(!hook_entry_is_sce_owned(&json!({
            "hooks": [{
                "type": "command",
                "command": "bun",
                "args": [".claude/plugins/other-hook.ts", "SessionStart"]
            }]
        })));
        assert!(!hook_entry_is_sce_owned(&json!({
            "hooks": [{
                "type": "command",
                "command": "bash",
                "args": [LEGACY_CLAUDE_AGENT_TRACE_PLUGIN, "SessionStart"]
            }]
        })));
    }

    #[test]
    fn preserves_user_bun_and_command_hooks_during_merge() {
        let existing = json!({
            "hooks": {
                "SessionStart": [user_bun_hook_entry(), user_hook_entry()]
            }
        });

        let merged =
            merge_claude_settings(&existing, &generated_settings(), "settings.json").unwrap();
        let session_start = merged["hooks"]["SessionStart"].as_array().unwrap();

        assert_eq!(session_start[0], user_bun_hook_entry());
        assert_eq!(session_start[1], user_hook_entry());
    }

    #[test]
    fn replaces_historical_claude_agent_trace_hooks_and_is_idempotent() {
        let existing = json!({
            "permissions": {"allow": ["Bash(git *)"]},
            "hooks": {
                "SessionStart": [user_hook_entry(), legacy_sce_hook_entry("SessionStart")],
                "UserPromptSubmit": [legacy_sce_hook_entry("UserPromptSubmit")],
                "PostToolUse": [legacy_sce_hook_entry("PostToolUse")],
                "Stop": [legacy_sce_hook_entry("Stop")]
            }
        });

        let once =
            merge_claude_settings(&existing, &generated_settings(), "settings.json").unwrap();
        let twice = merge_claude_settings(&once, &generated_settings(), "settings.json").unwrap();

        assert_eq!(once, twice);
        assert_eq!(once["permissions"]["allow"][0], "Bash(git *)");
        for event in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"] {
            let entries = once["hooks"][event].as_array().unwrap();
            assert!(entries
                .iter()
                .all(|entry| { !entry.to_string().contains(LEGACY_CLAUDE_AGENT_TRACE_PLUGIN) }));
            assert!(
                entries.iter().any(|entry| entry == &user_hook_entry()) || event != "SessionStart"
            );
        }
        assert_eq!(
            once["hooks"]["SessionStart"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| hook_entry_is_sce_owned(entry))
                .count(),
            1
        );
        assert_eq!(
            once["hooks"]["PostModelSwitch"].as_array().unwrap().len(),
            1
        );
        assert_eq!(once["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(once["hooks"]["PostToolUse"].as_array().unwrap().len(), 2);
        assert_eq!(
            once["hooks"]["UserPromptSubmit"].as_array().unwrap().len(),
            1
        );
        assert_eq!(once["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn replaces_stale_claude_model_state_commands_without_touching_user_hooks() {
        let existing = json!({
            "hooks": {
                "SessionStart": [
                    user_hook_entry(),
                    sce_hook_entry("sce hooks session-model")
                ],
                "PostModelSwitch": [sce_hook_entry("sce hooks old-model-state")]
            }
        });

        let merged =
            merge_claude_settings(&existing, &generated_settings(), "settings.json").unwrap();

        assert_eq!(
            merged["hooks"]["SessionStart"].as_array().unwrap(),
            &[
                user_hook_entry(),
                sce_hook_entry("sce hooks claude-model-state")
            ]
        );
        assert_eq!(
            merged["hooks"]["PostModelSwitch"].as_array().unwrap(),
            &[sce_hook_entry("sce hooks claude-model-state")]
        );
    }

    #[test]
    fn drops_sce_entry_the_generated_document_no_longer_declares() {
        let existing = json!({
            "hooks": {
                "PreToolUse": [sce_hook_entry("sce policy bash"), user_hook_entry()],
                "Stop": [sce_hook_entry("stale command")]
            }
        });

        let generated = json!({
            "$schema": "https://json.schemastore.org/claude-code-settings.json",
            "hooks": {
                "PreToolUse": [sce_hook_entry("sce policy bash")],
                "Stop": []
            }
        });

        let merged = merge_claude_settings(&existing, &generated, "settings.json").unwrap();

        let stop = merged["hooks"]["Stop"].as_array().unwrap();
        assert!(stop.is_empty());
        let pre_tool_use = merged["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool_use.len(), 2);
        assert_eq!(pre_tool_use[0], user_hook_entry());
    }

    #[test]
    fn leaves_event_keys_generated_does_not_declare_untouched() {
        let existing = json!({
            "hooks": {
                "Notification": [user_hook_entry()]
            }
        });

        let merged =
            merge_claude_settings(&existing, &generated_settings(), "settings.json").unwrap();

        assert_eq!(
            merged["hooks"]["Notification"].as_array().unwrap()[0],
            user_hook_entry()
        );
    }

    #[test]
    fn missing_file_returns_generated_bytes_verbatim() {
        let generated_bytes = b"{\"$schema\":\"x\"}";
        let result =
            merge_or_create_claude_settings(None, generated_bytes, "settings.json").unwrap();
        assert_eq!(result, generated_bytes);
    }

    #[test]
    fn unparseable_existing_file_fails_naming_the_path_and_does_not_write() {
        let generated_bytes = serde_json::to_vec(&generated_settings()).unwrap();
        let error = merge_or_create_claude_settings(
            Some(b"{ not valid json"),
            &generated_bytes,
            ".claude/settings.json",
        )
        .unwrap_err();

        assert!(error.to_string().contains(".claude/settings.json"));
    }

    #[test]
    fn missing_existing_hooks_key_is_populated_from_generated() {
        let existing = json!({"permissions": {"allow": []}});

        let merged =
            merge_claude_settings(&existing, &generated_settings(), "settings.json").unwrap();

        assert_eq!(merged["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    }

    fn generated_opencode_config() -> Value {
        json!({
            "$schema": "https://opencode.ai/config.json",
            "plugin": [
                "./plugins/sce-bash-policy.ts",
                "./plugins/sce-agent-trace.ts",
                "./plugins/sce-mutation-scope.ts"
            ]
        })
    }

    #[test]
    fn opencode_merge_preserves_user_keys_and_user_plugin() {
        let existing = json!({
            "model": "anthropic/claude",
            "mcp": {"my-server": {"command": "my-server"}},
            "plugin": ["./plugins/my-plugin.ts"]
        });

        let merged =
            merge_opencode_config(&existing, &generated_opencode_config(), "opencode.json")
                .unwrap();

        assert_eq!(merged["model"], "anthropic/claude");
        assert_eq!(merged["mcp"]["my-server"]["command"], "my-server");

        let plugin = merged["plugin"].as_array().unwrap();
        assert_eq!(plugin.len(), 4);
        assert_eq!(plugin[0], "./plugins/my-plugin.ts");
        assert!(plugin.contains(&json!("./plugins/sce-bash-policy.ts")));
        assert!(plugin.contains(&json!("./plugins/sce-agent-trace.ts")));
        assert_eq!(
            plugin.last().unwrap(),
            "./plugins/sce-mutation-scope.ts",
            "the mutation-scope plugin must remain the final plugin after merging user plugins"
        );
    }

    #[test]
    fn opencode_merge_keeps_mutation_scope_last_after_arbitrary_user_plugins() {
        let existing = json!({
            "plugin": [
                "./plugins/user-a.ts",
                "./plugins/sce-bash-policy.ts",
                "./plugins/user-b.ts"
            ]
        });

        let merged =
            merge_opencode_config(&existing, &generated_opencode_config(), "opencode.json")
                .unwrap();

        let plugin = merged["plugin"].as_array().unwrap();
        assert_eq!(plugin.last().unwrap(), "./plugins/sce-mutation-scope.ts");
        let sce_positions: Vec<usize> = plugin
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry
                    .as_str()
                    .filter(|path| path.starts_with("./plugins/sce-"))
                    .map(|_| index)
            })
            .collect();
        assert_eq!(
            sce_positions,
            vec![plugin.len() - 3, plugin.len() - 2, plugin.len() - 1],
            "the three SCE plugins are appended contiguously after every user plugin"
        );
    }

    #[test]
    fn opencode_merge_is_idempotent_across_two_merges() {
        let existing = json!({});

        let once = merge_opencode_config(&existing, &generated_opencode_config(), "opencode.json")
            .unwrap();
        let twice =
            merge_opencode_config(&once, &generated_opencode_config(), "opencode.json").unwrap();

        assert_eq!(once, twice);
        assert_eq!(twice["plugin"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn opencode_merge_drops_stale_sce_shaped_plugin_path_current_catalog_no_longer_declares() {
        let existing = json!({
            "plugin": ["./plugins/sce-old-feature.ts", "./plugins/my-plugin.ts"]
        });

        let merged =
            merge_opencode_config(&existing, &generated_opencode_config(), "opencode.json")
                .unwrap();

        let plugin = merged["plugin"].as_array().unwrap();
        assert!(!plugin.contains(&json!("./plugins/sce-old-feature.ts")));
        assert!(plugin.contains(&json!("./plugins/my-plugin.ts")));
        assert!(plugin.contains(&json!("./plugins/sce-bash-policy.ts")));
        assert!(plugin.contains(&json!("./plugins/sce-agent-trace.ts")));
    }

    #[test]
    fn opencode_missing_file_returns_generated_bytes_verbatim() {
        let generated_bytes = b"{\"$schema\":\"x\"}";
        let result =
            merge_or_create_opencode_config(None, generated_bytes, "opencode.json").unwrap();
        assert_eq!(result, generated_bytes);
    }

    #[test]
    fn opencode_unparseable_existing_file_fails_naming_the_path_and_does_not_write() {
        let generated_bytes = serde_json::to_vec(&generated_opencode_config()).unwrap();
        let error = merge_or_create_opencode_config(
            Some(b"{ not valid json"),
            &generated_bytes,
            ".opencode/opencode.json",
        )
        .unwrap_err();

        assert!(error.to_string().contains(".opencode/opencode.json"));
    }

    #[test]
    fn opencode_missing_existing_plugin_key_is_populated_from_generated() {
        let existing = json!({"model": "anthropic/claude"});

        let merged =
            merge_opencode_config(&existing, &generated_opencode_config(), "opencode.json")
                .unwrap();

        assert_eq!(merged["plugin"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn claude_fragment_is_current_when_merged_settings_already_match() {
        let generated_bytes = serde_json::to_vec(&generated_settings()).unwrap();
        let installed =
            merge_or_create_claude_settings(None, &generated_bytes, "settings.json").unwrap();
        let with_user_keys =
            merge_or_create_claude_settings(Some(&installed), &generated_bytes, "settings.json")
                .unwrap();

        assert!(claude_settings_fragment_is_current(&with_user_keys, &generated_bytes).unwrap());
    }

    #[test]
    fn claude_fragment_is_not_current_when_sce_hook_entry_is_deleted() {
        let generated_bytes = serde_json::to_vec(&generated_settings()).unwrap();
        let installed_bytes =
            merge_or_create_claude_settings(None, &generated_bytes, "settings.json").unwrap();
        let mut installed: Value = serde_json::from_slice(&installed_bytes).unwrap();
        installed["permissions"] = json!({"allow": ["Bash(git *)"]});

        assert!(claude_settings_fragment_is_current(
            &serde_json::to_vec(&installed).unwrap(),
            &generated_bytes
        )
        .unwrap());

        installed["hooks"]["PreToolUse"] = json!([]);
        let drifted_bytes = serde_json::to_vec(&installed).unwrap();

        assert!(!claude_settings_fragment_is_current(&drifted_bytes, &generated_bytes).unwrap());
    }

    #[test]
    fn opencode_fragment_is_current_when_merged_plugins_already_match() {
        let generated_bytes = serde_json::to_vec(&generated_opencode_config()).unwrap();
        let installed_bytes =
            merge_or_create_opencode_config(None, &generated_bytes, "opencode.json").unwrap();
        let mut installed: Value = serde_json::from_slice(&installed_bytes).unwrap();
        installed["model"] = json!("anthropic/claude");
        installed["plugin"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!("./plugins/my-plugin.ts"));
        let existing_bytes = serde_json::to_vec(&installed).unwrap();

        assert!(opencode_config_fragment_is_current(&existing_bytes, &generated_bytes).unwrap());
    }

    #[test]
    fn opencode_fragment_is_not_current_when_sce_plugin_path_is_stale() {
        let generated_bytes = serde_json::to_vec(&generated_opencode_config()).unwrap();
        let existing = json!({
            "plugin": ["./plugins/sce-old-feature.ts"]
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();

        assert!(!opencode_config_fragment_is_current(&existing_bytes, &generated_bytes).unwrap());
    }
}
