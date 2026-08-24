//! Read-only diagnosis of Codex's own per-handler hook-*trust* bookkeeping
//! (enabled / `trusted_hash`) for SCE-owned `.codex/hooks.json`
//! registrations.
//!
//! This is deliberately only one of two independent dimensions Codex
//! requires before it will actually execute a project hook handler. This
//! module answers "given an eligible hook *source*, has this handler been
//! enabled and durably trusted?" — it says nothing about whether Codex
//! considers the *source* (SCE's project `.codex/hooks.json`) eligible at
//! all. That second dimension is effective hook-discovery *policy*
//! (`allow_managed_hooks_only`), owned entirely by `codex_hook_policy`. A
//! project registration is only executable when both are satisfied:
//! structurally current, policy-eligible, *and* trusted. See
//! `codex_hook_policy`'s module documentation for why policy cannot be
//! determined by reading any file this module could read, and for the
//! upstream discovery-policy source references.
//!
//! Mirrors current upstream `openai/codex` (commit
//! `8e649e3afa5cdddfb09a1b85a090b94775045d9b`):
//! `hooks/src/engine/discovery.rs` (`hook_hash`, `hook_trust_status`,
//! `hook_enabled`, `hook_trusted_hash`, `NormalizedHookIdentity`),
//! `config/src/fingerprint.rs` (`version_for_toml`), and `hooks/src/lib.rs`
//! (`hook_key`, `hook_event_key_label`). SCE never writes this state; see
//! `codex_hook_config` for the SCE-owned merge/diagnosis boundary this module
//! deliberately stays out of (no auto-trust, no state.toml writes).
//!
//! Scope limitation: Codex's effective hook state is layered from its user
//! config (`$CODEX_HOME/config.toml`) and ephemeral, process-local session
//! flags (`hooks/src/config_rules.rs` `hook_states_from_stack`). Doctor is a
//! static, out-of-process inspection, so it can only ever read the durable
//! user-config layer; a live Codex session with session-flag overrides can
//! diverge from what doctor reports here.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Default output-context token budget Codex applies when
/// `additionalContextLimit` is unset (`hooks/src/output_spill.rs`
/// `DEFAULT_HOOK_OUTPUT_TOKEN_LIMIT`). An explicit value equal to this
/// default is normalized away before hashing, exactly as upstream does.
const DEFAULT_HOOK_OUTPUT_TOKEN_LIMIT: u64 = 2_500;

/// Events whose hooks may carry `additionalContext`
/// (`hooks/src/engine/discovery.rs`); `Stop` cannot, so an
/// `additionalContextLimit` set on a Stop handler is dropped before hashing,
/// matching upstream's own normalization.
const EVENTS_SUPPORTING_ADDITIONAL_CONTEXT: [&str; 4] = [
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "SessionStart",
];

/// Effective trust readiness for one Codex hook registration's current
/// on-disk handler. `Managed` never applies here: SCE only ever registers
/// project-owned (non-managed) handlers in `.codex/hooks.json`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TrustReadiness {
    /// Enabled and `trusted_hash` matches the handler's current hash: this
    /// handler is durably trusted. This alone does **not** mean Codex will
    /// execute it — `Trusted` says nothing about whether Codex's
    /// hook-discovery policy considers this project handler's *source*
    /// eligible at all (see `codex_hook_policy`). The accurate reading is
    /// "the current non-managed handler is enabled and durably trusted,
    /// assuming Codex policy permits this hook source."
    Trusted,
    /// Enabled but no `trusted_hash` is recorded for this handler yet.
    Untrusted,
    /// Enabled but the recorded `trusted_hash` does not match the handler's
    /// current hash (the handler content changed since it was trusted).
    Modified,
    /// The user's Codex config explicitly disabled this handler
    /// (`hooks.state."<key>".enabled = false`).
    Disabled,
    /// Trust state could not be determined; carries a human-readable reason.
    Unknown(String),
}

/// Where doctor reads Codex's durable, user-scoped hook-trust state from.
/// Injectable so tests never touch the real `$CODEX_HOME`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustContext {
    pub(crate) codex_home: Option<PathBuf>,
}

/// Resolve `$CODEX_HOME`, falling back to `~/.codex` (Codex's own default;
/// see `openai/codex` `config/src/loader/local.rs`).
pub(crate) fn default_trust_context() -> TrustContext {
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")));
    TrustContext { codex_home }
}

