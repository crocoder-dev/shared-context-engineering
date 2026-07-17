//! Repository-scoped Agent Trace storage resolution.
//!
//! Resolves the active Agent Trace database for a Git repository checkout:
//! one logical Git repository maps to exactly one database at
//! `<state-root>/sce/repos/<repository-id>/agent-trace.db`. Clones and linked
//! worktrees of the same logical repository share that database while keeping
//! their own distinct checkout IDs. Legacy checkout-scoped
//! `agent-trace-<checkout-id>.db` files are never selected, created, or
//! touched by this resolver.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::services::agent_trace_db::AgentTraceDb;
use crate::services::checkout::{get_or_create_checkout_id, resolve_git_dir};
use crate::services::default_paths::{
    agent_trace_db_path_for_repository, agent_trace_db_path_for_repository_at,
};
use crate::services::repository_identity::resolve::{
    resolve_repository_identity, ResolvedRepositoryIdentity,
};

/// Inputs needed to resolve the active repository-scoped Agent Trace storage.
///
/// The identity inputs mirror the `agent_trace.repository_id` and
/// `agent_trace.repository_remote` configuration keys; callers pass the
/// already-resolved configuration values.
#[derive(Clone, Copy, Debug)]
pub struct AgentTraceStorageContext<'a> {
    /// Root of the Git working tree the current command runs in.
    pub repository_root: &'a Path,
    /// Explicit `agent_trace.repository_id` configuration value, if set.
    pub explicit_repository_id: Option<&'a str>,
    /// Configured `agent_trace.repository_remote` name (default `origin`).
    pub repository_remote: &'a str,
}

/// The resolved active Agent Trace storage for one repository checkout.
pub struct ResolvedAgentTraceStorage {
    /// Repository identity (canonical identity plus repository ID) and the
    /// source it was resolved from.
    pub repository_identity: ResolvedRepositoryIdentity,
    /// Stable identity of this clone/worktree. Kept for diagnostics only;
    /// never persisted on Agent Trace rows.
    pub checkout_id: String,
    /// Repository-scoped database path
    /// `<state-root>/sce/repos/<repository-id>/agent-trace.db`.
    pub db_path: PathBuf,
    /// Open repository-scoped Agent Trace database.
    pub db: AgentTraceDb,
}

/// Resolves the repository-scoped Agent Trace storage for a checkout using
/// the canonical state root from the default-path catalog.
pub fn resolve_agent_trace_storage(
    context: &AgentTraceStorageContext<'_>,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository(&repository_identity.identity.repository_id)?;
    open_storage(context, repository_identity, db_path)
}

/// Resolution core against an explicit state root, so tests can exercise the
/// full path without touching the real user state directory.
pub fn resolve_agent_trace_storage_at_state_root(
    context: &AgentTraceStorageContext<'_>,
    state_root: &Path,
) -> Result<ResolvedAgentTraceStorage> {
    let repository_identity = resolve_identity(context)?;
    let db_path = agent_trace_db_path_for_repository_at(
        state_root,
        &repository_identity.identity.repository_id,
    )?;
    open_storage(context, repository_identity, db_path)
}

