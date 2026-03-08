use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::services::auth::TokenResponse;

const TOKEN_FILE_SUBPATH: &str = "sce/auth/tokens.json";

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
    Io(std::io::Error),
    Serialization(serde_json::Error),
    CorruptedTokenFile(String),
    Permission(String),
}

impl fmt::Display for TokenStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathResolution(reason) => write!(
                f,
                "Unable to resolve token storage path: {reason}. Try: set a valid user home/state directory and retry."
            ),
            Self::Io(error) => write!(
                f,
                "Failed to read or write authentication tokens: {error}. Try: verify file permissions for the auth state directory."
            ),
            Self::Serialization(error) => write!(
                f,
                "Failed to serialize authentication tokens: {error}. Try: rerun login to regenerate credentials."
            ),
            Self::CorruptedTokenFile(reason) => write!(
                f,
                "Stored authentication tokens are invalid: {reason}. Try: run 'sce logout' and then 'sce login'."
            ),
            Self::Permission(reason) => write!(
                f,
                "Unable to apply secure token file permissions: {reason}. Try: verify local account permissions and retry."
            ),
        }
    }
}

impl std::error::Error for TokenStorageError {}

impl From<std::io::Error> for TokenStorageError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for TokenStorageError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value)
    }
}

pub fn save_tokens(token: &TokenResponse) -> Result<StoredTokens, TokenStorageError> {
    let token_path = token_file_path()?;
    let stored = StoredTokens::from_token_response(token)?;
    save_tokens_at_path(&token_path, &stored)?;
    Ok(stored)
}

pub fn load_tokens() -> Result<Option<StoredTokens>, TokenStorageError> {
    let token_path = token_file_path()?;
    load_tokens_from_path(&token_path)
}

pub fn token_file_path() -> Result<PathBuf, TokenStorageError> {
    #[cfg(target_os = "linux")]
    {
        return linux_token_file_path();
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let Some(data_dir) = dirs::data_dir() else {
            return Err(TokenStorageError::PathResolution(
                "data directory could not be resolved".to_string(),
            ));
        };
        return Ok(data_dir.join(TOKEN_FILE_SUBPATH));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        if let Some(state_dir) = dirs::state_dir() {
            return Ok(state_dir.join(TOKEN_FILE_SUBPATH));
        }
        if let Some(data_dir) = dirs::data_dir() {
            return Ok(data_dir.join(TOKEN_FILE_SUBPATH));
        }
        Err(TokenStorageError::PathResolution(
            "state and data directories could not be resolved".to_string(),
        ))
    }
}

fn save_tokens_at_path(path: &Path, stored: &StoredTokens) -> Result<(), TokenStorageError> {
    ensure_parent_directory(path)?;

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;

    apply_secure_file_permissions(path)?;

    let encoded = serde_json::to_vec_pretty(stored)?;
    file.write_all(&encoded)?;
    file.write_all(b"\n")?;
    file.sync_all()?;

    Ok(())
}

fn load_tokens_from_path(path: &Path) -> Result<Option<StoredTokens>, TokenStorageError> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(TokenStorageError::Io(error)),
    };

    let parsed: StoredTokens = serde_json::from_str(&content).map_err(|error| {
        TokenStorageError::CorruptedTokenFile(format!("{} ({error})", path.display()))
    })?;

    Ok(Some(parsed))
}

fn ensure_parent_directory(path: &Path) -> Result<(), TokenStorageError> {
    let Some(parent) = path.parent() else {
        return Err(TokenStorageError::PathResolution(format!(
            "token path '{}' has no parent directory",
            path.display()
        )));
    };

    fs::create_dir_all(parent)?;
    apply_secure_directory_permissions(parent)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_token_file_path() -> Result<PathBuf, TokenStorageError> {
    if let Some(state_dir) = dirs::state_dir() {
        return Ok(state_dir.join(TOKEN_FILE_SUBPATH));
    }

    let Some(home_dir) = dirs::home_dir() else {
        return Err(TokenStorageError::PathResolution(
            "home directory could not be resolved for Linux fallback".to_string(),
        ));
    };

    Ok(home_dir
        .join(".local")
        .join("state")
        .join(TOKEN_FILE_SUBPATH))
}

fn current_unix_timestamp_seconds() -> Result<u64, TokenStorageError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            TokenStorageError::PathResolution(format!("system clock is invalid: {error}"))
        })?
        .as_secs())
}

