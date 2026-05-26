//! Encryption key management backed by the OS credential store.
//!
//! Provides a single entry point to get-or-create a 64-character hex
//! encryption key stored in the platform-native credential store
//! (macOS Keychain, Linux keyutils, Windows Credential Store).
//!
//! On first use for a given database name (when the database file does
//! not yet exist), a random 32-byte key is generated, hex-encoded,
//! persisted in the credential store, and returned. Subsequent calls
//! read the key from the credential store.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use keyring_core::Entry;

/// Guards the one-time registration of the platform-native credential store.
///
/// A `Mutex` is used instead of `OnceLock::get_or_try_init` because that
/// API is still unstable in the current toolchain (1.95.0). The mutex
/// ensures thread-safe single initialization and naturally retries on
/// transient failures (the lock is released when the error propagates).
static DEFAULT_STORE: Mutex<bool> = Mutex::new(false);

fn ensure_default_store() -> Result<()> {
    let mut guard = DEFAULT_STORE
        .lock()
        .map_err(|_| anyhow::anyhow!("internal error: credential store mutex poisoned"))?;

    if !*guard {
        #[cfg(target_os = "linux")]
        {
            keyring_core::set_default_store(
                linux_keyutils_keyring_store::Store::new()
                    .context("failed to create Linux keyutils keyring store")?,
            );
        }
        #[cfg(target_os = "macos")]
        {
            keyring_core::set_default_store(
                apple_native_keyring_store::keychain::Store::new()
                    .context("failed to create macOS keychain store")?,
            );
        }
        #[cfg(target_os = "windows")]
        {
            keyring_core::set_default_store(
                windows_native_keyring_store::Store::new()
                    .context("failed to create Windows credential store")?,
            );
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            anyhow::bail!(
                "unsupported platform: no OS credential store available for encryption key \
                 management. Try: run 'sce' on a supported platform (Linux, macOS, or Windows)."
            );
        }

        *guard = true;
    }

    Ok(())
}

/// Retrieve or create the encryption key for the named database.
///
/// The key is stored in the platform-native credential store under the
/// service name `"sce"` with the database name as the user/account
/// identifier.
///
/// # Arguments
/// * `db_path` — Canonical path to the database file. Used to decide
///   whether this is a first-use (generate new key) or a subsequent
///   access (read existing key).
/// * `db_name` — Logical database name used as the credential store
///   username (e.g. `"auth_db"`, `"agent_trace_db"`).
///
/// # Returns
/// A 64-character lowercase hex string that is the encryption key.
///
/// # Errors
/// - Returns an error if the credential store cannot be initialised on
///   the current platform.
/// - Returns an error if the database file exists but the keyring entry
///   is missing (e.g. keyring was cleared or has expired on Linux).
/// - Returns an error if key generation or credential store I/O fails.
pub fn get_or_create_encryption_key(db_path: &Path, db_name: &str) -> Result<String> {
    ensure_default_store()?;

    let entry = Entry::new("sce", db_name).with_context(|| {
        format!(
            "failed to create keyring entry for service 'sce' / user '{db_name}'. \
             Try: ensure the OS credential store is available and accessible."
        )
    })?;

    // Try to retrieve an existing password from the credential store.
    if let Ok(password) = entry.get_password() {
        return Ok(password);
    }

    // No existing key was found. If the database file does not exist,
    // this is a first-use scenario: generate and store a new key.
    if !db_path.exists() {
        let hex_key = generate_key();

        entry.set_password(&hex_key).with_context(|| {
            format!(
                "failed to store encryption key for '{db_name}' in credential store. \
                 Try: ensure the OS credential store is operational (e.g. 'gnome-keyring' \
                 or 'secret-service' on Linux, Keychain Access on macOS)."
            )
        })?;

        return Ok(hex_key);
    }

    // The database file exists but the keyring entry is missing.
    // This can happen on Linux when the persistent keyring expires
    // (kernel keyutils is in-memory). Provide clear remediation.
    anyhow::bail!(
        "encryption key for '{db_name}' not found in credential store.\n\
         The database file exists at '{}' but no matching credential was found \
         for service 'sce' / user '{db_name}'.\n\
         On Linux, this can happen when the kernel keyring session expires. \
         Try: ensure the OS credential store is available.\n\
         If the database file is also stale or you no longer need its data, \
         delete it and the key will be regenerated automatically on next use.",
        db_path.display()
    );
}

/// Generate a 64-character lowercase hex key from 32 random bytes.
fn generate_key() -> String {
    use rand::Rng;

    let key: [u8; 32] = rand::thread_rng().gen();
    hex_encode(&key)
}

/// Hex-encode a byte slice into a lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut hex = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_encode_empty() {
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn test_hex_encode_32_bytes() {
        let bytes = [0xdeu8; 32];
        let expected = "de".repeat(32);
        assert_eq!(hex_encode(&bytes), expected);
    }

    #[test]
    fn test_hex_encode_mixed() {
        let bytes = [0x00u8, 0x01, 0xfe, 0xff, 0xab, 0xcd];
        assert_eq!(hex_encode(&bytes), "0001feffabcd");
    }

    #[test]
    fn test_generate_key_length() {
        let key = generate_key();
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn test_generate_key_lowercase() {
        let key = generate_key();
        assert!(key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn test_generate_key_is_random() {
        let key1 = generate_key();
        let key2 = generate_key();
        assert_ne!(key1, key2);
    }
}
