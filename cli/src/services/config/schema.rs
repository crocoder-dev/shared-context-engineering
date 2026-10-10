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

use anyhow::{anyhow, bail, Context, Result};
use jsonschema::{validator_for, Validator};
use serde::Deserialize;
use serde_json::Value;

use super::policy::{parse_bash_policy_presets, parse_custom_bash_policies, CustomBashPolicyEntry};
use super::types::{
    parse_optional_workflow_id, AgentTraceDbRetryConfig, ConfigPathSource, DatabaseRetryConfig,
    IntegrationTargetId, IntegrationsConfig, LogFormat, LogLevel, PerDbRetryConfig,
    AGENT_TRACE_DB_BUSY_TIMEOUT_MAX_MS, AGENT_TRACE_DB_CONTENTION_DEADLINE_MAX_MS,
};
use crate::services::resilience::{RetryPolicy, RetryPolicyError};

pub(crate) const SCE_CONFIG_SCHEMA_JSON: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/pkl-generated/config/schema/sce-config.schema.json"
));

pub(crate) const CONFIG_SCHEMA_DECLARATION_KEY: &str = "$schema";

pub(crate) const TOP_LEVEL_CONFIG_KEYS: &[&str] = &[
    CONFIG_SCHEMA_DECLARATION_KEY,
    "log_level",
    "log_format",
    "log_to_file",
    "log_dir",
    "log_file_retention_limit",
    super::resolver::WORKOS_CLIENT_ID_KEY.config_key,
    super::resolver::CONTROL_PLANE_BASE_URL_KEY.config_key,
    "agent_trace",
    "policies",
    "integrations",
];

pub(crate) const TOP_LEVEL_CONFIG_KEYS_DESCRIPTION: &str =
    "$schema, log_level, log_format, log_to_file, workos_client_id, control_plane_base_url, agent_trace, policies, integrations, log_dir, log_file_retention_limit";