fn resolve_identity(context: &AgentTraceStorageContext<'_>) -> Result<ResolvedRepositoryIdentity> {
    resolve_repository_identity(
        context.repository_root,
        context.explicit_repository_id,
        context.repository_remote,
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}

fn open_storage(
    context: &AgentTraceStorageContext<'_>,
    repository_identity: ResolvedRepositoryIdentity,
    db_path: PathBuf,
) -> Result<ResolvedAgentTraceStorage> {
    let git_dir = resolve_git_dir(context.repository_root).with_context(|| {
        format!(
            "failed to resolve git directory for Agent Trace repository DB from '{}'",
            context.repository_root.display()
        )
    })?;
    let checkout_id = get_or_create_checkout_id(&git_dir).with_context(|| {
        format!(
            "failed to get or create checkout identity under '{}'",
            git_dir.display()
        )
    })?;

    // Opening the database creates `repos/<repository-id>/` when missing;
    // both directory creation and schema initialization are idempotent, so
    // concurrent first-time resolution is safe.
    let fast_open = AgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
        .and_then(|db| db.ensure_schema_ready_for_hooks().map(|()| db));
    let db = match fast_open {
        Ok(db) => db,
        Err(fast_error) => AgentTraceDb::open_at(&db_path).with_context(|| {
            format!(
                "failed to initialize repository-scoped Agent Trace DB for repository {} at '{}' (fast-path attempt: {fast_error})",
                repository_identity.identity.repository_id,
                db_path.display()
            )
        })?,
    };

    Ok(ResolvedAgentTraceStorage {
        repository_identity,
        checkout_id,
        db_path,
        db,
    })
}

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
            "sce-agent-trace-storage-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn git(repo_root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .unwrap_or_else(|error| panic!("git {args:?} failed to spawn: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git_repo_with_remote(label: &str, remote_url: &str) -> PathBuf {
        let repo = unique_temp_dir(label);
        git(&repo, &["init", "-q"]);
        git(&repo, &["remote", "add", "origin", remote_url]);
        repo
    }

    fn context_for(repository_root: &Path) -> AgentTraceStorageContext<'_> {
        AgentTraceStorageContext {
            repository_root,
            explicit_repository_id: None,
            repository_remote: "origin",
        }
    }

    fn assert_no_legacy_db_paths(state_root: &Path) {
        let sce_dir = state_root.join("sce");
        assert!(
            !sce_dir.join("agent-trace.db").exists(),
            "resolver must not create the legacy global agent-trace.db"
        );
        if let Ok(entries) = std::fs::read_dir(&sce_dir) {
            for entry in entries {
                let name = entry.expect("read sce dir entry").file_name();
                let name = name.to_string_lossy().into_owned();
                let is_db_file = Path::new(&name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("db"));
                assert!(
                    !(name.starts_with("agent-trace-") && is_db_file),
                    "resolver must not create checkout-scoped DB '{name}'"
                );
            }
        }
    }

    #[test]
    fn different_repository_identities_use_different_db_paths() {
        let state_root = unique_temp_dir("state-separate");
        let repo_a = init_git_repo_with_remote("repo-a", "git@github.com:acme/widgets.git");
        let repo_b = init_git_repo_with_remote("repo-b", "git@github.com:acme/gadgets.git");

        let storage_a =
            resolve_agent_trace_storage_at_state_root(&context_for(&repo_a), &state_root)
                .expect("repo A storage should resolve");
        let storage_b =
            resolve_agent_trace_storage_at_state_root(&context_for(&repo_b), &state_root)
                .expect("repo B storage should resolve");

        assert_ne!(
            storage_a.repository_identity.identity.repository_id,
            storage_b.repository_identity.identity.repository_id
        );
        assert_ne!(storage_a.db_path, storage_b.db_path);
        assert!(storage_a.db_path.is_file());
        assert!(storage_b.db_path.is_file());
        assert_no_legacy_db_paths(&state_root);

        std::fs::remove_dir_all(&state_root).expect("clean up state root");
        std::fs::remove_dir_all(&repo_a).expect("clean up repo A");
        std::fs::remove_dir_all(&repo_b).expect("clean up repo B");
    }

    #[test]
    fn clones_of_the_same_repository_share_the_db_path_with_distinct_checkout_ids() {
        let state_root = unique_temp_dir("state-clones");
        // Equivalent SSH and HTTPS remotes for the same logical repository.
        let clone_a = init_git_repo_with_remote("clone-a", "git@github.com:acme/widgets.git");
        let clone_b = init_git_repo_with_remote("clone-b", "https://github.com/acme/widgets.git");

        let storage_a =
            resolve_agent_trace_storage_at_state_root(&context_for(&clone_a), &state_root)
                .expect("clone A storage should resolve");
        let storage_b =
            resolve_agent_trace_storage_at_state_root(&context_for(&clone_b), &state_root)
                .expect("clone B storage should resolve");

        assert_eq!(
            storage_a.repository_identity.identity.repository_id,
            storage_b.repository_identity.identity.repository_id
        );
        assert_eq!(storage_a.db_path, storage_b.db_path);
        assert_ne!(storage_a.checkout_id, storage_b.checkout_id);
        assert_no_legacy_db_paths(&state_root);

        std::fs::remove_dir_all(&state_root).expect("clean up state root");
        std::fs::remove_dir_all(&clone_a).expect("clean up clone A");
        std::fs::remove_dir_all(&clone_b).expect("clean up clone B");
    }

    #[test]
    fn linked_worktree_shares_the_db_path_with_a_distinct_checkout_id() {
        let state_root = unique_temp_dir("state-worktree");
        let repo = init_git_repo_with_remote("worktree-main", "git@github.com:acme/widgets.git");
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["commit", "--allow-empty", "-q", "-m", "init"]);
        let worktree = unique_temp_dir("worktree-linked");
        // `git worktree add` refuses to use an existing directory unless empty;
        // the helper creates it empty, so add into it directly.
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                worktree.to_str().expect("utf-8 worktree path"),
            ],
        );

        let storage_main =
            resolve_agent_trace_storage_at_state_root(&context_for(&repo), &state_root)
                .expect("main checkout storage should resolve");
        let storage_worktree =
            resolve_agent_trace_storage_at_state_root(&context_for(&worktree), &state_root)
                .expect("worktree storage should resolve");

        assert_eq!(storage_main.db_path, storage_worktree.db_path);
        assert_ne!(storage_main.checkout_id, storage_worktree.checkout_id);
        assert_no_legacy_db_paths(&state_root);

        std::fs::remove_dir_all(&state_root).expect("clean up state root");
        std::fs::remove_dir_all(&worktree).expect("clean up worktree");
        std::fs::remove_dir_all(&repo).expect("clean up repo");
    }

    #[test]
    fn explicit_repository_id_overrides_the_remote() {
        let state_root = unique_temp_dir("state-explicit");
        let repo = init_git_repo_with_remote("explicit", "git@github.com:acme/widgets.git");
        let context = AgentTraceStorageContext {
            repository_root: &repo,
            explicit_repository_id: Some("acme-monorepo"),
            repository_remote: "origin",
        };

        let storage = resolve_agent_trace_storage_at_state_root(&context, &state_root)
            .expect("explicit identity storage should resolve");
        assert_eq!(
            storage.repository_identity.identity.canonical_identity,
            "acme-monorepo"
        );
        let expected_path = state_root
            .join("sce")
            .join("repos")
            .join(&storage.repository_identity.identity.repository_id)
            .join("agent-trace.db");
        assert_eq!(storage.db_path, expected_path);

        std::fs::remove_dir_all(&state_root).expect("clean up state root");
        std::fs::remove_dir_all(&repo).expect("clean up repo");
    }

    #[test]
    fn repeated_resolution_is_idempotent() {
        let state_root = unique_temp_dir("state-idempotent");
        let repo = init_git_repo_with_remote("idempotent", "git@github.com:acme/widgets.git");

        let first = resolve_agent_trace_storage_at_state_root(&context_for(&repo), &state_root)
            .expect("first resolution should succeed");
        let second = resolve_agent_trace_storage_at_state_root(&context_for(&repo), &state_root)
            .expect("second resolution should succeed");

        assert_eq!(first.db_path, second.db_path);
        assert_eq!(first.checkout_id, second.checkout_id);
        assert_eq!(
            first.repository_identity.identity.repository_id,
            second.repository_identity.identity.repository_id
        );

        std::fs::remove_dir_all(&state_root).expect("clean up state root");
        std::fs::remove_dir_all(&repo).expect("clean up repo");
    }

    #[test]
    fn missing_identity_fails_with_config_guidance_and_creates_nothing() {
        let state_root = unique_temp_dir("state-missing");
        let repo = unique_temp_dir("no-remote");
        git(&repo, &["init", "-q"]);

        let Err(error) =
            resolve_agent_trace_storage_at_state_root(&context_for(&repo), &state_root)
        else {
            panic!("repo without identity should fail")
        };
        assert!(error.to_string().contains(".sce/config.json"));
        assert!(
            !state_root.join("sce").exists(),
            "failed resolution must not create state directories"
        );

        std::fs::remove_dir_all(&state_root).expect("clean up state root");
        std::fs::remove_dir_all(&repo).expect("clean up repo");
    }

    #[test]
    fn empty_repository_id_path_is_rejected() {
        let error = agent_trace_db_path_for_repository_at(Path::new("/tmp/state"), "  ")
            .expect_err("empty repository ID should fail");
        assert!(error.to_string().contains("must not be empty"));
    }

    #[test]
    fn path_traversal_repository_id_is_rejected() {
        for bad in ["../escape", "a/b", "a\\b", ".", ".."] {
            let error = agent_trace_db_path_for_repository_at(Path::new("/tmp/state"), bad)
                .expect_err("path-unsafe repository ID should be rejected");
            assert!(
                error.to_string().contains("not a valid path segment"),
                "unexpected error for '{bad}': {error}"
            );
        }
    }
}