#[cfg(unix)]
fn apply_secure_directory_permissions(path: &Path) -> Result<(), TokenStorageError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn apply_secure_directory_permissions(_path: &Path) -> Result<(), TokenStorageError> {
    Ok(())
}

#[cfg(unix)]
fn apply_secure_file_permissions(path: &Path) -> Result<(), TokenStorageError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(windows)]
fn apply_secure_file_permissions(path: &Path) -> Result<(), TokenStorageError> {
    use std::process::Command;

    let username = std::env::var("USERNAME").map_err(|_| {
        TokenStorageError::Permission(
            "USERNAME environment variable is unavailable on Windows".to_string(),
        )
    })?;

    let grant_rule = format!("{username}:(R,W)");
    let output = Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(grant_rule)
        .output()
        .map_err(|error| {
            TokenStorageError::Permission(format!("failed to execute icacls: {error}"))
        })?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(TokenStorageError::Permission(format!(
            "icacls failed for '{}': {stderr}",
            path.display()
        )))
    }
}

#[cfg(not(any(unix, windows)))]
fn apply_secure_file_permissions(_path: &Path) -> Result<(), TokenStorageError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{load_tokens_from_path, save_tokens_at_path, StoredTokens};
    use std::fs;
    use std::path::PathBuf;

    fn unique_test_path(test_name: &str) -> PathBuf {
        let unique = format!(
            "sce-token-storage-{}-{}-{}",
            test_name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        );
        std::env::temp_dir().join(unique).join("tokens.json")
    }

    fn fixture_tokens() -> StoredTokens {
        StoredTokens {
            access_token: "access-token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            refresh_token: "refresh-token".to_string(),
            scope: Some("openid profile".to_string()),
            stored_at_unix_seconds: 1_700_000_000,
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let token_path = unique_test_path("round-trip");
        let tokens = fixture_tokens();

        save_tokens_at_path(&token_path, &tokens).expect("tokens should save");

        let loaded = load_tokens_from_path(&token_path)
            .expect("load should succeed")
            .expect("tokens should exist");
        assert_eq!(loaded, tokens);

        let _ = fs::remove_dir_all(
            token_path
                .parent()
                .and_then(|parent| parent.parent())
                .expect("temp tree should have two parent levels"),
        );
    }

    #[test]
    fn load_missing_token_file_returns_none() {
        let token_path = unique_test_path("missing-file");
        let loaded = load_tokens_from_path(&token_path).expect("missing file should not error");
        assert!(loaded.is_none());
    }

    #[test]
    fn load_invalid_json_returns_corruption_error() {
        let token_path = unique_test_path("invalid-json");
        let parent = token_path.parent().expect("token file should have parent");
        fs::create_dir_all(parent).expect("should create parent directory");
        fs::write(&token_path, "{not valid json").expect("should write invalid payload");

        let error = load_tokens_from_path(&token_path).expect_err("invalid json should fail");
        let message = error.to_string();
        assert!(message.contains("Stored authentication tokens are invalid"));

        let _ = fs::remove_dir_all(
            token_path
                .parent()
                .and_then(|path| path.parent())
                .expect("temp tree should have two parent levels"),
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_unix_file_permissions_to_0600() {
        use std::os::unix::fs::PermissionsExt;

        let token_path = unique_test_path("unix-perms");
        save_tokens_at_path(&token_path, &fixture_tokens()).expect("tokens should save");

        let metadata = fs::metadata(&token_path).expect("token file should exist");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(
            token_path
                .parent()
                .and_then(|path| path.parent())
                .expect("temp tree should have two parent levels"),
        );
    }
}
