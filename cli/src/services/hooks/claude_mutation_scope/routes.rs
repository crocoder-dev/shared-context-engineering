use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::AttemptKey;
use crate::services::hooks::mutation_scope_lock::AdapterLockSpec;

const SCE_STATE_DIR: &str = "sce";
const ROUTE_TABLE_FILE: &str = "claude-mutation-scope-routes.json";

const ROUTE_LOCK: AdapterLockSpec = AdapterLockSpec::state("claude-mutation-scope-routes.lock");

const ROUTE_TABLE_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RouteTarget {
    pub checkout_path: PathBuf,
    pub git_dir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RouteRecord {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub tool_use_id: String,
    pub target: RouteTarget,
}

impl RouteRecord {
    fn matches_key(&self, key: &AttemptKey) -> bool {
        self.session_id == key.session_id
            && self.agent_id == key.agent_id
            && self.tool_use_id == key.tool_use_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RouteTable {
    pub version: u32,
    pub routes: Vec<RouteRecord>,
}

impl Default for RouteTable {
    fn default() -> Self {
        RouteTable {
            version: ROUTE_TABLE_VERSION,
            routes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PersistRouteOutcome {
    Inserted,
    AlreadyRouted,
    Conflict { existing: RouteTarget },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RemoveRouteOutcome {
    Removed,
    Absent,
    TargetMismatch,
}

#[derive(Debug)]
pub(crate) enum RouteAnchorError {
    ProjectDirUnset,
    Unresolvable {
        project_dir: String,
        source: anyhow::Error,
    },
}

impl std::fmt::Display for RouteAnchorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteAnchorError::ProjectDirUnset => {
                write!(f, "CLAUDE_PROJECT_DIR is unset or blank")
            }
            RouteAnchorError::Unresolvable {
                project_dir,
                source,
            } => write!(
                f,
                "CLAUDE_PROJECT_DIR '{project_dir}' does not resolve to a Git directory: {source}"
            ),
        }
    }
}

impl std::error::Error for RouteAnchorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RouteAnchorError::ProjectDirUnset => None,
            RouteAnchorError::Unresolvable { source, .. } => Some(source.as_ref()),
        }
    }
}

/// Resolves the stable route lookup anchor: the Git directory of `CLAUDE_PROJECT_DIR`.
/// The anchor only locates the route table; it never identifies a mutation target.
pub(crate) async fn resolve_route_anchor(
    project_dir: Option<&str>,
    resolve_git_dir: &impl std::ops::AsyncFn(&str) -> Result<PathBuf>,
) -> Result<PathBuf, RouteAnchorError> {
    let project_dir = project_dir
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .ok_or(RouteAnchorError::ProjectDirUnset)?;
    resolve_git_dir(project_dir)
        .await
        .map_err(|source| RouteAnchorError::Unresolvable {
            project_dir: project_dir.to_string(),
            source,
        })
}

fn route_dir(anchor_git_dir: &Path) -> PathBuf {
    anchor_git_dir.join(SCE_STATE_DIR)
}

pub(crate) fn route_table_path(anchor_git_dir: &Path) -> PathBuf {
    route_dir(anchor_git_dir).join(ROUTE_TABLE_FILE)
}

pub(crate) async fn read_routes(anchor_git_dir: &Path) -> Result<RouteTable> {
    let anchor_git_dir = anchor_git_dir.to_owned();
    tokio::task::spawn_blocking(move || read_routes_sync(&anchor_git_dir))
        .await
        .context("Route table read worker failed")?
}

fn read_routes_sync(anchor_git_dir: &Path) -> Result<RouteTable> {
    let path = route_table_path(anchor_git_dir);
    if !path.exists() {
        return Ok(RouteTable::default());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read route table '{}'", path.display()))?;
    let table: RouteTable = serde_json::from_str(&content)
        .with_context(|| format!("Route table '{}' is malformed", path.display()))?;
    if table.version != ROUTE_TABLE_VERSION {
        return Err(anyhow!(
            "Route table '{}' has unsupported version {} (expected {})",
            path.display(),
            table.version,
            ROUTE_TABLE_VERSION
        ));
    }
    Ok(table)
}

fn write_routes_durably_sync<F>(
    anchor_git_dir: &Path,
    table: &RouteTable,
    before_rename: F,
) -> Result<()>
where
    F: FnOnce(&Path, &Path) -> Result<()>,
{
    let dir = route_dir(anchor_git_dir);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create route table directory '{}'", dir.display()))?;

    let path = dir.join(ROUTE_TABLE_FILE);
    let tmp_path = dir.join(format!("{ROUTE_TABLE_FILE}.tmp"));

    let serialized = serde_json::to_vec_pretty(table).context("Failed to serialize route table")?;

    let mut tmp_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .with_context(|| {
            format!(
                "Failed to open temporary route table file '{}'",
                tmp_path.display()
            )
        })?;
    tmp_file.write_all(&serialized).with_context(|| {
        format!(
            "Failed to write temporary route table file '{}'",
            tmp_path.display()
        )
    })?;
    tmp_file.sync_data().with_context(|| {
        format!(
            "Failed to sync temporary route table file '{}'",
            tmp_path.display()
        )
    })?;
    drop(tmp_file);

    before_rename(&tmp_path, &path)?;

    std::fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "Failed to rename '{}' to '{}'",
            tmp_path.display(),
            path.display()
        )
    })?;

    #[cfg(unix)]
    {
        if let Ok(dir_handle) = std::fs::File::open(&dir) {
            let _ = dir_handle.sync_all();
        }
    }

    Ok(())
}

