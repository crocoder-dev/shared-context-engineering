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
use super::types::{
    ConfigPathSource, DatabaseRetryConfig, IntegrationTargetId, IntegrationsConfig, LogFormat,
    LogLevel,
};
use crate::services::resilience::RetryPolicy;

pub(crate) const SCE_CONFIG_SCHEMA_JSON: &str =
    include_str!("../../../assets/generated/config/schema/sce-config.schema.json");

pub(crate) const CONFIG_SCHEMA_DECLARATION_KEY: &str = "$schema";

pub(crate) const TOP_LEVEL_CONFIG_KEYS: &[&str] = &[
    CONFIG_SCHEMA_DECLARATION_KEY,
    "log_level",
    "log_format",
    "log_dir",
    "log_file_retention_limit",
    "timeout_ms",
    super::resolver::WORKOS_CLIENT_ID_KEY.config_key,
    "agent_trace",
    "policies",
    "integrations",
];

pub(crate) const TOP_LEVEL_CONFIG_KEYS_DESCRIPTION: &str =
    "$schema, log_level, log_format, timeout_ms, workos_client_id, agent_trace, policies, integrations, log_dir, log_file_retention_limit";

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
    pub(crate) log_dir: Option<String>,
    pub(crate) log_file_retention_limit: Option<usize>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) workos_client_id: Option<String>,
    pub(crate) agent_trace: Option<ParsedAgentTraceConfigDocument>,
    pub(crate) policies: Option<ParsedPoliciesConfigDocument>,
    pub(crate) integrations: Option<ParsedIntegrationsConfigDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedAgentTraceConfigDocument {
    pub(crate) repository_id: Option<String>,
    pub(crate) repository_remote: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedIntegrationsConfigDocument {
    pub(crate) target: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedPoliciesConfigDocument {
    pub(crate) bash: Option<ParsedBashPolicyConfigDocument>,
    pub(crate) attribution_hooks: Option<ParsedAttributionHooksConfigDocument>,
    pub(crate) database_retry: Option<ParsedDatabaseRetryConfigDocument>,
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

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedDatabaseRetryConfigDocument {
    pub(crate) local_db: Option<ParsedPerDbRetryConfigDocument>,
    pub(crate) agent_trace_db: Option<ParsedPerDbRetryConfigDocument>,
    pub(crate) auth_db: Option<ParsedPerDbRetryConfigDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedPerDbRetryConfigDocument {
    pub(crate) connection_open: Option<ParsedRetryPolicyDocument>,
    pub(crate) query: Option<ParsedRetryPolicyDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedRetryPolicyDocument {
    pub(crate) max_attempts: Option<u32>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) initial_backoff_ms: Option<u64>,
    pub(crate) max_backoff_ms: Option<u64>,
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
    pub(crate) log_dir: Option<FileConfigValue<String>>,
    pub(crate) log_file_retention_limit: Option<FileConfigValue<usize>>,
    pub(crate) timeout_ms: Option<FileConfigValue<u64>>,
    pub(crate) attribution_hooks_enabled: Option<FileConfigValue<bool>>,
    pub(crate) workos_client_id: Option<FileConfigValue<String>>,
    pub(crate) agent_trace_repository_id: Option<FileConfigValue<String>>,
    pub(crate) agent_trace_repository_remote: Option<FileConfigValue<String>>,
    pub(crate) bash_policy_presets: Option<FileConfigValue<Vec<String>>>,
    pub(crate) bash_policy_custom: Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
    pub(crate) database_retry: Option<FileConfigValue<DatabaseRetryConfig>>,
    pub(crate) integrations: Option<FileConfigValue<IntegrationsConfig>>,
}

pub(crate) type ParsedBashPolicyConfig = (
    Option<FileConfigValue<Vec<String>>>,
    Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
);

pub(crate) type ParsedFilePolicies = (
    Option<FileConfigValue<bool>>,
    Option<FileConfigValue<Vec<String>>>,
    Option<FileConfigValue<Vec<CustomBashPolicyEntry>>>,
    Option<FileConfigValue<DatabaseRetryConfig>>,
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
    let log_dir = typed.log_dir.map(|value| FileConfigValue { value, source });
    let log_file_retention_limit = typed
        .log_file_retention_limit
        .map(|value| FileConfigValue { value, source });
    let timeout_ms = typed
        .timeout_ms
        .map(|value| FileConfigValue { value, source });
    let workos_client_id = typed
        .workos_client_id
        .map(|value| FileConfigValue { value, source });
    let (agent_trace_repository_id, agent_trace_repository_remote) =
        map_agent_trace_config(typed.agent_trace.as_ref(), object, path, source)?;
    let (attribution_hooks_enabled, bash_policy_presets, bash_policy_custom, database_retry) =
        map_policies_config(typed.policies.as_ref(), object, path, source)?;
    let integrations = map_integrations_config(typed.integrations.as_ref(), object, path, source)?;

    Ok(FileConfig {
        log_level,
        log_format,
        log_dir,
        log_file_retention_limit,
        timeout_ms,
        attribution_hooks_enabled,
        workos_client_id,
        agent_trace_repository_id,
        agent_trace_repository_remote,
        bash_policy_presets,
        bash_policy_custom,
        database_retry,
        integrations,
    })
}

pub(crate) fn map_policies_config(
    typed: Option<&ParsedPoliciesConfigDocument>,
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<ParsedFilePolicies> {
    let Some(policies_value) = object.get("policies") else {
        return Ok((None, None, None, None));
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
        &["bash", "attribution_hooks", "database_retry"],
        "bash, attribution_hooks, database_retry",
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
    let database_retry = map_database_retry_config(
        typed.and_then(|config| config.database_retry.as_ref()),
        policies_object,
        path,
        source,
    )?;

    Ok((
        attribution_hooks_enabled,
        bash_policy_presets,
        bash_policy_custom,
        database_retry,
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

#[allow(clippy::too_many_lines)]
pub(crate) fn map_database_retry_config(
    typed: Option<&ParsedDatabaseRetryConfigDocument>,
    policies_object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<Option<FileConfigValue<DatabaseRetryConfig>>> {
    let Some(database_retry_value) = policies_object.get("database_retry") else {
        return Ok(None);
    };

    let database_retry_object = database_retry_value.as_object().with_context(|| {
        format!(
            "Config key 'policies.database_retry' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        database_retry_object,
        path,
        Some("policies.database_retry"),
        &["local_db", "agent_trace_db", "auth_db"],
        "local_db, agent_trace_db, auth_db",
    )?;

    let build_retry_policy =
        |parsed: &ParsedRetryPolicyDocument, context: &str| -> Result<RetryPolicy> {
            let max_attempts = parsed.max_attempts.with_context(|| {
                format!(
                    "Config key '{context}.max_attempts' in '{}' must be present.",
                    path.display()
                )
            })?;
            let timeout_ms = parsed.timeout_ms.with_context(|| {
                format!(
                    "Config key '{context}.timeout_ms' in '{}' must be present.",
                    path.display()
                )
            })?;
            let initial_backoff_ms = parsed.initial_backoff_ms.with_context(|| {
                format!(
                    "Config key '{context}.initial_backoff_ms' in '{}' must be present.",
                    path.display()
                )
            })?;
            let max_backoff_ms = parsed.max_backoff_ms.with_context(|| {
                format!(
                    "Config key '{context}.max_backoff_ms' in '{}' must be present.",
                    path.display()
                )
            })?;

            if max_attempts == 0 {
                bail!(
                    "Config key '{context}.max_attempts' in '{}' must be >= 1.",
                    path.display()
                );
            }
            if timeout_ms == 0 {
                bail!(
                    "Config key '{context}.timeout_ms' in '{}' must be >= 1.",
                    path.display()
                );
            }
            if max_backoff_ms < initial_backoff_ms {
                bail!(
                    "Config key '{context}.max_backoff_ms' in '{}' must be >= initial_backoff_ms.",
                    path.display()
                );
            }

            Ok(RetryPolicy {
                max_attempts,
                timeout_ms,
                initial_backoff_ms,
                max_backoff_ms,
            })
        };

    let build_per_db = |db_key: &str| -> Result<Option<super::types::PerDbRetryConfig>> {
        let Some(db_value) = database_retry_object.get(db_key) else {
            return Ok(None);
        };

        let db_object = db_value.as_object().with_context(|| {
            format!(
                "Config key 'policies.database_retry.{db_key}' in '{}' must be an object.",
                path.display()
            )
        })?;

        validate_object_keys(
            db_object,
            path,
            Some(&format!("policies.database_retry.{db_key}")),
            &["connection_open", "query"],
            "connection_open, query",
        )?;

        let typed_db = typed.and_then(|doc| match db_key {
            "local_db" => doc.local_db.as_ref(),
            "agent_trace_db" => doc.agent_trace_db.as_ref(),
            "auth_db" => doc.auth_db.as_ref(),
            _ => None,
        });

        let build_policy = |op_key: &str| -> Result<Option<RetryPolicy>> {
            let Some(op_value) = db_object.get(op_key) else {
                return Ok(None);
            };

            let _op_object = op_value.as_object().with_context(|| {
                format!(
                    "Config key 'policies.database_retry.{db_key}.{op_key}' in '{}' must be an object.",
                    path.display()
                )
            })?;

            let typed_policy = typed_db.and_then(|db| match op_key {
                "connection_open" => db.connection_open.as_ref(),
                "query" => db.query.as_ref(),
                _ => None,
            });

            let parsed = typed_policy.with_context(|| {
                format!(
                    "Config key 'policies.database_retry.{db_key}.{op_key}' in '{}' could not be parsed.",
                    path.display()
                )
            })?;

            let context = format!("policies.database_retry.{db_key}.{op_key}");
            build_retry_policy(parsed, &context).map(Some)
        };

        Ok(Some(super::types::PerDbRetryConfig {
            connection_open: build_policy("connection_open")?,
            query: build_policy("query")?,
        }))
    };

    Ok(Some(FileConfigValue {
        value: DatabaseRetryConfig {
            local_db: build_per_db("local_db")?,
            agent_trace_db: build_per_db("agent_trace_db")?,
            auth_db: build_per_db("auth_db")?,
        },
        source,
    }))
}

pub(crate) type ParsedAgentTraceConfig = (
    Option<FileConfigValue<String>>,
    Option<FileConfigValue<String>>,
);

fn map_agent_trace_config(
    typed: Option<&ParsedAgentTraceConfigDocument>,
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<ParsedAgentTraceConfig> {
    let Some(agent_trace_value) = object.get("agent_trace") else {
        return Ok((None, None));
    };

    let agent_trace_object = agent_trace_value.as_object().with_context(|| {
        format!(
            "Config key 'agent_trace' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        agent_trace_object,
        path,
        Some("agent_trace"),
        &["repository_id", "repository_remote"],
        "repository_id, repository_remote",
    )?;

    let repository_id = typed
        .and_then(|config| config.repository_id.clone())
        .map(|value| FileConfigValue { value, source });
    let repository_remote = typed
        .and_then(|config| config.repository_remote.clone())
        .map(|value| FileConfigValue { value, source });

    Ok((repository_id, repository_remote))
}

fn map_integrations_config(
    typed: Option<&ParsedIntegrationsConfigDocument>,
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<Option<FileConfigValue<IntegrationsConfig>>> {
    let Some(integrations_value) = object.get("integrations") else {
        return Ok(None);
    };

    let integrations_object = integrations_value.as_object().with_context(|| {
        format!(
            "Config key 'integrations' in '{}' must be an object.",
            path.display()
        )
    })?;

    validate_object_keys(
        integrations_object,
        path,
        Some("integrations"),
        &["target"],
        "target",
    )?;

    let Some(raw_targets) = typed.and_then(|config| config.target.as_ref()) else {
        return Ok(None);
    };

    let targets: Vec<IntegrationTargetId> = raw_targets
        .iter()
        .map(|raw| IntegrationTargetId::parse(raw, &format!("config file '{}'", path.display())))
        .collect::<Result<Vec<_>>>()?;

    Ok(Some(FileConfigValue {
        value: IntegrationsConfig { target: targets },
        source,
    }))
}

#[cfg(test)]
mod agent_trace_config_tests {
    use std::path::Path;

    use super::{parse_file_config, ConfigPathSource};

    fn parse(raw: &str) -> anyhow::Result<super::FileConfig> {
        parse_file_config(
            raw,
            Path::new("/tmp/sce-config.json"),
            ConfigPathSource::Flag,
        )
    }

    #[test]
    fn parses_agent_trace_repository_identity_keys() {
        let config = parse(
            r#"{"agent_trace":{"repository_id":"team-monorepo","repository_remote":"upstream"}}"#,
        )
        .unwrap();

        assert_eq!(
            config
                .agent_trace_repository_id
                .as_ref()
                .map(|value| value.value.as_str()),
            Some("team-monorepo")
        );
        assert_eq!(
            config
                .agent_trace_repository_remote
                .as_ref()
                .map(|value| value.value.as_str()),
            Some("upstream")
        );
    }

    #[test]
    fn omitted_agent_trace_block_parses_as_unset() {
        let config = parse("{}").unwrap();

        assert_eq!(config.agent_trace_repository_id, None);
        assert_eq!(config.agent_trace_repository_remote, None);
    }

    #[test]
    fn rejects_unknown_agent_trace_key() {
        let error = parse(r#"{"agent_trace":{"repository_url":"x"}}"#)
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed schema validation"), "{error}");
    }

    #[test]
    fn rejects_non_object_agent_trace_value() {
        let error = parse(r#"{"agent_trace":"origin"}"#)
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed schema validation"), "{error}");
    }

    #[test]
    fn rejects_empty_agent_trace_string_values() {
        let error = parse(r#"{"agent_trace":{"repository_id":""}}"#)
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed schema validation"), "{error}");
    }

    #[test]
    fn rejects_non_string_repository_remote() {
        let error = parse(r#"{"agent_trace":{"repository_remote":7}}"#)
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed schema validation"), "{error}");
    }
}
