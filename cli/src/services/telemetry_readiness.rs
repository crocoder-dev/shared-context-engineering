use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::services::default_paths::telemetry_readiness_path;

const TEMP_SUFFIX: &str = "tmp";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct TelemetryReadiness {
    expires_at_unix_seconds: u64,
}

pub fn mark_ready(expires_at_unix_seconds: u64) -> Result<()> {
    mark_ready_at(&telemetry_readiness_path()?, expires_at_unix_seconds)
}

pub fn clear() -> Result<()> {
    clear_at(&telemetry_readiness_path()?)
}

fn mark_ready_at(path: &Path, expires_at_unix_seconds: u64) -> Result<()> {
    let parent = path.parent().with_context(|| {
        format!(
            "telemetry readiness path '{}' has no parent",
            path.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create '{}'", parent.display()))?;
    let bytes = serde_json::to_vec(&TelemetryReadiness {
        expires_at_unix_seconds,
    })
    .context("failed to serialize telemetry readiness")?;
    let temp_path = temp_path_for(path);
    fs::write(&temp_path, bytes)
        .with_context(|| format!("failed to write '{}'", temp_path.display()))?;
    fs::rename(&temp_path, path).with_context(|| {
        let _ = fs::remove_file(&temp_path);
        format!("failed to replace '{}'", path.display())
    })
}

fn clear_at(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove '{}'", path.display())),
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}.{TEMP_SUFFIX}", std::process::id()));
    PathBuf::from(name)
}
