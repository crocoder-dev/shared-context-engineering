//! `EmbeddedIntegrationAssetCatalog`: the `IntegrationAssetCatalog` outbound
//! adapter wrapping the existing generated embedded-asset catalog in
//! `services::setup`.

use std::convert::Infallible;

use crate::application::ports::integration_asset_catalog::IntegrationAssetCatalog;
use crate::domain::integration::{IntegrationAsset, IntegrationTarget};
use crate::services::setup;

fn setup_target_for(target: IntegrationTarget) -> setup::SetupTarget {
    match target {
        IntegrationTarget::OpenCode => setup::SetupTarget::OpenCode,
        IntegrationTarget::Claude => setup::SetupTarget::Claude,
        IntegrationTarget::Pi => setup::SetupTarget::Pi,
    }
}

/// Resolves embedded assets for a concrete integration target by delegating
/// to `services::setup::iter_embedded_assets_for_setup_target_with_selection`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EmbeddedIntegrationAssetCatalog;

impl IntegrationAssetCatalog for EmbeddedIntegrationAssetCatalog {
    type Error = Infallible;

    fn assets_for(
        &self,
        target: IntegrationTarget,
        optional_workflows: &[String],
    ) -> Result<Vec<IntegrationAsset>, Self::Error> {
        let assets = setup::iter_embedded_assets_for_setup_target_with_selection(
            setup_target_for(target),
            optional_workflows,
        )
        .map(|asset| IntegrationAsset {
            relative_path: asset.relative_path.to_string(),
            bytes: asset.bytes,
        })
        .collect();

        Ok(assets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_for_matches_the_underlying_generated_catalog() {
        let optional_workflows = vec!["research".to_string()];

        let adapter_assets = EmbeddedIntegrationAssetCatalog
            .assets_for(IntegrationTarget::Claude, &optional_workflows)
            .expect("adapter catalog lookup");

        let expected: Vec<IntegrationAsset> =
            setup::iter_embedded_assets_for_setup_target_with_selection(
                setup::SetupTarget::Claude,
                &optional_workflows,
            )
            .map(|asset| IntegrationAsset {
                relative_path: asset.relative_path.to_string(),
                bytes: asset.bytes,
            })
            .collect();

        assert_eq!(adapter_assets, expected);
    }

    #[test]
    fn assets_for_maps_every_concrete_target() {
        for target in [
            IntegrationTarget::OpenCode,
            IntegrationTarget::Claude,
            IntegrationTarget::Pi,
        ] {
            let adapter_assets = EmbeddedIntegrationAssetCatalog
                .assets_for(target, &[])
                .expect("adapter catalog lookup");

            let expected: Vec<IntegrationAsset> =
                setup::iter_embedded_assets_for_setup_target_with_selection(
                    setup_target_for(target),
                    &[] as &[String],
                )
                .map(|asset| IntegrationAsset {
                    relative_path: asset.relative_path.to_string(),
                    bytes: asset.bytes,
                })
                .collect();

            assert_eq!(adapter_assets, expected);
        }
    }
}
