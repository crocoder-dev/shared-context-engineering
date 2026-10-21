use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

const NEW_FILE_MODE: u32 = 0o600;
const TEMP_SUFFIX: &str = ".sce-tmp";

pub(super) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Settings path '{}' has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create '{}'", parent.display()))?;

    let mut temp_name = path.as_os_str().to_os_string();
    temp_name.push(TEMP_SUFFIX);
    let temp_path = PathBuf::from(temp_name);

    fs::write(&temp_path, bytes)
        .with_context(|| format!("Failed to write '{}'", temp_path.display()))?;
    apply_mode(path, &temp_path)?;
    fs::rename(&temp_path, path).with_context(|| {
        let _ = fs::remove_file(&temp_path);
        format!("Failed to replace '{}'", path.display())
    })
}

#[cfg(unix)]
pub(super) fn apply_mode(target: &Path, temp_path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = match fs::metadata(target) {
        Ok(metadata) => metadata.permissions().mode() & 0o7777,
        Err(_) => NEW_FILE_MODE,
    };
    fs::set_permissions(temp_path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set permissions on '{}'", temp_path.display()))
}

#[cfg(not(unix))]
pub(super) fn apply_mode(_target: &Path, _temp_path: &Path) -> Result<()> {
    Ok(())
}
