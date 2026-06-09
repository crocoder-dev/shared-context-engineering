//! Config schema embedding, JSON validation, serde DTO definitions,
//! and config-file load/parse helpers.
//!
//! This submodule owns the JSON Schema constant/validator, top-level
//! allowed-key validation, typed config-file deserialization, and the
//! file-parse orchestration that bridges schema validation to the
//! runtime config model. Policy-specific semantic validation (bash-policy
//! preset/custom conflict and redundancy checks) remains in the parent
//! module and is called through `super::` from parse helpers here.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use jsonschema::{validator_for, Validator};
use serde::Deserialize;
use serde_json::Value;

use super::policy::{parse_bash_policy_presets, parse_custom_bash_policies, CustomBashPolicyEntry};
use super::types::{ConfigPathSource, LogFileMode, LogFormat, LogLevel};

pub(crate) const SCE_CONFIG_SCHEMA_JSON: &str =
    include_str!("../../../assets/generated/config/schema/sce-config.schema.json");

pub(crate) const CONFIG_SCHEMA_DECLARATION_KEY: &str = "$schema";

pub(crate) const TOP_LEVEL_CONFIG_KEYS: &[&str] = &[
    CONFIG_SCHEMA_DECLARATION_KEY,
    "log_level",
    "log_format",
    "log_file",
    "log_file_mode",
    "timeout_ms",
    super::resolver::WORKOS_CLIENT_ID_KEY.config_key,
    "policies",
];

pub(crate) const TOP_LEVEL_CONFIG_KEYS_DESCRIPTION: &str =
    "$schema, log_level, log_format, log_file, log_file_mode, timeout_ms, workos_client_id, policies";

static CONFIG_SCHEMA_VALIDATOR: OnceLock<Validator> = OnceLock::new();

pub(crate) fn config_schema_validator() -> &'static Validator {
    CONFIG_SCHEMA_VALIDATOR.get_or_init(|| {
        let schema: Value =
            serde_json::from_str(SCE_CONFIG_SCHEMA_JSON).expect("config schema JSON should parse");
        validator_for(&schema).expect("config schema JSON should compile")
    })
}

