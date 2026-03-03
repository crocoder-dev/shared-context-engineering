use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

#[derive(Debug)]
pub struct TestTempDir {
    path: PathBuf,
}

impl TestTempDir {
    pub fn new(prefix: &str) -> Result<Self> {
        let epoch_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system time should be after unix epoch")?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{epoch_nanos}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
