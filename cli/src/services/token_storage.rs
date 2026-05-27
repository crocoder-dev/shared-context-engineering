use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::services::auth::TokenResponse;
use crate::services::auth_db::AuthDb;
use crate::services::default_paths::auth_db_path;

/// Constant row ID for the single token row in `auth_credentials`.
const DEFAULT_TOKEN_ROW_ID: i64 = 1;

/// Lazy singleton for the encrypted auth database.
///
/// Stores `Result` so initialization failures are preserved across calls.
static AUTH_DB: OnceLock<Result<AuthDb, String>> = OnceLock::new();

fn get_auth_db() -> Result<&'static AuthDb, TokenStorageError> {
    let result = AUTH_DB.get_or_init(|| AuthDb::new().map_err(|e| e.to_string()));
    match result {
        Ok(db) => Ok(db),
        Err(msg) => Err(TokenStorageError::Database(msg.clone())),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredTokens {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: Option<String>,
    pub stored_at_unix_seconds: u64,
}

impl StoredTokens {
    fn from_token_response(token: &TokenResponse) -> Result<Self, TokenStorageError> {
        let stored_at_unix_seconds = current_unix_timestamp_seconds()?;
        Ok(Self {
            access_token: token.access_token.clone(),
            token_type: token.token_type.clone(),
            expires_in: token.expires_in,
            refresh_token: token.refresh_token.clone(),
            scope: token.scope.clone(),
            stored_at_unix_seconds,
        })
    }
}

#[derive(Debug)]
pub enum TokenStorageError {
    PathResolution(String),
    Database(String),
}

impl fmt::Display for TokenStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathResolution(reason) => write!(
                f,
                "Unable to resolve token storage path: {reason}. Try: set a valid user home/state directory and retry."
            ),
            Self::Database(reason) => write!(
                f,
                "Token storage database error: {reason}. Try: ensure the OS credential store is available and the auth database is accessible."
            ),
        }
    }
}

impl std::error::Error for TokenStorageError {}

pub fn save_tokens(token: &TokenResponse) -> Result<StoredTokens, TokenStorageError> {
    let db = get_auth_db()?;
    let stored = StoredTokens::from_token_response(token)?;

    let expires_in = i64::try_from(stored.expires_in).map_err(|error| {
        TokenStorageError::Database(format!(
            "expires_in value is out of range for database storage: {error}"
        ))
    })?;
    let stored_at_unix_seconds = i64::try_from(stored.stored_at_unix_seconds).map_err(|error| {
        TokenStorageError::Database(format!(
            "stored_at_unix_seconds value is out of range for database storage: {error}"
        ))
    })?;

    let sql = "INSERT OR REPLACE INTO auth_credentials \
        (id, access_token, token_type, expires_in, refresh_token, scope, stored_at_unix_seconds) \
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

    db.execute(
        sql,
        (
            DEFAULT_TOKEN_ROW_ID,
            stored.access_token.as_str(),
            stored.token_type.as_str(),
            expires_in,
            stored.refresh_token.as_str(),
            stored.scope.as_deref(),
            stored_at_unix_seconds,
        ),
    )
    .map_err(|e| TokenStorageError::Database(e.to_string()))?;

    Ok(stored)
}

pub fn load_tokens() -> Result<Option<StoredTokens>, TokenStorageError> {
    let db = get_auth_db()?;

    let sql = "SELECT access_token, token_type, expires_in, refresh_token, scope, \
        stored_at_unix_seconds FROM auth_credentials WHERE id = ?1";

    let rows: Vec<StoredTokens> = db
        .query_map(sql, (DEFAULT_TOKEN_ROW_ID,), |row| {
            let access_token: String = row.get(0)?;
            let token_type: String = row.get(1)?;
            let expires_in: i64 = row.get(2)?;
            let refresh_token: String = row.get(3)?;
            let scope: Option<String> = row.get(4)?;
            let stored_at_unix_seconds: i64 = row.get(5)?;

            let expires_in = u64::try_from(expires_in).map_err(|error| {
                anyhow::anyhow!("expires_in must be a non-negative integer: {error}")
            })?;
            let stored_at_unix_seconds =
                u64::try_from(stored_at_unix_seconds).map_err(|error| {
                    anyhow::anyhow!(
                        "stored_at_unix_seconds must be a non-negative integer: {error}"
                    )
                })?;

            Ok(StoredTokens {
                access_token,
                token_type,
                expires_in,
                refresh_token,
                scope,
                stored_at_unix_seconds,
            })
        })
        .map_err(|e| TokenStorageError::Database(e.to_string()))?;

    Ok(rows.into_iter().next())
}

pub fn delete_tokens() -> Result<bool, TokenStorageError> {
    let db = get_auth_db()?;

    let affected = db
        .execute(
            "DELETE FROM auth_credentials WHERE id = ?1",
            (DEFAULT_TOKEN_ROW_ID,),
        )
        .map_err(|e| TokenStorageError::Database(e.to_string()))?;

    Ok(affected > 0)
}

pub fn token_file_path() -> Result<PathBuf, TokenStorageError> {
    auth_db_path().map_err(|error| TokenStorageError::PathResolution(error.to_string()))
}

fn current_unix_timestamp_seconds() -> Result<u64, TokenStorageError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            TokenStorageError::PathResolution(format!("system clock is invalid: {error}"))
        })?
        .as_secs())
}
