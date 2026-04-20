use serde_json::{Map, Value};

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

pub(super) fn parse_json_payload(payload: &str) -> Result<Value, String> {
    serde_json::from_str(payload).map_err(|error| format!("failed to parse JSON payload: {error}"))
}

pub(super) fn assert_json_string_field_equals(
    json: &Value,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = extract_json_string_field_from_value(json, field)?;
    if actual == expected {
        return Ok(());
    }

    Err(format!(
        "expected '{field}' to equal '{expected}', got '{actual}'"
    ))
}

pub(super) fn extract_json_string_field_from_value<'a>(
    json: &'a Value,
    field: &str,
) -> Result<&'a str, String> {
    let value = extract_required_json_field(json, field)?;
    value
        .as_str()
        .ok_or_else(|| format!("expected JSON string value for field '{field}'"))
}

pub(super) fn extract_json_bool_field_from_value(
    json: &Value,
    field: &str,
) -> Result<bool, String> {
    let value = extract_required_json_field(json, field)?;
    value
        .as_bool()
        .ok_or_else(|| format!("expected JSON boolean value for field '{field}'"))
}

pub(super) fn extract_json_object_field_from_value<'a>(
    json: &'a Value,
    field: &str,
) -> Result<&'a Map<String, Value>, String> {
    let value = extract_required_json_field(json, field)?;
    value
        .as_object()
        .ok_or_else(|| format!("expected JSON object value for field '{field}'"))
}

fn extract_required_json_field<'a>(json: &'a Value, field: &str) -> Result<&'a Value, String> {
    let object = json
        .as_object()
        .ok_or_else(|| "expected top-level JSON object payload".to_string())?;

    object
        .get(field)
        .ok_or_else(|| format!("missing JSON field '{field}'"))
}
