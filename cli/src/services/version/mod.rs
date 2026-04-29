pub mod command;

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