enum RouteTransaction<T> {
    Unchanged(T),
    Persist(T),
}

async fn with_locked_routes<T, B, F>(
    anchor_git_dir: &Path,
    before_rename: B,
    operation: F,
) -> Result<T>
where
    T: Send + 'static,
    B: FnOnce(&Path, &Path) -> Result<()> + Send + 'static,
    F: FnOnce(&mut RouteTable) -> Result<RouteTransaction<T>> + Send + 'static,
{
    let anchor_git_dir = anchor_git_dir.to_owned();
    ROUTE_LOCK
        .run_locked_blocking(&route_dir(&anchor_git_dir), move || {
            let mut table = read_routes_sync(&anchor_git_dir)?;
            match operation(&mut table)? {
                RouteTransaction::Unchanged(result) => Ok(result),
                RouteTransaction::Persist(result) => {
                    write_routes_durably_sync(&anchor_git_dir, &table, before_rename)?;
                    Ok(result)
                }
            }
        })
        .await
}

/// Durably records the target of one tool invocation. An existing route for the same key is never
/// retargeted: a different target is reported as `Conflict` so the caller can fail closed.
pub(crate) async fn persist_route(
    anchor_git_dir: &Path,
    key: &AttemptKey,
    target: &RouteTarget,
) -> Result<PersistRouteOutcome> {
    persist_route_inner(anchor_git_dir, key, target, |_, _| Ok(())).await
}

async fn persist_route_inner<B>(
    anchor_git_dir: &Path,
    key: &AttemptKey,
    target: &RouteTarget,
    before_rename: B,
) -> Result<PersistRouteOutcome>
where
    B: FnOnce(&Path, &Path) -> Result<()> + Send + 'static,
{
    let key = key.clone();
    let target = target.clone();
    with_locked_routes(anchor_git_dir, before_rename, move |table| {
        if let Some(existing) = table.routes.iter().find(|route| route.matches_key(&key)) {
            let outcome = if existing.target == target {
                PersistRouteOutcome::AlreadyRouted
            } else {
                PersistRouteOutcome::Conflict {
                    existing: existing.target.clone(),
                }
            };
            return Ok(RouteTransaction::Unchanged(outcome));
        }

        table.routes.push(RouteRecord {
            session_id: key.session_id,
            agent_id: key.agent_id,
            tool_use_id: key.tool_use_id,
            target,
        });
        Ok(RouteTransaction::Persist(PersistRouteOutcome::Inserted))
    })
    .await
}

