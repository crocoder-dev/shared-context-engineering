//! Central checkout registry.
//!
//! The checkout registry is a JSON file at `<state_root>/sce/checkout-registry.json`
//! that tracks all known checkouts with metadata like path, last-seen timestamp,
//! remote URL, and per-checkout database path.

#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::services::default_paths::resolve_state_data_root;

/// File name for the checkout registry inside `<state_root>/sce/`.
const CHECKOUT_REGISTRY_FILE: &str = "checkout-registry.json";

/// A single checkout record in the central registry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckoutRecord {
    /// Stable `UUIDv7` checkout identity.
    pub checkout_id: String,
    /// Absolute path to the repository root (or worktree root).
    pub path: String,
    /// ISO 8601 / RFC 3339 timestamp of the last time this checkout was seen.
    pub last_seen: String,
    /// Remote URL if available (e.g. the `origin` URL).
    pub remote_url: Option<String>,
    /// Absolute path to the per-checkout database file, if it has been created.
    pub database_path: Option<String>,
}

/// The central checkout registry, persisted as a JSON file.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CheckoutRegistry {
    /// Registered checkouts, keyed by checkout ID for efficient lookup.
    /// Serialized as a JSON array for human readability.
    #[serde(default)]
    pub checkouts: Vec<CheckoutRecord>,
}

/// Returns the canonical path to the checkout registry file.
///
/// The path is `<state_root>/sce/checkout-registry.json`, where `state_root`
/// comes from the shared default-path catalog.
pub fn checkout_registry_path() -> Result<PathBuf> {
    Ok(resolve_state_data_root()?
        .join("sce")
        .join(CHECKOUT_REGISTRY_FILE))
}

/// Reads the checkout registry from disk.
///
/// Returns an empty registry if the file does not exist.
/// Returns an error if the file exists but cannot be parsed.
pub fn read_registry() -> Result<CheckoutRegistry> {
    let path = checkout_registry_path()?;

    if !path.exists() {
        return Ok(CheckoutRegistry::default());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read checkout registry from '{}'", path.display()))?;

    // Empty or whitespace-only files are treated as corrupt — delete and start fresh.
    if content.trim().is_empty() {
        let _ = std::fs::remove_file(&path);
        eprintln!(
            "[WARN] Empty checkout registry at '{}' — removing and recreating from scratch",
            path.display()
        );
        return Ok(CheckoutRegistry::default());
    }

    let registry: CheckoutRegistry = serde_json::from_str(&content).with_context(|| {
        format!(
            "Failed to parse checkout registry from '{}'",
            path.display()
        )
    })?;

    Ok(registry)
}

/// Writes the checkout registry to disk using atomic write-through-rename.
///
/// This creates the parent directory if it doesn't exist, writes to a temporary
/// file, and then renames the temporary file to the target path. This ensures
/// that concurrent readers never see a partially-written registry.
pub fn write_registry(registry: &CheckoutRegistry) -> Result<()> {
    let path = checkout_registry_path()?;

    // Ensure the parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create registry directory '{}'", parent.display())
        })?;
    }

    // Serialize the registry.
    let content = serde_json::to_string_pretty(registry)
        .with_context(|| "Failed to serialize checkout registry")?;

    // Write to a temporary file first, then rename for atomicity.
    // Use a PID-unique suffix so concurrent `sce hooks` processes never
    // collide on the same temp file.
    let temp_path = path.with_extension(format!("json.tmp.{}", std::process::id()));

    std::fs::write(&temp_path, &content).with_context(|| {
        format!(
            "Failed to write temporary registry to '{}'",
            temp_path.display()
        )
    })?;

    std::fs::rename(&temp_path, &path).with_context(|| {
        format!(
            "Failed to rename temporary registry from '{}' to '{}'",
            temp_path.display(),
            path.display()
        )
    })?;

    // Clean up any stale `.tmp.*` files left behind by previous crashed
    // processes. The current process's temp file was just renamed, so
    // anything remaining matching the pattern is orphaned.
    if let Some(parent) = path.parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            let stale_prefix = format!("{CHECKOUT_REGISTRY_FILE}.tmp.");
            for entry in entries {
                let Ok(entry) = entry else {
                    continue;
                };
                let name = entry.file_name();
                if name.to_string_lossy().starts_with(&stale_prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    Ok(())
}

/// Registers a checkout in the central registry.
///
/// If a checkout with the same `checkout_id` already exists, its `path`,
/// `last_seen`, and `remote_url` fields are updated. The `database_path` field
/// is preserved from the existing record if present.
///
/// After registration, the registry is persisted to disk.
pub fn register_checkout(record: CheckoutRecord) -> Result<()> {
    let mut registry = read_registry()?;

    if let Some(existing) = registry
        .checkouts
        .iter_mut()
        .find(|r| r.checkout_id == record.checkout_id)
    {
        // Update existing record, preserving database_path if the new record
        // doesn't have one.
        existing.path = record.path;
        existing.last_seen = record.last_seen;
        existing.remote_url = record.remote_url;
        if record.database_path.is_some() {
            existing.database_path = record.database_path;
        }
    } else {
        registry.checkouts.push(record);
    }

    write_registry(&registry)
}

/// Updates the `last_seen` timestamp for a checkout in the registry.
///
/// Returns `Ok(())` if the checkout was found and updated, or an error if the
/// checkout ID is not found in the registry.
pub fn update_checkout_last_seen(checkout_id: &str, last_seen: &str) -> Result<()> {
    let mut registry = read_registry()?;

    let record = registry
        .checkouts
        .iter_mut()
        .find(|r| r.checkout_id == checkout_id)
        .ok_or_else(|| anyhow::anyhow!("Checkout ID '{checkout_id}' not found in registry"))?;

    record.last_seen = last_seen.to_string();
    write_registry(&registry)
}

/// Lists all registered checkouts from the central registry.
///
/// Returns an empty list if the registry file does not exist.
pub fn list_checkouts() -> Result<Vec<CheckoutRecord>> {
    let registry = read_registry()?;
    Ok(registry.checkouts)
}

/// Removes a checkout from the central registry by checkout ID.
///
/// Returns `Ok(true)` if the checkout was found and removed, `Ok(false)` if
/// the checkout was not found, or an error if the registry could not be
/// read or written.
pub fn remove_checkout(checkout_id: &str) -> Result<bool> {
    let mut registry = read_registry()?;

    let original_len = registry.checkouts.len();
    registry.checkouts.retain(|r| r.checkout_id != checkout_id);

    let removed = registry.checkouts.len() < original_len;

    if removed {
        write_registry(&registry)?;
    }

    Ok(removed)
}
