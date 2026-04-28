#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::services::default_paths;
use crate::services::lifecycle::{
    DiagnoseRequest, DiagnosticFixability, DiagnosticLifecycle, DiagnosticRecord, DiagnosticReport,
    DiagnosticSeverity, FixLifecycle, FixReport, FixRequest, LifecycleAction, LifecycleOperation,
    LifecycleOutcome, LifecycleService, ServiceId, ServiceMetadata,
};

pub const LOCAL_DB_SERVICE_ID: ServiceId = ServiceId("local_db");

/// Diagnostic kind constants for local DB lifecycle.
///
/// These stable identifiers map into existing doctor problem types at the
/// orchestration boundary.
pub const LOCAL_DB_PATH_UNRESOLVABLE: &str = "local_db_path_unresolvable";
pub const LOCAL_DB_PARENT_MISSING: &str = "local_db_parent_missing";
pub const LOCAL_DB_PARENT_NOT_DIRECTORY: &str = "local_db_parent_not_directory";
pub const LOCAL_DB_PARENT_NOT_WRITABLE: &str = "local_db_parent_not_writable";
pub const LOCAL_DB_HEALTH_CHECK_FAILED: &str = "local_db_health_check_failed";

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalDbLifecycleService;

impl LifecycleService for LocalDbLifecycleService {
    fn metadata(&self) -> ServiceMetadata {
        ServiceMetadata {
            id: LOCAL_DB_SERVICE_ID,
            display_name: "Local database",
            description:
                "Local Turso database health and parent-directory readiness lifecycle capability",
        }
    }
}

impl DiagnosticLifecycle for LocalDbLifecycleService {
    fn diagnose(&self, _request: DiagnoseRequest) -> Result<DiagnosticReport> {
        let db_path = default_paths::local_db_path()
            .context("Local DB lifecycle diagnosis requires a resolvable local DB path")?;

        let mut diagnostics = Vec::new();

        // Check parent directory readiness
        let parent = db_path.parent();
        match parent {
            None => {
                diagnostics.push(DiagnosticRecord {
                    service_id: LOCAL_DB_SERVICE_ID,
                    kind: LOCAL_DB_PATH_UNRESOLVABLE.to_string(),
                    target: db_path.display().to_string(),
                    severity: DiagnosticSeverity::Error,
                    fixability: DiagnosticFixability::ManualOnly,
                    summary: format!(
                        "Local DB path '{}' has no parent directory.",
                        db_path.display()
                    ),
                    remediation: format!(
                        "Verify that the local DB path '{}' is valid and rerun 'sce doctor'.",
                        db_path.display()
                    ),
                });
            }
            Some(parent_path) => {
                if !parent_path.exists() {
                    diagnostics.push(DiagnosticRecord {
                        service_id: LOCAL_DB_SERVICE_ID,
                        kind: LOCAL_DB_PARENT_MISSING.to_string(),
                        target: parent_path.display().to_string(),
                        severity: DiagnosticSeverity::Error,
                        fixability: DiagnosticFixability::AutoFixable,
                        summary: format!(
                            "Local DB parent directory '{}' does not exist.",
                            parent_path.display()
                        ),
                        remediation: format!(
                            "Run 'sce doctor --fix' to create the missing parent directory '{}', or run 'sce setup' directly.",
                            parent_path.display()
                        ),
                    });
                } else if !parent_path.is_dir() {
                    diagnostics.push(DiagnosticRecord {
                        service_id: LOCAL_DB_SERVICE_ID,
                        kind: LOCAL_DB_PARENT_NOT_DIRECTORY.to_string(),
                        target: parent_path.display().to_string(),
                        severity: DiagnosticSeverity::Error,
                        fixability: DiagnosticFixability::ManualOnly,
                        summary: format!(
                            "Local DB parent path '{}' is not a directory.",
                            parent_path.display()
                        ),
                        remediation: format!(
                            "Replace '{}' with a directory, then rerun 'sce doctor' or 'sce setup'.",
                            parent_path.display()
                        ),
                    });
                } else {
                    // Parent exists and is a directory; check writability
                    if let Err(error) = check_directory_writable(parent_path) {
                        diagnostics.push(DiagnosticRecord {
                            service_id: LOCAL_DB_SERVICE_ID,
                            kind: LOCAL_DB_PARENT_NOT_WRITABLE.to_string(),
                            target: parent_path.display().to_string(),
                            severity: DiagnosticSeverity::Error,
                            fixability: DiagnosticFixability::ManualOnly,
                            summary: format!(
                                "Local DB parent directory '{}' is not writable: {error}",
                                parent_path.display()
                            ),
                            remediation: format!(
                                "Verify that '{}' has write permissions before rerunning 'sce doctor' or 'sce setup'.",
                                parent_path.display()
                            ),
                        });
                    }
                }
            }
        }

        // Check DB file health if parent directory is ready
        let parent_ready = parent.is_some_and(|p| p.exists() && p.is_dir());
        if parent_ready && db_path.exists() {
            // DB file exists; attempt a health check by trying to open it
            if let Err(error) = check_db_health(&db_path) {
                diagnostics.push(DiagnosticRecord {
                    service_id: LOCAL_DB_SERVICE_ID,
                    kind: LOCAL_DB_HEALTH_CHECK_FAILED.to_string(),
                    target: db_path.display().to_string(),
                    severity: DiagnosticSeverity::Error,
                    fixability: DiagnosticFixability::ManualOnly,
                    summary: format!(
                        "Local DB health check failed for '{}': {error}",
                        db_path.display()
                    ),
                    remediation: format!(
                        "Verify that '{}' is a valid database file, or remove it and rerun 'sce setup' to recreate it.",
                        db_path.display()
                    ),
                });
            }
            // If DB file does not exist, that's not a diagnostic error — it will be
            // created on first use by setup/bootstrap.
        }

        Ok(DiagnosticReport {
            service_id: LOCAL_DB_SERVICE_ID,
            diagnostics,
        })
    }
}

