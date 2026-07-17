//! Repository-scoped `sce trace status` resolution.
//!
//! Resolves the current repository's active Agent Trace storage, probes schema
//! readiness, and (when ready) collects row counts plus the last-activity
//! timestamp.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};

#[cfg(test)]
use crate::services::agent_trace_storage::resolve_agent_trace_storage_at_state_root;
use crate::services::agent_trace_storage::{resolve_agent_trace_storage, AgentTraceStorageContext};
use crate::services::config;
use crate::services::repository_identity::resolve::RepositoryIdentitySource;
use crate::services::trace::discovery::{probe_readiness, Readiness};
use crate::services::trace::stats::{collect_agent_trace_db_stats, AgentTraceDbStats};

/// Verdict for a resolved checkout's Agent Trace DB.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DbStatus {
    Ready {
        stats: AgentTraceDbStats,
        last_activity: Option<DateTime<Utc>>,
    },
    Skipped {
        missing_table: String,
    },
}

/// Resolved status report ready for rendering.
#[derive(Clone, Debug)]
pub struct StatusReport {
    pub repository_id: Option<String>,
    pub repository_identity_source: Option<String>,
    pub canonical_identity: Option<String>,
    pub configured_remote: Option<String>,
    pub checkout_id: String,
    pub database_path: PathBuf,
    pub db_status: DbStatus,
}

/// Resolve `sce trace status` for the current working repository using the
/// default state-data root.
pub fn resolve_current_status(repo_root: &Path) -> Result<StatusReport, StatusErrorOrRuntime> {
    let storage_config = config::resolve_agent_trace_storage_runtime_config(repo_root)
        .map_err(StatusErrorOrRuntime::Runtime)?;
    let context = AgentTraceStorageContext {
        repository_root: repo_root,
        explicit_repository_id: storage_config.repository_id.as_deref(),
        repository_remote: &storage_config.repository_remote,
    };
    let storage = resolve_agent_trace_storage(&context).map_err(StatusErrorOrRuntime::Runtime)?;
    let repository_identity_source = identity_source_label(&storage.repository_identity.source);
    let configured_remote = configured_remote_name(&storage.repository_identity.source);
    status_report_from_path(
        Some(storage.repository_identity.identity.repository_id),
        Some(repository_identity_source),
        Some(storage.repository_identity.identity.canonical_identity),
        configured_remote,
        storage.checkout_id,
        storage.db_path,
    )
}

/// Testable repository-scoped variant taking the state root explicitly.
#[cfg(test)]
#[allow(dead_code)]
pub fn resolve_current_status_at_state_root(
    repo_root: &Path,
    state_root: &Path,
    explicit_repository_id: Option<&str>,
    repository_remote: &str,
) -> Result<StatusReport, StatusErrorOrRuntime> {
    let context = AgentTraceStorageContext {
        repository_root: repo_root,
        explicit_repository_id,
        repository_remote,
    };
    let storage = resolve_agent_trace_storage_at_state_root(&context, state_root)
        .map_err(StatusErrorOrRuntime::Runtime)?;
    let repository_identity_source = identity_source_label(&storage.repository_identity.source);
    let configured_remote = configured_remote_name(&storage.repository_identity.source);
    status_report_from_path(
        Some(storage.repository_identity.identity.repository_id),
        Some(repository_identity_source),
        Some(storage.repository_identity.identity.canonical_identity),
        configured_remote,
        storage.checkout_id,
        storage.db_path,
    )
}

fn status_report_from_path(
    repository_id: Option<String>,
    repository_identity_source: Option<String>,
    canonical_identity: Option<String>,
    configured_remote: Option<String>,
    checkout_id: String,
    database_path: PathBuf,
) -> Result<StatusReport, StatusErrorOrRuntime> {
    let readiness = probe_readiness(&database_path).map_err(StatusErrorOrRuntime::Runtime)?;
    let db_status = match readiness {
        Readiness::Ready => {
            let stats = collect_agent_trace_db_stats(&database_path)
                .map_err(StatusErrorOrRuntime::Runtime)?;
            let last_activity = stats.last_activity;
            DbStatus::Ready {
                stats,
                last_activity,
            }
        }
        Readiness::Skipped { missing_table } => DbStatus::Skipped { missing_table },
    };

    Ok(StatusReport {
        repository_id,
        repository_identity_source,
        canonical_identity,
        configured_remote,
        checkout_id,
        database_path,
        db_status,
    })
}

fn identity_source_label(source: &RepositoryIdentitySource) -> String {
    match source {
        RepositoryIdentitySource::ExplicitConfig => String::from("explicit_config"),
        RepositoryIdentitySource::RemoteUrl { .. } => String::from("remote_url"),
    }
}

fn configured_remote_name(source: &RepositoryIdentitySource) -> Option<String> {
    match source {
        RepositoryIdentitySource::ExplicitConfig => None,
        RepositoryIdentitySource::RemoteUrl { remote_name } => Some(remote_name.clone()),
    }
}

/// Wraps internal runtime failures surfaced by `sce trace status` resolution.
#[derive(Debug)]
pub enum StatusErrorOrRuntime {
    Runtime(anyhow::Error),
}

impl std::fmt::Display for StatusErrorOrRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runtime(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for StatusErrorOrRuntime {}

#[cfg(test)]
mod tests {
    use super::*;

    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-trace-status-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn init_git_repo(repo_root: &Path) {
        let output = Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo_root)
            .output()
            .expect("git init");
        assert!(output.status.success(), "git init failed");
    }

    fn add_remote(repo_root: &Path, name: &str, url: &str) {
        let output = Command::new("git")
            .args(["remote", "add", name, url])
            .current_dir(repo_root)
            .output()
            .expect("git remote add");
        assert!(output.status.success(), "git remote add failed");
    }

    #[test]
    fn repository_status_uses_safe_identity_metadata_for_credential_remote() {
        let repo = unique_temp_dir("repo-safe-identity");
        init_git_repo(&repo);
        add_remote(
            &repo,
            "origin",
            "https://alice:s3cr3t@github.com/acme/widgets.git",
        );
        let state_root = unique_temp_dir("repo-safe-identity-state");

        let report = resolve_current_status_at_state_root(&repo, &state_root, None, "origin")
            .expect("repository status should resolve");

        assert_eq!(
            report.repository_identity_source.as_deref(),
            Some("remote_url")
        );
        assert_eq!(report.configured_remote.as_deref(), Some("origin"));
        assert_eq!(
            report.canonical_identity.as_deref(),
            Some("github.com/acme/widgets")
        );
        assert!(!report.repository_id.unwrap().contains("s3cr3t"));
        assert!(!report
            .database_path
            .display()
            .to_string()
            .contains("s3cr3t"));
    }
}
