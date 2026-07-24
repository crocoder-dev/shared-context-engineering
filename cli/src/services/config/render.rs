use serde_json::{json, Value};

use crate::services::style;

use super::policy::{format_bash_policies_json, format_bash_policies_text};
use super::resolver::{
    AuthConfigKeySpec, RuntimeConfig, PRECEDENCE_DESCRIPTION, WORKOS_CLIENT_ID_KEY,
};
use super::types::DatabaseRetryConfig;
use super::{ConfigPathSource, ReportFormat, ResolvedOptionalValue, ValueSource};

pub(super) fn format_show_output(runtime: &RuntimeConfig, report_format: ReportFormat) -> String {
    let warnings = build_show_warnings(runtime);
    match report_format {
        ReportFormat::Text => {
            let mut lines = vec![
                format!(
                    "{}: {}",
                    style::success("SCE config"),
                    style::value("resolved")
                ),
                format!(
                    "{}: {}",
                    style::label("Precedence"),
                    style::value(PRECEDENCE_DESCRIPTION)
                ),
                format_config_paths_text(runtime),
                format_resolved_value_text(
                    "timeout_ms",
                    &runtime.timeout_ms.value.to_string(),
                    runtime.timeout_ms.source,
                ),
                format_optional_auth_resolved_value_text(
                    WORKOS_CLIENT_ID_KEY,
                    &runtime.workos_client_id,
                ),
                format_optional_resolved_value_text(
                    "agent_trace.repository_id",
                    &runtime.agent_trace_repository_id,
                ),
                format_resolved_value_text(
                    "agent_trace.repository_remote",
                    &runtime.agent_trace_repository_remote.value,
                    runtime.agent_trace_repository_remote.source,
                ),
                format_bash_policies_text(&runtime.bash_policies),
                format_database_retry_text(&runtime.database_retry),
                format_validation_warnings_text(&warnings),
            ];
            lines.splice(3..3, format_observability_text_lines(runtime));
            lines.join("\n")
        }
        ReportFormat::Json => {
            let payload = json!({
                "status": "ok",
                "result": {
                    "command": "config_show",
                    "precedence": PRECEDENCE_DESCRIPTION,
                    "config_paths": format_config_paths_json(runtime),
                    "resolved": {
                        "log_level": format_resolved_value_json(
                            runtime.log_level.value.as_str(),
                            runtime.log_level.source,
                        ),
                        "log_format": format_resolved_value_json(
                            runtime.log_format.value.as_str(),
                            runtime.log_format.source,
                        ),
                        "log_dir": format_optional_resolved_value_json(&runtime.log_dir),
                        "log_file_retention_limit": format_resolved_value_json(
                            runtime.log_file_retention_limit.value,
                            runtime.log_file_retention_limit.source,
                        ),
                        "timeout_ms": {
                            "value": runtime.timeout_ms.value,
                            "source": runtime.timeout_ms.source.as_str(),
                            "config_source": runtime.timeout_ms.source.config_source().map(ConfigPathSource::as_str),
                        },
                        "workos_client_id": format_optional_auth_resolved_value_json(WORKOS_CLIENT_ID_KEY, &runtime.workos_client_id),
                        "agent_trace": {
                            "repository_id": format_optional_resolved_value_json(&runtime.agent_trace_repository_id),
                            "repository_remote": format_resolved_value_json(
                                runtime.agent_trace_repository_remote.value.as_str(),
                                runtime.agent_trace_repository_remote.source,
                            ),
                        },
                        "policies": {
                            "bash": format_bash_policies_json(&runtime.bash_policies),
                            "database_retry": format_database_retry_json(&runtime.database_retry),
                        }
                    },
                    "warnings": warnings,
                }
            });
            serde_json::to_string_pretty(&payload).expect("config show payload should serialize")
        }
    }
}

pub(super) fn format_validate_output(
    runtime: &RuntimeConfig,
    report_format: ReportFormat,
) -> String {
    let valid = runtime.validation_errors.is_empty();
    match report_format {
        ReportFormat::Text => {
            let lines = [
                format!(
                    "{}: {}",
                    style::success("SCE config validation"),
                    style::value(if valid { "valid" } else { "invalid" })
                ),
                format_validation_issues_text(&runtime.validation_errors),
                format_validation_warnings_text(&runtime.validation_warnings),
            ];
            lines.join("\n")
        }
        ReportFormat::Json => {
            let payload = json!({
                "status": "ok",
                "result": {
                    "command": "config_validate",
                    "valid": valid,
                    "issues": runtime.validation_errors,
                    "warnings": runtime.validation_warnings,
                }
            });
            serde_json::to_string_pretty(&payload)
                .expect("config validate payload should serialize")
        }
    }
}

fn format_config_paths_text(runtime: &RuntimeConfig) -> String {
    if runtime.loaded_config_paths.is_empty() {
        return format!(
            "{}: {}",
            style::label("Config files"),
            style::value("(none discovered)")
        );
    }

    let mut lines = vec![format!("{}:", style::label("Config files"))];
    for path in &runtime.loaded_config_paths {
        lines.push(format!(
            "  - {} (source: {})",
            style::value(&path.path.display().to_string()),
            style::label(path.source.as_str())
        ));
    }
    lines.join("\n")
}

