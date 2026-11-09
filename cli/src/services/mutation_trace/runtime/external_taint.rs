use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const SCE_RUNTIME_DIR: &str = "sce";

const MARKER_FILE: &str = "mutation-cursor-tainted";

#[derive(Debug, Clone)]
pub struct ExternalTaintMarker {
    marker_dir: PathBuf,
    marker_path: PathBuf,
}

impl ExternalTaintMarker {
    #[must_use]
    pub fn new(git_dir: &Path) -> Self {
        let marker_dir = git_dir.join(SCE_RUNTIME_DIR);
        let marker_path = marker_dir.join(MARKER_FILE);
        Self {
            marker_dir,
            marker_path,
        }
    }

    pub fn exists(&self) -> Result<bool> {
        match std::fs::symlink_metadata(&self.marker_path) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| {
                format!(
                    "Failed to inspect external-taint marker '{}'",
                    self.marker_path.display()
                )
            }),
        }
    }

    pub fn persist(&self) -> Result<()> {
        std::fs::create_dir_all(&self.marker_dir).with_context(|| {
            format!(
                "Failed to create external-taint marker directory '{}'",
                self.marker_dir.display()
            )
        })?;

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.marker_path)
            .with_context(|| {
                format!(
                    "Failed to create external-taint marker '{}'",
                    self.marker_path.display()
                )
            })?;
        file.sync_data().with_context(|| {
            format!(
                "Failed to sync external-taint marker '{}'",
                self.marker_path.display()
            )
        })?;
        drop(file);

        self.best_effort_sync_marker_dir();

        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        match std::fs::remove_file(&self.marker_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "Failed to clear external-taint marker '{}'",
                        self.marker_path.display()
                    )
                });
            }
        }

        self.best_effort_sync_marker_dir();

        Ok(())
    }

    #[cfg(unix)]
    fn best_effort_sync_marker_dir(&self) {
        if let Ok(dir) = std::fs::File::open(&self.marker_dir) {
            let _ = dir.sync_all();
        }
    }

    #[cfg(not(unix))]
    fn best_effort_sync_marker_dir(&self) {}
}
