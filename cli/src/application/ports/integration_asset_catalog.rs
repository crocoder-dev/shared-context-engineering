//! `IntegrationAssetCatalog` port: resolves the embedded assets to install
//! for an integration target, owned by an outbound adapter and consumed by
//! the `InstallIntegrationAssets` use case.

use crate::domain::integration::{IntegrationAsset, IntegrationTarget};

/// Resolves the embedded assets to install for a concrete integration
/// target, filtered by the caller's selected optional workflows.
#[allow(dead_code)] // consumed starting with the InstallIntegrationAssets use case (T03)
pub(crate) trait IntegrationAssetCatalog {
    type Error;

    fn assets_for(
        &self,
        target: IntegrationTarget,
        optional_workflows: &[String],
    ) -> Result<Vec<IntegrationAsset>, Self::Error>;
}
