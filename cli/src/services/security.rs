use anyhow::Result;

const REDACTED: &str = "[REDACTED]";
const SENSITIVE_KEYS: &[&str] = &["password", "passwd", "secret", "token", "api_key", "apikey"];

pub fn redact_sensitive_text(input: &str) -> String {
    let mut output = input.to_string();
    output = redact_bearer_token(&output);
    output = redact_json_value(&output, "authorization");
    output = redact_authorization_value(&output);

    for key in SENSITIVE_KEYS {
        output = redact_json_value(&output, key);
        output = redact_assignment_value(&output, key, '=');
        output = redact_assignment_value(&output, key, ':');
    }

    output
}

fn redact_authorization_value(input: &str) -> String {
    let mut output = input.to_string();
    output = redact_assignment_value(&output, "authorization", '=');
    output = redact_assignment_value(&output, "authorization", ':');
    output
}

fn redact_bearer_token(input: &str) -> String {
    let mut output = input.to_string();
    let mut search_start = 0usize;

    loop {
        let lower = output.to_lowercase();
        let Some(relative_start) = lower[search_start..].find("bearer ") else {
            break;
        };
        let token_start = search_start + relative_start + "bearer ".len();
        let token_end = find_token_end(&output, token_start);
        if token_end == token_start {
            break;
        }

        output.replace_range(token_start..token_end, REDACTED);
        search_start = token_start + REDACTED.len();
    }

    output
}

fn redact_json_value(input: &str, key: &str) -> String {
    let mut output = input.to_string();
    let mut search_start = 0usize;
    let needle = format!("\"{key}\":\"");
    let needle_len = needle.len();

    loop {
        let lower = output.to_lowercase();
        let Some(relative_start) = lower[search_start..].find(&needle) else {
            break;
        };
        let value_start = search_start + relative_start + needle_len;
        let Some(relative_end) = output[value_start..].find('"') else {
            break;
        };
        let value_end = value_start + relative_end;
        output.replace_range(value_start..value_end, REDACTED);
        search_start = value_start + REDACTED.len();
    }

    output
}

fn redact_assignment_value(input: &str, key: &str, separator: char) -> String {
    let mut output = input.to_string();
    let mut search_start = 0usize;
    let needle = format!("{key}{separator}");

    loop {
        let lower = output.to_lowercase();
        let Some(relative_start) = lower[search_start..].find(&needle) else {
            break;
        };

        let mut value_start = search_start + relative_start + needle.len();
        while let Some(next) = output[value_start..].chars().next() {
            if next == ' ' || next == '\t' {
                value_start += next.len_utf8();
                continue;
            }
            break;
        }

        let Some(first_char) = output[value_start..].chars().next() else {
            break;
        };

        let (replace_start, replace_end) = if first_char == '"' || first_char == '\'' {
            let quote = first_char;
            let quoted_value_start = value_start + quote.len_utf8();
            let Some(relative_end) = output[quoted_value_start..].find(quote) else {
                break;
            };
            let quoted_value_end = quoted_value_start + relative_end;
            (quoted_value_start, quoted_value_end)
        } else {
            if key.eq_ignore_ascii_case("authorization")
                && output[value_start..].to_lowercase().starts_with("bearer ")
            {
                let bearer_token_start = value_start + "bearer ".len();
                let bearer_token_end = find_token_end(&output, bearer_token_start);
                (bearer_token_start, bearer_token_end)
            } else {
                let plain_value_end = find_token_end(&output, value_start);
                (value_start, plain_value_end)
            }
        };

        if replace_end == replace_start {
            break;
        }

        output.replace_range(replace_start..replace_end, REDACTED);
        search_start = replace_start + REDACTED.len();
    }

    output
}

fn find_token_end(input: &str, start: usize) -> usize {
    for (offset, ch) in input[start..].char_indices() {
        if ch.is_whitespace() || matches!(ch, ',' | ';' | ')') {
            return start + offset;
        }
    }

    input.len()
}

pub fn ensure_directory_is_writable(path: &std::path::Path, context: &str) -> Result<()> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        anyhow::anyhow!(
            "Failed to inspect {} '{}': {}",
            context,
            path.display(),
            error
        )
    })?;

    if !metadata.is_dir() {
        anyhow::bail!("{} '{}' is not a directory", context, path.display());
    }

    let probe_name = format!(
        ".sce-write-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| anyhow::anyhow!("System clock is before UNIX_EPOCH: {}", error))?
            .as_nanos()
    );

    let probe_path = path.join(probe_name);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
        .map_err(|error| {
            anyhow::anyhow!(
                "Failed to verify write permissions for {} '{}': {}. Try: grant write access and retry.",
                context,
                path.display(),
                error
            )
        })?;

    let _ = std::fs::remove_file(&probe_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::redact_sensitive_text;

    #[test]
    fn redacts_assignment_values() {
        let message = "failed password=hunter2 token=abc123";
        assert_eq!(
            redact_sensitive_text(message),
            "failed password=[REDACTED] token=[REDACTED]"
        );
    }

    #[test]
    fn redacts_json_values() {
        let message = "{\"api_key\":\"secret-value\",\"status\":\"ok\"}";
        assert_eq!(
            redact_sensitive_text(message),
            "{\"api_key\":\"[REDACTED]\",\"status\":\"ok\"}"
        );
    }

    #[test]
    fn redacts_bearer_tokens() {
        let message = "authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        assert_eq!(
            redact_sensitive_text(message),
            "authorization: Bearer [REDACTED]"
        );
    }
}