/// Diagnose whether Codex will actually execute the given SCE-owned handler
/// for one required registration. `hooks_json_path` must be the on-disk path
/// to the `.codex/hooks.json` file the handler was read from, and `position`
/// the `(group_index, handler_index)` it occupies there (from
/// `codex_hook_config::RegistrationDiagnosis::position`).
pub(crate) fn trust_readiness(
    context: &TrustContext,
    hooks_json_path: &Path,
    event: &str,
    matcher: Option<&str>,
    handler: &Value,
    position: (usize, usize),
) -> TrustReadiness {
    let Some(codex_home) = context.codex_home.as_ref() else {
        return TrustReadiness::Unknown(
            "Unable to resolve a Codex home directory (CODEX_HOME is unset and no home \
             directory could be determined)."
                .to_string(),
        );
    };

    let current_hash = match hash_command_handler(event, matcher, handler) {
        Ok(hash) => hash,
        Err(error) => return TrustReadiness::Unknown(error),
    };

    let key = match state_key(hooks_json_path, event, position) {
        Ok(key) => key,
        Err(error) => return TrustReadiness::Unknown(error),
    };

    let config_path = codex_home.join("config.toml");
    let state = match read_hook_state(&config_path, &key) {
        Ok(state) => state,
        Err(error) => return TrustReadiness::Unknown(error),
    };

    if state.enabled == Some(false) {
        return TrustReadiness::Disabled;
    }
    match state.trusted_hash {
        Some(trusted_hash) if trusted_hash == current_hash => TrustReadiness::Trusted,
        Some(_) => TrustReadiness::Modified,
        None => TrustReadiness::Untrusted,
    }
}

/// Mirrors upstream `HookStateToml` (`config/src/hook_config.rs`) exactly:
/// both fields optional, no `deny_unknown_fields` (an unrecognized extra key
/// is ignored, matching upstream). Deriving `Deserialize` from this shape
/// (rather than reading `enabled`/`trusted_hash` independently) is what lets
/// `read_hook_state` reject the whole entry, not just one bad field, exactly
/// as upstream's `hook_states_from_stack` does.
#[derive(Debug, Default, Clone, serde::Deserialize)]
struct HookStateEntry {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    trusted_hash: Option<String>,
}

/// Read `hooks.state."<key>"` from the user's Codex config, treating a
/// missing config file as "no state recorded" (a normal, common state) rather
/// than an error. A config file that exists but cannot be read or parsed is
/// an error, since doctor cannot tell whether trust was actually granted.
///
/// A present entry that fails to deserialize as a whole (e.g. `enabled` set
/// to a non-boolean) is treated as absent, matching upstream
/// `hook_states_from_stack`'s `Err(_) => continue`: Codex never salvages
/// individual fields from a malformed state entry, so neither does doctor —
/// a malformed entry must never read as `Trusted` just because its
/// `trusted_hash` string happens to be well-formed.
fn read_hook_state(config_path: &Path, key: &str) -> Result<HookStateEntry, String> {
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HookStateEntry::default())
        }
        Err(error) => {
            return Err(format!(
                "Unable to read Codex user config '{}': {error}",
                config_path.display()
            ))
        }
    };
    // Parsed as a document-level `Table` (supports `[section]` headers), not
    // as a bare `Value` (whose `FromStr` parses a single TOML value and would
    // misread a leading `[` as an array literal).
    let document: toml::Table = contents.parse().map_err(|error| {
        format!(
            "Unable to parse Codex user config '{}' as TOML: {error}",
            config_path.display()
        )
    })?;

    let entry = document
        .get("hooks")
        .and_then(|hooks| hooks.get("state"))
        .and_then(|state| state.get(key));
    let Some(entry) = entry else {
        return Ok(HookStateEntry::default());
    };

    Ok(HookStateEntry::deserialize(entry.clone()).unwrap_or_default())
}

/// Build Codex's persisted hook-state key for one registration, matching
/// `hooks::hook_key` (`"{key_source}:{event_label}:{group_index}:{handler_index}"`
/// where `key_source` is the hooks file's absolute display path).
fn state_key(
    hooks_json_path: &Path,
    event: &str,
    (group_index, handler_index): (usize, usize),
) -> Result<String, String> {
    let absolute = std::fs::canonicalize(hooks_json_path).map_err(|error| {
        format!(
            "Unable to resolve the absolute path of '{}': {error}",
            hooks_json_path.display()
        )
    })?;
    Ok(format!(
        "{}:{}:{group_index}:{handler_index}",
        absolute.display(),
        super::codex_hook_config::hook_event_key_label(event)
    ))
}

