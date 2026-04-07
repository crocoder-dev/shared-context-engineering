use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::harness::{copy_directory_recursive, ChannelHarness, HarnessRequest};

pub(crate) fn run(request: HarnessRequest) -> Result<(), String> {
    let harness = ChannelHarness::new(request.channel())?;
    println!("{}", harness.setup_message());

    let repo_root = find_repo_root(request.channel().as_str())?;
    let package_tarball =
        build_local_npm_fixture(&harness, &repo_root, request.channel().as_str())?;

    install_npm_package(&harness, &repo_root, &package_tarball)?;

    let sce_binary = harness.resolve_program("sce")?;
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
) -> Result<(), String> {
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
        let mut message = format!(
            "[FAIL] channel=npm npm install failed for {}",
            package_tarball.display()
        );
        if !install_output.stdout.is_empty() {
            message.push('\n');
            message.push_str(&install_output.stdout);
        }
        if !install_output.stderr.is_empty() {
            message.push('\n');
            message.push_str(&install_output.stderr);
        }
        return Err(message);
    }

    println!(
        "[PASS] channel=npm npm install completed from {}",
        package_tarball.display()
    );
    Ok(())
}

pub(super) fn find_repo_root(channel_name: &str) -> Result<PathBuf, String> {
    let mut current = std::env::current_dir()
        .map_err(|error| format!("failed to resolve current directory: {error}"))?;

    loop {
        if current.join("flake.nix").is_file() {
            return Ok(current);
        }

        if !current.pop() {
            return Err(format!(
                "[FAIL] channel={channel_name} could not locate repository root containing flake.nix."
            ));
        }
    }
}

pub(super) fn build_local_npm_fixture(
    harness: &ChannelHarness,
    repo_root: &Path,
    channel_name: &str,
) -> Result<PathBuf, String> {
    let fixture_root = harness.create_temp_subdir("npm-package-fixture")?;
    let stage_dir = fixture_root.join("package");
    let pack_dir = fixture_root.join("packed");
    let npm_source_dir = repo_root.join("npm");
    let packaged_sce_binary = harness.resolve_sce_binary()?;

    copy_directory_recursive(&npm_source_dir, &stage_dir)?;
    add_runtime_to_staged_package_manifest(&stage_dir.join("package.json"))?;
    fs::create_dir_all(stage_dir.join("runtime")).map_err(|error| {
        format!(
            "failed to create npm runtime directory {}: {error}",
            stage_dir.join("runtime").display()
        )
    })?;
    fs::create_dir_all(&pack_dir)
        .map_err(|error| format!("failed to create {}: {error}", pack_dir.display()))?;

    let staged_binary = stage_dir.join("runtime/sce");
    fs::copy(&packaged_sce_binary, &staged_binary).map_err(|error| {
        format!(
            "failed to stage {} into {}: {error}",
            packaged_sce_binary.display(),
            staged_binary.display()
        )
    })?;

    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&staged_binary)
            .map_err(|error| format!("failed to inspect {}: {error}", staged_binary.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&staged_binary, permissions).map_err(|error| {
            format!(
                "failed to set executable permissions on {}: {error}",
                staged_binary.display()
            )
        })?;
    }

    let npm = harness.resolve_program("npm")?;
    let pack_output = harness.run_command_in_dir_with_env(
        &npm,
        ["pack", "--silent", stage_dir.to_string_lossy().as_ref()],
        &pack_dir,
        [("SCE_NPM_SKIP_DOWNLOAD", "1")],
    )?;

    if !pack_output.status.success() {
        let mut message =
            format!("[FAIL] channel={channel_name} npm pack failed for local fixture");
        if !pack_output.stdout.is_empty() {
            message.push('\n');
            message.push_str(&pack_output.stdout);
        }
        if !pack_output.stderr.is_empty() {
            message.push('\n');
            message.push_str(&pack_output.stderr);
        }
        return Err(message);
    }

    let package_name = pack_output
        .stdout
        .lines()
        .last()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| {
            format!("[FAIL] channel={channel_name} npm pack did not report a tarball name.")
        })?;
    let package_tarball = pack_dir.join(package_name);

    if !package_tarball.is_file() {
        return Err(format!(
            "[FAIL] channel={channel_name} expected packed tarball was not created: {}",
            package_tarball.display()
        ));
    }

    println!(
        "[PASS] channel={channel_name} local package fixture built from @.version state: {}",
        package_tarball.display()
    );
    Ok(package_tarball)
}

fn add_runtime_to_staged_package_manifest(package_json_path: &Path) -> Result<(), String> {
    let package_json = fs::read_to_string(package_json_path)
        .map_err(|error| format!("failed to read {}: {error}", package_json_path.display()))?;
    let updated_package_json = package_json.replace(
        "\"lib\",\n\t\t\"README.md\"",
        "\"lib\",\n\t\t\"runtime\",\n\t\t\"README.md\"",
    );

    if updated_package_json == package_json {
        return Err(format!(
            "failed to inject runtime/ into staged package manifest {}",
            package_json_path.display()
        ));
    }

    fs::write(package_json_path, updated_package_json)
        .map_err(|error| format!("failed to write {}: {error}", package_json_path.display()))
}

fn assert_sce_doctor_success(harness: &ChannelHarness, sce_binary: &Path) -> Result<(), String> {
    let output = harness.run_command(sce_binary, ["doctor", "--format", "json"])?;

    if !output.status.success() {
        let mut message = format!(
            "[FAIL] channel=npm sce doctor failed via {}",
            sce_binary.display()
        );
        if !output.stdout.is_empty() {
            message.push('\n');
            message.push_str(&output.stdout);
        }
        if !output.stderr.is_empty() {
            message.push('\n');
            message.push_str(&output.stderr);
        }
        return Err(message);
    }

    Ok(())
}
