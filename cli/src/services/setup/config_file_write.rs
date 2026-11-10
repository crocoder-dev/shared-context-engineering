use std::{fs, io, path::Path};

use anyhow::Context;
use serde_json::{Map, Value};

use crate::services::agent_trace::sce_config_schema_url;
use crate::services::config::schema::CONFIG_SCHEMA_DECLARATION_KEY;
use crate::services::config::ConfigPathSource;

use super::atomic_write::write_atomically;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConfigLayer {
    Global,
    Repository,
}

impl ConfigLayer {
    pub(super) fn path_source(self) -> ConfigPathSource {
        match self {
            Self::Global => ConfigPathSource::DefaultDiscoveredGlobal,
            Self::Repository => ConfigPathSource::DefaultDiscoveredLocal,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Global => "global SCE config",
            Self::Repository => "repository SCE config",
        }
    }

    pub(super) fn bootstrap_value(self) -> Value {
        match self {
            Self::Global => Value::Object(Map::from_iter([(
                CONFIG_SCHEMA_DECLARATION_KEY.to_string(),
                Value::String(sce_config_schema_url()),
            )])),
            Self::Repository => serde_json::from_str(&super::repo_local_config_bootstrap_payload())
                .expect("repo-local config bootstrap payload must be valid JSON"),
        }
    }
}

#[derive(Debug)]
pub(super) enum GuardedWriteError {
    ConcurrentModification,
    SymlinkManaged,
    Io(anyhow::Error),
}

/// Writes `serialized` only when the file still holds exactly `original` and is
/// not a symlink, so setup never clobbers concurrent edits or symlink-managed
/// config sources.
pub(super) fn write_guarded<R>(
    path: &Path,
    layer: ConfigLayer,
    original: Option<&[u8]>,
    serialized: &[u8],
    reread: R,
) -> Result<(), GuardedWriteError>
where
    R: FnOnce(&Path) -> anyhow::Result<Option<Vec<u8>>>,
{
    let current = reread(path).map_err(GuardedWriteError::Io)?;
    if current.as_deref() != original {
        return Err(GuardedWriteError::ConcurrentModification);
    }
    if is_symlink(path, layer).map_err(GuardedWriteError::Io)? {
        return Err(GuardedWriteError::SymlinkManaged);
    }

    write_atomically(path, serialized).map_err(GuardedWriteError::Io)
}

pub(super) fn read_optional(path: &Path, layer: ConfigLayer) -> anyhow::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to read {} '{}'", layer.label(), path.display())),
    }
}

fn is_symlink(path: &Path, layer: ConfigLayer) -> anyhow::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to inspect {} '{}'", layer.label(), path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reread(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
        read_optional(path, ConfigLayer::Global)
    }

    #[test]
    fn write_guarded_creates_missing_file_when_still_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        write_guarded(&path, ConfigLayer::Global, None, b"{}\n", reread).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
    }

    #[test]
    fn write_guarded_refuses_file_created_after_planning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"{\"concurrent\": true}\n").unwrap();

        let error = write_guarded(&path, ConfigLayer::Global, None, b"{}\n", reread).unwrap_err();

        assert!(matches!(error, GuardedWriteError::ConcurrentModification));
        assert_eq!(fs::read(&path).unwrap(), b"{\"concurrent\": true}\n");
    }

    #[cfg(unix)]
    #[test]
    fn write_guarded_refuses_dangling_symlink_and_leaves_it_unchanged() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let target = dir.path().join("missing-target.json");
        symlink(&target, &path).unwrap();

        let error = write_guarded(&path, ConfigLayer::Global, None, b"{}\n", reread).unwrap_err();

        assert!(matches!(error, GuardedWriteError::SymlinkManaged));
        assert_eq!(fs::read_link(&path).unwrap(), target);
        assert!(!target.exists());
    }
}
