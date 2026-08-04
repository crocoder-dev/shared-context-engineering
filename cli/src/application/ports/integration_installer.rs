//! `IntegrationInstaller` port: installs resolved integration assets into a
//! repository, owned by an outbound adapter and consumed by the
//! `InstallIntegrationAssets` use case.

use std::path::{Path, PathBuf};

use crate::domain::integration::{IntegrationAsset, IntegrationTarget};

/// The outcome of installing assets for a single concrete integration
/// target.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // consumed starting with the InstallIntegrationAssets use case (T03)
pub(crate) struct InstalledIntegrationTarget {
    pub(crate) target: IntegrationTarget,
    pub(crate) destination_root: PathBuf,
    pub(crate) installed_file_count: usize,
}

/// Installs resolved integration assets for a concrete target into a
/// repository.
#[allow(dead_code)] // consumed starting with the InstallIntegrationAssets use case (T03)
pub(crate) trait IntegrationInstaller {
    type Error;

    fn install(
        &self,
        repository_root: &Path,
        target: IntegrationTarget,
        assets: &[IntegrationAsset],
    ) -> Result<InstalledIntegrationTarget, Self::Error>;
}
