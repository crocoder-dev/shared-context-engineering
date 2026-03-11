use anyhow::{Context, Result};
use serde_json::json;

use crate::services::output_format::OutputFormat;

pub const NAME: &str = "version";

const BINARY_NAME: &str = env!("CARGO_PKG_NAME");
const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub type VersionFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionRequest {
    pub format: VersionFormat,
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
        VersionFormat::Text => Ok(format!("{BINARY_NAME} {PACKAGE_VERSION} ({build_profile})")),
        VersionFormat::Json => serde_json::to_string_pretty(&report)
            .context("failed to serialize version report to JSON"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{render_version, VersionFormat, VersionRequest, NAME};

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
    fn render_json_is_deterministic_for_same_request() {
        let first = render_version(VersionRequest {
            format: VersionFormat::Json,
        })
        .expect("first json render should succeed");
        let second = render_version(VersionRequest {
            format: VersionFormat::Json,
        })
        .expect("second json render should succeed");

        assert_eq!(first, second);
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