/// Hash one existing `command` handler's normalized identity exactly as
/// upstream `hook_hash` does: build the same `{event_name, matcher?, hooks:
/// [<one normalized handler>]}` shape, canonicalize (recursively sort object
/// keys, matching `fingerprint::canonical_json`), and SHA-256 the compact
/// JSON encoding.
pub(crate) fn hash_command_handler(
    event: &str,
    matcher: Option<&str>,
    handler: &Value,
) -> Result<String, String> {
    let object = handler
        .as_object()
        .ok_or_else(|| "Codex hook handler must be a JSON object".to_string())?;
    let handler_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Codex hook handler must have a string 'type'".to_string())?;
    if handler_type != "command" {
        return Err(format!(
            "Codex hook trust hashing only supports 'command' handlers, found '{handler_type}'"
        ));
    }
    let command = object
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "Codex 'command' hook handler must have a string 'command'".to_string())?;

    let mut handler_fields = serde_json::Map::new();
    handler_fields.insert("type".to_string(), Value::String("command".to_string()));
    handler_fields.insert("command".to_string(), Value::String(command.to_string()));
    let timeout = object
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(600)
        .max(1);
    handler_fields.insert("timeout".to_string(), Value::from(timeout));
    let is_async = object
        .get("async")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    handler_fields.insert("async".to_string(), Value::Bool(is_async));
    if let Some(status_message) = object.get("statusMessage").and_then(Value::as_str) {
        handler_fields.insert(
            "statusMessage".to_string(),
            Value::String(status_message.to_string()),
        );
    }
    if EVENTS_SUPPORTING_ADDITIONAL_CONTEXT.contains(&event) {
        if let Some(limit) = object.get("additionalContextLimit").and_then(Value::as_u64) {
            if limit != DEFAULT_HOOK_OUTPUT_TOKEN_LIMIT {
                handler_fields.insert("additionalContextLimit".to_string(), Value::from(limit));
            }
        }
    }

    let mut identity = serde_json::Map::new();
    identity.insert(
        "event_name".to_string(),
        Value::String(super::codex_hook_config::hook_event_key_label(event).to_string()),
    );
    if let Some(matcher) = matcher {
        identity.insert("matcher".to_string(), Value::String(matcher.to_string()));
    }
    identity.insert(
        "hooks".to_string(),
        Value::Array(vec![Value::Object(handler_fields)]),
    );

    Ok(version_for_canonical_json(&Value::Object(identity)))
}

fn version_for_canonical_json(value: &Value) -> String {
    use std::fmt::Write as _;

    let canonical = canonical_json(value);
    let serialized = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&serialized);
    let hash = hasher.finalize();
    let hex = hash
        .iter()
        .fold(String::with_capacity(hash.len() * 2), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    format!("sha256:{hex}")
}

