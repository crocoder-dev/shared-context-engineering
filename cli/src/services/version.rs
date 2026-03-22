use anyhow::{Context, Result};
use serde_json::json;

use crate::services::output_format::OutputFormat;
use crate::services::style::{self};

pub const NAME: &str = "version";

const BINARY_NAME: &str = env!("CARGO_PKG_NAME");
const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_COMMIT: &str = match option_env!("SCE_GIT_COMMIT") {
    Some(commit) => commit,
    None => "unknown",
};

pub type VersionFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionRequest {
    pub format: VersionFormat,
}

pub fn render_version(request: VersionRequest) -> Result<String> {
    let report = json!({
        "status": "ok",
        "command": NAME,
        "binary": BINARY_NAME,
        "version": PACKAGE_VERSION,
        "git_commit": GIT_COMMIT,
    });

    match request.format {
        VersionFormat::Text => Ok(format!(
            "{} {} ({})",
            style::command_name(BINARY_NAME),
            style::value(PACKAGE_VERSION),
            style::value(GIT_COMMIT)
        )),
        VersionFormat::Json => serde_json::to_string_pretty(&report)
            .context("failed to serialize version report to JSON"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{render_version, VersionFormat, VersionRequest, GIT_COMMIT, NAME};

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
        assert_eq!(parsed["git_commit"], GIT_COMMIT);
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
    fn render_text_includes_binary_version_and_git_commit() {
        let output = render_version(VersionRequest {
            format: VersionFormat::Text,
        })
        .expect("text render should succeed");
        assert_eq!(
            output,
            format!(
                "{} {} ({})",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
                GIT_COMMIT
            )
        );
    }
}
