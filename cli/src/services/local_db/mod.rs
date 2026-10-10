//! Local Turso database adapter.

use std::path::PathBuf;

use anyhow::Result;

use crate::services::{
    db::{DatabaseKind, DbSpec, TursoDb},
    default_paths::local_db_path,
};

pub mod lifecycle;

/// Local database configuration.
pub struct LocalDbSpec;

impl DbSpec for LocalDbSpec {
    fn db_name() -> &'static str {
        "local DB"
    }

    fn db_path() -> Result<PathBuf> {
        local_db_path()
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        &[]
    }

    const KIND: DatabaseKind = DatabaseKind::Local;
}

/// Local Turso database adapter.
pub type LocalDb = TursoDb<LocalDbSpec>;
