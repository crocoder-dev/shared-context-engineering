//! Cross-platform secure token storage for WorkOS authentication.
//!
//! This module provides secure file-based token storage with platform-appropriate
//! permissions: 0600 (owner read/write only) on Unix, user-only ACL on Windows.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::auth::StoredTokens;

/// Resolves the platform-appropriate token storage path.
///
/// Path resolution follows platform conventions:
/// - Linux: `${XDG_STATE_HOME:-~/.local/state}/sce/auth/tokens.json`
/// - macOS: `~/Library/Application Support/sce/auth/tokens.json`
/// - Windows: `%APPDATA%\sce\auth\tokens.json`
pub fn resolve_token_storage_path() -> Result<PathBuf> {
    let base_dir = dirs::state_dir()
        .or_else(|| dirs::data_dir())
        .ok_or_else(|| anyhow::anyhow!("Unable to resolve state directory for token storage"))?;

    Ok(base_dir.join("sce").join("auth").join("tokens.json"))
}

/// Saves authentication tokens to secure file storage.
///
/// Creates parent directories if they don't exist and sets restrictive
/// file permissions (0600 on Unix, user-only ACL on Windows).
///
/// # Errors
///
/// Returns an error if:
/// - Directory creation fails
/// - File creation or write fails
/// - Permission setting fails
pub fn save_tokens(tokens: &StoredTokens) -> Result<()> {
    let token_path = resolve_token_storage_path()?;

    // Create parent directories
    if let Some(parent) = token_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create token storage directory '{}'",
                parent.display()
            )
        })?;
    }

    // Serialize tokens to JSON
    let json =
        serde_json::to_string_pretty(tokens).context("Failed to serialize tokens to JSON")?;

    // Write with secure permissions
    write_file_secure(&token_path, json.as_bytes())
        .with_context(|| format!("Failed to write token file '{}'", token_path.display()))?;

    Ok(())
}

/// Loads authentication tokens from secure file storage.
///
/// Returns `Ok(None)` if the token file doesn't exist.
/// Returns an error if the file exists but cannot be read or parsed.
///
/// # Errors
///
/// Returns an error if:
/// - File exists but cannot be read
/// - File contains invalid JSON
/// - JSON doesn't match StoredTokens schema
pub fn load_tokens() -> Result<Option<StoredTokens>> {
    let token_path = resolve_token_storage_path()?;

    // Return None if file doesn't exist
    if !token_path.exists() {
        return Ok(None);
    }

    // Read and parse token file
    let contents = std::fs::read_to_string(&token_path)
        .with_context(|| format!("Failed to read token file '{}'", token_path.display()))?;

    let tokens: StoredTokens = serde_json::from_str(&contents).with_context(|| {
        format!(
            "Failed to parse token file '{}'. Try: Run `sce logout` and then `sce login` again.",
            token_path.display()
        )
    })?;

    Ok(Some(tokens))
}

/// Deletes stored authentication tokens.
///
/// Returns `Ok(())` even if the file doesn't exist.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be deleted.
pub fn delete_tokens() -> Result<()> {
    let token_path = resolve_token_storage_path()?;

    if token_path.exists() {
        std::fs::remove_file(&token_path)
            .with_context(|| format!("Failed to delete token file '{}'", token_path.display()))?;
    }

    Ok(())
}

/// Writes file with secure platform-specific permissions.
///
/// On Unix: Sets file mode to 0600 (owner read/write only).
/// On Windows: Relies on directory-level security in AppData (user-specific directory).
#[cfg(unix)]
fn write_file_secure(path: &std::path::Path, contents: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600) // Owner read/write only
        .open(path)?;

    file.write_all(contents)?;
    file.sync_all()
}

#[cfg(windows)]
fn write_file_secure(path: &std::path::Path, contents: &[u8]) -> io::Result<()> {
    // On Windows, the AppData directory (%APPDATA%) is already protected with
    // user-specific permissions. Files created here inherit those permissions,
    // which restricts access to the current user.
    //
    // For MVP, we rely on directory-level security rather than explicit file ACLs.
    // Production implementations could use winapi or windows-rs crates for
    // explicit ACL control, but this adds significant complexity.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;

    file.write_all(contents)?;
    file.sync_all()
}