pub(crate) fn generated_config_schema_path() -> String {
    format!(
        "{}/{}",
        crate::services::default_paths::schema::SCHEMA_DIR,
        crate::services::default_paths::schema::SCE_CONFIG_SCHEMA
    )
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedFileConfigDocument {
    #[serde(rename = "$schema")]
    pub(crate) _schema: Option<String>,
    pub(crate) log_level: Option<String>,
    pub(crate) log_format: Option<String>,
    pub(crate) log_file: Option<String>,
    pub(crate) log_file_mode: Option<String>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) workos_client_id: Option<String>,
    pub(crate) policies: Option<ParsedPoliciesConfigDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedPoliciesConfigDocument {
    pub(crate) bash: Option<ParsedBashPolicyConfigDocument>,
    pub(crate) attribution_hooks: Option<ParsedAttributionHooksConfigDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedBashPolicyConfigDocument {
    pub(crate) presets: Option<Vec<String>>,
    pub(crate) custom: Option<Vec<ParsedCustomBashPolicyEntryDocument>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedAttributionHooksConfigDocument {
    pub(crate) enabled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedCustomBashPolicyEntryDocument {
    pub(crate) id: Option<String>,
    #[serde(rename = "match")]
    pub(crate) matcher: Option<ParsedCustomBashPolicyMatchDocument>,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedCustomBashPolicyMatchDocument {
    pub(crate) argv_prefix: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileConfigValue<T> {
    pub(crate) value: T,
    pub(crate) source: ConfigPathSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileConfig {
    pub(crate) log_level: Option<FileConfigValue<LogLevel>>,
    pub(crate) log_format: Option<FileConfigValue<LogFormat>>,
    pub(crate) log_file: Option<FileConfigValue<String>>,
    pub(crate) log_file_mode: Option<FileConfigValue<LogFileMode>>,
    pub(crate) timeout_ms: Option<FileConfigValue<u64>>,
    pub(crate) attribution_hooks_enabled: Option<FileConfigValue<bool>>,
    pub(crate) workos_client_id: Option<FileConfigValue<String>>,
    pub(crate) bash_policy_presets: Option<FileConfigValue<Vec<String>>>,
    pub(crate) bash_policy_custom: Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
}

pub(crate) type ParsedBashPolicyConfig = (
    Option<FileConfigValue<Vec<String>>>,
    Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
);

pub(crate) type ParsedFilePolicies = (
    Option<FileConfigValue<bool>>,
    Option<FileConfigValue<Vec<String>>>,
    Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
);

pub(crate) fn validate_config_value_against_schema(value: &Value, path: &Path) -> Result<()> {
    let mut errors = config_schema_validator()
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();

    if errors.is_empty() {
        return Ok(());
    }

    errors.sort();
    let generated_schema_path = generated_config_schema_path();
    bail!(
        "Config file '{}' failed schema validation against generated schema '{}': {}",
        path.display(),
        generated_schema_path,
        errors.join(" | ")
    );
}

pub(crate) fn validate_object_keys(
    object: &serde_json::Map<String, Value>,
    path: &Path,
    context: Option<&str>,
    allowed_keys: &[&str],
    allowed_keys_description: &str,
) -> Result<()> {
    for key in object.keys() {
        if !allowed_keys.contains(&key.as_str()) {
            match context {
                Some(context) => bail!(
                    "Config key '{context}' in '{}' contains unknown key '{}'. Allowed keys: {allowed_keys_description}.",
                    path.display(),
                    key
                ),
                None => bail!(
                    "Config file '{}' contains unknown key '{}'. Allowed keys: {allowed_keys_description}.",
                    path.display(),
                    key
                ),
            }
        }
    }

    Ok(())
}

pub(crate) fn validate_config_file(path: &Path) -> Result<()> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file '{}'.", path.display()))?;
    parse_file_config(&raw, path, ConfigPathSource::Flag)?;
    Ok(())
}

pub(crate) fn deserialize_typed_config(
    parsed: Value,
    path: &Path,
) -> Result<ParsedFileConfigDocument> {
    serde_json::from_value(parsed).with_context(|| {
        format!(
            "Config file '{}' could not be mapped into the typed runtime config model.",
            path.display()
        )
    })
}

#[allow(clippy::too_many_lines)]
pub(crate) fn parse_file_config(
    raw: &str,
    path: &Path,
    source: ConfigPathSource,
) -> Result<FileConfig> {
    let parsed: Value = serde_json::from_str(raw)
        .with_context(|| format!("Config file '{}' must contain valid JSON.", path.display()))?;

    let object = parsed.as_object().with_context(|| {
        format!(
            "Config file '{}' must contain a top-level JSON object.",
            path.display()
        )
    })?;

    validate_config_value_against_schema(&parsed, path)?;
    validate_object_keys(
        object,
        path,
        None,
        TOP_LEVEL_CONFIG_KEYS,
        TOP_LEVEL_CONFIG_KEYS_DESCRIPTION,
    )?;

    let typed = deserialize_typed_config(parsed.clone(), path)?;
    let log_level = typed
        .log_level
        .map(|raw| -> Result<FileConfigValue<LogLevel>> {
            Ok(FileConfigValue {
                value: LogLevel::parse(&raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        })
        .transpose()?;
    let log_format = typed
        .log_format
        .map(|raw| -> Result<FileConfigValue<LogFormat>> {
            Ok(FileConfigValue {
                value: LogFormat::parse(&raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        })
        .transpose()?;
    let log_file = typed
        .log_file
        .map(|value| FileConfigValue { value, source });
    let log_file_mode = typed
        .log_file_mode
        .map(|raw| -> Result<FileConfigValue<LogFileMode>> {
            Ok(FileConfigValue {
                value: LogFileMode::parse(&raw, &format!("config file '{}'", path.display()))?,
                source,
            })
        })
        .transpose()?;
    let timeout_ms = typed
        .timeout_ms
        .map(|value| FileConfigValue { value, source });
    let workos_client_id = typed
        .workos_client_id
        .map(|value| FileConfigValue { value, source });
    let (attribution_hooks_enabled, bash_policy_presets, bash_policy_custom) =
        map_policies_config(typed.policies.as_ref(), object, path, source)?;

    Ok(FileConfig {
        log_level,
        log_format,
        log_file,
        log_file_mode,
        timeout_ms,
        attribution_hooks_enabled,
        workos_client_id,
        bash_policy_presets,
        bash_policy_custom,
    })
}

pub(crate) fn map_policies_config(
    typed: Option<&ParsedPoliciesConfigDocument>,
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<ParsedFilePolicies> {
    let Some(policies_value) = object.get("policies") else {
        return Ok((None, None, None));
    };

    let policies_object = policies_value.as_object().with_context(|| {
        format!(
            "Config key 'policies' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        policies_object,
        path,
        Some("policies"),
        &["bash", "attribution_hooks"],
        "bash, attribution_hooks",
    )?;

    let bash = typed.and_then(|config| config.bash.as_ref());
    let attribution_hooks_enabled = map_attribution_hooks_config(
        typed.and_then(|config| config.attribution_hooks.as_ref()),
        policies_object,
        path,
        source,
    )?;
    let (bash_policy_presets, bash_policy_custom) =
        map_bash_policy_config(bash, policies_object, path, source)?;

    Ok((
        attribution_hooks_enabled,
        bash_policy_presets,
        bash_policy_custom,
    ))
}

pub(crate) fn map_attribution_hooks_config(
    typed: Option<&ParsedAttributionHooksConfigDocument>,
    policies_object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<Option<FileConfigValue<bool>>> {
    let Some(attribution_hooks_value) = policies_object.get("attribution_hooks") else {
        return Ok(None);
    };

    let attribution_hooks_object = attribution_hooks_value.as_object().with_context(|| {
        format!(
            "Config key 'policies.attribution_hooks' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        attribution_hooks_object,
        path,
        Some("policies.attribution_hooks"),
        &["enabled"],
        "enabled",
    )?;

    Ok(typed
        .and_then(|config| config.enabled)
        .map(|value| FileConfigValue { value, source }))
}

pub(crate) fn map_bash_policy_config(
    typed: Option<&ParsedBashPolicyConfigDocument>,
    policies_object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<ParsedBashPolicyConfig> {
    let Some(bash_value) = policies_object.get("bash") else {
        return Ok((None, None));
    };

    let bash_object = bash_value.as_object().with_context(|| {
        format!(
            "Config key 'policies.bash' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        bash_object,
        path,
        Some("policies.bash"),
        &["presets", "custom"],
        "presets, custom",
    )?;

    let presets = typed
        .and_then(|config| config.presets.as_ref())
        .map(|presets| parse_bash_policy_presets(presets, path))
        .transpose()?
        .map(|value| FileConfigValue { value, source });
    let custom = typed
        .and_then(|config| config.custom.as_ref())
        .map(|custom| parse_custom_bash_policies(custom, path))
        .transpose()?
        .map(|value| FileConfigValue { value, source });

    Ok((presets, custom))
}
