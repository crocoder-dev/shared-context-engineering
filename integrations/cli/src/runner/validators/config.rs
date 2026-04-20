use super::super::catalog::CONFIG_PRECEDENCE_TEXT;
use super::shared::{assert_json_field_equals, assert_required_substrings};

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

    assert_json_field_equals(payload, "status", "ok")?;
    assert_json_field_equals(payload, "command", "config_show")?;
    assert_required_substrings(
        payload,
        &[
            "\"result\"",
            "\"precedence\"",
            "\"config_paths\"",
            "\"resolved\"",
            "\"log_level\"",
            "\"log_format\"",
            "\"log_file\"",
            "\"log_file_mode\"",
            "\"otel\"",
            "\"timeout_ms\"",
            "\"workos_client_id\"",
            "\"policies\"",
            "\"warnings\"",
        ],
        "config show json",
    )
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

    assert_json_field_equals(payload, "status", "ok")?;
    assert_json_field_equals(payload, "command", "config_validate")?;
    assert_required_substrings(
        payload,
        &["\"valid\": true", "\"issues\": []", "\"warnings\""],
        "config validate json",
    )
}