pub(crate) async fn load_route(
    anchor_git_dir: &Path,
    key: &AttemptKey,
) -> Result<Option<RouteTarget>> {
    let table = read_routes(anchor_git_dir).await?;
    Ok(table
        .routes
        .into_iter()
        .find(|route| route.matches_key(key))
        .map(|route| route.target))
}

pub(crate) async fn routes_for_session(
    anchor_git_dir: &Path,
    session_id: &str,
) -> Result<Vec<RouteRecord>> {
    let table = read_routes(anchor_git_dir).await?;
    Ok(table
        .routes
        .into_iter()
        .filter(|route| route.session_id == session_id)
        .collect())
}

/// Removes a route only while it still points at `expected_target`, so a stale caller cannot
/// delete a route that was re-established for a reused tool-use ID.
pub(crate) async fn remove_route(
    anchor_git_dir: &Path,
    key: &AttemptKey,
    expected_target: &RouteTarget,
) -> Result<RemoveRouteOutcome> {
    remove_route_inner(anchor_git_dir, key, expected_target, |_, _| Ok(())).await
}

async fn remove_route_inner<B>(
    anchor_git_dir: &Path,
    key: &AttemptKey,
    expected_target: &RouteTarget,
    before_rename: B,
) -> Result<RemoveRouteOutcome>
where
    B: FnOnce(&Path, &Path) -> Result<()> + Send + 'static,
{
    let key = key.clone();
    let expected_target = expected_target.clone();
    with_locked_routes(anchor_git_dir, before_rename, move |table| {
        let Some(index) = table
            .routes
            .iter()
            .position(|route| route.matches_key(&key))
        else {
            return Ok(RouteTransaction::Unchanged(RemoveRouteOutcome::Absent));
        };
        if table.routes[index].target != expected_target {
            return Ok(RouteTransaction::Unchanged(
                RemoveRouteOutcome::TargetMismatch,
            ));
        }
        table.routes.remove(index);
        Ok(RouteTransaction::Persist(RemoveRouteOutcome::Removed))
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use anyhow::anyhow;

    use super::{
        load_route, persist_route, persist_route_inner, read_routes, remove_route,
        remove_route_inner, resolve_route_anchor, route_table_path, routes_for_session,
        PersistRouteOutcome, RemoveRouteOutcome, RouteAnchorError, RouteTarget,
    };
    use crate::services::hooks::claude_mutation_scope::AttemptKey;

    const CONCURRENT_WRITERS: usize = 16;

    fn key(session_id: &str, agent_id: Option<&str>, tool_use_id: &str) -> AttemptKey {
        AttemptKey {
            session_id: session_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            tool_use_id: tool_use_id.to_string(),
        }
    }

    fn target(root: &Path, repo: &str) -> RouteTarget {
        RouteTarget {
            checkout_path: root.join(repo),
            git_dir: root.join(repo).join(".git"),
        }
    }

    fn injected_failure(_: &Path, _: &Path) -> anyhow::Result<()> {
        Err(anyhow!("injected crash before rename"))
    }

    #[tokio::test]
    async fn persisted_route_is_recovered_by_a_fresh_read_from_disk() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let repo_a = target(anchor.path(), "repoA");
        let tool = key("s1", None, "toolu_1");

        let outcome = persist_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("persist route");
        assert_eq!(outcome, PersistRouteOutcome::Inserted);

        assert_eq!(
            load_route(anchor.path(), &tool).await.expect("load route"),
            Some(repo_a)
        );
        assert!(route_table_path(anchor.path()).exists());
    }

    #[tokio::test]
    async fn missing_route_table_yields_no_route() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let route = load_route(anchor.path(), &key("s1", None, "toolu_1"))
            .await
            .expect("absent table reads as empty");
        assert_eq!(route, None);
    }

    #[tokio::test]
    async fn failed_route_write_leaves_no_route_and_preserves_existing_routes() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let existing = key("s1", None, "toolu_existing");
        let repo_a = target(anchor.path(), "repoA");
        persist_route(anchor.path(), &existing, &repo_a)
            .await
            .expect("seed route");

        let crashed = key("s1", None, "toolu_crashed");
        let repo_b = target(anchor.path(), "repoB");
        persist_route_inner(anchor.path(), &crashed, &repo_b, injected_failure)
            .await
            .expect_err("injected failure must surface so PreToolUse can deny");

        assert_eq!(
            load_route(anchor.path(), &crashed).await.expect("load"),
            None
        );
        assert_eq!(
            load_route(anchor.path(), &existing).await.expect("load"),
            Some(repo_a.clone())
        );

        let retried = persist_route(anchor.path(), &crashed, &repo_b)
            .await
            .expect("a leftover temporary file must not block a retry");
        assert_eq!(retried, PersistRouteOutcome::Inserted);
        assert_eq!(
            load_route(anchor.path(), &crashed).await.expect("load"),
            Some(repo_b)
        );
        assert_eq!(
            load_route(anchor.path(), &existing).await.expect("load"),
            Some(repo_a)
        );
    }

    #[tokio::test]
    async fn failed_route_removal_keeps_the_route_recoverable() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let tool = key("s1", None, "toolu_1");
        let repo_a = target(anchor.path(), "repoA");
        persist_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("seed route");

        remove_route_inner(anchor.path(), &tool, &repo_a, injected_failure)
            .await
            .expect_err("injected failure must surface");

        assert_eq!(
            load_route(anchor.path(), &tool).await.expect("load"),
            Some(repo_a.clone())
        );
        assert_eq!(
            remove_route(anchor.path(), &tool, &repo_a)
                .await
                .expect("retry removal"),
            RemoveRouteOutcome::Removed
        );
    }

    #[tokio::test]
    async fn duplicate_persist_for_the_same_target_is_idempotent() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let tool = key("s1", None, "toolu_1");
        let repo_a = target(anchor.path(), "repoA");

        persist_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("first persist");
        let replay = persist_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("replayed persist");

        assert_eq!(replay, PersistRouteOutcome::AlreadyRouted);
        assert_eq!(
            read_routes(anchor.path()).await.expect("read").routes.len(),
            1
        );
    }

    #[tokio::test]
    async fn live_route_is_never_silently_retargeted() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let tool = key("s1", None, "toolu_1");
        let repo_a = target(anchor.path(), "repoA");
        let nested_b = target(anchor.path(), "repoA/nestedB");
        persist_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("first persist");

        let outcome = persist_route(anchor.path(), &tool, &nested_b)
            .await
            .expect("conflicting persist reads successfully");

        assert_eq!(
            outcome,
            PersistRouteOutcome::Conflict {
                existing: repo_a.clone()
            }
        );
        assert_eq!(
            load_route(anchor.path(), &tool).await.expect("load"),
            Some(repo_a)
        );
    }

    #[tokio::test]
    async fn reused_tool_use_id_after_removal_routes_to_its_new_target() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let tool = key("s1", None, "toolu_1");
        let repo_a = target(anchor.path(), "repoA");
        let repo_b = target(anchor.path(), "repoB");
        persist_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("first persist");
        remove_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("terminal removal");

        let reused = persist_route(anchor.path(), &tool, &repo_b)
            .await
            .expect("reuse persist");

        assert_eq!(reused, PersistRouteOutcome::Inserted);
        assert_eq!(
            load_route(anchor.path(), &tool).await.expect("load"),
            Some(repo_b.clone())
        );

        let stale_removal = remove_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("stale removal");
        assert_eq!(stale_removal, RemoveRouteOutcome::TargetMismatch);
        assert_eq!(
            load_route(anchor.path(), &tool).await.expect("load"),
            Some(repo_b)
        );
    }

    #[tokio::test]
    async fn duplicate_removal_is_safe() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let tool = key("s1", None, "toolu_1");
        let repo_a = target(anchor.path(), "repoA");
        persist_route(anchor.path(), &tool, &repo_a)
            .await
            .expect("persist");

        assert_eq!(
            remove_route(anchor.path(), &tool, &repo_a)
                .await
                .expect("first removal"),
            RemoveRouteOutcome::Removed
        );
        assert_eq!(
            remove_route(anchor.path(), &tool, &repo_a)
                .await
                .expect("duplicate removal"),
            RemoveRouteOutcome::Absent
        );
    }

    #[tokio::test]
    async fn main_agent_and_subagent_with_the_same_tool_use_id_do_not_share_a_route() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let main_tool = key("s1", None, "toolu_1");
        let subagent_tool = key("s1", Some("agent_1"), "toolu_1");
        let other_session_tool = key("s2", None, "toolu_1");
        let repo_a = target(anchor.path(), "repoA");
        let repo_b = target(anchor.path(), "repoB");

        persist_route(anchor.path(), &main_tool, &repo_a)
            .await
            .expect("main persist");
        assert_eq!(
            persist_route(anchor.path(), &subagent_tool, &repo_b)
                .await
                .expect("subagent persist"),
            PersistRouteOutcome::Inserted
        );

        assert_eq!(
            load_route(anchor.path(), &main_tool).await.expect("load"),
            Some(repo_a.clone())
        );
        assert_eq!(
            load_route(anchor.path(), &subagent_tool)
                .await
                .expect("load"),
            Some(repo_b)
        );
        assert_eq!(
            load_route(anchor.path(), &other_session_tool)
                .await
                .expect("load"),
            None
        );

        assert_eq!(
            remove_route(anchor.path(), &subagent_tool, &repo_a)
                .await
                .expect("cross-key removal"),
            RemoveRouteOutcome::TargetMismatch
        );
        assert_eq!(
            load_route(anchor.path(), &main_tool).await.expect("load"),
            Some(repo_a)
        );
    }

    #[tokio::test]
    async fn session_routes_include_only_that_session() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let repo_a = target(anchor.path(), "repoA");
        let repo_b = target(anchor.path(), "repoB");
        persist_route(anchor.path(), &key("s1", None, "t1"), &repo_a)
            .await
            .expect("persist");
        persist_route(anchor.path(), &key("s1", Some("agent_1"), "t2"), &repo_b)
            .await
            .expect("persist");
        persist_route(anchor.path(), &key("s2", None, "t3"), &repo_a)
            .await
            .expect("persist");

        let mut tool_use_ids: Vec<String> = routes_for_session(anchor.path(), "s1")
            .await
            .expect("session routes")
            .into_iter()
            .map(|route| route.tool_use_id)
            .collect();
        tool_use_ids.sort();

        assert_eq!(tool_use_ids, vec!["t1".to_string(), "t2".to_string()]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_writers_for_distinct_keys_lose_no_routes() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let anchor_path: PathBuf = anchor.path().to_owned();

        let writers: Vec<_> = (0..CONCURRENT_WRITERS)
            .map(|index| {
                let anchor_path = anchor_path.clone();
                tokio::spawn(async move {
                    let repo = if index % 2 == 0 { "repoA" } else { "repoB" };
                    persist_route(
                        &anchor_path,
                        &key("s1", None, &format!("toolu_{index}")),
                        &target(&anchor_path, repo),
                    )
                    .await
                })
            })
            .collect();
        for writer in writers {
            assert_eq!(
                writer.await.expect("join").expect("persist"),
                PersistRouteOutcome::Inserted
            );
        }

        assert_eq!(
            read_routes(&anchor_path).await.expect("read").routes.len(),
            CONCURRENT_WRITERS
        );
        assert_eq!(
            load_route(&anchor_path, &key("s1", None, "toolu_7"))
                .await
                .expect("load"),
            Some(target(&anchor_path, "repoB"))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_duplicate_writers_insert_exactly_once() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let anchor_path: PathBuf = anchor.path().to_owned();
        let tool = key("s1", None, "toolu_dup");

        let writers: Vec<_> = (0..CONCURRENT_WRITERS)
            .map(|_| {
                let anchor_path = anchor_path.clone();
                let tool = tool.clone();
                tokio::spawn(async move {
                    persist_route(&anchor_path, &tool, &target(&anchor_path, "repoA")).await
                })
            })
            .collect();
        let mut inserted = 0;
        for writer in writers {
            match writer.await.expect("join").expect("persist") {
                PersistRouteOutcome::Inserted => inserted += 1,
                PersistRouteOutcome::AlreadyRouted => {}
                PersistRouteOutcome::Conflict { existing } => {
                    panic!("same-target replay reported a conflict with {existing:?}")
                }
            }
        }

        assert_eq!(inserted, 1);
        assert_eq!(
            read_routes(&anchor_path).await.expect("read").routes.len(),
            1
        );
    }

    #[tokio::test]
    async fn malformed_route_table_fails_closed() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let path = route_table_path(anchor.path());
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"{ not json").expect("write");

        load_route(anchor.path(), &key("s1", None, "toolu_1"))
            .await
            .expect_err("malformed table must not read as empty");
        persist_route(
            anchor.path(),
            &key("s1", None, "toolu_1"),
            &target(anchor.path(), "repoA"),
        )
        .await
        .expect_err("malformed table must not be overwritten");
        assert_eq!(
            std::fs::read(&path).expect("read"),
            b"{ not json",
            "malformed table must be preserved for diagnosis"
        );
    }

    #[tokio::test]
    async fn unsupported_route_table_version_fails_closed() {
        let anchor = tempfile::tempdir().expect("temp dir");
        let path = route_table_path(anchor.path());
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, br#"{"version":2,"routes":[]}"#).expect("write");

        let error = read_routes(anchor.path())
            .await
            .expect_err("future version must not be read");
        assert!(error.to_string().contains("unsupported version 2"));
    }

    #[tokio::test]
    async fn route_anchor_requires_a_project_dir() {
        let resolver = async |_: &str| -> anyhow::Result<PathBuf> {
            panic!("resolver must not run without a project dir")
        };

        for project_dir in [None, Some(""), Some("   ")] {
            assert!(matches!(
                resolve_route_anchor(project_dir, &resolver).await,
                Err(RouteAnchorError::ProjectDirUnset)
            ));
        }
    }

    #[tokio::test]
    async fn route_anchor_reports_an_unresolvable_project_dir() {
        let resolver =
            async |_: &str| -> anyhow::Result<PathBuf> { Err(anyhow!("not a git repo")) };

        let error = resolve_route_anchor(Some("/not/a/repo"), &resolver)
            .await
            .expect_err("unresolvable project dir");

        assert!(matches!(
            &error,
            RouteAnchorError::Unresolvable { project_dir, .. } if project_dir == "/not/a/repo"
        ));
    }

    #[tokio::test]
    async fn route_anchor_resolves_the_project_dir_git_dir_not_a_nested_repository() {
        let root = tempfile::tempdir().expect("temp dir");
        let repo_a = root.path().join("repoA");
        let nested_b = repo_a.join("nestedB");
        for repo in [&repo_a, &nested_b] {
            std::fs::create_dir_all(repo).expect("mkdir");
            let status = std::process::Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(repo)
                .status()
                .expect("git init runs");
            assert!(status.success());
        }
        let resolver = async |dir: &str| {
            crate::services::mutation_trace::runtime::resolve_git_dir(Path::new(dir)).await
        };

        let anchor = resolve_route_anchor(repo_a.to_str(), &resolver)
            .await
            .expect("anchor resolves");

        assert_eq!(
            anchor.canonicalize().expect("canonical anchor"),
            repo_a
                .join(".git")
                .canonicalize()
                .expect("canonical git dir")
        );
    }
}
