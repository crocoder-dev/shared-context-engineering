use std::path::Path;

use crate::error::HarnessError;
use crate::harness::{ChannelHarness, HarnessRequest};

use super::npm::find_repo_root;

pub(crate) fn run(
    request: HarnessRequest,
    explicit_repo_root: Option<&Path>,
) -> Result<(), HarnessError> {
    let harness = ChannelHarness::new(request.channel())?;
    println!("{}", harness.setup_message());

    let repo_root = find_repo_root(request.channel().as_str(), explicit_repo_root)?;
    let cli_path = repo_root.join("cli");

    install_cargo_package(&harness, &cli_path)?;

    let sce_binary = harness.resolve_program("sce")?;
    let version_output = harness.assert_sce_version_success(&sce_binary)?;

    println!("{}", harness.version_success_message(&version_output));
    println!(
        "cargo install-and-verify flow passed for channel={} via the Rust runner (mode={}).",
        request.channel().as_str(),
        request.mode().as_str()
    );
    Ok(())
}

fn install_cargo_package(harness: &ChannelHarness, cli_path: &Path) -> Result<(), HarnessError> {
    let cargo = harness.resolve_program("cargo")?;

    let install_output = harness.run_command_in_dir_with_env(
        &cargo,
        [
            "install",
            "--path",
            cli_path.to_string_lossy().as_ref(),
            "--locked",
        ],
        cli_path,
        std::iter::empty::<(&str, &str)>(),
    )?;

    if !install_output.status.success() {
        return Err(HarnessError::CargoInstallFailed {
            channel: "cargo".to_string(),
            path: cli_path.to_path_buf(),
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
        "[PASS] channel=cargo cargo install completed from {}",
        cli_path.display()
    );
    Ok(())
}
