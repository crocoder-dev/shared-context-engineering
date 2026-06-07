//! Bash-policy and attribution-hooks semantic validation, merge helpers,
//! and policy rendering.
//!
//! This submodule owns built-in/custom bash-policy validation, duplicate/conflict/
//! redundancy checks, attribution-hooks config parsing helpers, policy resolved-data
//! structs, and policy-specific rendering. The parent `mod.rs` re-exports items
//! needed by resolution and rendering consumers.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::schema::{
    FileConfigValue, ParsedCustomBashPolicyEntryDocument, ParsedCustomBashPolicyMatchDocument,
};
use super::types::{ConfigPathSource, ResolvedOptionalValue, ValueSource};
use crate::services::style;

const BASH_POLICY_PRESET_CATALOG_JSON: &str =
    include_str!("../../../assets/generated/config/opencode/lib/bash-policy-presets.json");

static BUILTIN_BASH_POLICY_CATALOG: OnceLock<BuiltinBashPolicyCatalog> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BashPolicyConfig {
    pub(super) presets: Vec<String>,
    pub(super) custom: Vec<CustomBashPolicyEntry>,
}

#[derive(Debug, Deserialize)]
struct BuiltinBashPolicyCatalog {
    presets: Vec<BuiltinBashPolicyPreset>,
    mutually_exclusive: Vec<Vec<String>>,
    redundancy_warnings: Vec<BuiltinBashPolicyRedundancyWarning>,
}

#[derive(Debug, Deserialize)]
struct BuiltinBashPolicyPreset {
    id: String,
    #[serde(rename = "match")]
    matcher: BuiltinBashPolicyMatcher,
    message: String,
}

#[derive(Debug, Deserialize)]
struct BuiltinBashPolicyMatcher {
    argv_prefixes: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct BuiltinBashPolicyRedundancyWarning {
    if_enabled: Vec<String>,
    warning: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CustomBashPolicyEntry {
    pub(crate) id: String,
    pub(crate) argv_prefix: Vec<String>,
    pub(crate) message: String,
}

impl CustomBashPolicyEntry {
    fn json_value(&self) -> Value {
        json!({
            "id": self.id,
            "match": {
                "argv_prefix": self.argv_prefix,
            },
            "message": self.message,
        })
    }

    fn text_summary(&self) -> String {
        format!(
            "{} => [{}] :: {}",
            self.id,
            self.argv_prefix.join(" "),
            self.message
        )
    }
}

fn builtin_bash_policy_catalog() -> &'static BuiltinBashPolicyCatalog {
    BUILTIN_BASH_POLICY_CATALOG.get_or_init(|| {
        let catalog: BuiltinBashPolicyCatalog =
            serde_json::from_str(BASH_POLICY_PRESET_CATALOG_JSON)
                .expect("bash policy preset catalog JSON must remain valid");
        debug_assert!(catalog.presets.iter().all(|preset| !preset.id.is_empty()
            && !preset.message.is_empty()
            && !preset.matcher.argv_prefixes.is_empty()));
        catalog
    })
}

fn builtin_bash_policy_preset_ids() -> Vec<&'static str> {
    builtin_bash_policy_catalog()
        .presets
        .iter()
        .map(|preset| preset.id.as_str())
        .collect()
}

pub(crate) fn is_builtin_bash_policy_preset_id(id: &str) -> bool {
    builtin_bash_policy_catalog()
        .presets
        .iter()
        .any(|preset| preset.id == id)
}

pub(super) fn resolve_bash_policy_config(
    presets: Option<&FileConfigValue<Vec<String>>>,
    custom: Option<&FileConfigValue<Vec<CustomBashPolicyEntry>>>,
) -> ResolvedOptionalValue<BashPolicyConfig> {
    let resolved_presets = presets.map(|value| value.value.clone());
    let resolved_custom = custom.map(|value| value.value.clone());
    let source = custom
        .map(|value| value.source)
        .or_else(|| presets.map(|value| value.source));

    if resolved_presets.as_ref().is_none_or(Vec::is_empty)
        && resolved_custom.as_ref().is_none_or(Vec::is_empty)
    {
        return ResolvedOptionalValue {
            value: None,
            source: None,
        };
    }

    ResolvedOptionalValue {
        value: Some(BashPolicyConfig {
            presets: resolved_presets.unwrap_or_default(),
            custom: resolved_custom.unwrap_or_default(),
        }),
        source: source.map(ValueSource::ConfigFile),
    }
}

