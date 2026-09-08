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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexHookCommand {
    Codex,
    MutationScope,
}

impl CodexHookCommand {
    const ALL: [Self; 2] = [Self::Codex, Self::MutationScope];

    const fn command_words(self) -> &'static [&'static str] {
        match self {
            Self::Codex => &["sce", "hooks", "codex"],
            Self::MutationScope => &["sce", "hooks", "codex-mutation-scope"],
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Codex => "sce hooks codex",
            Self::MutationScope => "sce hooks codex-mutation-scope",
        }
    }
}

const REQUIRED_EVENTS: [(CodexHookCommand, &str, Option<&str>); 10] = [
    (CodexHookCommand::Codex, "UserPromptSubmit", None),
    (CodexHookCommand::Codex, "Stop", None),
    (CodexHookCommand::Codex, "PreToolUse", Some("Bash")),
    (CodexHookCommand::Codex, "PostToolUse", Some("apply_patch")),
    (CodexHookCommand::MutationScope, "PreToolUse", None),
    (CodexHookCommand::MutationScope, "PostToolUse", None),
    (CodexHookCommand::MutationScope, "Stop", None),
    (CodexHookCommand::MutationScope, "Interrupt", None),
    (CodexHookCommand::MutationScope, "SubagentStop", None),
    (CodexHookCommand::MutationScope, "SessionEnd", None),
];

pub(crate) fn required_registrations(
) -> [(CodexHookCommand, &'static str, Option<&'static str>); 10] {
    REQUIRED_EVENTS
}

pub(crate) fn hook_event_key_label(event: &str) -> &'static str {
    match event {
        "UserPromptSubmit" => "user_prompt_submit",
        "Stop" => "stop",
        "PreToolUse" => "pre_tool_use",
        "PostToolUse" => "post_tool_use",
        "Interrupt" => "interrupt",
        "SubagentStop" => "subagent_stop",
        "SessionEnd" => "session_end",
        other => unreachable!("unexpected Codex hook event name '{other}'"),
    }
}

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

#[derive(Clone)]
struct Registration {
    command: CodexHookCommand,
    event: &'static str,
    matcher: Option<&'static str>,
    group: Value,
    handler: Value,
}

/// The structural state of one required Codex hook registration, independent
/// of Codex's own separate hook-trust bookkeeping (see `codex_hook_trust`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationStructuralState {
    /// Exactly one SCE-owned handler exists anywhere for this event, it sits
    /// in the registration's canonical matcher group, and it matches the
    /// canonical generated handler byte-for-byte.
    PresentAndCurrent,
    /// No SCE-owned handler exists in any matcher group for this event.
    Missing,
    /// An SCE-owned handler exists somewhere for this event, but the
    /// registration is not `PresentAndCurrent`: more than one owned handler
    /// (whether duplicated within one group or spread across groups), one
    /// sitting in the wrong matcher group, or one whose content does not
    /// match the canonical generated handler.
    Stale,
}

/// One required Codex hook registration's structural diagnosis, carrying the
/// existing owned handler JSON (when present) so callers can compute Codex's
/// own trust hash for it without re-parsing the document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RegistrationDiagnosis {
    pub(crate) command: CodexHookCommand,
    pub(crate) event: &'static str,
    pub(crate) matcher: Option<&'static str>,
    pub(crate) state: RegistrationStructuralState,
    pub(crate) owned_handler: Option<Value>,
    /// Position of the matching matcher group among `hooks.<event>`, and of
    /// the owned handler within that group's `hooks` array, exactly as
    /// upstream's `hook_key` enumerates them. `None` when no owned handler
    /// was found (state is `Missing`), since there is nothing to key.
    pub(crate) position: Option<(usize, usize)>,
}

/// Whole-document diagnosis backing `sce doctor`'s Codex hook-registration
/// reporting. `Malformed` covers both unparsable JSON and JSON that fails
/// Codex's own structural schema; either way no per-registration state can be
/// determined and the document cannot be safely merged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HooksDocumentDiagnosis {
    Absent,
    Malformed(String),
    Registrations(Vec<RegistrationDiagnosis>),
}

