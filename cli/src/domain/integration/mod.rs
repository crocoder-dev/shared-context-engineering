//! Integration-asset domain model: install targets, target selection, and
//! the embedded assets installed for them.

mod asset;
mod target;

#[allow(unused_imports)] // consumed starting with the IntegrationAssetCatalog port (T02)
pub(crate) use asset::IntegrationAsset;
#[allow(unused_imports)]
// consumed starting with the IntegrationAssetCatalog/IntegrationInstaller ports (T02)
pub(crate) use target::{IntegrationTarget, IntegrationTargetSelection};
