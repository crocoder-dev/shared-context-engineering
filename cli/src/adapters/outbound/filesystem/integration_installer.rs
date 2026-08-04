//! `FilesystemIntegrationInstaller`: the `IntegrationInstaller` outbound
//! adapter that stages, writes, and swaps in embedded integration assets on
//! disk, porting the staging/replace/rename/cleanup logic previously inline
//! in `services::setup::install`.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};

use crate::application::ports::integration_installer::{
    InstalledIntegrationTarget, IntegrationInstaller,
};
use crate::domain::integration::{IntegrationAsset, IntegrationTarget};
use crate::services::default_paths::InstallTargetPaths;
use crate::services::security::ensure_directory_is_writable;
use crate::services::setup::{self, cleanup_path_if_exists, setup_install_recovery_guidance};

fn setup_target_for(target: IntegrationTarget) -> setup::SetupTarget {
    match target {
        IntegrationTarget::OpenCode => setup::SetupTarget::OpenCode,
        IntegrationTarget::Claude => setup::SetupTarget::Claude,
        IntegrationTarget::Pi => setup::SetupTarget::Pi,
    }
}

fn destination_root_for(repository_root: &Path, target: IntegrationTarget) -> PathBuf {
    let install_targets = InstallTargetPaths::new(repository_root);
    match target {
        IntegrationTarget::OpenCode => install_targets.opencode_target_dir(),
        IntegrationTarget::Claude => install_targets.claude_target_dir(),
        IntegrationTarget::Pi => install_targets.pi_target_dir(),
    }
}

/// Installs embedded integration assets directly onto the filesystem via a
/// stage-then-rename swap, with no backup of any replaced target.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FilesystemIntegrationInstaller;

impl IntegrationInstaller for FilesystemIntegrationInstaller {
    type Error = anyhow::Error;

    fn install(
        &self,
        repository_root: &Path,
        target: IntegrationTarget,
        assets: &[IntegrationAsset],
    ) -> Result<InstalledIntegrationTarget> {
        install_with_rename(repository_root, target, assets, |from, to| {
            fs::rename(from, to)
        })
    }
}

fn install_with_rename<F>(
    repository_root: &Path,
    target: IntegrationTarget,
    assets: &[IntegrationAsset],
    mut rename_fn: F,
) -> Result<InstalledIntegrationTarget>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    ensure_directory_is_writable(repository_root, "setup repository root")?;

    let destination_root = destination_root_for(repository_root, target);
    let staging_root = create_staging_root(repository_root, target)?;

    if let Err(error) = write_assets_to_staging(&staging_root, assets) {
        cleanup_path_if_exists(&staging_root);
        return Err(error);
    }

    if destination_root.exists() {
        remove_existing_install_target(&destination_root).with_context(|| {
            format!(
                "Failed to replace existing setup target '{}' without creating a backup",
                destination_root.display()
            )
        })?;
    }

    if let Err(error) = rename_fn(&staging_root, &destination_root).with_context(|| {
        format!(
            "Failed to swap staged install '{}' into destination '{}'",
            staging_root.display(),
            destination_root.display()
        )
    }) {
        cleanup_path_if_exists(&staging_root);
        return Err(error.context(setup_install_recovery_guidance(
            setup_target_for(target),
            &destination_root,
        )));
    }

    Ok(InstalledIntegrationTarget {
        target,
        destination_root,
        installed_file_count: assets.len(),
    })
}

fn remove_existing_install_target(destination_root: &Path) -> Result<()> {
    let metadata = fs::metadata(destination_root).with_context(|| {
        format!(
            "Failed to inspect existing setup target '{}'",
            destination_root.display()
        )
    })?;

    if metadata.is_dir() {
        fs::remove_dir_all(destination_root).with_context(|| {
            format!(
                "Failed to remove existing setup target directory '{}'",
                destination_root.display()
            )
        })?;
    } else {
        fs::remove_file(destination_root).with_context(|| {
            format!(
                "Failed to remove existing setup target file '{}'",
                destination_root.display()
            )
        })?;
    }

    Ok(())
}

fn write_assets_to_staging(staging_root: &Path, assets: &[IntegrationAsset]) -> Result<()> {
    for asset in assets {
        validate_embedded_relative_path(&asset.relative_path)?;
        let destination = staging_root.join(&asset.relative_path);
        let parent = destination
            .parent()
            .context("Embedded asset destination should have a parent directory")?;

        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create staged parent directory '{}'",
                parent.display()
            )
        })?;

        fs::write(&destination, asset.bytes).with_context(|| {
            format!(
                "Failed to write staged embedded asset '{}'",
                destination.display()
            )
        })?;
    }

    Ok(())
}