/// Diagnose each required registration's structural state without writing
/// anything. Mirrors `merge_or_create`'s validation rules exactly so a
/// `PresentAndCurrent` result here always implies a no-op merge.
pub(crate) fn diagnose_document(
    existing_bytes: Option<&[u8]>,
    generated_bytes: &[u8],
) -> Result<HooksDocumentDiagnosis> {
    let Some(existing_bytes) = existing_bytes else {
        return Ok(HooksDocumentDiagnosis::Absent);
    };

    let existing: Value = match serde_json::from_slice(existing_bytes) {
        Ok(value) => value,
        Err(error) => {
            return Ok(HooksDocumentDiagnosis::Malformed(format!(
                "Existing Codex hook config must contain valid JSON: {error}"
            )))
        }
    };
    if let Err(error) = validate_document(&existing, "existing Codex hook config") {
        return Ok(HooksDocumentDiagnosis::Malformed(error.to_string()));
    }

    let generated: Value = serde_json::from_slice(generated_bytes)
        .context("Generated Codex hook config must contain valid JSON")?;
    let registrations = validate_generated_document_value(&generated)?;

    let hooks = existing
        .get(CODEX_HOOKS_ROOT)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let diagnoses = registrations
        .iter()
        .map(|registration| diagnose_registration(&hooks, registration))
        .collect();

    Ok(HooksDocumentDiagnosis::Registrations(diagnoses))
}

struct OwnedHandlerSighting {
    group_index: usize,
    handler_index: usize,
    handler: Value,
    in_canonical_group: bool,
}

