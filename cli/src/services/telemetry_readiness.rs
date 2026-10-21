use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::services::default_paths::{auth_db_path, telemetry_readiness_path};

const TEMP_SUFFIX: &str = "tmp";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessState {
    NotConfigured,
    Expired,
    Ready,
}

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

pub fn is_ready(now_unix_seconds: u64) -> bool {
    state(now_unix_seconds).is_ok_and(|state| state == ReadinessState::Ready)
}

pub(crate) fn state(now_unix_seconds: u64) -> Result<ReadinessState> {
    Ok(state_at(
        &telemetry_readiness_path()?,
        &auth_db_path()?,
        now_unix_seconds,
    ))
}

pub(crate) fn mark_ready_at(path: &Path, expires_at_unix_seconds: u64) -> Result<()> {
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

pub(crate) fn clear_at(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove '{}'", path.display())),
    }
}

#[cfg(test)]
pub(crate) fn is_ready_at(path: &Path, auth_db_path: &Path, now_unix_seconds: u64) -> bool {
    state_at(path, auth_db_path, now_unix_seconds) == ReadinessState::Ready
}

pub(crate) fn state_at(path: &Path, auth_db_path: &Path, now_unix_seconds: u64) -> ReadinessState {
    if !auth_db_path.is_file() {
        return ReadinessState::NotConfigured;
    }
    match fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<TelemetryReadiness>(&bytes).ok())
    {
        None => ReadinessState::NotConfigured,
        Some(readiness) if readiness.expires_at_unix_seconds > now_unix_seconds => {
            ReadinessState::Ready
        }
        Some(_) => ReadinessState::Expired,
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}.{TEMP_SUFFIX}", std::process::id()));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000;

    struct Paths {
        _dir: tempfile::TempDir,
        readiness: PathBuf,
        auth_db: PathBuf,
    }

    fn paths_with_auth_db() -> Paths {
        let dir = tempfile::tempdir().unwrap();
        let readiness = dir.path().join("sce").join("telemetry-ready.json");
        let auth_db = dir.path().join("sce").join("auth.db");
        fs::create_dir_all(auth_db.parent().unwrap()).unwrap();
        fs::write(&auth_db, b"encrypted").unwrap();
        Paths {
            _dir: dir,
            readiness,
            auth_db,
        }
    }

    #[test]
    fn readiness_state_requires_valid_unexpired_marker_and_auth_db() {
        for (case, marker, auth_db, expected) in [
            ("missing marker", None, true, ReadinessState::NotConfigured),
            (
                "malformed marker",
                Some(b"not json".to_vec()),
                true,
                ReadinessState::NotConfigured,
            ),
            (
                "valid marker",
                Some(format!(r#"{{"expires_at_unix_seconds":{}}}"#, NOW + 1).into_bytes()),
                true,
                ReadinessState::Ready,
            ),
            (
                "expired marker",
                Some(format!(r#"{{"expires_at_unix_seconds":{NOW}}}"#).into_bytes()),
                true,
                ReadinessState::Expired,
            ),
            (
                "missing auth DB",
                Some(format!(r#"{{"expires_at_unix_seconds":{}}}"#, NOW + 1).into_bytes()),
                false,
                ReadinessState::NotConfigured,
            ),
        ] {
            let paths = paths_with_auth_db();
            if let Some(marker) = marker {
                fs::write(&paths.readiness, marker).unwrap();
            }
            if !auth_db {
                fs::remove_file(&paths.auth_db).unwrap();
            }
            assert_eq!(
                state_at(&paths.readiness, &paths.auth_db, NOW),
                expected,
                "{case}"
            );
        }
    }

    #[test]
    fn marked_readiness_is_ready_until_expiry_and_holds_no_secret() {
        let paths = paths_with_auth_db();
        mark_ready_at(&paths.readiness, NOW + 1).unwrap();

        assert!(is_ready_at(&paths.readiness, &paths.auth_db, NOW));
        assert!(!is_ready_at(&paths.readiness, &paths.auth_db, NOW + 1));
        assert_eq!(
            fs::read_to_string(&paths.readiness).unwrap(),
            format!(r#"{{"expires_at_unix_seconds":{}}}"#, NOW + 1)
        );
    }

    #[test]
    fn clearing_readiness_disables_it_and_is_idempotent() {
        let paths = paths_with_auth_db();
        mark_ready_at(&paths.readiness, NOW + 1).unwrap();

        clear_at(&paths.readiness).unwrap();
        clear_at(&paths.readiness).unwrap();

        assert!(!is_ready_at(&paths.readiness, &paths.auth_db, NOW));
    }
}