fn validate_embedded_relative_path(relative_path: &str) -> Result<()> {
    let path = Path::new(relative_path);

    if path.is_absolute() {
        bail!("Embedded asset path '{relative_path}' must be relative, not absolute");
    }

    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => {
                bail!("Embedded asset path '{relative_path}' contains disallowed component");
            }
        }
    }

    Ok(())
}

fn create_staging_root(repository_root: &Path, target: IntegrationTarget) -> Result<PathBuf> {
    let target_dir = destination_root_for(repository_root, target);
    let target_label = target_dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("Setup target directory should have a valid UTF-8 file name")?
        .trim_start_matches('.');
    let epoch_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_nanos();

    for attempt in 0..1000_u16 {
        let candidate = repository_root.join(format!(
            ".sce-setup-staging-{target_label}-{epoch_nanos}-{}-{attempt}",
            std::process::id()
        ));

        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to create staging directory '{}'",
                        candidate.display()
                    )
                });
            }
        }
    }

    bail!(
        "Could not allocate a unique staging directory under '{}'",
        repository_root.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-filesystem-integration-installer-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn asset(relative_path: &str, bytes: &'static [u8]) -> IntegrationAsset {
        IntegrationAsset {
            relative_path: relative_path.to_string(),
            bytes,
        }
    }

    #[test]
    fn install_writes_assets_to_the_target_directory() {
        let repo = unique_temp_dir("success");
        let assets = vec![
            asset("command/next-task.md", b"next-task"),
            asset("lib/helper.json", b"{}"),
        ];

        let installed = FilesystemIntegrationInstaller
            .install(&repo, IntegrationTarget::Claude, &assets)
            .expect("install should succeed");

        assert_eq!(installed.target, IntegrationTarget::Claude);
        assert_eq!(installed.installed_file_count, 2);
        assert_eq!(
            fs::read(installed.destination_root.join("command/next-task.md"))
                .expect("read installed asset"),
            b"next-task"
        );
        assert_eq!(
            fs::read(installed.destination_root.join("lib/helper.json"))
                .expect("read installed asset"),
            b"{}"
        );

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn install_rejects_absolute_and_parent_component_paths() {
        let repo = unique_temp_dir("invalid-path");

        for invalid_path in ["/etc/passwd", "../escape.txt"] {
            let assets = vec![asset(invalid_path, b"payload")];

            let error = FilesystemIntegrationInstaller
                .install(&repo, IntegrationTarget::Pi, &assets)
                .expect_err("invalid embedded path should be rejected");

            assert!(error.to_string().contains(invalid_path));
            assert!(!InstallTargetPaths::new(&repo).pi_target_dir().exists());
        }

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn install_replaces_an_existing_target_without_a_backup() {
        let repo = unique_temp_dir("replace-existing");
        let assets = vec![asset("command/next-task.md", b"first")];

        FilesystemIntegrationInstaller
            .install(&repo, IntegrationTarget::OpenCode, &assets)
            .expect("first install should succeed");

        let replacement_assets = vec![asset("command/next-task.md", b"second")];
        let installed = FilesystemIntegrationInstaller
            .install(&repo, IntegrationTarget::OpenCode, &replacement_assets)
            .expect("second install should replace the first");

        let entries: Vec<_> = fs::read_dir(&installed.destination_root)
            .expect("read destination root")
            .collect();
        assert_eq!(entries.len(), 1, "replaced target should have no backup");
        assert_eq!(
            fs::read(installed.destination_root.join("command/next-task.md"))
                .expect("read replaced asset"),
            b"second"
        );

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn install_cleans_up_staging_and_reports_recovery_guidance_on_rename_failure() {
        let repo = unique_temp_dir("rename-failure");
        let assets = vec![asset("command/next-task.md", b"payload")];

        let error = install_with_rename(&repo, IntegrationTarget::Claude, &assets, |_, _| {
            Err(io::Error::other("simulated rename failure"))
        })
        .expect_err("rename failure should surface an error");

        let message = error.to_string();
        assert!(message.contains("does not create backups"));
        assert!(!InstallTargetPaths::new(&repo).claude_target_dir().exists());

        let leftover_staging = fs::read_dir(&repo)
            .expect("read repo root")
            .filter_map(std::result::Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".sce-setup-staging-")
            });
        assert!(!leftover_staging, "staging directory should be cleaned up");

        let _ = fs::remove_dir_all(&repo);
    }
}
