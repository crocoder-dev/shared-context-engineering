use anyhow::{bail, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub fn parse(raw: &str, help_command: &str) -> Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => bail!(
                "Invalid --format value '{}'. Valid values: text, json. Run '{}' to see valid usage.",
                raw,
                help_command
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutputFormat;

    #[test]
    fn parser_accepts_text_and_json() {
        assert_eq!(
            OutputFormat::parse("text", "sce version --help").expect("text should parse"),
            OutputFormat::Text
        );
        assert_eq!(
            OutputFormat::parse("json", "sce version --help").expect("json should parse"),
            OutputFormat::Json
        );
    }

    #[test]
    fn parser_rejects_unknown_format_with_help_guidance() {
        let error = OutputFormat::parse("xml", "sce config --help")
            .expect_err("unknown format should fail");
        assert_eq!(
            error.to_string(),
            "Invalid --format value 'xml'. Valid values: text, json. Run 'sce config --help' to see valid usage."
        );
    }
}
