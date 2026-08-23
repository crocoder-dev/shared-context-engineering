//! Shared structural ownership and merge logic for Codex's repository hook config.
//!
//! The accepted shape intentionally mirrors the relevant current upstream
//! `HooksFile`, `HookEventsToml`, `MatcherGroup`, and `HookHandlerConfig` JSON
//! deserialization rules. This keeps setup and doctor aligned without taking a
//! dependency on Codex's source or preserving JSON that Codex cannot load.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{Map, Value};

const CODEX_HOOKS_ROOT: &str = "hooks";
const CODEX_HELPER_PATH: &str = ".codex/hooks/run-sce-or-show-install-guidance.sh";
const CODEX_ROOTED_HELPER_PATH: &str = "$root/.codex/hooks/run-sce-or-show-install-guidance.sh";
const CODEX_COMMAND_WORDS: [&str; 3] = ["sce", "hooks", "codex"];
const REQUIRED_EVENTS: [(&str, Option<&str>); 4] = [
    ("UserPromptSubmit", None),
    ("Stop", None),
    ("PreToolUse", Some("Bash")),
    ("PostToolUse", Some("apply_patch")),
];

/// Merge the canonical generated Codex hooks into an existing file.
///
/// A missing file is installed verbatim. An existing file is parsed and
/// structurally validated before any merged bytes are returned, allowing the
/// caller to preserve it unchanged when parsing or validation fails.
pub(crate) fn merge_or_create(
    existing_bytes: Option<&[u8]>,
    generated_bytes: &[u8],
    source_path: &str,
) -> Result<Vec<u8>> {
    let Some(existing_bytes) = existing_bytes else {
        validate_generated_document(generated_bytes)?;
        return Ok(generated_bytes.to_vec());
    };

    let existing: Value = serde_json::from_slice(existing_bytes).with_context(|| {
        format!("Existing Codex hook config '{source_path}' must contain valid JSON.")
    })?;
    validate_document(&existing, source_path)?;

    let generated: Value = serde_json::from_slice(generated_bytes)
        .context("Generated Codex hook config must contain valid JSON")?;
    let registrations = validate_generated_document_value(&generated)?;
    let merged = merge_document(existing, &registrations, source_path)?;

    let mut serialized = serde_json::to_string_pretty(&merged)
        .context("Failed to serialize merged Codex hook config")?;
    serialized.push('\n');
    Ok(serialized.into_bytes())
}

/// Returns whether the existing file already contains exactly the current SCE
/// fragment. Unrelated valid Codex configuration is intentionally ignored.
pub(crate) fn fragment_is_current(existing_bytes: &[u8], generated_bytes: &[u8]) -> Result<bool> {
    let existing: Value = serde_json::from_slice(existing_bytes)
        .context("Existing Codex hook config must contain valid JSON")?;
    validate_document(&existing, "existing Codex hook config")?;
    let generated: Value = serde_json::from_slice(generated_bytes)
        .context("Generated Codex hook config must contain valid JSON")?;
    let registrations = validate_generated_document_value(&generated)?;
    Ok(merge_document(
        existing.clone(),
        &registrations,
        "existing Codex hook config",
    )? == existing)
}