fn format_config_paths_json(runtime: &RuntimeConfig) -> Value {
    Value::Array(
        runtime
            .loaded_config_paths
            .iter()
            .map(|path| {
                json!({
                "path": path.path.display().to_string(),
                "source": path.source.as_str(),
                    })
            })
            .collect(),
    )
}

fn build_show_warnings(runtime: &RuntimeConfig) -> Vec<String> {
    let mut warnings = runtime
        .validation_errors
        .iter()
        .map(|error| format!("Skipped invalid config: {error}"))
        .collect::<Vec<_>>();
    warnings.extend(runtime.validation_warnings.iter().cloned());
    warnings
}

fn format_validation_issues_text(issues: &[String]) -> String {
    if issues.is_empty() {
        return format!(
            "{}: {}",
            style::label("Validation issues"),
            style::value("none")
        );
    }

    format!(
        "{}: {}",
        style::label("Validation issues"),
        style::value(&issues.join(" | "))
    )
}

fn format_validation_warnings_text(warnings: &[String]) -> String {
    if warnings.is_empty() {
        return format!(
            "{}: {}",
            style::label("Validation warnings"),
            style::value("none")
        );
    }

    format!(
        "{}: {}",
        style::label("Validation warnings"),
        style::value(&warnings.join(" | "))
    )
}

fn format_observability_text_lines(runtime: &RuntimeConfig) -> Vec<String> {
    vec![
        format_resolved_value_text(
            "log_level",
            runtime.log_level.value.as_str(),
            runtime.log_level.source,
        ),
        format_resolved_value_text(
            "log_format",
            runtime.log_format.value.as_str(),
            runtime.log_format.source,
        ),
        format_optional_resolved_value_text("log_dir", &runtime.log_dir),
        format_resolved_value_text(
            "log_file_retention_limit",
            &runtime.log_file_retention_limit.value.to_string(),
            runtime.log_file_retention_limit.source,
        ),
    ]
}

fn format_resolved_value_json<T>(value: T, source: ValueSource) -> Value
where
    T: serde::Serialize,
{
    json!({
        "value": value,
        "source": source.as_str(),
        "config_source": source.config_source().map(ConfigPathSource::as_str),
    })
}

fn format_resolved_value_text(key: &str, value_text: &str, source: ValueSource) -> String {
    match source.config_source() {
        Some(config_source) => format!(
            "- {}: {} (source: {}, config_source: {})",
            style::label(key),
            style::value(value_text),
            style::label(source.as_str()),
            style::label(config_source.as_str())
        ),
        None => format!(
            "- {}: {} (source: {})",
            style::label(key),
            style::value(value_text),
            style::label(source.as_str())
        ),
    }
}

fn format_optional_resolved_value_text(key: &str, value: &ResolvedOptionalValue<String>) -> String {
    match (value.value.as_deref(), value.source) {
        (Some(raw_value), Some(source)) => match source.config_source() {
            Some(config_source) => format!(
                "- {}: {} (source: {}, config_source: {})",
                style::label(key),
                style::value(raw_value),
                style::label(source.as_str()),
                style::label(config_source.as_str())
            ),
            None => format!(
                "- {}: {} (source: {})",
                style::label(key),
                style::value(raw_value),
                style::label(source.as_str())
            ),
        },
        _ => format!(
            "- {}: {} (source: {})",
            style::label(key),
            style::value("(unset)"),
            style::label("none")
        ),
    }
}

fn format_optional_resolved_value_json(value: &ResolvedOptionalValue<String>) -> Value {
    json!({
        "value": value.value,
        "source": value.source.map(ValueSource::as_str),
        "config_source": value.source.and_then(ValueSource::config_source).map(ConfigPathSource::as_str),
    })
}

fn format_optional_auth_resolved_value_text(
    key: AuthConfigKeySpec,
    value: &ResolvedOptionalValue<String>,
) -> String {
    match (value.value.as_deref(), value.source) {
        (Some(raw_value), Some(source)) => {
            let display_value = format_text_display_value(key.config_key, raw_value);
            match source.config_source() {
                Some(config_source) => format!(
                    "- {}: {} (source: {}, config_source: {}, auth_precedence: {})",
                    style::label(key.config_key),
                    style::value(&display_value),
                    style::label(source.as_str()),
                    style::label(config_source.as_str()),
                    style::value(&key.precedence_description())
                ),
                None => format!(
                    "- {}: {} (source: {}, auth_precedence: {})",
                    style::label(key.config_key),
                    style::value(&display_value),
                    style::label(source.as_str()),
                    style::value(&key.precedence_description())
                ),
            }
        }
        _ => format!(
            "- {}: {} (source: {}, auth_precedence: {})",
            style::label(key.config_key),
            style::value("(unset)"),
            style::label("none"),
            style::value(&key.precedence_description())
        ),
    }
}