/// Diagnose one required registration by scanning **every** matcher group
/// under `hooks.<event>`, not just the first one whose matcher matches.
/// Setup's merge (`merge_event_groups`) strips SCE-owned handlers from every
/// group for the event, so a duplicate or misplaced SCE handler sitting in a
/// second group is exactly as stale as one in the first; scoping discovery
/// to only the first matching group would let such a document read
/// `PresentAndCurrent` even though `merge_or_create` would still rewrite it.
fn diagnose_registration(
    hooks: &Map<String, Value>,
    registration: &Registration,
) -> RegistrationDiagnosis {
    let missing = || RegistrationDiagnosis {
        command: registration.command,
        event: registration.event,
        matcher: registration.matcher,
        state: RegistrationStructuralState::Missing,
        owned_handler: None,
        position: None,
    };

    let groups = hooks
        .get(registration.event)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut sightings = Vec::new();
    for (group_index, group) in groups.iter().enumerate() {
        let Some(group_object) = group.as_object() else {
            continue;
        };
        let in_canonical_group = group_matches(group_object, registration.matcher);
        let Some(handlers) = group_object.get("hooks").and_then(Value::as_array) else {
            continue;
        };
        for (handler_index, handler) in handlers.iter().enumerate() {
            if !handler_owned_by(handler, registration.command) {
                continue;
            }
            sightings.push(OwnedHandlerSighting {
                group_index,
                handler_index,
                handler: handler.clone(),
                in_canonical_group,
            });
        }
    }

    let Some((only, [])) = sightings.split_first() else {
        return match sightings.first() {
            None => missing(),
            Some(first) => RegistrationDiagnosis {
                command: registration.command,
                event: registration.event,
                matcher: registration.matcher,
                state: RegistrationStructuralState::Stale,
                owned_handler: Some(first.handler.clone()),
                position: Some((first.group_index, first.handler_index)),
            },
        };
    };

    if only.in_canonical_group && only.handler == registration.handler {
        RegistrationDiagnosis {
            command: registration.command,
            event: registration.event,
            matcher: registration.matcher,
            state: RegistrationStructuralState::PresentAndCurrent,
            owned_handler: Some(only.handler.clone()),
            position: Some((only.group_index, only.handler_index)),
        }
    } else {
        RegistrationDiagnosis {
            command: registration.command,
            event: registration.event,
            matcher: registration.matcher,
            state: RegistrationStructuralState::Stale,
            owned_handler: Some(only.handler.clone()),
            position: Some((only.group_index, only.handler_index)),
        }
    }
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
    #[serde(rename = "Interrupt", default)]
    interrupt: Vec<CodexMatcherGroup>,
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
    for (command, event, matcher) in REQUIRED_EVENTS {
        let groups = hooks
            .get(event)
            .with_context(|| {
                format!(
                    "Generated Codex hook config is missing '{event}' for '{}'",
                    command.label()
                )
            })?
            .as_array()
            .with_context(|| {
                format!("Generated Codex hook config key 'hooks.{event}' must be a JSON array")
            })?;

        let mut found: Option<Value> = None;
        for group in groups {
            let group_object = group.as_object().with_context(|| {
                format!("Generated Codex hook config 'hooks.{event}' group must be a JSON object")
            })?;
            if group_object.get("matcher").and_then(Value::as_str) != matcher {
                continue;
            }
            let handlers = group_object
                .get("hooks")
                .and_then(Value::as_array)
                .with_context(|| {
                    format!(
                        "Generated Codex hook config '{event}' group must contain a 'hooks' array"
                    )
                })?;
            for handler in handlers {
                if !handler_owned_by(handler, command) {
                    continue;
                }
                validate_handler(handler, "generated Codex hook config", event, 0, 0)?;
                if found.is_some() {
                    bail!(
                        "Generated Codex hook config '{event}' contains more than one '{}' handler",
                        command.label()
                    );
                }
                found = Some(handler.clone());
            }
        }

        let handler = found.with_context(|| {
            format!(
                "Generated Codex hook config is missing the '{}' registration for '{event}'",
                command.label()
            )
        })?;
        let canonical_group = match matcher {
            Some(matcher) => {
                serde_json::json!({ "matcher": matcher, "hooks": [handler.clone()] })
            }
            None => serde_json::json!({ "hooks": [handler.clone()] }),
        };

        registrations.push(Registration {
            command,
            event,
            matcher,
            group: canonical_group,
            handler,
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
        ("Interrupt", typed.hooks.interrupt),
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
                registration.command,
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
    command: CodexHookCommand,
    current_handler: &Value,
    canonical_group: &Value,
) -> Vec<Value> {
    let mut owned_sightings: Vec<(usize, usize)> = Vec::new();
    let mut canonical_group_sightings: Vec<(usize, usize)> = Vec::new();
    let mut first_appendable_group_index: Option<usize> = None;

    for (group_index, group) in groups.iter().enumerate() {
        let Some(group_object) = group.as_object() else {
            continue;
        };
        let matcher_matches = group_matches(group_object, matcher);
        let handlers = group_object.get("hooks").and_then(Value::as_array);
        let holds_other_command = handlers.is_some_and(|handlers| {
            handlers.iter().any(|handler| {
                handler_owning_command(handler).is_some_and(|owner| owner != command)
            })
        });
        if matcher_matches && !holds_other_command && first_appendable_group_index.is_none() {
            first_appendable_group_index = Some(group_index);
        }
        let Some(handlers) = handlers else {
            continue;
        };
        for (handler_index, handler) in handlers.iter().enumerate() {
            if !handler_owned_by(handler, command) {
                continue;
            }
            owned_sightings.push((group_index, handler_index));
            if matcher_matches {
                canonical_group_sightings.push((group_index, handler_index));
            }
        }
    }

    if let [(group_index, handler_index)] = owned_sightings.as_slice() {
        let (group_index, handler_index) = (*group_index, *handler_index);
        if canonical_group_sightings.len() == 1 {
            let existing_handler = groups
                .get(group_index)
                .and_then(|group| group.get("hooks"))
                .and_then(Value::as_array)
                .and_then(|handlers| handlers.get(handler_index));
            if existing_handler == Some(current_handler) {
                return groups;
            }
        }
    }

    let target_group_index = canonical_group_sightings
        .first()
        .map(|(group_index, _)| *group_index)
        .or(first_appendable_group_index);

    let mut merged_groups = groups;
    let mut insert_at_in_target: Option<usize> = None;

    for (group_index, group) in merged_groups.iter_mut().enumerate() {
        let Some(group_object) = group.as_object_mut() else {
            continue;
        };
        let Some(handlers) = group_object.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        if target_group_index == Some(group_index) {
            insert_at_in_target = handlers
                .iter()
                .position(|handler| handler_owned_by(handler, command));
        }
        handlers.retain(|handler| !handler_owned_by(handler, command));
    }

    match target_group_index {
        Some(group_index) => {
            let group_object = merged_groups[group_index]
                .as_object_mut()
                .expect("validated group object");
            let handlers = group_object
                .entry("hooks".to_string())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("validated group's hooks field is a JSON array");
            let insert_at = insert_at_in_target
                .unwrap_or(handlers.len())
                .min(handlers.len());
            handlers.insert(insert_at, current_handler.clone());
        }
        None => {
            merged_groups.push(canonical_group.clone());
        }
    }

    merged_groups
}

fn group_matches(group: &Map<String, Value>, matcher: Option<&str>) -> bool {
    group.get("matcher").and_then(Value::as_str) == matcher
}

fn handler_owned_by(handler: &Value, command: CodexHookCommand) -> bool {
    handler_owning_command(handler) == Some(command)
}

fn handler_owning_command(handler: &Value) -> Option<CodexHookCommand> {
    let command = handler
        .as_object()
        .and_then(|handler| handler.get("command"))
        .and_then(Value::as_str)?;
    command_owning_contract(command)
}

fn command_owning_contract(command: &str) -> Option<CodexHookCommand> {
    command.split(';').find_map(|segment| {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let offset = usize::from(tokens.first() == Some(&"exec"));
        if tokens.len() != offset + 5 || tokens.get(offset) != Some(&"bash") {
            return None;
        }
        if !helper_path_token_is_valid(tokens[offset + 1]) {
            return None;
        }
        let words = &tokens[offset + 2..];
        CodexHookCommand::ALL
            .into_iter()
            .find(|contract| contract.command_words() == words)
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

    const CANONICAL_COMMAND: &str = "root=\"$(git rev-parse --show-toplevel 2>/dev/null)\" || exit 0; exec bash \"$root/.codex/hooks/run-sce-or-show-install-guidance.sh\" sce hooks codex";
    const MUTATION_SCOPE_COMMAND: &str = "root=\"$(git rev-parse --show-toplevel 2>/dev/null)\" || exit 0; exec bash \"$root/.codex/hooks/run-sce-or-show-install-guidance.sh\" sce hooks codex-mutation-scope";

    fn codex_handler() -> Value {
        json!({"type": "command", "command": CANONICAL_COMMAND})
    }

    fn mutation_scope_handler() -> Value {
        json!({"type": "command", "command": MUTATION_SCOPE_COMMAND})
    }

    fn canonical_document() -> Value {
        json!({
            "hooks": {
                "UserPromptSubmit": [{"hooks": [codex_handler()]}],
                "Stop": [{"hooks": [codex_handler()]}, {"hooks": [mutation_scope_handler()]}],
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [codex_handler()]},
                    {"hooks": [mutation_scope_handler()]}
                ],
                "PostToolUse": [
                    {"matcher": "apply_patch", "hooks": [codex_handler()]},
                    {"hooks": [mutation_scope_handler()]}
                ],
                "Interrupt": [{"hooks": [mutation_scope_handler()]}],
                "SubagentStop": [{"hooks": [mutation_scope_handler()]}],
                "SessionEnd": [{"hooks": [mutation_scope_handler()]}]
            }
        })
    }

    fn generated() -> Vec<u8> {
        let mut generated = serde_json::to_string_pretty(&canonical_document()).unwrap();
        generated.push('\n');
        generated.into_bytes()
    }

    fn with_mutation_scope_registrations(mut doc: Value) -> Value {
        let hooks = doc["hooks"].as_object_mut().expect("hooks object");
        for event in ["PreToolUse", "PostToolUse", "Stop"] {
            hooks
                .entry(event.to_string())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("event array")
                .push(json!({"hooks": [mutation_scope_handler()]}));
        }
        for event in ["Interrupt", "SubagentStop", "SessionEnd"] {
            hooks.insert(
                event.to_string(),
                json!([{"hooks": [mutation_scope_handler()]}]),
            );
        }
        doc
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
        let post_tool_use = value["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post_tool_use.len(), 3);
        assert_eq!(post_tool_use[0]["matcher"], "Write");
        assert_eq!(post_tool_use[0]["hooks"].as_array().unwrap().len(), 4);
        assert_eq!(post_tool_use[1]["matcher"], "apply_patch");
        assert_eq!(post_tool_use[1]["hooks"][0]["command"], CANONICAL_COMMAND);
        assert!(!post_tool_use[2]
            .as_object()
            .unwrap()
            .contains_key("matcher"));
        assert_eq!(
            post_tool_use[2]["hooks"][0]["command"],
            MUTATION_SCOPE_COMMAND
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
        let document_diagnosis = diagnose_document(Some(&first), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        assert!(diagnoses
            .iter()
            .all(|diagnosis| diagnosis.state == RegistrationStructuralState::PresentAndCurrent));
    }

    #[test]
    fn ownership_requires_a_bounded_helper_invocation_shape() {
        assert_eq!(
            command_owning_contract(
                "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex"
            ),
            Some(CodexHookCommand::Codex)
        );
        assert_eq!(
            command_owning_contract(
                r#"root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0; exec bash "$root/.codex/hooks/run-sce-or-show-install-guidance.sh" sce hooks codex"#
            ),
            Some(CodexHookCommand::Codex)
        );
        assert_eq!(
            command_owning_contract(
                r#"root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0; exec bash "$root/.codex/hooks/run-sce-or-show-install-guidance.sh" sce hooks codex-mutation-scope"#
            ),
            Some(CodexHookCommand::MutationScope)
        );
        for command in [
            "echo .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex",
            "printf '%s' '.codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex'",
            "foo='.codex/hooks/run-sce-or-show-install-guidance.sh'; echo sce hooks codex",
            "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex && echo user",
            "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex-other",
        ] {
            assert!(
                command_owning_contract(command).is_none(),
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

    fn registration<'a>(
        diagnoses: &'a [RegistrationDiagnosis],
        event: &str,
    ) -> &'a RegistrationDiagnosis {
        diagnoses
            .iter()
            .find(|diagnosis| diagnosis.event == event)
            .unwrap_or_else(|| panic!("no diagnosis for event '{event}'"))
    }

    fn registration_of<'a>(
        diagnoses: &'a [RegistrationDiagnosis],
        command: CodexHookCommand,
        event: &str,
    ) -> &'a RegistrationDiagnosis {
        diagnoses
            .iter()
            .find(|diagnosis| diagnosis.command == command && diagnosis.event == event)
            .unwrap_or_else(|| panic!("no {} diagnosis for event '{event}'", command.label()))
    }

    #[test]
    fn diagnose_document_reports_absent_for_a_missing_file() {
        assert_eq!(
            diagnose_document(None, &generated()).unwrap(),
            HooksDocumentDiagnosis::Absent
        );
    }

    #[test]
    fn diagnose_document_reports_malformed_for_invalid_json() {
        let document_diagnosis = diagnose_document(
            Some(br#"{\"hooks\":{\"Stop\":\"not-an-array\"}}"#),
            &generated(),
        )
        .unwrap();
        assert!(matches!(
            document_diagnosis,
            HooksDocumentDiagnosis::Malformed(_)
        ));
    }

    #[test]
    fn diagnose_document_reports_malformed_for_codex_invalid_structure() {
        let existing = serde_json::to_vec(&json!({"custom": true})).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing), &generated()).unwrap();
        assert!(matches!(
            document_diagnosis,
            HooksDocumentDiagnosis::Malformed(_)
        ));
    }

    #[test]
    fn diagnose_document_reports_missing_registrations_for_an_empty_valid_document() {
        let existing = serde_json::to_vec(&json!({})).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected a validated document with per-registration diagnoses");
        };
        assert_eq!(diagnoses.len(), REQUIRED_EVENTS.len());
        for diagnosis in &diagnoses {
            assert_eq!(diagnosis.state, RegistrationStructuralState::Missing);
            assert!(diagnosis.owned_handler.is_none());
            assert!(diagnosis.position.is_none());
        }
        assert!(diagnoses.iter().any(|diagnosis| diagnosis.command
            == CodexHookCommand::MutationScope
            && diagnosis.event == "SessionEnd"));
    }

    #[test]
    fn diagnose_document_reports_present_and_current_after_a_fresh_merge() {
        let installed = merge_or_create(None, &generated(), "hooks.json").unwrap();
        let document_diagnosis = diagnose_document(Some(&installed), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        for diagnosis in &diagnoses {
            assert_eq!(
                diagnosis.state,
                RegistrationStructuralState::PresentAndCurrent
            );
            assert!(diagnosis.owned_handler.is_some());
        }
        assert_eq!(
            registration_of(&diagnoses, CodexHookCommand::Codex, "Stop").position,
            Some((0, 0))
        );
        assert_eq!(
            registration_of(&diagnoses, CodexHookCommand::MutationScope, "Stop").position,
            Some((1, 0))
        );
        assert_eq!(
            registration_of(&diagnoses, CodexHookCommand::MutationScope, "PreToolUse").position,
            Some((1, 0))
        );
        assert_eq!(
            registration_of(&diagnoses, CodexHookCommand::MutationScope, "Interrupt").position,
            Some((0, 0))
        );
    }

    #[test]
    fn diagnose_document_reports_stale_for_a_legacy_owned_handler() {
        let existing = json!({
            "hooks": {
                "Stop": [{"hooks": [
                    {"type": "command", "command": "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex", "timeout": 30}
                ]}]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        let stop = registration(&diagnoses, "Stop");
        assert_eq!(stop.state, RegistrationStructuralState::Stale);
        assert_eq!(stop.position, Some((0, 0)));
        assert_eq!(
            registration(&diagnoses, "UserPromptSubmit").state,
            RegistrationStructuralState::Missing
        );
    }

    #[test]
    fn diagnose_document_reports_stale_for_duplicate_owned_handlers() {
        let owned_command = "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex";
        let existing = json!({
            "hooks": {
                "Stop": [{"hooks": [
                    {"type": "command", "command": owned_command},
                    {"type": "command", "command": owned_command}
                ]}]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        assert_eq!(
            registration(&diagnoses, "Stop").state,
            RegistrationStructuralState::Stale
        );
    }

    #[test]
    fn diagnose_document_treats_an_owned_handler_in_the_wrong_matcher_group_as_stale() {
        // Codex still discovers this handler (it just never dispatches for a
        // Bash PreToolUse call, since the matcher does not match); doctor
        // must not report "nothing is here" when something structurally
        // wrong is actually present.
        let owned_command = "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex";
        let existing = json!({
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Write", "hooks": [{"type": "command", "command": owned_command}]}
                ]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        assert_eq!(
            registration(&diagnoses, "PreToolUse").state,
            RegistrationStructuralState::Stale
        );
    }

    #[test]
    fn diagnose_document_reports_stale_for_a_canonical_handler_duplicated_in_a_second_matcher_group(
    ) {
        let existing = json!({
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": CANONICAL_COMMAND}]},
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": CANONICAL_COMMAND}]}
                ]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        assert_eq!(
            registration(&diagnoses, "PreToolUse").state,
            RegistrationStructuralState::Stale,
            "an owned handler duplicated across two matcher groups must not read PresentAndCurrent, \
             since merge_or_create would still collapse it to one handler"
        );
    }

    #[test]
    fn diagnose_document_finds_the_canonical_handler_in_a_non_first_matcher_group() {
        let existing = json!({
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo user only"}]},
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": CANONICAL_COMMAND}]}
                ]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        let pre_tool_use = registration(&diagnoses, "PreToolUse");
        assert_eq!(
            pre_tool_use.state,
            RegistrationStructuralState::PresentAndCurrent
        );
        assert_eq!(
            pre_tool_use.position,
            Some((1, 0)),
            "position must name the second group, not wrongly default to the first"
        );
    }

    #[test]
    fn diagnose_document_reports_stale_for_a_canonical_handler_plus_a_wrong_matcher_duplicate() {
        let owned_command = "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex";
        let existing = json!({
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": CANONICAL_COMMAND}]},
                    {"matcher": "Write", "hooks": [{"type": "command", "command": owned_command}]}
                ]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        assert_eq!(
            registration(&diagnoses, "PreToolUse").state,
            RegistrationStructuralState::Stale,
            "a canonical placement plus any other owned handler anywhere must still read Stale"
        );
    }

    #[test]
    fn diagnose_document_reports_missing_when_every_group_holds_only_non_owned_handlers() {
        let existing = json!({
            "hooks": {
                "Stop": [
                    {"hooks": [{"type": "command", "command": "echo one"}]},
                    {"hooks": [{"type": "command", "command": "echo two"}]}
                ]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        assert_eq!(
            registration(&diagnoses, "Stop").state,
            RegistrationStructuralState::Missing
        );
    }

    #[test]
    fn diagnose_document_finds_the_canonical_handler_mixed_with_arbitrary_user_handlers() {
        let existing = json!({
            "hooks": {
                "Stop": [{"hooks": [
                    {"type": "command", "command": "echo user one"},
                    {"type": "command", "command": CANONICAL_COMMAND},
                    {"type": "command", "command": "echo user two"}
                ]}]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let document_diagnosis = diagnose_document(Some(&existing_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        let stop = registration(&diagnoses, "Stop");
        assert_eq!(stop.state, RegistrationStructuralState::PresentAndCurrent);
        assert_eq!(stop.position, Some((0, 1)));
    }

    #[test]
    fn present_and_current_implies_merge_or_create_is_a_semantic_no_op() {
        let canonical = merge_or_create(None, &generated(), "hooks.json").unwrap();
        let document_diagnosis = diagnose_document(Some(&canonical), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        assert!(diagnoses
            .iter()
            .all(|diagnosis| diagnosis.state == RegistrationStructuralState::PresentAndCurrent));

        let merged_again = merge_or_create(Some(&canonical), &generated(), "hooks.json").unwrap();
        let canonical_value: Value = serde_json::from_slice(&canonical).unwrap();
        let merged_again_value: Value = serde_json::from_slice(&merged_again).unwrap();
        assert_eq!(
            canonical_value, merged_again_value,
            "PresentAndCurrent for every registration must imply merge_or_create is a no-op"
        );
    }

    /// A matrix proving `merge_or_create` never rewrites a document every
    /// registration is diagnosed `PresentAndCurrent` for, including the
    /// specific relocation bug: a canonical handler already sitting in a
    /// *non-first* matcher group must stay exactly where it is, not be
    /// moved into an earlier matcher group merely because one exists.
    #[test]
    fn merge_or_create_is_a_no_op_for_every_present_and_current_placement() {
        let user_prompt_submit =
            json!({"hooks": [{"type": "command", "command": CANONICAL_COMMAND}]});
        let stop = json!({"hooks": [{"type": "command", "command": CANONICAL_COMMAND}]});
        let post_tool_use = json!({"matcher": "apply_patch", "hooks": [{"type": "command", "command": CANONICAL_COMMAND}]});

        let cases: Vec<(&str, Value)> = vec![
            (
                "canonical handler in the only (first) matching group",
                json!({
                    "hooks": {
                        "UserPromptSubmit": [user_prompt_submit.clone()],
                        "Stop": [stop.clone()],
                        "PreToolUse": [
                            {"matcher": "Bash", "hooks": [{"type": "command", "command": CANONICAL_COMMAND}]}
                        ],
                        "PostToolUse": [post_tool_use.clone()]
                    }
                }),
            ),
            (
                "canonical handler in a second matching group, behind a user-only first group",
                json!({
                    "hooks": {
                        "UserPromptSubmit": [user_prompt_submit.clone()],
                        "Stop": [stop.clone()],
                        "PreToolUse": [
                            {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo user only"}]},
                            {"matcher": "Bash", "hooks": [{"type": "command", "command": CANONICAL_COMMAND}]}
                        ],
                        "PostToolUse": [post_tool_use.clone()]
                    }
                }),
            ),
            (
                "canonical handler mixed with arbitrary user handlers in the same group",
                json!({
                    "hooks": {
                        "UserPromptSubmit": [user_prompt_submit.clone()],
                        "Stop": [stop.clone()],
                        "PreToolUse": [{
                            "matcher": "Bash",
                            "hooks": [
                                {"type": "command", "command": "echo user one"},
                                {"type": "command", "command": CANONICAL_COMMAND},
                                {"type": "command", "command": "echo user two"}
                            ]
                        }],
                        "PostToolUse": [post_tool_use.clone()]
                    }
                }),
            ),
        ];

        for (label, existing) in cases {
            let existing = with_mutation_scope_registrations(existing);
            let existing_bytes = serde_json::to_vec(&existing).unwrap();
            let document_diagnosis =
                diagnose_document(Some(&existing_bytes), &generated()).unwrap();
            let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
                panic!("case '{label}': expected per-registration diagnoses");
            };
            assert!(
                diagnoses
                    .iter()
                    .all(|diagnosis| diagnosis.state
                        == RegistrationStructuralState::PresentAndCurrent),
                "case '{label}': every registration must diagnose PresentAndCurrent, got {diagnoses:?}"
            );

            let merged_bytes =
                merge_or_create(Some(&existing_bytes), &generated(), "hooks.json").unwrap();
            let existing_value: Value = serde_json::from_slice(&existing_bytes).unwrap();
            let merged_value: Value = serde_json::from_slice(&merged_bytes).unwrap();
            assert_eq!(
                existing_value, merged_value,
                "case '{label}': merge_or_create must be a semantic no-op when every \
                 registration is already PresentAndCurrent"
            );
        }
    }

    #[test]
    fn merge_relocates_a_wrong_matcher_owned_handler_into_the_correct_matcher_group() {
        let owned_command = "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex";
        let existing = json!({
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo user only"}]},
                    {"matcher": "Write", "hooks": [{"type": "command", "command": owned_command}]}
                ]
            }
        });
        let existing_bytes = serde_json::to_vec(&existing).unwrap();
        let merged_bytes =
            merge_or_create(Some(&existing_bytes), &generated(), "hooks.json").unwrap();
        let merged: Value = serde_json::from_slice(&merged_bytes).unwrap();

        let bash_group = &merged["hooks"]["PreToolUse"][0];
        assert_eq!(bash_group["matcher"], "Bash");
        assert_eq!(bash_group["hooks"][0]["command"], "echo user only");
        assert_eq!(bash_group["hooks"][1]["command"], CANONICAL_COMMAND);

        let write_group = &merged["hooks"]["PreToolUse"][1];
        assert_eq!(write_group["matcher"], "Write");
        assert_eq!(
            write_group["hooks"].as_array().unwrap().len(),
            0,
            "the misplaced handler must be removed from the Write group, not left duplicated"
        );

        let document_diagnosis = diagnose_document(Some(&merged_bytes), &generated()).unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) = document_diagnosis else {
            panic!("expected per-registration diagnoses");
        };
        assert_eq!(
            registration(&diagnoses, "PreToolUse").state,
            RegistrationStructuralState::PresentAndCurrent
        );
    }

    fn canonical_four_document() -> Value {
        json!({
            "hooks": {
                "UserPromptSubmit": [{"hooks": [codex_handler()]}],
                "Stop": [{"hooks": [codex_handler()]}],
                "PreToolUse": [{"matcher": "Bash", "hooks": [codex_handler()]}],
                "PostToolUse": [{"matcher": "apply_patch", "hooks": [codex_handler()]}]
            }
        })
    }

    #[test]
    fn upgrading_the_canonical_four_document_preserves_existing_trust_identity() {
        let mut existing = canonical_four_document();
        existing["hooks"]["Stop"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"type": "command", "command": "echo user stop"}));
        existing["hooks"]["SessionEnd"] =
            json!([{"hooks": [{"type": "command", "command": "echo user session end"}]}]);
        let existing_bytes = serde_json::to_vec(&existing).unwrap();

        let merged =
            merge_or_create(Some(&existing_bytes), &generated(), ".codex/hooks.json").unwrap();
        let value: Value = serde_json::from_slice(&merged).unwrap();

        let HooksDocumentDiagnosis::Registrations(diagnoses) =
            diagnose_document(Some(&merged), &generated()).unwrap()
        else {
            panic!("expected per-registration diagnoses");
        };

        for (event, expected_position) in [
            ("UserPromptSubmit", (0usize, 0usize)),
            ("Stop", (0, 1)),
            ("PreToolUse", (0, 0)),
            ("PostToolUse", (0, 0)),
        ] {
            let registration = registration_of(&diagnoses, CodexHookCommand::Codex, event);
            assert_eq!(
                registration.state,
                RegistrationStructuralState::PresentAndCurrent,
                "{event} must stay present and current"
            );
            assert_eq!(
                registration.position,
                Some(expected_position),
                "{event} identity tuple (group/handler index) must not move"
            );
            assert_eq!(
                registration.owned_handler.as_ref().unwrap(),
                &codex_handler()
            );
        }

        for event in [
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "Interrupt",
            "SubagentStop",
            "SessionEnd",
        ] {
            let registration = registration_of(&diagnoses, CodexHookCommand::MutationScope, event);
            assert_eq!(
                registration.state,
                RegistrationStructuralState::PresentAndCurrent,
                "mutation-scope {event} must be added"
            );
        }

        let mutation_scope_count = value["hooks"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|groups| groups.as_array().unwrap())
            .flat_map(|group| group["hooks"].as_array().unwrap())
            .filter(|handler| handler["command"] == MUTATION_SCOPE_COMMAND)
            .count();
        assert_eq!(
            mutation_scope_count, 6,
            "each mutation-scope handler must appear exactly once"
        );

        assert_eq!(
            value["hooks"]["Stop"][0]["hooks"][0]["command"],
            "echo user stop"
        );
        assert!(value["hooks"]["SessionEnd"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["hooks"].as_array().unwrap())
            .any(|handler| handler["command"] == "echo user session end"));

        let merged_again =
            merge_or_create(Some(&merged), &generated(), ".codex/hooks.json").unwrap();
        assert_eq!(
            merged, merged_again,
            "a second merge over the upgraded document must be byte-identical"
        );
    }

    #[test]
    fn merge_appends_the_mutation_scope_pre_tool_use_group_without_touching_the_bash_group() {
        let existing = serde_json::to_vec(&canonical_four_document()).unwrap();
        let merged = merge_or_create(Some(&existing), &generated(), ".codex/hooks.json").unwrap();
        let value: Value = serde_json::from_slice(&merged).unwrap();

        let pre_tool_use = value["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool_use.len(), 2);
        assert_eq!(pre_tool_use[0]["matcher"], "Bash");
        assert_eq!(pre_tool_use[0]["hooks"][0]["command"], CANONICAL_COMMAND);
        assert!(!pre_tool_use[1].as_object().unwrap().contains_key("matcher"));
        assert_eq!(
            pre_tool_use[1]["hooks"][0]["command"],
            MUTATION_SCOPE_COMMAND
        );
    }

    #[test]
    fn the_two_unmatched_stop_groups_diagnose_independently_by_command() {
        let installed = merge_or_create(None, &generated(), ".codex/hooks.json").unwrap();
        let HooksDocumentDiagnosis::Registrations(diagnoses) =
            diagnose_document(Some(&installed), &generated()).unwrap()
        else {
            panic!("expected per-registration diagnoses");
        };
        let codex_stop = registration_of(&diagnoses, CodexHookCommand::Codex, "Stop");
        let mutation_stop = registration_of(&diagnoses, CodexHookCommand::MutationScope, "Stop");
        assert_eq!(
            codex_stop.state,
            RegistrationStructuralState::PresentAndCurrent
        );
        assert_eq!(
            mutation_stop.state,
            RegistrationStructuralState::PresentAndCurrent
        );
        assert_eq!(codex_stop.position, Some((0, 0)));
        assert_eq!(mutation_stop.position, Some((1, 0)));
    }

    #[test]
    fn interrupt_event_key_label_matches_upstream() {
        assert_eq!(hook_event_key_label("Interrupt"), "interrupt");
    }

    #[test]
    fn subagent_stop_event_key_label_matches_upstream() {
        assert_eq!(hook_event_key_label("SubagentStop"), "subagent_stop");
    }

    #[test]
    fn session_end_event_key_label_matches_upstream() {
        assert_eq!(hook_event_key_label("SessionEnd"), "session_end");
    }

    #[test]
    fn every_required_event_has_an_upstream_key_label() {
        for (_, event, _) in REQUIRED_EVENTS {
            let _ = hook_event_key_label(event);
        }
    }
}
