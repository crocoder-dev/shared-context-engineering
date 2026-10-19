use std::fmt;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock, PoisonError,
};
use std::thread;

use anyhow::{Context, Result};
use keyring_core::Entry;
use sha2::{Digest, Sha256};

pub const AUTH_DB_ENCRYPTION_KEY_ENV: &str = "SCE_AUTH_DB_ENCRYPTION_KEY";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialAccess {
    Interactive,
    NonInteractive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialAccessUnavailable {
    db_name: String,
}

impl fmt::Display for CredentialAccessUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "encryption key for '{}' is not accessible without user interaction. \
             Try: run 'sce auth login' to authorize credential store access.",
            self.db_name
        )
    }
}

impl std::error::Error for CredentialAccessUnavailable {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialAccessDenied {
    db_name: String,
}

impl fmt::Display for CredentialAccessDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "credential store denied access to the encryption key for '{}'",
            self.db_name
        )
    }
}

impl std::error::Error for CredentialAccessDenied {}

#[cfg(target_os = "macos")]
const ERR_SEC_USER_CANCELED: i32 = -128;
#[cfg(target_os = "macos")]
const ERR_SEC_NO_ACCESS_FOR_ITEM: i32 = -25243;
#[cfg(target_os = "macos")]
const ERR_SEC_AUTH_FAILED: i32 = -25293;
#[cfg(target_os = "macos")]
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
#[cfg(target_os = "macos")]
const ERR_SEC_INTERACTION_REQUIRED: i32 = -25315;
#[cfg(target_os = "macos")]
const MACOS_AUTHORIZATION_FAILURE_CODES: [i32; 5] = [
    ERR_SEC_USER_CANCELED,
    ERR_SEC_NO_ACCESS_FOR_ITEM,
    ERR_SEC_AUTH_FAILED,
    ERR_SEC_INTERACTION_NOT_ALLOWED,
    ERR_SEC_INTERACTION_REQUIRED,
];

static CREDENTIAL_STORE_ACCESS: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CredentialStorePlatform {
    #[cfg(target_os = "linux")]
    Linux,
    #[cfg(target_os = "macos")]
    Macos,
    #[cfg(target_os = "windows")]
    Windows,
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Unsupported,
}

/// Guards the one-time registration of the platform-native credential store.
///
/// `OnceLock::get_or_try_init` is still unstable in the effective stable
/// toolchain, so a small atomic in-progress guard serializes fallible
/// initialization. The `OnceLock` is set only after success; on panic or
/// error, the in-progress guard is released and the next caller can retry.
static DEFAULT_STORE: OnceLock<bool> = OnceLock::new();
static DEFAULT_STORE_INITIALIZING: AtomicBool = AtomicBool::new(false);

struct DefaultStoreInitGuard;

impl Drop for DefaultStoreInitGuard {
    fn drop(&mut self) {
        DEFAULT_STORE_INITIALIZING.store(false, Ordering::Release);
    }
}

fn ensure_default_store() -> Result<()> {
    if DEFAULT_STORE.get().is_some() {
        return Ok(());
    }

    loop {
        if DEFAULT_STORE.get().is_some() {
            return Ok(());
        }

        if DEFAULT_STORE_INITIALIZING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _guard = DefaultStoreInitGuard;
            register_default_store()?;
            let _ = DEFAULT_STORE.set(true);
            return Ok(());
        }

        thread::yield_now();
    }
}

fn register_default_store() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        keyring_core::set_default_store(
            zbus_secret_service_keyring_store::Store::new().with_context(|| {
                credential_store_unavailable_message(CredentialStorePlatform::Linux)
            })?,
        );
    }
    #[cfg(target_os = "macos")]
    {
        keyring_core::set_default_store(
            apple_native_keyring_store::keychain::Store::new().with_context(|| {
                credential_store_unavailable_message(CredentialStorePlatform::Macos)
            })?,
        );
    }
    #[cfg(target_os = "windows")]
    {
        keyring_core::set_default_store(windows_native_keyring_store::Store::new().with_context(
            || credential_store_unavailable_message(CredentialStorePlatform::Windows),
        )?);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        anyhow::bail!(credential_store_unavailable_message(
            CredentialStorePlatform::Unsupported
        ));
    }

    Ok(())
}

