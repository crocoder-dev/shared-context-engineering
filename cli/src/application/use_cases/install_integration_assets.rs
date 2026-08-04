//! `InstallIntegrationAssets` use case: orchestrates `IntegrationAssetCatalog`
//! and `IntegrationInstaller` over an expanded target selection.

use std::path::Path;

use crate::application::ports::integration_asset_catalog::IntegrationAssetCatalog;
use crate::application::ports::integration_installer::{
    InstalledIntegrationTarget, IntegrationInstaller,
};
use crate::domain::integration::IntegrationTargetSelection;

/// The outcome of installing integration assets for an expanded target
/// selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstallIntegrationAssetsReport {
    pub(crate) targets: Vec<InstalledIntegrationTarget>,
}

/// Either collaborator's failure, surfaced without losing which port failed.
#[derive(Debug)]
pub(crate) enum InstallIntegrationAssetsError<CE, IE> {
    Catalog(CE),
    Installer(IE),
}

/// Installs the embedded assets for an expanded integration target
/// selection, via injected `IntegrationAssetCatalog` and
/// `IntegrationInstaller` collaborators.
pub(crate) struct InstallIntegrationAssets<C: IntegrationAssetCatalog, I: IntegrationInstaller> {
    catalog: C,
    installer: I,
}

impl<C: IntegrationAssetCatalog, I: IntegrationInstaller> InstallIntegrationAssets<C, I> {
    pub(crate) fn new(catalog: C, installer: I) -> Self {
        Self { catalog, installer }
    }

    pub(crate) fn execute(
        &self,
        repository_root: &Path,
        selection: IntegrationTargetSelection,
        optional_workflows: &[String],
    ) -> Result<InstallIntegrationAssetsReport, InstallIntegrationAssetsError<C::Error, I::Error>>
    {
        let mut targets = Vec::new();

        for &target in selection.targets() {
            let assets = self
                .catalog
                .assets_for(target, optional_workflows)
                .map_err(InstallIntegrationAssetsError::Catalog)?;

            let installed = self
                .installer
                .install(repository_root, target, &assets)
                .map_err(InstallIntegrationAssetsError::Installer)?;

            targets.push(installed);
        }

        Ok(InstallIntegrationAssetsReport { targets })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    use crate::domain::integration::{IntegrationAsset, IntegrationTarget};

    #[derive(Default)]
    struct FakeCatalog {
        calls: RefCell<Vec<(IntegrationTarget, Vec<String>)>>,
        fail_on: Option<IntegrationTarget>,
    }

    impl IntegrationAssetCatalog for FakeCatalog {
        type Error = &'static str;

        fn assets_for(
            &self,
            target: IntegrationTarget,
            optional_workflows: &[String],
        ) -> Result<Vec<IntegrationAsset>, Self::Error> {
            self.calls
                .borrow_mut()
                .push((target, optional_workflows.to_vec()));

            if self.fail_on == Some(target) {
                return Err("catalog failed");
            }

            Ok(vec![IntegrationAsset {
                relative_path: "file.txt".to_string(),
                bytes: b"content",
            }])
        }
    }

    #[derive(Default)]
    struct FakeInstaller {
        calls: RefCell<Vec<(PathBuf, IntegrationTarget, Vec<IntegrationAsset>)>>,
    }

    impl IntegrationInstaller for FakeInstaller {
        type Error = &'static str;

        fn install(
            &self,
            repository_root: &Path,
            target: IntegrationTarget,
            assets: &[IntegrationAsset],
        ) -> Result<InstalledIntegrationTarget, Self::Error> {
            self.calls
                .borrow_mut()
                .push((repository_root.to_path_buf(), target, assets.to_vec()));

            Ok(InstalledIntegrationTarget {
                target,
                destination_root: repository_root.join("dest"),
                installed_file_count: assets.len(),
            })
        }
    }

    #[test]
    fn one_selection_invokes_both_ports_once_with_that_target() {
        let catalog = FakeCatalog::default();
        let installer = FakeInstaller::default();
        let use_case = InstallIntegrationAssets::new(catalog, installer);
        let repository_root = PathBuf::from("/repo");
        let optional_workflows = vec!["research".to_string()];

        let report = use_case
            .execute(
                &repository_root,
                IntegrationTargetSelection::One(IntegrationTarget::Claude),
                &optional_workflows,
            )
            .unwrap();

        assert_eq!(report.targets.len(), 1);
        assert_eq!(report.targets[0].target, IntegrationTarget::Claude);

        let catalog_calls = use_case.catalog.calls.borrow();
        assert_eq!(catalog_calls.len(), 1);
        assert_eq!(catalog_calls[0].0, IntegrationTarget::Claude);
        assert_eq!(catalog_calls[0].1, optional_workflows);

        let installer_calls = use_case.installer.calls.borrow();
        assert_eq!(installer_calls.len(), 1);
        assert_eq!(installer_calls[0].0, repository_root);
        assert_eq!(installer_calls[0].1, IntegrationTarget::Claude);
    }

    #[test]
    fn all_selection_invokes_both_ports_three_times_in_order() {
        let catalog = FakeCatalog::default();
        let installer = FakeInstaller::default();
        let use_case = InstallIntegrationAssets::new(catalog, installer);
        let repository_root = PathBuf::from("/repo");

        let report = use_case
            .execute(&repository_root, IntegrationTargetSelection::All, &[])
            .unwrap();

        assert_eq!(report.targets.len(), 3);

        let catalog_calls = use_case.catalog.calls.borrow();
        let catalog_order: Vec<IntegrationTarget> =
            catalog_calls.iter().map(|(target, _)| *target).collect();
        assert_eq!(
            catalog_order,
            vec![
                IntegrationTarget::OpenCode,
                IntegrationTarget::Claude,
                IntegrationTarget::Pi,
            ]
        );

        let installer_calls = use_case.installer.calls.borrow();
        let installer_order: Vec<IntegrationTarget> = installer_calls
            .iter()
            .map(|(_, target, _)| *target)
            .collect();
        assert_eq!(
            installer_order,
            vec![
                IntegrationTarget::OpenCode,
                IntegrationTarget::Claude,
                IntegrationTarget::Pi,
            ]
        );
    }

    #[test]
    fn catalog_error_short_circuits_before_installer_and_later_targets() {
        let catalog = FakeCatalog {
            calls: RefCell::new(Vec::new()),
            fail_on: Some(IntegrationTarget::Claude),
        };
        let installer = FakeInstaller::default();
        let use_case = InstallIntegrationAssets::new(catalog, installer);
        let repository_root = PathBuf::from("/repo");

        let result = use_case.execute(&repository_root, IntegrationTargetSelection::All, &[]);

        assert!(matches!(
            result,
            Err(InstallIntegrationAssetsError::Catalog("catalog failed"))
        ));

        let catalog_calls = use_case.catalog.calls.borrow();
        let catalog_order: Vec<IntegrationTarget> =
            catalog_calls.iter().map(|(target, _)| *target).collect();
        assert_eq!(
            catalog_order,
            vec![IntegrationTarget::OpenCode, IntegrationTarget::Claude]
        );

        let installer_calls = use_case.installer.calls.borrow();
        let installer_order: Vec<IntegrationTarget> = installer_calls
            .iter()
            .map(|(_, target, _)| *target)
            .collect();
        assert_eq!(installer_order, vec![IntegrationTarget::OpenCode]);
    }
}
