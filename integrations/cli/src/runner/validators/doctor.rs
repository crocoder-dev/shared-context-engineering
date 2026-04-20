use super::shared::{
    assert_json_string_field_equals, assert_non_empty_bounded_field, assert_required_substrings,
    extract_json_string_field_from_value, parse_json_payload,
};

pub(super) fn validate_doctor_text_output(stream: &str) -> Result<(), String> {
    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty doctor text payload".to_string());
    }

    assert_required_substrings(
        payload,
        &[
            "SCE doctor diagnose",
            "Environment:",
            "Configuration:",
            "Repository:",
            "Git Hooks:",
            "Integrations:",
            "Summary:",
        ],
        "doctor text",
    )
}

pub(super) fn validate_doctor_json_output(stream: &str) -> Result<(), String> {
    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty doctor JSON payload".to_string());
    }

    let json = parse_json_payload(payload)?;

    assert_json_string_field_equals(&json, "status", "ok")?;
    assert_json_string_field_equals(&json, "command", "doctor")?;
    assert_json_string_field_equals(&json, "mode", "diagnose")?;

    let readiness = extract_json_string_field_from_value(&json, "readiness")?;
    if readiness != "ready" && readiness != "not_ready" {
        return Err(format!(
            "expected 'readiness' to be 'ready' or 'not_ready', got '{readiness}'"
        ));
    }

    let hook_path_source = extract_json_string_field_from_value(&json, "hook_path_source")?;
    assert_non_empty_bounded_field("hook_path_source", hook_path_source, 64)?;

    let _ = json
        .get("config_paths")
        .ok_or_else(|| "missing JSON field 'config_paths'".to_string())?
        .as_array()
        .ok_or_else(|| "expected JSON array value for field 'config_paths'".to_string())?;

    let _ = json
        .get("hooks")
        .ok_or_else(|| "missing JSON field 'hooks'".to_string())?
        .as_array()
        .ok_or_else(|| "expected JSON array value for field 'hooks'".to_string())?;

    let _ = json
        .get("problems")
        .ok_or_else(|| "missing JSON field 'problems'".to_string())?
        .as_array()
        .ok_or_else(|| "expected JSON array value for field 'problems'".to_string())?;

    let fix_results = json
        .get("fix_results")
        .ok_or_else(|| "missing JSON field 'fix_results'".to_string())?
        .as_array()
        .ok_or_else(|| "expected JSON array value for field 'fix_results'".to_string())?;
    if !fix_results.is_empty() {
        return Err("expected 'fix_results' to be an empty array in diagnose mode".to_string());
    }

    Ok(())
}
