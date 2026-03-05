use anyhow::{bail, Context, Result};
use lexopt::Arg;
use lexopt::ValueExt;
use serde_json::json;

pub const NAME: &str = "version";

const BINARY_NAME: &str = env!("CARGO_PKG_NAME");
const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionFormat {
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionRequest {
    pub format: VersionFormat,
}

pub fn version_usage_text() -> &'static str {
    "Usage:\n  sce version [--format <text|json>]\n\nExamples:\n  sce version\n  sce version --format json"
}

pub fn parse_version_request(args: Vec<String>) -> Result<VersionRequest> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut format = VersionFormat::Text;

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Long("format") => {
                let value = parser
                    .value()
                    .context("Option '--format' requires a value")?;
                let raw = value.string()?;
                format = parse_version_format(&raw)?;
            }
            Arg::Long("help") | Arg::Short('h') => {
                bail!("Use 'sce version --help' for version usage.");
            }
            Arg::Long(option) => {
                bail!(
                    "Unknown version option '--{}'. Run 'sce version --help' to see valid usage.",
                    option
                );
            }
            Arg::Short(option) => {
                bail!(
                    "Unknown version option '-{}'. Run 'sce version --help' to see valid usage.",
                    option
                );
            }
            Arg::Value(value) => {
                bail!(
                    "Unexpected version argument '{}'. Run 'sce version --help' to see valid usage.",
                    value.string()?
                );
            }
        }
    }

    Ok(VersionRequest { format })
}

pub fn render_version(request: VersionRequest) -> Result<String> {
    let build_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    let report = json!({
        "status": "ok",
        "command": NAME,
        "binary": BINARY_NAME,
        "version": PACKAGE_VERSION,
        "build_profile": build_profile,
    });

    match request.format {
        VersionFormat::Text => Ok(format!(
            "{} {} ({})",
            BINARY_NAME, PACKAGE_VERSION, build_profile
        )),
        VersionFormat::Json => serde_json::to_string_pretty(&report)
            .context("failed to serialize version report to JSON"),
    }
}

fn parse_version_format(raw: &str) -> Result<VersionFormat> {
    match raw {
        "text" => Ok(VersionFormat::Text),
        "json" => Ok(VersionFormat::Json),
        _ => bail!(
            "Unsupported --format value '{}'. Valid values: text, json.",
            raw
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{parse_version_request, render_version, VersionFormat, VersionRequest, NAME};

    #[test]
    fn parse_defaults_to_text_format() {
        let request = parse_version_request(vec![]).expect("request should parse");
        assert_eq!(request.format, VersionFormat::Text);
    }

    #[test]
    fn parse_accepts_json_format() {
        let request = parse_version_request(vec!["--format".to_string(), "json".to_string()])
            .expect("request should parse");
        assert_eq!(request.format, VersionFormat::Json);
    }

    #[test]
    fn render_json_includes_stable_fields() {
        let output = render_version(VersionRequest {
            format: VersionFormat::Json,
        })
        .expect("json render should succeed");

        let parsed: Value = serde_json::from_str(&output).expect("json output should parse");
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["command"], NAME);
        assert!(parsed["binary"].as_str().is_some());
        assert!(parsed["version"].as_str().is_some());
        assert!(parsed["build_profile"].as_str().is_some());
    }

    #[test]
    fn render_text_includes_binary_and_version() {
        let output = render_version(VersionRequest {
            format: VersionFormat::Text,
        })
        .expect("text render should succeed");
        assert!(output.contains(env!("CARGO_PKG_NAME")));
        assert!(output.contains(env!("CARGO_PKG_VERSION")));
    }
}
