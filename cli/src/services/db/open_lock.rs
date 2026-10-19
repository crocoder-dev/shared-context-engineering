use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use anyhow::{anyhow, Result};

#[cfg(target_os = "linux")]
const SHARED_LOCK_COMMAND: libc::c_int = libc::F_OFD_SETLK;
#[cfg(all(unix, not(target_os = "linux")))]
const SHARED_LOCK_COMMAND: libc::c_int = libc::F_SETLK;

pub(super) fn open_lock_file(db_path: &Path, create: bool) -> Result<Option<File>> {
    match OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .open(db_path)
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if !create && error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(anyhow!(
            "failed to open database file {} for its compatibility lock: {error}",
            db_path.display()
        )),
    }
}

pub(super) fn lock_shared(file: &File, db_name: &str, db_path: &Path) -> Result<()> {
    match try_lock_shared(file) {
        Ok(()) => Ok(()),
        Err(error) if is_lock_conflict(&error) => Err(anyhow!(
            "{db_name} database at {} is open in an older SCE process that does not support shared access. Try: wait for older SCE processes to exit, upgrade every installed 'sce' binary, and rerun the command",
            db_path.display()
        )),
        Err(error) => Err(anyhow!(
            "failed to lock {db_name} database at {}: {error}",
            db_path.display()
        )),
    }
}

#[cfg(unix)]
fn try_lock_shared(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = lock_field(libc::F_RDLCK)?;
    lock.l_whence = lock_field(libc::SEEK_SET)?;
    if unsafe { libc::fcntl(file.as_raw_fd(), SHARED_LOCK_COMMAND, &raw const lock) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn lock_field(value: impl TryInto<libc::c_short>) -> io::Result<libc::c_short> {
    value
        .try_into()
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

#[cfg(not(unix))]
fn try_lock_shared(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn is_lock_conflict(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::EAGAIN | libc::EACCES))
}

#[cfg(not(unix))]
fn is_lock_conflict(_error: &io::Error) -> bool {
    false
}
