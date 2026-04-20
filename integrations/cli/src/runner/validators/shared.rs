pub(super) fn assert_non_empty_payload(stream: &str, contract_name: &str) -> Result<(), String> {
    if stream.trim().is_empty() {
        return Err(format!("expected non-empty {contract_name} payload"));
    }

    Ok(())
}

pub(super) fn assert_required_substrings(
    payload: &str,
    required_substrings: &[&str],
    contract_name: &str,
) -> Result<(), String> {
    for required in required_substrings {
        if !payload.contains(required) {
            return Err(format!(
                "expected {contract_name} payload to contain '{required}'"
            ));
        }
    }

    Ok(())
}

pub(super) fn assert_json_field_equals(
    payload: &str,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = extract_json_string_field(payload, field)?;
    if actual == expected {
        return Ok(());
    }

    Err(format!(
        "expected '{field}' to equal '{expected}', got '{actual}'"
    ))
}

pub(super) fn assert_non_empty_bounded_field(
    field: &str,
    value: &str,
    max_length: usize,
) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("expected '{field}' to be non-empty"));
    }

    if value.len() > max_length {
        return Err(format!(
            "expected '{field}' length <= {max_length}, got {}",
            value.len()
        ));
    }

    Ok(())
}

pub(super) fn extract_json_string_field(payload: &str, field: &str) -> Result<String, String> {
    let field_token = format!("\"{field}\"");
    let field_start = payload
        .find(&field_token)
        .ok_or_else(|| format!("missing JSON string field '{field}'"))?;
    let after_field = &payload[field_start + field_token.len()..];
    let colon_offset = after_field
        .find(':')
        .ok_or_else(|| format!("missing ':' after JSON field '{field}'"))?;
    let after_colon = after_field[colon_offset + 1..].trim_start();

    if !after_colon.starts_with('"') {
        return Err(format!("expected JSON string value for field '{field}'"));
    }

    let mut value = String::new();
    let mut escaped = false;

    for character in after_colon[1..].chars() {
        if escaped {
            value.push(character);
            escaped = false;
            continue;
        }

        if character == '\\' {
            escaped = true;
            continue;
        }

        if character == '"' {
            return Ok(value);
        }

        value.push(character);
    }

    Err(format!(
        "unterminated JSON string value for field '{field}'"
    ))
}