pub fn get_or_create_encryption_key(
    db_path: &Path,
    db_name: &str,
    access: CredentialAccess,
) -> Result<String> {
    match access {
        CredentialAccess::Interactive => get_or_create_encryption_key_with_keyring_resolver(
            db_path,
            db_name,
            get_or_create_keyring_encryption_key,
        ),
        CredentialAccess::NonInteractive => {
            get_or_create_encryption_key_with_keyring_resolver(db_path, db_name, |_, db_name| {
                read_keyring_encryption_key_without_interaction(db_name)
            })
        }
    }
}

fn read_keyring_encryption_key_without_interaction(db_name: &str) -> Result<String> {
    let unavailable = || {
        anyhow::Error::new(CredentialAccessUnavailable {
            db_name: db_name.to_owned(),
        })
    };
    ensure_default_store().map_err(|_| unavailable())?;
    let _serialized = CREDENTIAL_STORE_ACCESS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    #[cfg(target_os = "macos")]
    let _interaction_disabled =
        security_framework::os::macos::keychain::SecKeychain::disable_user_interaction()
            .map_err(|_| unavailable())?;
    Entry::new("sce", db_name)
        .and_then(|entry| entry.get_password())
        .map_err(|_| unavailable())
}

fn get_or_create_encryption_key_with_keyring_resolver<F>(
    db_path: &Path,
    db_name: &str,
    keyring_resolver: F,
) -> Result<String>
where
    F: FnOnce(&Path, &str) -> Result<String>,
{
    if let Some(env_key) = env_encryption_key()? {
        return Ok(env_key);
    }

    keyring_resolver(db_path, db_name)
}

fn get_or_create_keyring_encryption_key(db_path: &Path, db_name: &str) -> Result<String> {
    ensure_default_store()?;

    let entry = Entry::new("sce", db_name).with_context(|| {
        format!(
            "failed to create keyring entry for service 'sce' / user '{db_name}'. \
             {}",
            credential_store_unavailable_message(current_credential_store_platform())
        )
    })?;

    let _serialized = CREDENTIAL_STORE_ACCESS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);

    match entry.get_password() {
        Ok(password) => return Ok(password),
        Err(keyring_core::Error::NoEntry) => {}
        Err(error) => {
            return Err(keyring_error(error, db_name)).with_context(|| {
                format!(
                    "failed to read encryption key for '{db_name}' from credential store. {}",
                    credential_store_unavailable_message(current_credential_store_platform())
                )
            });
        }
    }

    if !db_path.exists() {
        let hex_key = generate_key();

        entry
            .set_password(&hex_key)
            .map_err(|error| keyring_error(error, db_name))
            .with_context(|| {
                format!(
                    "failed to store encryption key for '{db_name}' in credential store. {}",
                    credential_store_unavailable_message(current_credential_store_platform())
                )
            })?;

        return Ok(hex_key);
    }

    anyhow::bail!(
        "encryption key for '{db_name}' not found in credential store.\n\
         The database file exists at '{}' but no matching credential was found \
         for service 'sce' / user '{db_name}'.\n\
         This can happen when the {} was cleared or unavailable when the key \
         was first stored. {}\n\
         If the database file is also stale or you no longer need its data, \
         delete it and the key will be regenerated automatically on next use.",
        db_path.display(),
        credential_store_name(current_credential_store_platform()),
        credential_store_unavailable_message(current_credential_store_platform())
    );
}

fn keyring_error(error: keyring_core::Error, db_name: &str) -> anyhow::Error {
    let denied = is_authorization_failure(&error);
    let error = anyhow::Error::new(error);
    if denied {
        error.context(CredentialAccessDenied {
            db_name: db_name.to_owned(),
        })
    } else {
        error
    }
}

