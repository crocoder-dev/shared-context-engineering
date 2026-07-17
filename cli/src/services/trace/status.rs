//! Repository-scoped `sce trace status` resolution.
//!
//! Resolves the current repository's active Agent Trace storage, probes schema
//! readiness, and (when ready) collects row counts plus the last-activity
//! timestamp. Legacy checkout-scoped status remains available for `--legacy`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};

#[cfg(test)]
use crate::services::agent_trace_storage::resolve_agent_trace_storage_at_state_root;
use crate::services::agent_trace_storage::{resolve_agent_trace_storage, AgentTraceStorageContext};
use crate::services::checkout::{read_checkout_id, resolve_git_dir};
use crate::services::config;
use crate::services::repository_identity::resolve::RepositoryIdentitySource;
use crate::services::trace::discovery::{probe_readiness, Readiness};
use crate::services::trace::stats::{collect_agent_trace_db_stats, AgentTraceDbStats};

/// Errors that map directly to user-facing `sce trace status` guidance.
#[derive(Debug)]
pub enum StatusError {
    NotInGitRepo { repo_root: PathBuf, detail: String },
    NoCheckoutId { git_dir: PathBuf },
    DbMissing { checkout_id: String, path: PathBuf },
}

impl StatusError {
    pub fn user_message(&self) -> String {
        match self {
            Self::NotInGitRepo { repo_root, detail } => format!(
                "sce trace status: '{}' is not inside a git repository ({detail}); cd into a git repository and retry",
                repo_root.display()
            ),
            Self::NoCheckoutId { git_dir } => format!(
                "sce trace status: no checkout id found at '{}'; run `sce setup` to initialize this repository",
                git_dir.join("sce").join("checkout-id").display()
            ),
            Self::DbMissing { checkout_id, path } => format!(
                "sce trace status: no agent-trace database for checkout {checkout_id} at '{}'; no traces have been recorded yet (the SCE Claude Code hook records traces on commits)",
                path.display()
            ),
        }
    }
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for StatusError {}

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

/// Legacy testable variant taking the `sce` directory explicitly.
pub fn resolve_current_legacy_status_in(
    repo_root: &Path,
    sce_dir: &Path,
) -> Result<StatusReport, StatusErrorOrRuntime> {
    let git_dir = resolve_git_dir(repo_root).map_err(|err| {
        StatusErrorOrRuntime::Status(StatusError::NotInGitRepo {
            repo_root: repo_root.to_path_buf(),
            detail: format!("{err:#}"),
        })
    })?;

    let Some(checkout_id) = read_checkout_id(&git_dir).map_err(StatusErrorOrRuntime::Runtime)?
    else {
        return Err(StatusErrorOrRuntime::Status(StatusError::NoCheckoutId {
            git_dir,
        }));
    };

    let database_path = sce_dir.join(format!("agent-trace-{checkout_id}.db"));
    if !database_path.exists() {
        return Err(StatusErrorOrRuntime::Status(StatusError::DbMissing {
            checkout_id,
            path: database_path,
        }));
    }

    status_report_from_path(None, None, None, None, checkout_id, database_path)
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

/// Distinguishes user-actionable status errors from internal runtime failures.
#[derive(Debug)]
pub enum StatusErrorOrRuntime {
    Status(StatusError),
    Runtime(anyhow::Error),
}

impl std::fmt::Display for StatusErrorOrRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(err) => write!(f, "{err}"),
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

    use crate::services::agent_trace_db::AgentTraceDb;

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

    fn write_checkout_id(repo_root: &Path, id: &str) -> PathBuf {
        let git_dir = repo_root.join(".git");
        let sce = git_dir.join("sce");
        std::fs::create_dir_all(&sce).expect("create .git/sce");
        let id_path = sce.join("checkout-id");
        std::fs::write(&id_path, id).expect("write checkout-id");
        id_path
    }

    fn create_full_schema_db(path: &Path) {
        let db = AgentTraceDb::open_at(path).expect("migrated DB should open");
        drop(db);
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

    #[test]
    fn missing_git_repo_reports_not_in_git_repo() {
        let repo = unique_temp_dir("not-git");
        let sce_dir = unique_temp_dir("not-git-sce");

        let err = resolve_current_legacy_status_in(&repo, &sce_dir).expect_err("should error");
        match err {
            StatusErrorOrRuntime::Status(StatusError::NotInGitRepo { .. }) => {}
            other => panic!("expected NotInGitRepo, got {other:?}"),
        }
    }

    #[test]
    fn missing_checkout_id_reports_no_checkout_id() {
        let repo = unique_temp_dir("no-id");
        init_git_repo(&repo);
        let sce_dir = unique_temp_dir("no-id-sce");

        let err = resolve_current_legacy_status_in(&repo, &sce_dir).expect_err("should error");
        match err {
            StatusErrorOrRuntime::Status(StatusError::NoCheckoutId { .. }) => {}
            other => panic!("expected NoCheckoutId, got {other:?}"),
        }
    }

    #[test]
    fn missing_db_file_reports_db_missing() {
        let repo = unique_temp_dir("no-db");
        init_git_repo(&repo);
        let id = "01900000-0000-7000-8000-000000000001";
        write_checkout_id(&repo, id);
        let sce_dir = unique_temp_dir("no-db-sce");
        std::fs::create_dir_all(&sce_dir).expect("create sce dir");

        let err = resolve_current_legacy_status_in(&repo, &sce_dir).expect_err("should error");
        match err {
            StatusErrorOrRuntime::Status(StatusError::DbMissing { checkout_id, .. }) => {
                assert_eq!(checkout_id, id);
            }
            other => panic!("expected DbMissing, got {other:?}"),
        }
    }

    #[test]
    fn ready_db_returns_stats_report() {
        let repo = unique_temp_dir("ready");
        init_git_repo(&repo);
        let id = "01900000-0000-7000-8000-000000000002";
        write_checkout_id(&repo, id);
        let sce_dir = unique_temp_dir("ready-sce");
        std::fs::create_dir_all(&sce_dir).expect("create sce dir");
        let db_path = sce_dir.join(format!("agent-trace-{id}.db"));
        create_full_schema_db(&db_path);

        let report =
            resolve_current_legacy_status_in(&repo, &sce_dir).expect("ready report should resolve");

        assert_eq!(report.checkout_id, id);
        assert_eq!(report.database_path, db_path);
        match report.db_status {
            DbStatus::Ready {
                stats,
                last_activity,
            } => {
                assert_eq!(stats.diff_traces, 0);
                assert_eq!(stats.messages, 0);
                assert_eq!(stats.parts, 0);
                assert_eq!(stats.agent_traces, 0);
                assert_eq!(stats.post_commit_patch_intersections, 0);
                assert!(last_activity.is_none());
            }
            DbStatus::Skipped { missing_table } => {
                panic!("expected Ready, got Skipped (missing_table={missing_table})")
            }
        }
    }

    #[test]
    fn partial_schema_db_returns_skipped_status() {
        let repo = unique_temp_dir("skipped");
        init_git_repo(&repo);
        let id = "01900000-0000-7000-8000-000000000003";
        write_checkout_id(&repo, id);
        let sce_dir = unique_temp_dir("skipped-sce");
        std::fs::create_dir_all(&sce_dir).expect("create sce dir");
        let db_path = sce_dir.join(format!("agent-trace-{id}.db"));

        let db = AgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
            .expect("open without migrations");
        db.execute(
            "CREATE TABLE IF NOT EXISTS diff_traces (id INTEGER PRIMARY KEY)",
            (),
        )
        .expect("create diff_traces");
        drop(db);

        let report = resolve_current_legacy_status_in(&repo, &sce_dir)
            .expect("skipped report should resolve");

        match report.db_status {
            DbStatus::Skipped { missing_table } => {
                assert_eq!(missing_table, "post_commit_patch_intersections");
            }
            DbStatus::Ready { .. } => panic!("expected Skipped, got Ready"),
        }
    }
}