pub(super) fn build_validation_warnings(
    value: &ResolvedOptionalValue<BashPolicyConfig>,
) -> Vec<String> {
    let Some(config) = value.value.as_ref() else {
        return Vec::new();
    };

    builtin_bash_policy_catalog()
        .redundancy_warnings
        .iter()
        .filter(|warning| {
            warning
                .if_enabled
                .iter()
                .all(|preset| config.presets.iter().any(|enabled| enabled == preset))
        })
        .map(|warning| warning.warning.clone())
        .collect()
}

pub(crate) fn parse_bash_policy_presets(items: &[String], path: &Path) -> Result<Vec<String>> {
    let mut presets = Vec::with_capacity(items.len());
    let builtin_preset_ids = builtin_bash_policy_preset_ids();
    for item in items {
        let preset = item.as_str();
        if !builtin_preset_ids.contains(&preset) {
            bail!(
                "Config key 'policies.bash.presets' in '{}' contains unknown preset '{}'. Allowed presets: {}.",
                path.display(),
                preset,
                builtin_preset_ids.join(", ")
            );
        }
        if presets.iter().any(|existing| existing == preset) {
            bail!(
                "Config key 'policies.bash.presets' in '{}' contains duplicate preset '{}'.",
                path.display(),
                preset
            );
        }
        presets.push(preset.to_string());
    }

    for conflict_group in &builtin_bash_policy_catalog().mutually_exclusive {
        if conflict_group
            .iter()
            .all(|preset| presets.iter().any(|enabled| enabled == preset))
        {
            let joined = conflict_group
                .iter()
                .map(|preset| format!("'{preset}'"))
                .collect::<Vec<_>>()
                .join(" and ");
            bail!(
                "Config key 'policies.bash.presets' in '{}' cannot enable both {}.",
                path.display(),
                joined
            );
        }
    }

    Ok(presets)
}

pub(crate) fn parse_custom_bash_policies(
    items: &[ParsedCustomBashPolicyEntryDocument],
    path: &Path,
) -> Result<Vec<CustomBashPolicyEntry>> {
    let mut policies = Vec::with_capacity(items.len());
    let mut argv_prefixes: Vec<Vec<String>> = Vec::new();
    for item in items {
        let policy = parse_custom_bash_policy_entry(item, path)?;
        if policies
            .iter()
            .any(|existing: &CustomBashPolicyEntry| existing.id == policy.id)
        {
            bail!(
                "Config key 'policies.bash.custom' in '{}' contains duplicate id '{}'.",
                path.display(),
                policy.id
            );
        }

        if argv_prefixes
            .iter()
            .any(|existing| existing == &policy.argv_prefix)
        {
            bail!(
                "Config key 'policies.bash.custom' in '{}' contains duplicate argv_prefix [{}].",
                path.display(),
                policy.argv_prefix.join(" ")
            );
        }
        argv_prefixes.push(policy.argv_prefix.clone());
        policies.push(policy);
    }

    Ok(policies)
}

