//! Encrypted auth Turso database adapter.

use std::path::PathBuf;

use anyhow::Result;

use crate::services::{
    db::{DbSpec, EncryptedTursoDb},
    default_paths::auth_db_path,
};

const CREATE_AUTH_TOKENS_MIGRATION: &str =
    include_str!("../../../migrations/auth/001_create_auth_tokens.sql");
const CREATE_AUTH_TOKENS_EMAIL_INDEX_MIGRATION: &str =
    include_str!("../../../migrations/auth/002_create_auth_tokens_email_index.sql");

const AUTH_MIGRATIONS: &[(&str, &str)] = &[
    ("001_create_auth_tokens", CREATE_AUTH_TOKENS_MIGRATION),
    (
        "002_create_auth_tokens_email_index",
        CREATE_AUTH_TOKENS_EMAIL_INDEX_MIGRATION,
    ),
];

/// Encrypted auth database configuration.
pub struct AuthDbSpec;

impl DbSpec for AuthDbSpec {
    fn db_name() -> &'static str {
        "auth DB"
    }

    fn db_path() -> Result<PathBuf> {
        auth_db_path()
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        AUTH_MIGRATIONS
    }
}

/// Encrypted auth Turso database adapter.
pub type AuthDb = EncryptedTursoDb<AuthDbSpec>;

pub mod lifecycle;