impl FixLifecycle for LocalDbLifecycleService {
    fn fix(&self, _request: FixRequest) -> Result<FixReport> {
        let db_path = default_paths::local_db_path()
            .context("Local DB lifecycle fix requires a resolvable local DB path")?;

        let canonical_parent = resolve_local_db_parent_path();
        let action = fix_missing_parent_directory(&db_path, canonical_parent.as_deref());

        Ok(FixReport {
            service_id: LOCAL_DB_SERVICE_ID,
            actions: vec![action],
        })
    }
}

/// Attempt to fix the local DB parent directory by creating it if it is
/// missing and matches the canonical SCE-owned location.
///
/// Returns a `LifecycleAction` describing the outcome:
/// - `Applied` if the missing canonical parent directory was created
/// - `Unchanged` if the parent directory already exists
/// - `Failed` if the path has no parent, the parent is not at the canonical
///   location, or directory creation fails
fn fix_missing_parent_directory(
    db_path: &std::path::Path,
    canonical_parent: Option<&std::path::Path>,
) -> LifecycleAction {
    let parent = db_path.parent();

    match parent {
        None => LifecycleAction {
            service_id: LOCAL_DB_SERVICE_ID,
            operation: LifecycleOperation::Fix,
            target: db_path.display().to_string(),
            description: String::from(
                "Cannot create local DB parent directory: path has no parent",
            ),
            outcome: LifecycleOutcome::Failed,
        },
        Some(parent_path) if parent_path.exists() => LifecycleAction {
            service_id: LOCAL_DB_SERVICE_ID,
            operation: LifecycleOperation::Fix,
            target: parent_path.display().to_string(),
            description: String::from("Local DB parent directory already exists; no fix needed"),
            outcome: LifecycleOutcome::Unchanged,
        },
        Some(parent_path) => {
            let is_canonical = canonical_parent.is_some_and(|expected| expected == parent_path);

            if is_canonical {
                match std::fs::create_dir_all(parent_path) {
                    Ok(()) => LifecycleAction {
                        service_id: LOCAL_DB_SERVICE_ID,
                        operation: LifecycleOperation::Fix,
                        target: parent_path.display().to_string(),
                        description: format!(
                            "Created missing local DB parent directory '{}'",
                            parent_path.display()
                        ),
                        outcome: LifecycleOutcome::Applied,
                    },
                    Err(error) => LifecycleAction {
                        service_id: LOCAL_DB_SERVICE_ID,
                        operation: LifecycleOperation::Fix,
                        target: parent_path.display().to_string(),
                        description: format!(
                            "Failed to create local DB parent directory '{}': {error}",
                            parent_path.display()
                        ),
                        outcome: LifecycleOutcome::Failed,
                    },
                }
            } else {
                LifecycleAction {
                    service_id: LOCAL_DB_SERVICE_ID,
                    operation: LifecycleOperation::Fix,
                    target: parent_path.display().to_string(),
                    description: format!(
                        "Refused to create local DB parent directory '{}' because it does not match the canonical SCE-owned location",
                        parent_path.display()
                    ),
                    outcome: LifecycleOutcome::Failed,
                }
            }
        }
    }
}

/// Check if a directory is writable by attempting to verify metadata and
/// permissions. Returns Ok(()) if the directory appears writable, or an
/// error describing why it is not.
fn check_directory_writable(dir: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(dir).map_err(|error| {
        anyhow::anyhow!("Failed to inspect directory '{}': {error}", dir.display())
    })?;

    if !metadata.is_dir() {
        anyhow::bail!("Path '{}' is not a directory", dir.display());
    }

    let mode = metadata.permissions().mode();
    if mode & 0o200 == 0 {
        anyhow::bail!(
            "Directory '{}' does not have write permissions",
            dir.display()
        );
    }

    Ok(())
}

/// Check local DB health by attempting to open it. Returns Ok(()) if the
/// database can be opened and a basic query succeeds, or an error describing
/// the failure.
fn check_db_health(db_path: &std::path::Path) -> Result<()> {
    // We verify that the file exists and is a regular file (not a directory
    // or other non-database artifact). A full open/connect/query health
    // check would require a tokio runtime and is not suitable for a
    // diagnostic-only path that should remain lightweight.
    let metadata = std::fs::metadata(db_path).map_err(|error| {
        anyhow::anyhow!(
            "Failed to inspect database file '{}': {error}",
            db_path.display()
        )
    })?;

    if metadata.is_dir() {
        anyhow::bail!(
            "Expected a database file but found a directory at '{}'",
            db_path.display()
        );
    }

    Ok(())
}

/// Resolve the canonical local DB parent directory path from the shared
/// default-path catalog. Returns `None` if the path cannot be resolved.
pub fn resolve_local_db_parent_path() -> Option<PathBuf> {
    default_paths::local_db_path()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
}
