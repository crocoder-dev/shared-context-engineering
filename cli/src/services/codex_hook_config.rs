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

pub(crate) const CODEX_MUTATION_SCOPE_TOOL_MATCHER: &str = "^(Bash|apply_patch)$";

const CODEX_PRE_TOOL_USE_FAIL_CLOSED_ASSIGNMENT: &str = "SCE_CODEX_PRE_TOOL_USE_FAIL_CLOSED=1";

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
    (
        CodexHookCommand::MutationScope,
        "PreToolUse",
        Some(CODEX_MUTATION_SCOPE_TOOL_MATCHER),
    ),
    (
        CodexHookCommand::MutationScope,
        "PostToolUse",
        Some(CODEX_MUTATION_SCOPE_TOOL_MATCHER),
    ),
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
        let all_tokens: Vec<&str> = segment.split_whitespace().collect();
        let tokens: &[&str] = match all_tokens.split_first() {
            Some((first, rest)) if *first == CODEX_PRE_TOOL_USE_FAIL_CLOSED_ASSIGNMENT => rest,
            _ => all_tokens.as_slice(),
        };
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
