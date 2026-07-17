//! Runtime repository identity resolution.
//!
//! Applies the repository identity precedence: an explicit configured
//! identity (`agent_trace.repository_id`) wins, otherwise the URL of the
//! configured Git remote (`agent_trace.repository_remote`, default `origin`)
//! is canonicalized, otherwise resolution fails with actionable
//! `.sce/config.json` guidance. Local paths are never used implicitly.
//!
//! Errors intentionally never echo remote URLs so credential-bearing
//! remotes cannot leak through diagnostics; remote names are operator-chosen
//! configuration values and are safe to display.

use std::path::Path;
use std::process::Command;

use super::{
    repository_identity_from_explicit, repository_identity_from_remote_url, RepositoryIdentity,
    RepositoryIdentityError,
};

/// Where a resolved repository identity came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryIdentitySource {
    /// Explicit `agent_trace.repository_id` configuration value.
    ExplicitConfig,
    /// URL of the named Git remote.
    RemoteUrl { remote_name: String },
}

/// A repository identity plus the source it was resolved from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRepositoryIdentity {
    pub identity: RepositoryIdentity,
    pub source: RepositoryIdentitySource,
}

/// Resolution failure. Variants never carry remote URLs or explicit
/// identity values, only the configured remote name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryIdentityResolutionError {
    /// `agent_trace.repository_id` is configured but unusable.
    InvalidExplicitIdentity(RepositoryIdentityError),
    /// The configured remote exists but its URL cannot serve as a
    /// repository identity (for example a local path remote).
    InvalidRemoteUrl {
        remote_name: String,
        error: RepositoryIdentityError,
    },
    /// No explicit identity is configured and the configured remote has
    /// no URL.
    MissingIdentity { remote_name: String },
}

impl std::fmt::Display for RepositoryIdentityResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidExplicitIdentity(error) => write!(
                f,
                "configured agent_trace.repository_id is not usable ({error}); update agent_trace.repository_id in .sce/config.json"
            ),
            Self::InvalidRemoteUrl { remote_name, error } => write!(
                f,
                "remote '{remote_name}' URL cannot be used as a repository identity ({error}); set agent_trace.repository_id in .sce/config.json or point agent_trace.repository_remote at a remote with a supported URL"
            ),
            Self::MissingIdentity { remote_name } => write!(
                f,
                "no repository identity: agent_trace.repository_id is not configured and remote '{remote_name}' has no URL; set agent_trace.repository_id or agent_trace.repository_remote in .sce/config.json"
            ),
        }
    }
}

impl std::error::Error for RepositoryIdentityResolutionError {}

/// Resolves the repository identity for a Git repository checkout, applying
/// the explicit-config-then-remote precedence.
pub fn resolve_repository_identity(
    repository_root: &Path,
    explicit_identity: Option<&str>,
    remote_name: &str,
) -> Result<ResolvedRepositoryIdentity, RepositoryIdentityResolutionError> {
    resolve_repository_identity_with_lookup(explicit_identity, remote_name, |remote| {
        lookup_remote_url(repository_root, remote)
    })
}

/// Precedence core with an injectable remote URL lookup, so callers and
/// tests can resolve without spawning `git`.
pub fn resolve_repository_identity_with_lookup(
    explicit_identity: Option<&str>,
    remote_name: &str,
    lookup_remote_url: impl FnOnce(&str) -> Option<String>,
) -> Result<ResolvedRepositoryIdentity, RepositoryIdentityResolutionError> {
    if let Some(explicit) = explicit_identity {
        let identity = repository_identity_from_explicit(explicit)
            .map_err(RepositoryIdentityResolutionError::InvalidExplicitIdentity)?;
        return Ok(ResolvedRepositoryIdentity {
            identity,
            source: RepositoryIdentitySource::ExplicitConfig,
        });
    }

    let Some(remote_url) = lookup_remote_url(remote_name) else {
        return Err(RepositoryIdentityResolutionError::MissingIdentity {
            remote_name: remote_name.to_string(),
        });
    };

    let identity = repository_identity_from_remote_url(&remote_url).map_err(|error| {
        RepositoryIdentityResolutionError::InvalidRemoteUrl {
            remote_name: remote_name.to_string(),
            error,
        }
    })?;
    Ok(ResolvedRepositoryIdentity {
        identity,
        source: RepositoryIdentitySource::RemoteUrl {
            remote_name: remote_name.to_string(),
        },
    })
}

