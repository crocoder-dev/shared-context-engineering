use super::shared::{
    assert_json_field_equals, assert_required_substrings, extract_json_string_field,
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

    assert_json_field_equals(payload, "status", "ok")?;
    assert_json_field_equals(payload, "command", "doctor")?;
    assert_json_field_equals(payload, "mode", "diagnose")?;

    let readiness = extract_json_string_field(payload, "readiness")?;
    if readiness != "ready" && readiness != "not_ready" {
        return Err(format!(
            "expected 'readiness' to be 'ready' or 'not_ready', got '{readiness}'"
        ));
    }

    assert_required_substrings(
        payload,
        &[
            "\"hook_path_source\"",
            "\"config_paths\"",
            "\"hooks\"",
            "\"problems\"",
            "\"fix_results\": []",
        ],
        "doctor json",
    )
}
