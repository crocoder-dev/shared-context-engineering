use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

const COLD_OPEN_LOCK_SUFFIX: &str = "-open.lock";

#[cfg(target_os = "linux")]
const SHARED_LOCK_COMMAND: libc::c_int = libc::F_OFD_SETLK;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
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

#[derive(Debug)]
pub(super) struct SharedLockConflict {
    db_name: String,
    db_path: PathBuf,
}

impl fmt::Display for SharedLockConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} database at {} is open in an older SCE process that does not support shared access. Try: wait for older SCE processes to exit, upgrade every installed 'sce' binary, and rerun the command",
            self.db_name,
            self.db_path.display()
        )
    }
}

impl std::error::Error for SharedLockConflict {}

pub(super) fn lock_shared(file: &File, db_name: &str, db_path: &Path) -> Result<()> {
    match try_lock_shared(file) {
        Ok(()) => Ok(()),
        Err(error) if is_lock_conflict(&error) => Err(SharedLockConflict {
            db_name: db_name.to_owned(),
            db_path: db_path.to_path_buf(),
        }
        .into()),
        Err(error) => Err(anyhow!(
            "failed to lock {db_name} database at {}: {error}",
            db_path.display()
        )),
    }
}

fn cold_open_lock_path(db_path: &Path) -> PathBuf {
    let mut path = db_path.as_os_str().to_owned();
    path.push(COLD_OPEN_LOCK_SUFFIX);
    PathBuf::from(path)
}

pub(super) async fn lock_cold_open(db_path: &Path, db_name: &str) -> Result<File> {
    let lock_path = cold_open_lock_path(db_path);
    let db_name = db_name.to_owned();
    tokio::task::spawn_blocking(move || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .and_then(|file| file.lock().map(|()| file))
            .map_err(|error| {
                anyhow!(
                    "failed to serialize opening the {db_name} database with {}: {error}",
                    lock_path.display()
                )
            })
    })
    .await
    .context("database cold-open lock worker failed")?
}

#[cfg(target_os = "macos")]
fn try_lock_shared(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
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

#[cfg(all(unix, not(target_os = "macos")))]
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
    matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN || code == libc::EACCES
    )
}

#[cfg(not(unix))]
fn is_lock_conflict(_error: &io::Error) -> bool {
    false
}
