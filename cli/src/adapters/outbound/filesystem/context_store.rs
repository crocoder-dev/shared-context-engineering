//! `FilesystemContextStore`: the `ContextStore` outbound adapter that
//! persists the durable-context baseline to disk.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::application::ports::context_store::{ContextBaselineChanges, ContextStore};
use crate::domain::context::baseline::ContextBaseline;

/// A filesystem operation failed for a specific baseline path.
#[derive(Debug)]
pub(crate) struct ContextStoreError {
    pub(crate) path: PathBuf,
    pub(crate) source: std::io::Error,
}

impl fmt::Display for ContextStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to write context baseline path '{}': {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for ContextStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Persists the durable-context baseline directly to the filesystem.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FilesystemContextStore;

impl ContextStore for FilesystemContextStore {
    type Error = ContextStoreError;

    fn ensure_baseline(
        &self,
        repository_root: &Path,
        baseline: &ContextBaseline,
    ) -> Result<ContextBaselineChanges, Self::Error> {
        let mut changes = ContextBaselineChanges::default();

        for relative_directory in &baseline.directories {
            let directory = repository_root.join(relative_directory);
            if directory.exists() {
                changes.existing_directories.push(directory);
            } else {
                create_dir_all(&directory)?;
                changes.created_directories.push(directory);
            }
        }

        for file in &baseline.files {
            let path = repository_root.join(&file.relative_path);
            if path.exists() {
                changes.existing_files.push(path);
                continue;
            }

            if let Some(parent) = path.parent() {
                create_dir_all(parent)?;
            }

            fs::write(&path, file.initial_content).map_err(|source| ContextStoreError {
                path: path.clone(),
                source,
            })?;
            changes.created_files.push(path);
        }

        Ok(changes)
    }
}

fn create_dir_all(path: &Path) -> Result<(), ContextStoreError> {
    fs::create_dir_all(path).map_err(|source| ContextStoreError {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-filesystem-context-store-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn ensure_baseline_creates_a_missing_baseline_fully() {
        let repo = unique_temp_dir("full");
        let baseline = ContextBaseline::sce_default();
        let store = FilesystemContextStore;

        let changes = store
            .ensure_baseline(&repo, &baseline)
            .expect("ensure baseline");

        assert_eq!(
            changes.created_directories.len(),
            baseline.directories.len()
        );
        assert!(changes.existing_directories.is_empty());
        assert_eq!(changes.created_files.len(), baseline.files.len());
        assert!(changes.existing_files.is_empty());

        for directory in &baseline.directories {
            assert!(repo.join(directory).is_dir());
        }
        for file in &baseline.files {
            let content = fs::read_to_string(repo.join(&file.relative_path)).expect("read file");
            assert_eq!(content, file.initial_content);
        }
    }

    #[test]
    fn ensure_baseline_leaves_existing_custom_content_byte_for_byte_unchanged() {
        let repo = unique_temp_dir("idempotent");
        let baseline = ContextBaseline::sce_default();
        let store = FilesystemContextStore;

        store
            .ensure_baseline(&repo, &baseline)
            .expect("initial bootstrap");

        let overview_path = repo.join("context/overview.md");
        let sentinel = "SENTINEL_OVERVIEW_CONTENT\n";
        fs::write(&overview_path, sentinel).expect("seed sentinel");

        store
            .ensure_baseline(&repo, &baseline)
            .expect("rerun bootstrap");

        let content = fs::read_to_string(&overview_path).expect("read overview");
        assert_eq!(content, sentinel);
    }

    #[test]
    fn ensure_baseline_second_run_reports_only_existing_paths() {
        let repo = unique_temp_dir("second-run");
        let baseline = ContextBaseline::sce_default();
        let store = FilesystemContextStore;

        store
            .ensure_baseline(&repo, &baseline)
            .expect("initial bootstrap");

        let changes = store
            .ensure_baseline(&repo, &baseline)
            .expect("second bootstrap");

        assert!(changes.created_directories.is_empty());
        assert!(changes.created_files.is_empty());
        assert_eq!(
            changes.existing_directories.len(),
            baseline.directories.len()
        );
        assert_eq!(changes.existing_files.len(), baseline.files.len());
    }

    #[test]
    fn ensure_baseline_creates_only_missing_paths_in_a_partially_bootstrapped_tree() {
        let repo = unique_temp_dir("partial");
        let baseline = ContextBaseline::sce_default();
        let store = FilesystemContextStore;

        fs::create_dir_all(repo.join("context")).expect("seed context dir");
        fs::write(repo.join("context/overview.md"), "existing\n").expect("seed overview file");

        let changes = store
            .ensure_baseline(&repo, &baseline)
            .expect("partial bootstrap");

        assert_eq!(changes.existing_directories, vec![repo.join("context")]);
        assert_eq!(
            changes.created_directories.len(),
            baseline.directories.len() - 1
        );
        assert_eq!(
            changes.existing_files,
            vec![repo.join("context/overview.md")]
        );
        assert_eq!(changes.created_files.len(), baseline.files.len() - 1);

        let overview_content =
            fs::read_to_string(repo.join("context/overview.md")).expect("read overview");
        assert_eq!(overview_content, "existing\n");
    }
}
