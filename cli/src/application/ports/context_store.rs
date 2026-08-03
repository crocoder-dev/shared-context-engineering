//! `ContextStore` port: durable-context baseline persistence, owned by an
//! outbound adapter and consumed by the `EnsureContextBaseline` use case.

use std::path::{Path, PathBuf};

use crate::domain::context::baseline::ContextBaseline;

/// The directories and files a baseline-ensure operation created versus
/// found already present.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
#[allow(dead_code)] // consumed starting with the EnsureContextBaseline use case (T03)
pub(crate) struct ContextBaselineChanges {
    pub(crate) created_directories: Vec<PathBuf>,
    pub(crate) existing_directories: Vec<PathBuf>,
    pub(crate) created_files: Vec<PathBuf>,
    pub(crate) existing_files: Vec<PathBuf>,
}

/// Persists the durable-context baseline additively against a repository
/// root.
#[allow(dead_code)] // consumed starting with the EnsureContextBaseline use case (T03)
pub(crate) trait ContextStore {
    type Error;

    fn ensure_baseline(
        &self,
        repository_root: &Path,
        baseline: &ContextBaseline,
    ) -> Result<ContextBaselineChanges, Self::Error>;
}
