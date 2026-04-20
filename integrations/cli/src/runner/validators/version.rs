use super::shared::{
    assert_json_string_field_equals, assert_non_empty_bounded_field,
    extract_json_string_field_from_value, parse_json_payload,
};

pub(super) fn validate_version_text_output(stream: &str) -> Result<(), String> {
    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty text payload".to_string());
    }

    let mut parts = payload.splitn(3, ' ');
    let binary = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    let profile = parts.next().unwrap_or_default();

    if binary.is_empty() {
        return Err("expected non-empty binary segment".to_string());
    }
    if binary.chars().any(char::is_whitespace) {
        return Err("expected binary segment without whitespace".to_string());
    }

    if version.is_empty() {
        return Err("expected non-empty version segment".to_string());
    }
    if version.chars().any(char::is_whitespace) {
        return Err("expected version segment without whitespace".to_string());
    }

    if !profile.starts_with('(') || !profile.ends_with(')') || profile.len() <= 2 {
        return Err("expected profile segment formatted as '(...)'".to_string());
    }

    Ok(())
}

pub(super) fn validate_version_json_output(stream: &str) -> Result<(), String> {
    const MAX_DYNAMIC_FIELD_LENGTH: usize = 64;

    let payload = stream.trim();
    if payload.is_empty() {
        return Err("expected non-empty JSON payload".to_string());
    }

    let json = parse_json_payload(payload)?;

    assert_json_string_field_equals(&json, "status", "ok")?;
    assert_json_string_field_equals(&json, "command", "version")?;
    assert_json_string_field_equals(&json, "binary", "shared-context-engineering")?;

    let version = extract_json_string_field_from_value(&json, "version")?;
    assert_non_empty_bounded_field("version", version, MAX_DYNAMIC_FIELD_LENGTH)?;
    if !version
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || ".-+".contains(character))
    {
        return Err(
            "expected 'version' to contain only ASCII alphanumeric characters or one of: '.', '-', '+'"
                .to_string(),
        );
    }
    if !version.chars().any(|character| character.is_ascii_digit()) {
        return Err("expected 'version' to contain at least one digit".to_string());
    }

    let git_commit = extract_json_string_field_from_value(&json, "git_commit")?;
    assert_non_empty_bounded_field("git_commit", git_commit, MAX_DYNAMIC_FIELD_LENGTH)?;
    if !git_commit
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(
            "expected 'git_commit' to contain only ASCII alphanumeric characters or one of: '.', '_', '-'"
                .to_string(),
        );
    }

    Ok(())
}