#[derive(Clone)]
struct Registration {
    event: &'static str,
    matcher: Option<&'static str>,
    group: Value,
    handler: Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexHooksFile {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    hooks: CodexHookEvents,
}

#[derive(Debug, Default, Deserialize)]
struct CodexHookEvents {
    #[serde(rename = "PreToolUse", default)]
    pre_tool_use: Vec<CodexMatcherGroup>,
    #[serde(rename = "PermissionRequest", default)]
    permission_request: Vec<CodexMatcherGroup>,
    #[serde(rename = "PostToolUse", default)]
    post_tool_use: Vec<CodexMatcherGroup>,
    #[serde(rename = "PreCompact", default)]
    pre_compact: Vec<CodexMatcherGroup>,
    #[serde(rename = "PostCompact", default)]
    post_compact: Vec<CodexMatcherGroup>,
    #[serde(rename = "SessionStart", default)]
    session_start: Vec<CodexMatcherGroup>,
    #[serde(rename = "SessionEnd", default)]
    session_end: Vec<CodexMatcherGroup>,
    #[serde(rename = "UserPromptSubmit", default)]
    user_prompt_submit: Vec<CodexMatcherGroup>,
    #[serde(rename = "SubagentStart", default)]
    subagent_start: Vec<CodexMatcherGroup>,
    #[serde(rename = "SubagentStop", default)]
    subagent_stop: Vec<CodexMatcherGroup>,
    #[serde(rename = "Stop", default)]
    stop: Vec<CodexMatcherGroup>,
}

#[derive(Debug, Default, Deserialize)]
struct CodexMatcherGroup {
    #[serde(default)]
    matcher: Option<String>,
    #[serde(default)]
    hooks: Vec<Value>,
}

fn validate_generated_document(bytes: &[u8]) -> Result<()> {
    let generated: Value = serde_json::from_slice(bytes)
        .context("Generated Codex hook config must contain valid JSON")?;
    validate_generated_document_value(&generated).map(|_| ())
}

fn validate_generated_document_value(generated: &Value) -> Result<Vec<Registration>> {
    validate_document(generated, "generated Codex hook config")?;
    let object = generated
        .as_object()
        .context("Generated Codex hook config must contain a top-level JSON object")?;
    let hooks = object
        .get(CODEX_HOOKS_ROOT)
        .context("Generated Codex hook config must contain a 'hooks' object")?
        .as_object()
        .context("Generated Codex hook config key 'hooks' must be a JSON object")?;

    let mut registrations = Vec::with_capacity(REQUIRED_EVENTS.len());
    for (event, matcher) in REQUIRED_EVENTS {
        let groups = hooks
            .get(event)
            .with_context(|| format!("Generated Codex hook config is missing '{event}'"))?
            .as_array()
            .with_context(|| {
                format!("Generated Codex hook config key 'hooks.{event}' must be a JSON array")
            })?;
        if groups.len() != 1 {
            bail!(
                "Generated Codex hook config key 'hooks.{event}' must contain exactly one matcher group"
            );
        }
        let group = groups[0].as_object().with_context(|| {
            format!("Generated Codex hook config 'hooks.{event}[0]' must be a JSON object")
        })?;
        validate_matcher(group, event, matcher)?;
        let handlers = group
            .get("hooks")
            .with_context(|| {
                format!("Generated Codex hook config '{event}' group must contain 'hooks'")
            })?
            .as_array()
            .with_context(|| {
                format!("Generated Codex hook config '{event}' group 'hooks' must be a JSON array")
            })?;
        if handlers.len() != 1 {
            bail!("Generated Codex hook config '{event}' must contain exactly one handler");
        }
        validate_handler(&handlers[0], "generated Codex hook config", event, 0, 0)?;
        let handler = handlers[0].as_object().expect("validated handler object");
        let command = handler
            .get("command")
            .and_then(Value::as_str)
            .with_context(|| {
                format!(
                    "Generated Codex hook config '{event}' handler must have a string 'command'"
                )
            })?;
        if !command_is_current_sce_contract(command) {
            bail!("Generated Codex hook config '{event}' handler does not use the current SCE command contract");
        }

        registrations.push(Registration {
            event,
            matcher,
            group: Value::Object(group.clone()),
            handler: Value::Object(handler.clone()),
        });
    }

    Ok(registrations)
}

fn validate_document(document: &Value, source_path: &str) -> Result<()> {
    let typed: CodexHooksFile = serde_json::from_value(document.clone()).with_context(|| {
        format!("Existing Codex hook config '{source_path}' has an invalid Codex structure")
    })?;

    let _ = typed.description;
    let event_groups = [
        ("PreToolUse", typed.hooks.pre_tool_use),
        ("PermissionRequest", typed.hooks.permission_request),
        ("PostToolUse", typed.hooks.post_tool_use),
        ("PreCompact", typed.hooks.pre_compact),
        ("PostCompact", typed.hooks.post_compact),
        ("SessionStart", typed.hooks.session_start),
        ("SessionEnd", typed.hooks.session_end),
        ("UserPromptSubmit", typed.hooks.user_prompt_submit),
        ("SubagentStart", typed.hooks.subagent_start),
        ("SubagentStop", typed.hooks.subagent_stop),
        ("Stop", typed.hooks.stop),
    ];
    for (event, groups) in event_groups {
        for (group_index, group) in groups.iter().enumerate() {
            let _ = &group.matcher;
            for (handler_index, handler) in group.hooks.iter().enumerate() {
                validate_handler(handler, source_path, event, group_index, handler_index)?;
            }
        }
    }
    Ok(())
}

fn validate_handler(
    handler: &Value,
    source_path: &str,
    event: &str,
    group_index: usize,
    handler_index: usize,
) -> Result<()> {
    let handler = handler.as_object().with_context(|| {
        format!(
            "Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] must be a JSON object"
        )
    })?;
    let handler_type = handler
        .get("type")
        .and_then(Value::as_str)
        .with_context(|| format!("Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] must have a string 'type'"))?;

    match handler_type {
        "command" => {
            if handler.contains_key("commandWindows") && handler.contains_key("command_windows") {
                bail!("Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] cannot contain both 'commandWindows' and 'command_windows'");
            }
            required_string(handler, "command", source_path, event, group_index, handler_index)?;
            optional_string(handler, "commandWindows", source_path, event, group_index, handler_index)?;
            optional_string(handler, "command_windows", source_path, event, group_index, handler_index)?;
            optional_u64(handler, "timeout", source_path, event, group_index, handler_index)?;
            if let Some(value) = handler.get("async") {
                if !value.is_boolean() {
                    bail!("Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] field 'async' must be a boolean");
                }
            }
            optional_string(handler, "statusMessage", source_path, event, group_index, handler_index)?;
            optional_usize(
                handler,
                "additionalContextLimit",
                source_path,
                event,
                group_index,
                handler_index,
            )?;
        }
        "mcp_tool" => {
            required_string(handler, "server", source_path, event, group_index, handler_index)?;
            required_string(handler, "tool", source_path, event, group_index, handler_index)?;
            if let Some(input) = handler.get("input") {
                let input = input.as_object().with_context(|| {
                    format!("Codex hook config '{source_path}' MCP handler input must be a JSON object")
                })?;
                for (key, value) in input {
                    if !toml_compatible_json(value) {
                        bail!("Codex hook config '{source_path}' MCP handler input '{key}' is not representable as TOML");
                    }
                }
            }
            optional_u64(handler, "timeout", source_path, event, group_index, handler_index)?;
            optional_string(handler, "statusMessage", source_path, event, group_index, handler_index)?;
        }
        "prompt" | "agent" => {}
        other => bail!(
            "Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] has unsupported type '{other}'"
        ),
    }
    Ok(())
}

fn required_string(
    object: &Map<String, Value>,
    field: &str,
    source_path: &str,
    event: &str,
    group_index: usize,
    handler_index: usize,
) -> Result<()> {
    if object.get(field).and_then(Value::as_str).is_none() {
        bail!(
            "Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] field '{field}' must be a string"
        );
    }
    Ok(())
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
    source_path: &str,
    event: &str,
    group_index: usize,
    handler_index: usize,
) -> Result<()> {
    if let Some(value) = object.get(field) {
        if !value.is_null() && !value.is_string() {
            bail!(
                "Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] field '{field}' must be a string or null"
            );
        }
    }
    Ok(())
}

fn optional_u64(
    object: &Map<String, Value>,
    field: &str,
    source_path: &str,
    event: &str,
    group_index: usize,
    handler_index: usize,
) -> Result<()> {
    if let Some(value) = object.get(field) {
        if !value.is_null() && value.as_u64().is_none() {
            bail!(
                "Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] field '{field}' must be a non-negative integer or null"
            );
        }
    }
    Ok(())
}

fn optional_usize(
    object: &Map<String, Value>,
    field: &str,
    source_path: &str,
    event: &str,
    group_index: usize,
    handler_index: usize,
) -> Result<()> {
    if let Some(value) = object.get(field) {
        if !value.is_null()
            && value
                .as_u64()
                .is_none_or(|number| usize::try_from(number).is_err())
        {
            bail!(
                "Codex hook config '{source_path}' handler hooks.{event}[{group_index}].hooks[{handler_index}] field '{field}' must be a platform-sized non-negative integer or null"
            );
        }
    }
    Ok(())
}

fn toml_compatible_json(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(_) | Value::String(_) => true,
        Value::Number(number) => {
            number.as_i64().is_some()
                || number
                    .as_u64()
                    .is_some_and(|number| i64::try_from(number).is_ok())
                || (number.as_i64().is_none()
                    && number.as_u64().is_none()
                    && number.as_f64().is_some())
        }
        Value::Array(values) => {
            let Some(first) = values.first() else {
                return true;
            };
            let first_kind = toml_json_kind(first);
            values
                .iter()
                .all(|value| toml_json_kind(value) == first_kind && toml_compatible_json(value))
        }
        Value::Object(object) => object.values().all(toml_compatible_json),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TomlJsonKind {
    Bool,
    String,
    Number,
    Array,
    Object,
}

fn toml_json_kind(value: &Value) -> Option<TomlJsonKind> {
    match value {
        Value::Null => None,
        Value::Bool(_) => Some(TomlJsonKind::Bool),
        Value::String(_) => Some(TomlJsonKind::String),
        Value::Number(_) => Some(TomlJsonKind::Number),
        Value::Array(_) => Some(TomlJsonKind::Array),
        Value::Object(_) => Some(TomlJsonKind::Object),
    }
}

fn validate_matcher(group: &Map<String, Value>, event: &str, expected: Option<&str>) -> Result<()> {
    let actual = group.get("matcher").and_then(Value::as_str);
    if actual != expected {
        if expected.is_some() {
            bail!("Generated Codex hook config '{event}' group must have matcher '{expected:?}'");
        }
        bail!("Generated Codex hook config '{event}' group must not have a non-null matcher");
    }
    Ok(())
}

fn merge_document(
    mut existing: Value,
    registrations: &[Registration],
    source_path: &str,
) -> Result<Value> {
    let object = existing.as_object_mut().with_context(|| {
        format!("Existing Codex hook config '{source_path}' must contain a top-level JSON object.")
    })?;
    let mut hooks = object
        .remove(CODEX_HOOKS_ROOT)
        .map_or_else(Map::new, |value| {
            value.as_object().cloned().unwrap_or_default()
        });

    for registration in registrations {
        let existing_groups = hooks
            .remove(registration.event)
            .map_or_else(Vec::new, |value| {
                value.as_array().cloned().unwrap_or_default()
            });
        hooks.insert(
            registration.event.to_string(),
            Value::Array(merge_event_groups(
                existing_groups,
                registration.matcher,
                &registration.handler,
                &registration.group,
            )),
        );
    }

    object.insert(CODEX_HOOKS_ROOT.to_string(), Value::Object(hooks));
    Ok(existing)
}

fn merge_event_groups(
    groups: Vec<Value>,
    matcher: Option<&str>,
    current_handler: &Value,
    canonical_group: &Value,
) -> Vec<Value> {
    let mut inserted = false;
    let mut merged_groups = Vec::with_capacity(groups.len().saturating_add(1));

    for group in groups {
        let Some(group_object) = group.as_object() else {
            continue;
        };
        let matcher_matches = group_matches(group_object, matcher);
        let handlers = group_object
            .get("hooks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let has_owned = handlers.iter().any(handler_is_sce_owned);
        if !has_owned && !matcher_matches {
            merged_groups.push(group);
            continue;
        }

        let mut group = group;
        let group_object = group.as_object_mut().expect("validated group object");
        let mut handlers = group_object
            .remove("hooks")
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        let first_owned = handlers.iter().position(handler_is_sce_owned);
        handlers.retain(|handler| !handler_is_sce_owned(handler));

        if matcher_matches && !inserted {
            let insert_at = first_owned.unwrap_or(handlers.len()).min(handlers.len());
            handlers.insert(insert_at, current_handler.clone());
            inserted = true;
        }
        group_object.insert("hooks".to_string(), Value::Array(handlers));
        merged_groups.push(group);
    }

    if !inserted {
        merged_groups.push(canonical_group.clone());
    }
    merged_groups
}

fn group_matches(group: &Map<String, Value>, matcher: Option<&str>) -> bool {
    group.get("matcher").and_then(Value::as_str) == matcher
}

fn handler_is_sce_owned(handler: &Value) -> bool {
    handler
        .as_object()
        .and_then(|handler| handler.get("command"))
        .and_then(Value::as_str)
        .is_some_and(command_is_current_sce_contract)
}

fn command_is_current_sce_contract(command: &str) -> bool {
    command.split(';').any(|segment| {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let offset = usize::from(tokens.first() == Some(&"exec"));
        tokens.len() == offset + 5
            && tokens.get(offset) == Some(&"bash")
            && helper_path_token_is_valid(tokens[offset + 1])
            && tokens[offset + 2..] == CODEX_COMMAND_WORDS
    })
}

fn helper_path_token_is_valid(token: &str) -> bool {
    let token = token.trim_matches(['"', '\'']);
    token == CODEX_HELPER_PATH || token == CODEX_ROOTED_HELPER_PATH
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn generated() -> Vec<u8> {
        let mut generated = serde_json::to_string_pretty(&json!({
            "hooks": {
                "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "root=\"$(git rev-parse --show-toplevel 2>/dev/null)\" || exit 0; exec bash \"$root/.codex/hooks/run-sce-or-show-install-guidance.sh\" sce hooks codex"}]}],
                "Stop": [{"hooks": [{"type": "command", "command": "root=\"$(git rev-parse --show-toplevel 2>/dev/null)\" || exit 0; exec bash \"$root/.codex/hooks/run-sce-or-show-install-guidance.sh\" sce hooks codex"}]}],
                "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "root=\"$(git rev-parse --show-toplevel 2>/dev/null)\" || exit 0; exec bash \"$root/.codex/hooks/run-sce-or-show-install-guidance.sh\" sce hooks codex"}]}],
                "PostToolUse": [{"matcher": "apply_patch", "hooks": [{"type": "command", "command": "root=\"$(git rev-parse --show-toplevel 2>/dev/null)\" || exit 0; exec bash \"$root/.codex/hooks/run-sce-or-show-install-guidance.sh\" sce hooks codex"}]}]
            }
        }))
        .unwrap();
        generated.push('\n');
        generated.into_bytes()
    }

    #[test]
    fn accepts_upstream_defaulted_groups_and_events() {
        let existing = json!({
            "description": "user hooks",
            "hooks": {
                "Stop": [{}],
                "PreToolUse": [{"matcher": null, "hooks": []}],
                "SessionStart": [{}]
            }
        });
        merge_or_create(
            Some(&serde_json::to_vec(&existing).unwrap()),
            &generated(),
            ".codex/hooks.json",
        )
        .expect("upstream-valid defaulted groups should merge");
    }

    #[test]
    fn preserves_valid_user_handlers_and_optional_fields() {
        let existing = json!({
            "description": "user hooks",
            "hooks": {
                "PostToolUse": [{"matcher": "Write", "hooks": [
                    {"type": "command", "command": "python3 /tmp/pre.py", "commandWindows": "powershell -File C:\\\\pre.ps1", "timeout": 10, "async": true, "statusMessage": "checking", "additionalContextLimit": 4096},
                    {"type": "mcp_tool", "server": "security", "tool": "scan", "input": {"file_path": "${tool_input.file_path}", "include_ignored": false}, "timeout": 30, "statusMessage": "Scanning"},
                    {"type": "prompt"},
                    {"type": "agent"}
                ]}]
            }
        });
        let merged = merge_or_create(
            Some(&serde_json::to_vec(&existing).unwrap()),
            &generated(),
            ".codex/hooks.json",
        )
        .expect("valid handlers should survive");
        let value: Value = serde_json::from_slice(&merged).unwrap();
        assert_eq!(value["description"], "user hooks");
        assert_eq!(value["hooks"]["PostToolUse"].as_array().unwrap().len(), 2);
        assert_eq!(
            value["hooks"]["PostToolUse"][0]["hooks"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn rejects_unknown_top_level_fields() {
        let existing = json!({"custom": true});
        assert!(merge_or_create(
            Some(&serde_json::to_vec(&existing).unwrap()),
            &generated(),
            ".codex/hooks.json"
        )
        .is_err());
    }

    // Upstream `HookEventsToml`, `MatcherGroup`, and `HookHandlerConfig` variant
    // structs do not use `#[serde(deny_unknown_fields)]` (only `HooksFile` does;
    // see openai/codex codex-rs/config/src/hook_config.rs), so Codex silently
    // ignores unrecognized nested keys instead of rejecting the file. SCE must
    // accept and preserve them rather than fail the merge.
    #[test]
    fn preserves_unknown_nested_events_groups_and_handler_fields_as_codex_does() {
        let existing = json!({
            "description": "user hooks",
            "hooks": {
                "CustomEvent": [{"hooks": []}],
                "Stop": [
                    {
                        "customGroupField": "keep",
                        "hooks": [
                            {
                                "type": "command",
                                "command": "echo user",
                                "customHandlerField": "keep"
                            }
                        ]
                    }
                ]
            }
        });
        let merged = merge_or_create(
            Some(&serde_json::to_vec(&existing).unwrap()),
            &generated(),
            ".codex/hooks.json",
        )
        .expect("Codex-compatible unknown nested fields must not be rejected");
        let value: Value = serde_json::from_slice(&merged).unwrap();

        assert_eq!(
            value["hooks"]["CustomEvent"],
            json!([{"hooks": []}]),
            "unknown top-level event name must be preserved"
        );
        assert_eq!(
            value["hooks"]["Stop"][0]["customGroupField"], "keep",
            "unknown matcher group field must be preserved"
        );
        let user_handler = value["hooks"]["Stop"][0]["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|handler| handler["command"] == "echo user")
            .expect("user handler must survive merge");
        assert_eq!(user_handler["customHandlerField"], "keep");
    }

    #[test]
    fn rejects_invalid_matcher_and_handler_shapes() {
        for existing in [
            json!({"hooks": {"Stop": [{"matcher": 42} ]}}),
            json!({"hooks": {"Stop": [{"hooks": [{"nonsense": true}]}]}}),
            json!({"hooks": {"Stop": [{"hooks": [{"type": "unknown"}]}]}}),
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command"}]}]}}),
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo ok", "timeout": "fast"}]}]}}),
            json!({"hooks": {"Stop": [{"hooks": [{"type": "mcp_tool", "server": "s", "tool": "t", "input": {"x": null}}]}]}}),
        ] {
            assert!(merge_or_create(
                Some(&serde_json::to_vec(&existing).unwrap()),
                &generated(),
                ".codex/hooks.json"
            )
            .is_err());
        }
    }

    #[test]
    fn preserves_defaulted_groups_without_rewriting_optional_fields() {
        let existing = json!({"hooks": {"Stop": [{}]}});
        let merged = merge_or_create(
            Some(&serde_json::to_vec(&existing).unwrap()),
            &generated(),
            ".codex/hooks.json",
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&merged).unwrap();
        assert!(!value["hooks"]["Stop"][0]
            .as_object()
            .unwrap()
            .contains_key("matcher"));
        assert_eq!(
            value["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn preserves_unrelated_fields_groups_and_handlers() {
        let existing = json!({
            "description": "user hooks",
            "hooks": {
                "UserPromptSubmit": [{"hooks": [
                    {"type": "command", "command": "echo user"},
                    {"type": "command", "command": "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex"}
                ]}],
                "SessionStart": [{"hooks": [{"type": "command", "command": "echo session"}]}]
            }
        });
        let merged = merge_or_create(
            Some(&serde_json::to_vec(&existing).unwrap()),
            &generated(),
            ".codex/hooks.json",
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&merged).unwrap();
        assert_eq!(value["description"], "user hooks");
        assert_eq!(
            value["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "echo session"
        );
        assert_eq!(
            value["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
            "echo user"
        );
    }

    #[test]
    fn stale_and_duplicate_owned_handlers_become_one_current_handler() {
        let existing = json!({
            "hooks": {
                "Stop": [{"hooks": [
                    {"type": "command", "command": "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex"},
                    {"type": "command", "command": "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex"}
                ]}]
            }
        });
        let merged = merge_or_create(
            Some(&serde_json::to_vec(&existing).unwrap()),
            &generated(),
            "hooks.json",
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&merged).unwrap();
        let handlers = value["hooks"]["Stop"][0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 1);
        assert!(handlers[0]["command"]
            .as_str()
            .unwrap()
            .contains("$root/.codex/hooks"));
    }

    #[test]
    fn repeated_merge_is_idempotent() {
        let first = merge_or_create(None, &generated(), "hooks.json").unwrap();
        let second = merge_or_create(Some(&first), &generated(), "hooks.json").unwrap();
        assert_eq!(first, second);
        assert!(fragment_is_current(&first, &generated()).unwrap());
    }

    #[test]
    fn ownership_requires_a_bounded_helper_invocation_shape() {
        assert!(command_is_current_sce_contract(
            "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex"
        ));
        assert!(command_is_current_sce_contract(
            r#"root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0; exec bash "$root/.codex/hooks/run-sce-or-show-install-guidance.sh" sce hooks codex"#
        ));
        for command in [
            "echo .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex",
            "printf '%s' '.codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex'",
            "foo='.codex/hooks/run-sce-or-show-install-guidance.sh'; echo sce hooks codex",
            "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex && echo user",
        ] {
            assert!(
                !command_is_current_sce_contract(command),
                "claimed ownership for {command}"
            );
        }
    }

    #[test]
    fn malformed_existing_json_is_rejected_without_a_replacement() {
        let existing = br#"{\"hooks\":{\"Stop\":\"not-an-array\"}}"#;
        let error = merge_or_create(Some(existing), &generated(), ".codex/hooks.json").unwrap_err();
        assert!(error.to_string().contains(".codex/hooks.json"));
        assert_eq!(existing, br#"{\"hooks\":{\"Stop\":\"not-an-array\"}}"#);
    }
}
