use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::error::HarnessError;

pub fn ensure_executable(path: &Path) -> Result<(), HarnessError> {
    let metadata = path.metadata().map_err(|e| HarnessError::FileInspect {
        path: path.to_path_buf(),
        error: e.to_string(),
    })?;

    if !metadata.is_file() {
        return Err(HarnessError::NotAFile {
            path: path.to_path_buf(),
        });
    }

    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(HarnessError::MissingExecutePermission {
            path: path.to_path_buf(),
        });
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
    let current_mode = permissions.mode();
    let new_mode = current_mode | 0o111;
    permissions.set_mode(new_mode);
    fs::set_permissions(path, permissions).map_err(|e| HarnessError::PermissionSet {
        path: path.to_path_buf(),
        error: e.to_string(),
    })?;
    Ok(())
}