fn format_optional_auth_resolved_value_json(
    key: AuthConfigKeySpec,
    value: &ResolvedOptionalValue<String>,
) -> Value {
    json!({
        "value": value.value,
        "display_value": value.value.as_deref().map(|raw| format_text_display_value(key.config_key, raw)),
        "source": value.source.map(ValueSource::as_str),
        "config_source": value.source.and_then(ValueSource::config_source).map(ConfigPathSource::as_str),
        "precedence": key.precedence_description(),
    })
}

fn format_text_display_value(key: &str, value: &str) -> String {
    if should_fully_redact_text_value(key) {
        return String::from("[REDACTED]");
    }

    if looks_credential_like(value) {
        return abbreviate_text_value(value);
    }

    value.to_string()
}

fn should_fully_redact_text_value(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["password", "passwd", "secret", "token", "api_key", "apikey"]
        .iter()
        .any(|needle| key.contains(needle))
}

fn looks_credential_like(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 16
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
}

fn abbreviate_text_value(value: &str) -> String {
    let total = value.chars().count();
    if total <= 8 {
        return value.to_string();
    }

    let prefix: String = value.chars().take(4).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

fn retry_policy_display(policy: &crate::services::resilience::RetryPolicy) -> String {
    format!(
        "{} attempts, {}ms timeout, {}..{}ms backoff",
        policy.max_attempts, policy.timeout_ms, policy.initial_backoff_ms, policy.max_backoff_ms
    )
}

fn format_per_db_retry_text(
    config: &super::types::PerDbRetryConfig,
    db_label: &str,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(ref policy) = config.connection_open {
        lines.push(format!(
            "      {}: {} (connection_open)",
            style::label(db_label),
            style::value(&retry_policy_display(policy))
        ));
    }
    if let Some(ref policy) = config.query {
        lines.push(format!(
            "      {}: {} (query)",
            style::label(db_label),
            style::value(&retry_policy_display(policy))
        ));
    }
    lines
}

fn format_database_retry_text(value: &ResolvedOptionalValue<DatabaseRetryConfig>) -> String {
    match (value.value.as_ref(), value.source) {
        (Some(config), Some(source)) => {
            let mut lines = vec![format!("  {}:", style::label("policies.database_retry"))];
            if let Some(ref per_db) = config.local_db {
                lines.extend(format_per_db_retry_text(per_db, "local_db"));
            }
            if let Some(ref per_db) = config.agent_trace_db {
                lines.extend(format_per_db_retry_text(per_db, "agent_trace_db"));
            }
            if let Some(ref per_db) = config.auth_db {
                lines.extend(format_per_db_retry_text(per_db, "auth_db"));
            }
            match source.config_source() {
                Some(config_source) => {
                    lines.push(format!(
                        "    (source: {}, config_source: {})",
                        style::label(source.as_str()),
                        style::label(config_source.as_str())
                    ));
                }
                None => {
                    lines.push(format!("    (source: {})", style::label(source.as_str())));
                }
            }
            lines.join("\n")
        }
        _ => format!(
            "  {}: {} (source: {})",
            style::label("policies.database_retry"),
            style::value("(unset)"),
            style::label("none")
        ),
    }
}

fn format_per_db_retry_json(config: &super::types::PerDbRetryConfig) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(ref policy) = config.connection_open {
        obj.insert(
            "connection_open".to_string(),
            json!({
                "max_attempts": policy.max_attempts,
                "timeout_ms": policy.timeout_ms,
                "initial_backoff_ms": policy.initial_backoff_ms,
                "max_backoff_ms": policy.max_backoff_ms,
            }),
        );
    }
    if let Some(ref policy) = config.query {
        obj.insert(
            "query".to_string(),
            json!({
                "max_attempts": policy.max_attempts,
                "timeout_ms": policy.timeout_ms,
                "initial_backoff_ms": policy.initial_backoff_ms,
                "max_backoff_ms": policy.max_backoff_ms,
            }),
        );
    }
    Value::Object(obj)
}

fn format_database_retry_json(value: &ResolvedOptionalValue<DatabaseRetryConfig>) -> Value {
    let config = value.value.as_ref();
    let mut resolved = serde_json::Map::new();
    if let Some(c) = config {
        if let Some(ref per_db) = c.local_db {
            resolved.insert("local_db".to_string(), format_per_db_retry_json(per_db));
        }
        if let Some(ref per_db) = c.agent_trace_db {
            resolved.insert(
                "agent_trace_db".to_string(),
                format_per_db_retry_json(per_db),
            );
        }
        if let Some(ref per_db) = c.auth_db {
            resolved.insert("auth_db".to_string(), format_per_db_retry_json(per_db));
        }
    }
    json!({
        "resolved": if resolved.is_empty() { Value::Null } else { Value::Object(resolved) },
        "source": value.source.map(ValueSource::as_str),
        "config_source": value.source.and_then(ValueSource::config_source).map(ConfigPathSource::as_str),
    })
}
