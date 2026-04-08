use std::fs;
use std::path::{Path, PathBuf};

use crate::error::HarnessError;
use crate::harness::{copy_directory_recursive, ChannelHarness, HarnessRequest};
use crate::platform::set_executable_permissions;

pub(crate) fn run(
    request: HarnessRequest,
    explicit_repo_root: Option<&Path>,
) -> Result<(), HarnessError> {
    let harness = ChannelHarness::new(request.channel())?;
    println!("{}", harness.setup_message());

    let repo_root = find_repo_root(request.channel().as_str(), explicit_repo_root)?;
    let package_tarball =
        build_local_npm_fixture(&harness, &repo_root, request.channel().as_str())?;

    install_npm_package(&harness, &repo_root, &package_tarball)?;

    let sce_binary = harness.resolve_program_in_harness_bins("sce")?;
    let version_output = harness.assert_sce_version_success(&sce_binary)?;
    assert_sce_doctor_success(&harness, &sce_binary)?;

    println!("{}", harness.version_success_message(&version_output));
    println!(
        "[PASS] channel={} sce doctor completed successfully via installed npm launcher.",
        request.channel().as_str()
    );
    println!(
        "npm install-and-verify flow passed for channel={} via the Rust runner (mode={}).",
        request.channel().as_str(),
        request.mode().as_str()
    );
    Ok(())
}

fn install_npm_package(
    harness: &ChannelHarness,
    repo_root: &Path,
    package_tarball: &Path,
) -> Result<(), HarnessError> {
    let npm = harness.resolve_program("npm")?;

    let install_output = harness.run_command_in_dir_with_env(
        &npm,
        [
            "install",
            "--global",
            package_tarball.to_string_lossy().as_ref(),
        ],
        repo_root,
        [("SCE_NPM_SKIP_DOWNLOAD", "1")],
    )?;

    if !install_output.status.success() {
        return Err(HarnessError::NpmInstallFailed {
            channel: "npm".to_string(),
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
        "[PASS] channel=npm npm install completed from {}",
        package_tarball.display()
    );
    Ok(())
}

pub(super) fn find_repo_root(
    channel_name: &str,
    explicit_root: Option<&Path>,
) -> Result<PathBuf, HarnessError> {
    // First, check explicit path if provided
    if let Some(explicit) = explicit_root {
        if explicit.join("flake.nix").is_file() {
            return Ok(explicit.to_path_buf());
        }
    }

    // Fall back to upward walk from current directory
    let mut current = std::env::current_dir().map_err(|e| HarnessError::CurrentDir {
        error: e.to_string(),
    })?;

    loop {
        if current.join("flake.nix").is_file() {
            return Ok(current);
        }

        if !current.pop() {
            return Err(HarnessError::RepoRootMissing {
                channel: channel_name.to_string(),
            });
        }
    }
}

pub(super) fn build_local_npm_fixture(
    harness: &ChannelHarness,
    repo_root: &Path,
    channel_name: &str,
) -> Result<PathBuf, HarnessError> {
    let fixture_root = harness.create_temp_subdir("npm-package-fixture")?;
    let stage_dir = fixture_root.join("package");
    let pack_dir = fixture_root.join("packed");
    let npm_source_dir = repo_root.join("npm");
    let packaged_sce_binary = harness.resolve_sce_binary()?;

    copy_directory_recursive(&npm_source_dir, &stage_dir)?;
    add_runtime_to_staged_package_manifest(&stage_dir.join("package.json"))?;
    fs::create_dir_all(stage_dir.join("runtime")).map_err(|e| HarnessError::DirectoryCreate {
        path: stage_dir.join("runtime"),
        error: e.to_string(),
    })?;
    fs::create_dir_all(&pack_dir).map_err(|e| HarnessError::DirectoryCreate {
        path: pack_dir.clone(),
        error: e.to_string(),
    })?;

    let staged_binary = stage_dir.join("runtime/sce");
    fs::copy(&packaged_sce_binary, &staged_binary).map_err(|e| HarnessError::BinaryStage {
        binary: packaged_sce_binary.clone(),
        path: staged_binary.clone(),
        error: e.to_string(),
    })?;

    set_executable_permissions(&staged_binary)?;

    let npm = harness.resolve_program("npm")?;
    let pack_output = harness.run_command_in_dir_with_env(
        &npm,
        ["pack", "--silent", stage_dir.to_string_lossy().as_ref()],
        &pack_dir,
        [("SCE_NPM_SKIP_DOWNLOAD", "1")],
    )?;

    if !pack_output.status.success() {
        return Err(HarnessError::NpmPackFailed {
            channel: channel_name.to_string(),
            stdout: if pack_output.stdout.is_empty() {
                None
            } else {
                Some(pack_output.stdout)
            },
            stderr: if pack_output.stderr.is_empty() {
                None
            } else {
                Some(pack_output.stderr)
            },
        });
    }

    let package_name = pack_output
        .stdout
        .lines()
        .last()
        .map(str::trim)
        .filter(|line: &&str| !line.is_empty())
        .ok_or_else(|| HarnessError::NpmPackNoTarball {
            channel: channel_name.to_string(),
        })?;
    let package_tarball = pack_dir.join(package_name);

    if !package_tarball.is_file() {
        return Err(HarnessError::NpmPackTarballMissing {
            channel: channel_name.to_string(),
            path: package_tarball.clone(),
        });
    }

    println!(
        "[PASS] channel={channel_name} local package fixture built from @.version state: {}",
        package_tarball.display()
    );
    Ok(package_tarball)
}

fn add_runtime_to_staged_package_manifest(package_json_path: &Path) -> Result<(), HarnessError> {
    let package_json =
        fs::read_to_string(package_json_path).map_err(|e| HarnessError::FileRead {
            path: package_json_path.to_path_buf(),
            error: e.to_string(),
        })?;
    let updated_package_json = package_json.replace(
        "\"lib\",\n\t\t\"README.md\"",
        "\"lib\",\n\t\t\"runtime\",\n\t\t\"README.md\"",
    );

    if updated_package_json == package_json {
        return Err(HarnessError::ManifestInject {
            path: package_json_path.to_path_buf(),
        });
    }

    fs::write(package_json_path, updated_package_json).map_err(|e| HarnessError::FileWrite {
        path: package_json_path.to_path_buf(),
        error: e.to_string(),
    })
}

fn assert_sce_doctor_success(
    harness: &ChannelHarness,
    sce_binary: &Path,
) -> Result<(), HarnessError> {
    let output = harness.run_command(sce_binary, ["doctor", "--format", "json"])?;

    if !output.status.success() {
        return Err(HarnessError::CommandFailed {
            channel: "npm".to_string(),
            program: sce_binary.display().to_string(),
            error: if output.stderr.is_empty() {
                output.stdout
            } else {
                output.stderr
            },
        });
    }

    Ok(())
}