fn is_authorization_failure(error: &keyring_core::Error) -> bool {
    match error {
        keyring_core::Error::NoStorageAccess(_) => true,
        #[cfg(target_os = "macos")]
        keyring_core::Error::PlatformFailure(platform) => platform
            .downcast_ref::<security_framework::base::Error>()
            .is_some_and(|error| MACOS_AUTHORIZATION_FAILURE_CODES.contains(&error.code())),
        _ => false,
    }
}

fn current_credential_store_platform() -> CredentialStorePlatform {
    #[cfg(target_os = "linux")]
    {
        CredentialStorePlatform::Linux
    }
    #[cfg(target_os = "macos")]
    {
        CredentialStorePlatform::Macos
    }
    #[cfg(target_os = "windows")]
    {
        CredentialStorePlatform::Windows
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        CredentialStorePlatform::Unsupported
    }
}

fn credential_store_name(platform: CredentialStorePlatform) -> &'static str {
    match platform {
        #[cfg(target_os = "linux")]
        CredentialStorePlatform::Linux => "Linux system keyring/Secret Service",
        #[cfg(target_os = "macos")]
        CredentialStorePlatform::Macos => "macOS Keychain",
        #[cfg(target_os = "windows")]
        CredentialStorePlatform::Windows => "Windows Credential Store",
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        CredentialStorePlatform::Unsupported => "OS credential store",
    }
}

fn credential_store_unavailable_message(platform: CredentialStorePlatform) -> String {
    match platform {
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        CredentialStorePlatform::Unsupported => format!(
            "No supported OS credential store is available for auth DB encryption key management. \
             For headless or CI use, set {AUTH_DB_ENCRYPTION_KEY_ENV} to provide the auth DB \
             encryption secret."
        ),
        _ => format!(
            "Could not access the {}. For headless or CI use, set {AUTH_DB_ENCRYPTION_KEY_ENV} \
             to provide the auth DB encryption secret.",
            credential_store_name(platform)
        ),
    }
}

fn env_encryption_key() -> Result<Option<String>> {
    match std::env::var(AUTH_DB_ENCRYPTION_KEY_ENV) {
        Ok(secret) => Ok(Some(derive_env_encryption_key(&secret)?)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => anyhow::bail!(
            "{AUTH_DB_ENCRYPTION_KEY_ENV} must contain valid UTF-8 secret text. \
             Try: set {AUTH_DB_ENCRYPTION_KEY_ENV} to a non-empty auth DB encryption secret."
        ),
    }
}

fn derive_env_encryption_key(secret: &str) -> Result<String> {
    if secret.trim().is_empty() {
        anyhow::bail!(
            "{AUTH_DB_ENCRYPTION_KEY_ENV} is set but empty. \
             Try: set {AUTH_DB_ENCRYPTION_KEY_ENV} to a non-empty auth DB encryption secret."
        );
    }

    Ok(hex_encode(&Sha256::digest(secret.as_bytes())))
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
    fn storage_access_failures_are_tagged_as_credential_access_denied() {
        let error = keyring_error(
            keyring_core::Error::NoStorageAccess(Box::new(std::io::Error::other("locked"))),
            "auth_db",
        )
        .context("failed to read encryption key");
        assert!(error.is::<CredentialAccessDenied>());
        assert_eq!(error.to_string(), "failed to read encryption key");

        for untagged in [
            keyring_core::Error::NoEntry,
            keyring_core::Error::PlatformFailure(Box::new(std::io::Error::other("other"))),
        ] {
            assert!(!keyring_error(untagged, "auth_db").is::<CredentialAccessDenied>());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_authorization_status_codes_are_tagged_by_code() {
        let platform_failure = |code| {
            keyring_core::Error::PlatformFailure(Box::new(
                security_framework::base::Error::from_code(code),
            ))
        };
        for code in MACOS_AUTHORIZATION_FAILURE_CODES {
            assert!(
                keyring_error(platform_failure(code), "auth_db").is::<CredentialAccessDenied>(),
                "{code}"
            );
        }
        for code in [-50, -25299, -34018] {
            assert!(
                !keyring_error(platform_failure(code), "auth_db").is::<CredentialAccessDenied>(),
                "{code}"
            );
        }
    }
}