/// Reads the URL of a named Git remote from repository configuration.
/// Returns `None` when git is unavailable, the directory is not a
/// repository, or the remote has no URL.
pub fn lookup_remote_url(repository_root: &Path, remote_name: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["config", "--get", &format!("remote.{remote_name}.url")])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-repo-identity-{label}-{}-{nonce}",
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
    fn explicit_identity_overrides_remote_lookup() {
        let resolved =
            resolve_repository_identity_with_lookup(Some("my-monorepo"), "origin", |_| {
                panic!("remote lookup must not run when explicit identity is set")
            })
            .expect("explicit identity should resolve");
        assert_eq!(resolved.identity.canonical_identity, "my-monorepo");
        assert_eq!(resolved.source, RepositoryIdentitySource::ExplicitConfig);
    }

    #[test]
    fn invalid_explicit_identity_errors_without_remote_fallback() {
        let error = resolve_repository_identity_with_lookup(Some("   "), "origin", |_| {
            panic!("remote lookup must not run when explicit identity is set")
        })
        .expect_err("blank explicit identity should fail");
        assert_eq!(
            error,
            RepositoryIdentityResolutionError::InvalidExplicitIdentity(
                RepositoryIdentityError::EmptyExplicitIdentity
            )
        );
        assert!(error.to_string().contains(".sce/config.json"));
    }

    #[test]
    fn configured_remote_name_is_honored() {
        let resolved = resolve_repository_identity_with_lookup(None, "upstream", |remote| {
            assert_eq!(remote, "upstream");
            Some("git@github.com:acme/widgets.git".to_string())
        })
        .expect("remote identity should resolve");
        assert_eq!(
            resolved.identity.canonical_identity,
            "github.com/acme/widgets"
        );
        assert_eq!(
            resolved.source,
            RepositoryIdentitySource::RemoteUrl {
                remote_name: "upstream".to_string(),
            }
        );
    }

    #[test]
    fn missing_remote_errors_with_config_guidance() {
        let error = resolve_repository_identity_with_lookup(None, "origin", |_| None)
            .expect_err("missing remote should fail");
        assert_eq!(
            error,
            RepositoryIdentityResolutionError::MissingIdentity {
                remote_name: "origin".to_string(),
            }
        );
        let rendered = error.to_string();
        assert!(rendered.contains(".sce/config.json"));
        assert!(rendered.contains("agent_trace.repository_id"));
        assert!(rendered.contains("agent_trace.repository_remote"));
    }

    #[test]
    fn unusable_remote_url_errors_without_leaking_the_url() {
        let error = resolve_repository_identity_with_lookup(None, "origin", |_| {
            Some("/local/path/to/s3cr3t-repo".to_string())
        })
        .expect_err("local path remote should fail");
        assert_eq!(
            error,
            RepositoryIdentityResolutionError::InvalidRemoteUrl {
                remote_name: "origin".to_string(),
                error: RepositoryIdentityError::UnsupportedRemoteUrl,
            }
        );
        let rendered = error.to_string();
        assert!(!rendered.contains("s3cr3t"), "error leaked URL: {rendered}");
        assert!(rendered.contains(".sce/config.json"));
    }

    #[test]
    fn resolves_origin_remote_from_temp_git_repo() {
        let repo = unique_temp_dir("origin");
        init_git_repo(&repo);
        add_remote(&repo, "origin", "https://github.com/acme/widgets.git");

        let resolved = resolve_repository_identity(&repo, None, "origin")
            .expect("origin remote should resolve");
        assert_eq!(
            resolved.identity.canonical_identity,
            "github.com/acme/widgets"
        );
        assert_eq!(
            resolved.source,
            RepositoryIdentitySource::RemoteUrl {
                remote_name: "origin".to_string(),
            }
        );

        std::fs::remove_dir_all(&repo).expect("clean up temp repo");
    }

    #[test]
    fn resolves_configured_non_origin_remote_from_temp_git_repo() {
        let repo = unique_temp_dir("upstream");
        init_git_repo(&repo);
        add_remote(&repo, "origin", "git@github.com:acme/widgets.git");
        add_remote(&repo, "upstream", "git@github.com:acme/gadgets.git");

        let resolved = resolve_repository_identity(&repo, None, "upstream")
            .expect("upstream remote should resolve");
        assert_eq!(
            resolved.identity.canonical_identity,
            "github.com/acme/gadgets"
        );

        std::fs::remove_dir_all(&repo).expect("clean up temp repo");
    }

    #[test]
    fn repo_without_remotes_reports_missing_identity() {
        let repo = unique_temp_dir("no-remote");
        init_git_repo(&repo);

        let error = resolve_repository_identity(&repo, None, "origin")
            .expect_err("repo without remotes should fail");
        assert_eq!(
            error,
            RepositoryIdentityResolutionError::MissingIdentity {
                remote_name: "origin".to_string(),
            }
        );

        std::fs::remove_dir_all(&repo).expect("clean up temp repo");
    }

    #[test]
    fn explicit_identity_wins_over_real_remote() {
        let repo = unique_temp_dir("explicit-wins");
        init_git_repo(&repo);
        add_remote(&repo, "origin", "git@github.com:acme/widgets.git");

        let resolved = resolve_repository_identity(&repo, Some("acme-monorepo"), "origin")
            .expect("explicit identity should resolve");
        assert_eq!(resolved.identity.canonical_identity, "acme-monorepo");
        assert_eq!(resolved.source, RepositoryIdentitySource::ExplicitConfig);

        std::fs::remove_dir_all(&repo).expect("clean up temp repo");
    }

    #[test]
    fn non_repository_directory_reports_missing_identity() {
        let dir = unique_temp_dir("not-a-repo");

        let error = resolve_repository_identity(&dir, None, "origin")
            .expect_err("non-repository directory should fail");
        assert_eq!(
            error,
            RepositoryIdentityResolutionError::MissingIdentity {
                remote_name: "origin".to_string(),
            }
        );

        std::fs::remove_dir_all(&dir).expect("clean up temp dir");
    }
}