/// Recursively sort object keys, matching `fingerprint::canonical_json`
/// exactly (arrays keep their order; scalars pass through unchanged).
fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                if let Some(inner) = map.get(&key) {
                    sorted.insert(key, canonical_json(inner));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sce-codex-hook-trust-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bare_command_handler() -> Value {
        json!({
            "type": "command",
            "command": "root=\"$(git rev-parse --show-toplevel 2>/dev/null)\" || exit 0; exec bash \"$root/.codex/hooks/run-sce-or-show-install-guidance.sh\" sce hooks codex"
        })
    }

    #[test]
    fn hashing_a_bare_handler_is_deterministic_and_matcher_sensitive() {
        let handler = bare_command_handler();
        let a = hash_command_handler("Stop", None, &handler).unwrap();
        let b = hash_command_handler("Stop", None, &handler).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));

        let c = hash_command_handler("PreToolUse", Some("Bash"), &handler).unwrap();
        assert_ne!(a, c, "different event/matcher must hash differently");
    }

    #[test]
    fn hashing_ignores_a_default_valued_additional_context_limit() {
        let handler = bare_command_handler();
        let mut with_default_limit = handler.as_object().unwrap().clone();
        with_default_limit.insert("additionalContextLimit".to_string(), json!(2500));
        let with_default_limit = Value::Object(with_default_limit);

        let without = hash_command_handler("UserPromptSubmit", None, &handler).unwrap();
        let with_default =
            hash_command_handler("UserPromptSubmit", None, &with_default_limit).unwrap();
        assert_eq!(without, with_default);
    }

    #[test]
    fn hashing_a_non_default_additional_context_limit_changes_the_hash() {
        let handler = bare_command_handler();
        let mut with_limit = handler.as_object().unwrap().clone();
        with_limit.insert("additionalContextLimit".to_string(), json!(1000));
        let with_limit = Value::Object(with_limit);

        let without = hash_command_handler("UserPromptSubmit", None, &handler).unwrap();
        let with_limit = hash_command_handler("UserPromptSubmit", None, &with_limit).unwrap();
        assert_ne!(without, with_limit);
    }

    #[test]
    fn hashing_drops_additional_context_limit_on_stop_since_it_is_unsupported() {
        let handler = bare_command_handler();
        let mut with_limit = handler.as_object().unwrap().clone();
        with_limit.insert("additionalContextLimit".to_string(), json!(1000));
        let with_limit = Value::Object(with_limit);

        let without = hash_command_handler("Stop", None, &handler).unwrap();
        let with_limit = hash_command_handler("Stop", None, &with_limit).unwrap();
        assert_eq!(
            without, with_limit,
            "Stop cannot carry additionalContext, so the field must not affect its hash"
        );
    }

    #[test]
    fn trust_readiness_is_untrusted_when_no_user_config_exists() {
        let dir = temp_dir("no-config");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let context = TrustContext {
            codex_home: Some(dir.join("codex-home-does-not-exist")),
        };
        let handler = bare_command_handler();
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert_eq!(readiness, TrustReadiness::Untrusted);
    }

    #[test]
    fn trust_readiness_is_trusted_when_the_recorded_hash_matches() {
        let dir = temp_dir("trusted");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let absolute = fs::canonicalize(&hooks_json).unwrap();
        let handler = bare_command_handler();
        let hash = hash_command_handler("Stop", None, &handler).unwrap();
        let key = format!("{}:stop:0:0", absolute.display());

        let codex_home = dir.join("codex-home");
        fs::create_dir_all(&codex_home).unwrap();
        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");
        fs::write(
            codex_home.join("config.toml"),
            format!("[hooks.state.\"{escaped_key}\"]\ntrusted_hash = \"{hash}\"\n"),
        )
        .unwrap();

        let context = TrustContext {
            codex_home: Some(codex_home),
        };
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert_eq!(readiness, TrustReadiness::Trusted);
    }

    #[test]
    fn trust_readiness_is_modified_when_the_recorded_hash_differs() {
        let dir = temp_dir("modified");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let absolute = fs::canonicalize(&hooks_json).unwrap();
        let handler = bare_command_handler();
        let key = format!("{}:stop:0:0", absolute.display());

        let codex_home = dir.join("codex-home");
        fs::create_dir_all(&codex_home).unwrap();
        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");
        fs::write(
            codex_home.join("config.toml"),
            format!("[hooks.state.\"{escaped_key}\"]\ntrusted_hash = \"sha256:stale\"\n"),
        )
        .unwrap();

        let context = TrustContext {
            codex_home: Some(codex_home),
        };
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert_eq!(readiness, TrustReadiness::Modified);
    }

    #[test]
    fn trust_readiness_is_disabled_when_the_state_disables_it_even_if_trusted() {
        let dir = temp_dir("disabled");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let absolute = fs::canonicalize(&hooks_json).unwrap();
        let handler = bare_command_handler();
        let hash = hash_command_handler("Stop", None, &handler).unwrap();
        let key = format!("{}:stop:0:0", absolute.display());

        let codex_home = dir.join("codex-home");
        fs::create_dir_all(&codex_home).unwrap();
        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");
        fs::write(
            codex_home.join("config.toml"),
            format!(
                "[hooks.state.\"{escaped_key}\"]\ntrusted_hash = \"{hash}\"\nenabled = false\n"
            ),
        )
        .unwrap();

        let context = TrustContext {
            codex_home: Some(codex_home),
        };
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert_eq!(readiness, TrustReadiness::Disabled);
    }

    #[test]
    fn trust_readiness_is_unknown_when_the_user_config_cannot_be_parsed() {
        let dir = temp_dir("malformed-config");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let codex_home = dir.join("codex-home");
        fs::create_dir_all(&codex_home).unwrap();
        fs::write(codex_home.join("config.toml"), "not = [valid").unwrap();

        let context = TrustContext {
            codex_home: Some(codex_home),
        };
        let handler = bare_command_handler();
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert!(matches!(readiness, TrustReadiness::Unknown(_)));
    }

    #[test]
    fn trust_readiness_is_unknown_when_codex_home_cannot_be_resolved() {
        let dir = temp_dir("no-home");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let context = TrustContext { codex_home: None };
        let handler = bare_command_handler();
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert!(matches!(readiness, TrustReadiness::Unknown(_)));
    }

    /// Writes `[hooks.state."<key for hooks_json/event/position>"]` plus
    /// `body` verbatim into a fresh `$CODEX_HOME/config.toml`, returning the
    /// `TrustContext` pointed at it.
    fn write_state_toml(
        dir: &std::path::Path,
        label: &str,
        hooks_json: &std::path::Path,
        event_label: &str,
        body: &str,
    ) -> TrustContext {
        let absolute = fs::canonicalize(hooks_json).unwrap();
        let key = format!("{}:{event_label}:0:0", absolute.display());
        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");

        let codex_home = dir.join(format!("codex-home-{label}"));
        fs::create_dir_all(&codex_home).unwrap();
        fs::write(
            codex_home.join("config.toml"),
            format!("[hooks.state.\"{escaped_key}\"]\n{body}\n"),
        )
        .unwrap();

        TrustContext {
            codex_home: Some(codex_home),
        }
    }

    #[test]
    fn trust_readiness_ignores_the_whole_entry_when_enabled_has_the_wrong_type() {
        // Upstream deserializes the complete `HookStateToml` entry; a
        // present field with the wrong type fails the whole entry, so a
        // syntactically-correct `trusted_hash` next to it must never be
        // salvaged into a false `Trusted` result.
        let dir = temp_dir("malformed-enabled");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let handler = bare_command_handler();
        let hash = hash_command_handler("Stop", None, &handler).unwrap();

        let context = write_state_toml(
            &dir,
            "malformed-enabled",
            &hooks_json,
            "stop",
            &format!("enabled = \"not-a-bool\"\ntrusted_hash = \"{hash}\""),
        );
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert_eq!(
            readiness,
            TrustReadiness::Untrusted,
            "a malformed 'enabled' field must drop the whole entry, never read Trusted"
        );
    }

    #[test]
    fn trust_readiness_ignores_the_whole_entry_when_trusted_hash_has_the_wrong_type() {
        let dir = temp_dir("malformed-hash-type");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let handler = bare_command_handler();

        // `enabled = false` here is the distinguishing signal: if only the
        // malformed `trusted_hash` field were dropped in isolation,
        // `enabled = false` would still be honored and this would read
        // `Disabled`. The correct whole-entry-drop behavior discards
        // `enabled` too, so the result must be `Untrusted` (the same as no
        // entry at all).
        let context = write_state_toml(
            &dir,
            "malformed-hash-type",
            &hooks_json,
            "stop",
            "enabled = false\ntrusted_hash = 12345",
        );
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert_eq!(
            readiness,
            TrustReadiness::Untrusted,
            "a malformed 'trusted_hash' field must drop the whole entry (including 'enabled'), \
             not just be ignored on its own"
        );
    }

    #[test]
    fn trust_readiness_ignores_a_completely_malformed_state_entry() {
        let dir = temp_dir("malformed-entry");
        let hooks_json = dir.join("hooks.json");
        fs::write(&hooks_json, "{}").unwrap();
        let handler = bare_command_handler();
        let absolute = fs::canonicalize(&hooks_json).unwrap();
        let key = format!("{}:stop:0:0", absolute.display());
        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");

        let codex_home = dir.join("codex-home-malformed-entry");
        fs::create_dir_all(&codex_home).unwrap();
        // The entry itself is a plain string, not a table: cannot
        // deserialize as `HookStateToml` at all, so it must not panic and
        // must fall back to "no state recorded" like a missing entry.
        fs::write(
            codex_home.join("config.toml"),
            format!("[hooks.state]\n\"{escaped_key}\" = \"not a table\"\n"),
        )
        .unwrap();

        let context = TrustContext {
            codex_home: Some(codex_home),
        };
        let readiness = trust_readiness(&context, &hooks_json, "Stop", None, &handler, (0, 0));
        assert_eq!(readiness, TrustReadiness::Untrusted);
    }
}