const PER_DB_RETRY_KEYS: &[&str] = &["connection_open", "query"];
const PER_DB_RETRY_KEYS_DESCRIPTION: &str = "connection_open, query";
const AGENT_TRACE_DB_RETRY_KEYS: &[&str] = &[
    "connection_open",
    "busy_timeout_ms",
    "contention_deadline_ms",
    "query",
];
const AGENT_TRACE_DB_RETRY_KEYS_DESCRIPTION: &str =
    "connection_open, busy_timeout_ms, contention_deadline_ms, query";

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
    pub(crate) log_to_file: Option<bool>,
    pub(crate) log_dir: Option<String>,
    pub(crate) log_file_retention_limit: Option<usize>,
    pub(crate) workos_client_id: Option<String>,
    pub(crate) control_plane_base_url: Option<String>,
    pub(crate) agent_trace: Option<ParsedAgentTraceConfigDocument>,
    pub(crate) policies: Option<ParsedPoliciesConfigDocument>,
    pub(crate) integrations: Option<ParsedIntegrationsConfigDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedAgentTraceConfigDocument {
    pub(crate) repository_id: Option<String>,
    pub(crate) repository_remote: Option<String>,
    pub(crate) auto_sync: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedIntegrationsConfigDocument {
    pub(crate) target: Option<Vec<String>>,
    pub(crate) optional_workflows: Option<Vec<String>>,
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
    pub(crate) satisfied_by: Option<Vec<Vec<String>>>,
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
    pub(crate) agent_trace_db: Option<ParsedAgentTraceDbRetryConfigDocument>,
    pub(crate) auth_db: Option<ParsedPerDbRetryConfigDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct ParsedAgentTraceDbRetryConfigDocument {
    pub(crate) connection_open: Option<ParsedRetryPolicyDocument>,
    pub(crate) query: Option<ParsedRetryPolicyDocument>,
    pub(crate) busy_timeout_ms: Option<u64>,
    pub(crate) contention_deadline_ms: Option<u64>,
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
    pub(crate) log_to_file: Option<FileConfigValue<bool>>,
    pub(crate) log_dir: Option<FileConfigValue<String>>,
    pub(crate) log_file_retention_limit: Option<FileConfigValue<usize>>,
    pub(crate) attribution_hooks_enabled: Option<FileConfigValue<bool>>,
    pub(crate) workos_client_id: Option<FileConfigValue<String>>,
    pub(crate) control_plane_base_url: Option<FileConfigValue<String>>,
    pub(crate) agent_trace_repository_id: Option<FileConfigValue<String>>,
    pub(crate) agent_trace_repository_remote: Option<FileConfigValue<String>>,
    pub(crate) agent_trace_auto_sync: Option<FileConfigValue<bool>>,
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
        .map(|error| {
            let location = error.instance_path().to_string();
            if location.is_empty() {
                error.to_string()
            } else {
                format!("{location}: {error}")
            }
        })
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
    let log_to_file = typed
        .log_to_file
        .map(|value| FileConfigValue { value, source });
    let log_dir = typed.log_dir.map(|value| FileConfigValue { value, source });
    let log_file_retention_limit = typed
        .log_file_retention_limit
        .map(|value| FileConfigValue { value, source });
    let workos_client_id = typed
        .workos_client_id
        .map(|value| FileConfigValue { value, source });
    let control_plane_base_url = typed
        .control_plane_base_url
        .map(|value| FileConfigValue { value, source });
    let (agent_trace_repository_id, agent_trace_repository_remote, agent_trace_auto_sync) =
        map_agent_trace_config(typed.agent_trace.as_ref(), object, path, source)?;
    let (attribution_hooks_enabled, bash_policy_presets, bash_policy_custom, database_retry) =
        map_policies_config(typed.policies.as_ref(), object, path, source)?;
    let integrations = map_integrations_config(typed.integrations.as_ref(), object, path, source)?;

    Ok(FileConfig {
        log_level,
        log_format,
        log_to_file,
        log_dir,
        log_file_retention_limit,
        attribution_hooks_enabled,
        workos_client_id,
        control_plane_base_url,
        agent_trace_repository_id,
        agent_trace_repository_remote,
        agent_trace_auto_sync,
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

            RetryPolicy::new(max_attempts, timeout_ms, initial_backoff_ms, max_backoff_ms).map_err(
                |error| {
                    let message = match error {
                        RetryPolicyError::ZeroMaxAttempts => {
                            format!(
                                "'{context}.max_attempts' in '{}' must be >= 1.",
                                path.display()
                            )
                        }
                        RetryPolicyError::ZeroTimeout => {
                            format!(
                                "'{context}.timeout_ms' in '{}' must be >= 1.",
                                path.display()
                            )
                        }
                        RetryPolicyError::MaxBackoffBelowInitial => format!(
                            "'{context}.max_backoff_ms' in '{}' must be >= initial_backoff_ms.",
                            path.display()
                        ),
                    };
                    anyhow!("Config key {message}")
                },
            )
        };

    let per_db_object = |db_key: &str,
                         allowed_keys: &[&str],
                         allowed_keys_description: &str|
     -> Result<Option<&serde_json::Map<String, Value>>> {
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
            allowed_keys,
            allowed_keys_description,
        )?;

        Ok(Some(db_object))
    };

    let build_policy = |db_key: &str,
                        db_object: &serde_json::Map<String, Value>,
                        op_key: &str,
                        typed_policy: Option<&ParsedRetryPolicyDocument>|
     -> Result<Option<RetryPolicy>> {
        let Some(op_value) = db_object.get(op_key) else {
            return Ok(None);
        };

        let _op_object = op_value.as_object().with_context(|| {
            format!(
                "Config key 'policies.database_retry.{db_key}.{op_key}' in '{}' must be an object.",
                path.display()
            )
        })?;

        let parsed = typed_policy.with_context(|| {
            format!(
                "Config key 'policies.database_retry.{db_key}.{op_key}' in '{}' could not be parsed.",
                path.display()
            )
        })?;

        let context = format!("policies.database_retry.{db_key}.{op_key}");
        build_retry_policy(parsed, &context).map(Some)
    };

    let build_per_db = |db_key: &str| -> Result<Option<PerDbRetryConfig>> {
        let Some(db_object) =
            per_db_object(db_key, PER_DB_RETRY_KEYS, PER_DB_RETRY_KEYS_DESCRIPTION)?
        else {
            return Ok(None);
        };

        let typed_db = typed.and_then(|doc| match db_key {
            "local_db" => doc.local_db.as_ref(),
            "auth_db" => doc.auth_db.as_ref(),
            _ => None,
        });

        Ok(Some(PerDbRetryConfig {
            connection_open: build_policy(
                db_key,
                db_object,
                "connection_open",
                typed_db.and_then(|db| db.connection_open.as_ref()),
            )?,
            query: build_policy(
                db_key,
                db_object,
                "query",
                typed_db.and_then(|db| db.query.as_ref()),
            )?,
        }))
    };

    let build_agent_trace_db = || -> Result<Option<AgentTraceDbRetryConfig>> {
        let db_key = "agent_trace_db";
        let Some(db_object) = per_db_object(
            db_key,
            AGENT_TRACE_DB_RETRY_KEYS,
            AGENT_TRACE_DB_RETRY_KEYS_DESCRIPTION,
        )?
        else {
            return Ok(None);
        };

        let typed_db = typed.and_then(|doc| doc.agent_trace_db.as_ref());

        let bounded_millis = |key: &str, value: Option<u64>, max: u64| -> Result<Option<u64>> {
            let Some(value) = value else {
                if db_object.contains_key(key) {
                    bail!(
                        "Config key 'policies.database_retry.{db_key}.{key}' in '{}' could not be parsed.",
                        path.display()
                    );
                }
                return Ok(None);
            };
            if value > max {
                bail!(
                    "Config key 'policies.database_retry.{db_key}.{key}' in '{}' must be <= {max}.",
                    path.display()
                );
            }
            Ok(Some(value))
        };

        Ok(Some(AgentTraceDbRetryConfig {
            retry: PerDbRetryConfig {
                connection_open: build_policy(
                    db_key,
                    db_object,
                    "connection_open",
                    typed_db.and_then(|db| db.connection_open.as_ref()),
                )?,
                query: build_policy(
                    db_key,
                    db_object,
                    "query",
                    typed_db.and_then(|db| db.query.as_ref()),
                )?,
            },
            busy_timeout_ms: bounded_millis(
                "busy_timeout_ms",
                typed_db.and_then(|db| db.busy_timeout_ms),
                AGENT_TRACE_DB_BUSY_TIMEOUT_MAX_MS,
            )?,
            contention_deadline_ms: bounded_millis(
                "contention_deadline_ms",
                typed_db.and_then(|db| db.contention_deadline_ms),
                AGENT_TRACE_DB_CONTENTION_DEADLINE_MAX_MS,
            )?,
        }))
    };

    Ok(Some(FileConfigValue {
        value: DatabaseRetryConfig {
            local_db: build_per_db("local_db")?,
            agent_trace_db: build_agent_trace_db()?,
            auth_db: build_per_db("auth_db")?,
        },
        source,
    }))
}

pub(crate) type ParsedAgentTraceConfig = (
    Option<FileConfigValue<String>>,
    Option<FileConfigValue<String>>,
    Option<FileConfigValue<bool>>,
);

fn map_agent_trace_config(
    typed: Option<&ParsedAgentTraceConfigDocument>,
    object: &serde_json::Map<String, Value>,
    path: &Path,
    source: ConfigPathSource,
) -> Result<ParsedAgentTraceConfig> {
    let Some(agent_trace_value) = object.get("agent_trace") else {
        return Ok((None, None, None));
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
        &["repository_id", "repository_remote", "auto_sync"],
        "repository_id, repository_remote, auto_sync",
    )?;

    let repository_id = typed
        .and_then(|config| config.repository_id.clone())
        .map(|value| FileConfigValue { value, source });
    let repository_remote = typed
        .and_then(|config| config.repository_remote.clone())
        .map(|value| FileConfigValue { value, source });
    let auto_sync = typed
        .and_then(|config| config.auto_sync)
        .map(|value| FileConfigValue { value, source });

    Ok((repository_id, repository_remote, auto_sync))
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
        &["target", "optional_workflows"],
        "target, optional_workflows",
    )?;

    let raw_targets = typed.and_then(|config| config.target.as_ref());
    let raw_optional_workflows = typed.and_then(|config| config.optional_workflows.as_ref());

    if raw_targets.is_none() && raw_optional_workflows.is_none() {
        return Ok(None);
    }

    let source_description = format!("config file '{}'", path.display());

    let targets: Vec<IntegrationTargetId> = raw_targets
        .map(|raw_targets| {
            raw_targets
                .iter()
                .map(|raw| IntegrationTargetId::parse(raw, &source_description))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();

    let optional_workflows: Vec<String> = raw_optional_workflows
        .map(|raw_workflows| {
            raw_workflows
                .iter()
                .map(|raw| parse_optional_workflow_id(raw, &source_description))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();

    Ok(Some(FileConfigValue {
        value: IntegrationsConfig {
            target: targets,
            optional_workflows,
        },
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
    fn accepts_versioned_schema_declaration() {
        let schema_url = format!(
            "https://sce.crocoder.dev/v{}/config.json",
            env!("CARGO_PKG_VERSION")
        );
        parse(&format!(r#"{{"$schema":"{schema_url}"}}"#)).unwrap();
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
        assert_eq!(
            config
                .agent_trace_auto_sync
                .as_ref()
                .map(|value| value.value),
            None
        );
    }

    #[test]
    fn parses_log_to_file_boolean() {
        let config = parse(r#"{"log_to_file":false}"#).unwrap();

        assert_eq!(
            config.log_to_file.as_ref().map(|value| value.value),
            Some(false)
        );
    }

    #[test]
    fn rejects_empty_log_dir_when_file_logging_is_enabled() {
        let error = parse(r#"{"log_to_file":true,"log_dir":""}"#)
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed schema validation"), "{error}");
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

    #[test]
    fn parses_agent_trace_auto_sync() {
        let config = parse(r#"{"agent_trace":{"auto_sync":true}}"#).unwrap();

        assert_eq!(
            config
                .agent_trace_auto_sync
                .as_ref()
                .map(|value| value.value),
            Some(true)
        );
    }

    #[test]
    fn rejects_non_boolean_agent_trace_auto_sync() {
        let error = parse(r#"{"agent_trace":{"auto_sync":"true"}}"#)
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed schema validation"), "{error}");
    }
}

#[cfg(test)]
mod database_retry_config_tests {
    use std::path::Path;

    use serde_json::Value;

    use super::{parse_file_config, ConfigPathSource, FileConfig, SCE_CONFIG_SCHEMA_JSON};
    use crate::services::resilience::RetryPolicy;

    fn parse(raw: &str) -> anyhow::Result<FileConfig> {
        parse_file_config(
            raw,
            Path::new("/tmp/sce-config.json"),
            ConfigPathSource::Flag,
        )
    }

    fn parse_error(raw: &str) -> String {
        parse(raw).unwrap_err().to_string()
    }

    fn generated_database_retry_object(db_key: &str) -> Value {
        let schema: Value = serde_json::from_str(SCE_CONFIG_SCHEMA_JSON).unwrap();
        schema["properties"]["policies"]["properties"]["database_retry"]["properties"][db_key]
            .clone()
    }

    fn property_keys(object: &Value) -> Vec<String> {
        let mut keys = object["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    #[test]
    fn database_retry_agent_trace_db_accepts_contention_keys() {
        let config = parse(
            r#"{"policies":{"database_retry":{"agent_trace_db":{"busy_timeout_ms":750,"contention_deadline_ms":2000}}}}"#,
        )
        .unwrap();

        let agent_trace_db = config.database_retry.unwrap().value.agent_trace_db.unwrap();
        assert_eq!(agent_trace_db.busy_timeout_ms, Some(750));
        assert_eq!(agent_trace_db.contention_deadline_ms, Some(2000));
        assert_eq!(agent_trace_db.retry.connection_open, None);
        assert_eq!(agent_trace_db.retry.query, None);
    }

    #[test]
    fn database_retry_agent_trace_db_accepts_zero_and_upper_bounds() {
        for (busy_timeout_ms, contention_deadline_ms) in [(0, 0), (10_000, 30_000)] {
            let config = parse(&format!(
                r#"{{"policies":{{"database_retry":{{"agent_trace_db":{{"busy_timeout_ms":{busy_timeout_ms},"contention_deadline_ms":{contention_deadline_ms}}}}}}}}}"#
            ))
            .unwrap();
            let agent_trace_db = config.database_retry.unwrap().value.agent_trace_db.unwrap();
            assert_eq!(agent_trace_db.busy_timeout_ms, Some(busy_timeout_ms));
            assert_eq!(
                agent_trace_db.contention_deadline_ms,
                Some(contention_deadline_ms)
            );
        }
    }

    #[test]
    fn database_retry_agent_trace_db_omitted_contention_keys_stay_unset() {
        let config = parse(
            r#"{"policies":{"database_retry":{"agent_trace_db":{"query":{"max_attempts":3,"timeout_ms":150,"initial_backoff_ms":10,"max_backoff_ms":50}}}}}"#,
        )
        .unwrap();

        let agent_trace_db = config.database_retry.unwrap().value.agent_trace_db.unwrap();
        assert_eq!(agent_trace_db.busy_timeout_ms, None);
        assert_eq!(agent_trace_db.contention_deadline_ms, None);
    }

    #[test]
    fn database_retry_agent_trace_db_rejects_out_of_range_values() {
        for raw in [
            r#"{"policies":{"database_retry":{"agent_trace_db":{"busy_timeout_ms":10001}}}}"#,
            r#"{"policies":{"database_retry":{"agent_trace_db":{"busy_timeout_ms":-1}}}}"#,
            r#"{"policies":{"database_retry":{"agent_trace_db":{"contention_deadline_ms":30001}}}}"#,
            r#"{"policies":{"database_retry":{"agent_trace_db":{"contention_deadline_ms":-5}}}}"#,
        ] {
            let error = parse_error(raw);
            assert!(error.contains("failed schema validation"), "{error}");
        }
    }

    #[test]
    fn database_retry_agent_trace_db_rejects_wrong_type_values() {
        for raw in [
            r#"{"policies":{"database_retry":{"agent_trace_db":{"busy_timeout_ms":"500"}}}}"#,
            r#"{"policies":{"database_retry":{"agent_trace_db":{"busy_timeout_ms":1.5}}}}"#,
            r#"{"policies":{"database_retry":{"agent_trace_db":{"contention_deadline_ms":true}}}}"#,
            r#"{"policies":{"database_retry":{"agent_trace_db":{"contention_deadline_ms":null}}}}"#,
        ] {
            let error = parse_error(raw);
            assert!(error.contains("failed schema validation"), "{error}");
        }
    }

    #[test]
    fn database_retry_local_and_auth_db_reject_contention_keys() {
        for db_key in ["local_db", "auth_db"] {
            for key in ["busy_timeout_ms", "contention_deadline_ms"] {
                let error = parse_error(&format!(
                    r#"{{"policies":{{"database_retry":{{"{db_key}":{{"{key}":500}}}}}}}}"#
                ));
                assert!(
                    error.starts_with("Config file '/tmp/sce-config.json' failed schema validation against generated schema"),
                    "{error}"
                );
                assert!(error.contains(key), "{error}");
                assert!(
                    error.contains(&format!("/policies/database_retry/{db_key}")),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn database_retry_generated_schema_publishes_contention_keys_only_on_agent_trace_db() {
        for db_key in ["local_db", "auth_db"] {
            let object = generated_database_retry_object(db_key);
            assert_eq!(property_keys(&object), vec!["connection_open", "query"]);
            assert_eq!(object["additionalProperties"], Value::Bool(false));
        }

        let agent_trace_db = generated_database_retry_object("agent_trace_db");
        assert_eq!(
            property_keys(&agent_trace_db),
            vec![
                "busy_timeout_ms",
                "connection_open",
                "contention_deadline_ms",
                "query"
            ]
        );
        assert_eq!(agent_trace_db["additionalProperties"], Value::Bool(false));
        let busy_timeout = &agent_trace_db["properties"]["busy_timeout_ms"];
        assert_eq!(busy_timeout["minimum"], 0);
        assert_eq!(busy_timeout["maximum"], 10_000);
        assert_eq!(busy_timeout["default"], 1_000);
        let contention_deadline = &agent_trace_db["properties"]["contention_deadline_ms"];
        assert_eq!(contention_deadline["minimum"], 0);
        assert_eq!(contention_deadline["maximum"], 30_000);
        assert_eq!(contention_deadline["default"], 2_250);
    }

    #[test]
    fn database_retry_per_db_key_check_rejects_contention_keys_as_backstop() {
        let object = serde_json::json!({"busy_timeout_ms": 500});
        let error = super::validate_object_keys(
            object.as_object().unwrap(),
            Path::new("/tmp/sce-config.json"),
            Some("policies.database_retry.local_db"),
            super::PER_DB_RETRY_KEYS,
            super::PER_DB_RETRY_KEYS_DESCRIPTION,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error,
            "Config key 'policies.database_retry.local_db' in '/tmp/sce-config.json' contains unknown key 'busy_timeout_ms'. Allowed keys: connection_open, query."
        );
    }

    #[test]
    fn database_retry_query_timeout_ms_parsing_is_unchanged() {
        let config = parse(
            r#"{"policies":{"database_retry":{"local_db":{"query":{"max_attempts":4,"timeout_ms":300,"initial_backoff_ms":20,"max_backoff_ms":80}},"agent_trace_db":{"query":{"max_attempts":3,"timeout_ms":150,"initial_backoff_ms":10,"max_backoff_ms":50},"busy_timeout_ms":250}}}}"#,
        )
        .unwrap();

        let database_retry = config.database_retry.unwrap().value;
        assert_eq!(
            database_retry.local_db.unwrap().query,
            Some(RetryPolicy::new(4, 300, 20, 80).unwrap())
        );
        assert_eq!(
            database_retry.agent_trace_db.unwrap().retry.query,
            Some(RetryPolicy::new(3, 150, 10, 50).unwrap())
        );

        let error = parse_error(
            r#"{"policies":{"database_retry":{"agent_trace_db":{"query":{"max_attempts":3,"timeout_ms":0,"initial_backoff_ms":10,"max_backoff_ms":50}}}}}"#,
        );
        assert!(error.contains("failed schema validation"), "{error}");
    }
}