fn parse_custom_bash_policy_entry(
    item: &ParsedCustomBashPolicyEntryDocument,
    path: &Path,
) -> Result<CustomBashPolicyEntry> {
    let id = item
        .id
        .as_deref()
        .with_context(|| {
            format!(
                "Each 'policies.bash.custom' entry in '{}' must include string field 'id'.",
                path.display()
            )
        })?
        .to_string();
    if is_builtin_bash_policy_preset_id(&id) {
        bail!(
            "Custom bash policy id '{}' in '{}' collides with a built-in preset id.",
            id,
            path.display()
        );
    }

    let message = item.message.as_deref().with_context(|| {
        format!(
            "Custom bash policy '{}' in '{}' must include string field 'message'.",
            id,
            path.display()
        )
    })?;
    if message.is_empty() {
        bail!(
            "Custom bash policy '{}' in '{}' must use a non-empty 'message'.",
            id,
            path.display()
        );
    }

    let argv_prefix = parse_custom_bash_policy_match(&id, item.matcher.as_ref(), path)?;

    Ok(CustomBashPolicyEntry {
        id,
        argv_prefix,
        message: message.to_string(),
    })
}

fn parse_custom_bash_policy_match(
    id: &str,
    matcher: Option<&ParsedCustomBashPolicyMatchDocument>,
    path: &Path,
) -> Result<Vec<String>> {
    let matcher = matcher.with_context(|| {
        format!(
            "Custom bash policy '{}' in '{}' must include object field 'match'.",
            id,
            path.display()
        )
    })?;
    let argv_prefix_values = matcher.argv_prefix.as_deref().with_context(|| {
        format!(
            "Custom bash policy '{}' in '{}' must include array field 'match.argv_prefix'.",
            id,
            path.display()
        )
    })?;
    if argv_prefix_values.is_empty() {
        bail!(
            "Custom bash policy '{}' in '{}' must use a non-empty 'match.argv_prefix'.",
            id,
            path.display()
        );
    }

    parse_custom_bash_policy_argv_prefix(id, argv_prefix_values, path)
}

fn parse_custom_bash_policy_argv_prefix(
    id: &str,
    argv_prefix_values: &[String],
    path: &Path,
) -> Result<Vec<String>> {
    let mut argv_prefix = Vec::with_capacity(argv_prefix_values.len());
    for token in argv_prefix_values {
        if token.is_empty() {
            bail!(
                "Custom bash policy '{}' in '{}' cannot use empty argv_prefix tokens.",
                id,
                path.display()
            );
        }
        argv_prefix.push(token.clone());
    }

    Ok(argv_prefix)
}

pub(super) fn format_bash_policies_text(value: &ResolvedOptionalValue<BashPolicyConfig>) -> String {
    match (value.value.as_ref(), value.source) {
        (Some(config), Some(source)) => {
            let presets = if config.presets.is_empty() {
                String::from("(none)")
            } else {
                config.presets.join(", ")
            };
            let custom = if config.custom.is_empty() {
                String::from("(none)")
            } else {
                config
                    .custom
                    .iter()
                    .map(CustomBashPolicyEntry::text_summary)
                    .collect::<Vec<_>>()
                    .join(" | ")
            };
            match source.config_source() {
                Some(config_source) => format!(
                    "- {}: presets=[{}]; custom=[{}] (source: {}, config_source: {})",
                    style::label("policies.bash"),
                    style::value(&presets),
                    style::value(&custom),
                    style::label(source.as_str()),
                    style::label(config_source.as_str())
                ),
                None => format!(
                    "- {}: presets=[{}]; custom=[{}] (source: {})",
                    style::label("policies.bash"),
                    style::value(&presets),
                    style::value(&custom),
                    style::label(source.as_str())
                ),
            }
        }
        _ => format!(
            "- {}: {} (source: {})",
            style::label("policies.bash"),
            style::value("(unset)"),
            style::label("none")
        ),
    }
}

pub(super) fn format_bash_policies_json(value: &ResolvedOptionalValue<BashPolicyConfig>) -> Value {
    let config = value.value.as_ref();
    json!({
        "presets": config.map(|bash| bash.presets.clone()),
        "custom": config.map(|bash| bash.custom.iter().map(CustomBashPolicyEntry::json_value).collect::<Vec<_>>()),
        "source": value.source.map(ValueSource::as_str),
        "config_source": value.source.and_then(ValueSource::config_source).map(ConfigPathSource::as_str),
    })
}
