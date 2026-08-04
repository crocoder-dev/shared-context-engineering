//! `IntegrationConfigRepository` port: owns repo-local `.sce/config.json`
//! lifecycle for integration configuration, owned by an outbound adapter and
//! consumed by the repository-config use cases.

use std::path::Path;

use crate::domain::integration::IntegrationTarget;

/// Owns repo-local integration configuration persistence: bootstrap,
/// optional-workflow reads, and recording concrete installed targets.
pub(crate) trait IntegrationConfigRepository {
    type Error;

    /// Creates the repo-local config file with the canonical bootstrap
    /// payload if it does not already exist; leaves an existing file
    /// untouched.
    fn ensure_exists(&self, repository_root: &Path) -> Result<(), Self::Error>;

    /// The optional workflows currently recorded in the repo-local config.
    fn load_optional_workflows(&self, repository_root: &Path) -> Result<Vec<String>, Self::Error>;

    /// Records the given concrete targets as installed and replaces the
    /// recorded optional-workflow selection.
    fn record_installation(
        &self,
        repository_root: &Path,
        targets: &[IntegrationTarget],
        optional_workflows: &[String],
    ) -> Result<(), Self::Error>;
}