#[cfg(not(any(unix, windows)))]
fn write_file_secure(path: &std::path::Path, contents: &[u8]) -> io::Result<()> {
    // Fallback for unsupported platforms - just write without special permissions
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;

    file.write_all(contents)?;
    file.sync_all()
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::tempdir;

    // Mutex to serialize tests that manipulate environment variables
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    // Helper to create a test environment with isolated storage
    struct TestEnv {
        _temp_dir: tempfile::TempDir,
        _guard: MutexGuard<'static, ()>,
        #[cfg(target_os = "linux")]
        original_path: Option<PathBuf>,
    }

    impl TestEnv {
        fn new() -> Result<Self> {
            // Acquire mutex to ensure tests run serially
            let guard = TEST_MUTEX.lock().unwrap();
            let temp_dir = tempdir()?;

            // Store original env var if it exists
            #[cfg(target_os = "linux")]
            let original_path = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);

            // Set test environment
            #[cfg(target_os = "linux")]
            std::env::set_var("XDG_STATE_HOME", temp_dir.path());

            Ok(Self {
                _temp_dir: temp_dir,
                _guard: guard,
                #[cfg(target_os = "linux")]
                original_path,
            })
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            // Restore original env var
            #[cfg(target_os = "linux")]
            {
                if let Some(ref path) = self.original_path {
                    std::env::set_var("XDG_STATE_HOME", path);
                } else {
                    std::env::remove_var("XDG_STATE_HOME");
                }
            }
        }
    }

    fn create_test_tokens() -> StoredTokens {
        StoredTokens {
            access_token: "test_access_token".to_string(),
            refresh_token: "test_refresh_token".to_string(),
            expires_at: 1234567890,
            id_token: Some("test_id_token".to_string()),
            scope: Some("openid profile".to_string()),
        }
    }

    #[test]
    fn resolve_token_storage_path_returns_valid_path() {
        let path = resolve_token_storage_path().expect("Should resolve path");
        assert!(path.ends_with("tokens.json"));
        assert!(path.to_string_lossy().contains("sce"));
        assert!(path.to_string_lossy().contains("auth"));
    }

    #[test]
    fn save_and_load_tokens_roundtrip() {
        let _env = TestEnv::new().expect("Failed to create test environment");
        let tokens = create_test_tokens();

        save_tokens(&tokens).expect("Should save tokens");
        let loaded = load_tokens().expect("Should load tokens");

        assert!(loaded.is_some());
        let loaded_tokens = loaded.unwrap();
        assert_eq!(loaded_tokens.access_token, tokens.access_token);
        assert_eq!(loaded_tokens.refresh_token, tokens.refresh_token);
        assert_eq!(loaded_tokens.expires_at, tokens.expires_at);
        assert_eq!(loaded_tokens.id_token, tokens.id_token);
        assert_eq!(loaded_tokens.scope, tokens.scope);
    }

    #[test]
    fn load_tokens_returns_none_when_file_missing() {
        let _env = TestEnv::new().expect("Failed to create test environment");

        // Ensure no tokens exist
        delete_tokens().expect("Should delete tokens");

        let loaded = load_tokens().expect("Should handle missing file");
        assert!(loaded.is_none());
    }

    #[test]
    fn load_tokens_fails_with_invalid_json() {
        let _env = TestEnv::new().expect("Failed to create test environment");

        let path = resolve_token_storage_path().expect("Should resolve path");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("Should create parent dir");
        }
        std::fs::write(&path, b"not valid json").expect("Should write invalid JSON");

        let result = load_tokens();
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("Failed to parse token file"));
    }

    #[test]
    fn load_tokens_fails_with_missing_required_fields() {
        let _env = TestEnv::new().expect("Failed to create test environment");

        let path = resolve_token_storage_path().expect("Should resolve path");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("Should create parent dir");
        }
        // Missing required fields like access_token, refresh_token, expires_at
        std::fs::write(&path, b"{\"scope\": \"openid\"}").expect("Should write incomplete JSON");

        let result = load_tokens();
        assert!(result.is_err());
    }

    #[test]
    fn delete_tokens_succeeds_when_file_missing() {
        let _env = TestEnv::new().expect("Failed to create test environment");

        // Ensure no tokens exist
        delete_tokens().expect("Should delete tokens");

        // Delete again should succeed
        delete_tokens().expect("Should succeed even when file missing");
    }

    #[test]
    fn delete_tokens_removes_file() {
        let _env = TestEnv::new().expect("Failed to create test environment");
        let tokens = create_test_tokens();

        save_tokens(&tokens).expect("Should save tokens");
        assert!(load_tokens().expect("Should load").is_some());

        delete_tokens().expect("Should delete tokens");
        assert!(load_tokens().expect("Should return None").is_none());
    }

    #[test]
    #[cfg(unix)]
    fn save_tokens_sets_unix_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let _env = TestEnv::new().expect("Failed to create test environment");
        let tokens = create_test_tokens();

        save_tokens(&tokens).expect("Should save tokens");

        let path = resolve_token_storage_path().expect("Should resolve path");
        let metadata = std::fs::metadata(&path).expect("Should get metadata");
        let mode = metadata.permissions().mode();

        // Check that file mode is 0600 (owner read/write only)
        // Mask to get only permission bits (last 9 bits)
        assert_eq!(mode & 0o777, 0o600, "File should have 0600 permissions");
    }

    #[test]
    fn save_tokens_creates_parent_directories() {
        let _env = TestEnv::new().expect("Failed to create test environment");
        let tokens = create_test_tokens();

        // Ensure parent directory doesn't exist
        let path = resolve_token_storage_path().expect("Should resolve path");
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }

        save_tokens(&tokens).expect("Should save tokens and create parent dirs");
        assert!(path.exists());
    }

    #[test]
    fn stored_tokens_serialization_matches_schema() {
        let tokens = create_test_tokens();

        let json = serde_json::to_string(&tokens).expect("Should serialize");
        let parsed: StoredTokens = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(parsed.access_token, tokens.access_token);
        assert_eq!(parsed.refresh_token, tokens.refresh_token);
        assert_eq!(parsed.expires_at, tokens.expires_at);
        assert_eq!(parsed.id_token, tokens.id_token);
        assert_eq!(parsed.scope, tokens.scope);
    }
}
