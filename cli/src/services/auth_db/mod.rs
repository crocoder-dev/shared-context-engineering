use std::path::PathBuf;

use anyhow::Result;

use crate::{
    generated_migrations,
    services::{
        db::{DatabaseKind, DbSpec, EncryptedTursoDb},
        default_paths::auth_db_path,
    },
};

pub struct AuthDbSpec;

impl DbSpec for AuthDbSpec {
    fn db_name() -> &'static str {
        "auth DB"
    }

    fn db_path() -> Result<PathBuf> {
        auth_db_path()
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        generated_migrations::AUTH_MIGRATIONS
    }

    const KIND: DatabaseKind = DatabaseKind::Auth;
}

pub type AuthDb = EncryptedTursoDb<AuthDbSpec>;

pub mod lifecycle;
