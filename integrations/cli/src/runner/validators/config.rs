use super::super::catalog::CONFIG_PRECEDENCE_TEXT;
use super::shared::{
    assert_json_string_field_equals, assert_required_substrings,
    extract_json_bool_field_from_value, extract_json_object_field_from_value, parse_json_payload,
};

pub(super) fn validate_config_show_text_output(stream: &str) -> Result<(), String> {
    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty config show text payload".to_string());
    }

    assert_required_substrings(
        payload,
        &[
            "SCE config",
            "Precedence:",
            CONFIG_PRECEDENCE_TEXT,
            "Config files",
            "- log_level:",
            "- log_format:",
            "- log_file_mode:",
            "- otel.enabled:",
            "- timeout_ms:",
            "- workos_client_id:",
            "- policies.bash:",
            "Validation warnings:",
        ],
        "config show text",
    )
}

pub(super) fn validate_config_show_json_output(stream: &str) -> Result<(), String> {
    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty config show JSON payload".to_string());
    }

    let json = parse_json_payload(payload)?;

    assert_json_string_field_equals(&json, "status", "ok")?;

    let result = extract_json_object_field_from_value(&json, "result")?;
    let command = result
        .get("command")
        .ok_or_else(|| "missing JSON field 'result.command'".to_string())?
        .as_str()
        .ok_or_else(|| "expected JSON string value for field 'result.command'".to_string())?;
    if command != "config_show" {
        return Err(format!(
            "expected 'result.command' to equal 'config_show', got '{command}'"
        ));
    }

    let precedence = result
        .get("precedence")
        .ok_or_else(|| "missing JSON field 'result.precedence'".to_string())?;
    let _ = precedence
        .as_str()
        .ok_or_else(|| "expected JSON string value for field 'result.precedence'".to_string())?;

    let config_paths = result
        .get("config_paths")
        .ok_or_else(|| "missing JSON field 'result.config_paths'".to_string())?;
    let _ = config_paths
        .as_array()
        .ok_or_else(|| "expected JSON array value for field 'result.config_paths'".to_string())?;

    let resolved = result
        .get("resolved")
        .ok_or_else(|| "missing JSON field 'result.resolved'".to_string())?;
    let resolved_object = resolved
        .as_object()
        .ok_or_else(|| "expected JSON object value for field 'result.resolved'".to_string())?;
    for required_field in [
        "log_level",
        "log_format",
        "log_file",
        "log_file_mode",
        "otel",
        "timeout_ms",
        "workos_client_id",
        "policies",
    ] {
        if !resolved_object.contains_key(required_field) {
            return Err(format!(
                "missing JSON field 'result.resolved.{required_field}'"
            ));
        }
    }

    let warnings = result
        .get("warnings")
        .ok_or_else(|| "missing JSON field 'result.warnings'".to_string())?;
    let _ = warnings
        .as_array()
        .ok_or_else(|| "expected JSON array value for field 'result.warnings'".to_string())?;

    Ok(())
}

pub(super) fn validate_config_validate_text_output(stream: &str) -> Result<(), String> {
    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty config validate text payload".to_string());
    }

    assert_required_substrings(
        payload,
        &[
            "SCE config validation",
            "Validation issues:",
            "none",
            "Validation warnings:",
        ],
        "config validate text",
    )
}

pub(super) fn validate_config_validate_json_output(stream: &str) -> Result<(), String> {
    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty config validate JSON payload".to_string());
    }

    let json = parse_json_payload(payload)?;

    assert_json_string_field_equals(&json, "status", "ok")?;

    let result = json
        .get("result")
        .ok_or_else(|| "missing JSON field 'result'".to_string())?;
    let result_object = result
        .as_object()
        .ok_or_else(|| "expected JSON object value for field 'result'".to_string())?;

    let command = result_object
        .get("command")
        .ok_or_else(|| "missing JSON field 'result.command'".to_string())?
        .as_str()
        .ok_or_else(|| "expected JSON string value for field 'result.command'".to_string())?;
    if command != "config_validate" {
        return Err(format!(
            "expected 'result.command' to equal 'config_validate', got '{command}'"
        ));
    }

    let valid = extract_json_bool_field_from_value(result, "valid")?;

    if !valid {
        return Err("expected 'result.valid' to equal true".to_string());
    }

    let issues = result_object
        .get("issues")
        .ok_or_else(|| "missing JSON field 'result.issues'".to_string())?
        .as_array()
        .ok_or_else(|| "expected JSON array value for field 'result.issues'".to_string())?;
    if !issues.is_empty() {
        return Err("expected 'result.issues' to be an empty array".to_string());
    }

    let _ = result_object
        .get("warnings")
        .ok_or_else(|| "missing JSON field 'result.warnings'".to_string())?
        .as_array()
        .ok_or_else(|| "expected JSON array value for field 'result.warnings'".to_string())?;

    Ok(())
}
