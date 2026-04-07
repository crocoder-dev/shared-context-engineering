#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::{ensure_executable, set_executable_permissions};

#[cfg(not(unix))]
pub fn ensure_executable(_path: &std::path::Path) -> Result<(), crate::error::HarnessError> {
    Err(crate::error::HarnessError::UnixOnly)
}

#[cfg(not(unix))]
pub fn set_executable_permissions(
    _path: &std::path::Path,
) -> Result<(), crate::error::HarnessError> {
    Err(crate::error::HarnessError::UnixOnly)
}
