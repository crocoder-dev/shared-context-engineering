use anyhow::{bail, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    #[allow(dead_code)]
    pub fn parse(raw: &str, help_command: &str) -> Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => bail!(
                "Invalid --format value '{raw}'. Valid values: text, json. Run '{help_command}' to see valid usage."
            ),
        }
    }
}
