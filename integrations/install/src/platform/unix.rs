use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::error::HarnessError;

pub fn ensure_executable(path: &Path) -> Result<(), HarnessError> {
    let metadata = path.metadata().map_err(|_| HarnessError::UnixOnly)?;
    if !metadata.is_file() {
        return Err(HarnessError::UnixOnly);
    }

    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(HarnessError::UnixOnly);
    }

    Ok(())
}

pub fn set_executable_permissions(path: &Path) -> Result<(), HarnessError> {
    let mut permissions = fs::metadata(path)
        .map_err(|e| HarnessError::FileInspect {
            path: path.to_path_buf(),
            error: e.to_string(),
        })?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|e| HarnessError::PermissionSet {
        path: path.to_path_buf(),
        error: e.to_string(),
    })?;
    Ok(())
}
