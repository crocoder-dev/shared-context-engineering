use std::path::Path;

use crate::error::HarnessError;
use crate::harness::{ChannelHarness, HarnessRequest};

use super::npm::{build_local_npm_fixture, find_repo_root};

pub(crate) fn run(
    request: HarnessRequest,
    explicit_repo_root: Option<&Path>,
) -> Result<(), HarnessError> {
    let harness = ChannelHarness::new(request.channel())?;
    println!("{}", harness.setup_message());

    let repo_root = find_repo_root(request.channel().as_str(), explicit_repo_root)?;
    let package_tarball =
        build_local_npm_fixture(&harness, &repo_root, request.channel().as_str())?;

    install_bun_package(&harness, &repo_root, &package_tarball)?;

    let sce_binary = harness.resolve_program("sce")?;
    let version_output = harness.assert_sce_version_success(&sce_binary)?;

    println!("{}", harness.version_success_message(&version_output));
    println!(
        "bun install-and-verify flow passed for channel={} via the Rust runner (mode={}).",
        request.channel().as_str(),
        request.mode().as_str()
    );
    Ok(())
}

fn install_bun_package(
    harness: &ChannelHarness,
    repo_root: &Path,
    package_tarball: &Path,
) -> Result<(), HarnessError> {
    let bun = harness.resolve_program("bun")?;
    let install_output = harness.run_command_in_dir_with_env(
        &bun,
        [
            "add",
            "--global",
            package_tarball.to_string_lossy().as_ref(),
        ],
        repo_root,
        [("SCE_NPM_SKIP_DOWNLOAD", "1")],
    )?;

    if !install_output.status.success() {
        return Err(HarnessError::BunInstallFailed {
            channel: "bun".to_string(),
            tarball: package_tarball.to_path_buf(),
            stdout: if install_output.stdout.is_empty() {
                None
            } else {
                Some(install_output.stdout)
            },
            stderr: if install_output.stderr.is_empty() {
                None
            } else {
                Some(install_output.stderr)
            },
        });
    }

    println!(
        "[PASS] channel=bun bun global install completed from {}",
        package_tarball.display()
    );
    Ok(())
}
