use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, bail, Context, Result};
use tokio::process::Command;
use uuid::Uuid;

use crate::services::mutation_trace::types::{TreeId, WorktreeId};

use super::worktree_lock::WorktreeLockLease;

const SCE_RUNTIME_DIR: &str = "sce";
const TMP_INDEX_DIR: &str = "tmp";
const REF_NAMESPACE: &str = "refs/sce/mutation-cursor";

/// `git for-each-ref` format for pin inventory: four `%00`-separated fields —
/// refname, target object name, target object type, and the symbolic-ref
/// target (empty for a direct ref). NUL-separated so no field can be split or
/// trimmed ambiguously; the trailing symref field is always present (possibly
/// empty), so every well-formed line has exactly four fields.
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
const FOR_EACH_REF_PIN_FORMAT: &str =
    "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(symref)";

pub struct GitSnapshotService {
    git_dir: PathBuf,
    repository_root: PathBuf,
}

impl GitSnapshotService {
    pub async fn new(repository_root: &Path) -> Result<GitSnapshotService> {
        let repository_root = resolve_worktree_root(repository_root).await?;
        let git_dir = resolve_git_dir(&repository_root).await?;
        Ok(GitSnapshotService {
            git_dir,
            repository_root,
        })
    }

    pub async fn capture_tree(&self) -> Result<TreeId> {
        self.capture_tree_inner(|_| {}).await
    }

    async fn capture_tree_inner(&self, after_read_tree: impl FnOnce(&Path)) -> Result<TreeId> {
        let tmp_dir = self.git_dir.join(SCE_RUNTIME_DIR).join(TMP_INDEX_DIR);
        tokio::fs::create_dir_all(&tmp_dir).await.with_context(|| {
            format!(
                "Failed to create temporary index directory '{}'",
                tmp_dir.display()
            )
        })?;
        let mut index_guard = TempIndexGuard::reserve(&tmp_dir);

        let result = self
            .capture_tree_with_index(index_guard.path(), after_read_tree)
            .await;
        index_guard.cleanup().await;
        result
    }

    async fn capture_tree_with_index(
        &self,
        index_file: &Path,
        after_read_tree: impl FnOnce(&Path),
    ) -> Result<TreeId> {
        if self.head_exists().await? {
            self.run_git(&["read-tree", "HEAD"], Some(index_file))
                .await?;
        } else {
            self.run_git(&["read-tree", "--empty"], Some(index_file))
                .await?;
        }

        after_read_tree(index_file);

        self.run_git(&["add", "-A", "--", "."], Some(index_file))
            .await?;

        let tree_sha = self.run_git(&["write-tree"], Some(index_file)).await?;
        Ok(TreeId(tree_sha.trim().to_string()))
    }

    pub async fn pin_tree(
        &self,
        lease: WorktreeLockLease,
        worktree_id: &WorktreeId,
        tree: &TreeId,
    ) -> Result<()> {
        self.pin_tree_inner(lease, worktree_id, tree, || {}).await
    }

    async fn pin_tree_inner(
        &self,
        lease: WorktreeLockLease,
        worktree_id: &WorktreeId,
        tree: &TreeId,
        on_worker_entered: impl FnOnce() + Send + 'static,
    ) -> Result<()> {
        let ref_name = pin_ref_name(worktree_id, tree);
        self.run_ref_mutation_inner(
            lease,
            vec!["update-ref".to_string(), ref_name, tree.0.clone()],
            None,
            on_worker_entered,
        )
        .await?;
        Ok(())
    }

    pub async fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String> {
        self.run_git(
            &[
                "diff",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
                &before.0,
                &after.0,
            ],
            None,
        )
        .await
    }

    pub async fn head_tree(&self) -> Result<TreeId> {
        let tree = self.run_git(&["rev-parse", "HEAD^{tree}"], None).await?;
        Ok(TreeId(tree.trim().to_string()))
    }

    pub async fn file_at_tree(&self, tree: &TreeId, path: &str) -> Result<Option<String>> {
        let spec = format!("{}:{}", tree.0, path);
        let output = self
            .cancellable_git_command(&["cat-file", "blob", &spec], None)
            .output()
            .await
            .with_context(|| {
                format!(
                    "Failed to run git cat-file blob '{spec}' in '{}'",
                    self.repository_root.display()
                )
            })?;

        if !output.status.success() {
            return Ok(None);
        }
        Ok(String::from_utf8(output.stdout).ok())
    }

    /// Inventory every SCE snapshot pin owned by `worktree_id`.
    ///
    /// Runs `git for-each-ref` constrained to the single path prefix
    /// `refs/sce/mutation-cursor/<worktree_id>/`, so a ref owned by any other
    /// worktree or in an unrelated namespace is never returned. Each line is
    /// validated against the shape `pin_tree` produces: a **direct** ref (never
    /// a symbolic ref) whose target is a tree object and whose final path
    /// component equals the target SHA. A symbolic ref anywhere in the
    /// namespace is malformed state — it would let one worktree's pin resolve
    /// through another worktree's ref — and is rejected rather than followed. A
    /// `git for-each-ref` execution or exit failure is
    /// [`PinInventoryError::Git`]; anything malformed inside the namespace is
    /// [`PinInventoryError::MalformedRef`], matchable separately.
    #[allow(
        dead_code,
        reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
    )]
    pub async fn list_pins(
        &self,
        worktree_id: &WorktreeId,
    ) -> std::result::Result<Vec<PinnedRef>, PinInventoryError> {
        let prefix = pin_ref_prefix(worktree_id);
        let raw = self
            .run_git(&["for-each-ref", FOR_EACH_REF_PIN_FORMAT, &prefix], None)
            .await
            .map_err(PinInventoryError::Git)?;

        raw.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| parse_pin_line(line, &prefix))
            .collect()
    }

    #[allow(
        dead_code,
        reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
    )]
    pub async fn delete_pins(&self, lease: WorktreeLockLease, pins: &[PinnedRef]) -> Result<()> {
        self.delete_pins_inner(lease, pins, || {}, || {}).await
    }

    #[allow(
        dead_code,
        reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
    )]
    async fn delete_pins_inner(
        &self,
        lease: WorktreeLockLease,
        pins: &[PinnedRef],
        after_preflight: impl FnOnce(),
        on_worker_entered: impl FnOnce() + Send + 'static,
    ) -> Result<()> {
        if pins.is_empty() {
            return Ok(());
        }

        self.assert_pins_are_unchanged_direct_refs(pins).await?;

        after_preflight();

        let mut stdin_payload = String::new();
        for pin in pins {
            stdin_payload.push_str("delete ");
            stdin_payload.push_str(&pin.ref_name);
            stdin_payload.push(' ');
            stdin_payload.push_str(&pin.tree.0);
            stdin_payload.push('\n');
        }

        self.run_ref_mutation_inner(
            lease,
            vec![
                "update-ref".to_string(),
                "--no-deref".to_string(),
                "--stdin".to_string(),
            ],
            Some(stdin_payload.into_bytes()),
            on_worker_entered,
        )
        .await?;

        Ok(())
    }

    async fn run_ref_mutation_inner(
        &self,
        lease: WorktreeLockLease,
        args: Vec<String>,
        stdin: Option<Vec<u8>>,
        on_worker_entered: impl FnOnce() + Send + 'static,
    ) -> Result<String> {
        let repository_root = self.repository_root.clone();
        let git_dir = self.git_dir.clone();

        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            on_worker_entered();
            run_ref_mutation_blocking(&repository_root, &git_dir, &args, stdin.as_deref())
        })
        .await
        .map_err(|source| anyhow!("Git ref mutation worker failed: {source}"))?
    }

    /// Fail closed unless every supplied pin is still exactly the direct ref
    /// that was inventoried: present, a direct (non-symbolic) ref, targeting a
    /// tree, and pointing at the recorded SHA. Re-inventoried in a single
    /// `git for-each-ref` over the exact ref names, so no enumeration order is
    /// relied on. This closes the common inventory→delete race cleanly; the
    /// residual sub-transaction race is still contained by `--no-deref` plus
    /// the per-`delete` old-value condition, which together cannot follow a
    /// symbolic ref or mutate a ref the caller did not name.
    #[allow(
        dead_code,
        reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
    )]
    async fn assert_pins_are_unchanged_direct_refs(&self, pins: &[PinnedRef]) -> Result<()> {
        let mut args: Vec<&str> = vec!["for-each-ref", FOR_EACH_REF_PIN_FORMAT];
        args.extend(pins.iter().map(|pin| pin.ref_name.as_str()));
        let raw = self.run_git(&args, None).await?;

        let current: Vec<[&str; 4]> = raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                let fields: Vec<&str> = line.split('\0').collect();
                <[&str; 4]>::try_from(fields.as_slice())
                    .map_err(|_| anyhow!("git for-each-ref emitted an unparseable line: '{line}'"))
            })
            .collect::<Result<_>>()?;

        for pin in pins {
            let Some(entry) = current.iter().find(|entry| entry[0] == pin.ref_name) else {
                return Err(anyhow!(
                    "pin ref '{}' no longer exists; refusing to delete stale inventory",
                    pin.ref_name
                ));
            };
            let [_, object_name, object_type, symref] = *entry;

            if !symref.is_empty() {
                return Err(anyhow!(
                    "pin ref '{}' is now a symbolic ref pointing at '{symref}'; mutation-cursor \
                     pins must be direct refs, refusing to delete",
                    pin.ref_name
                ));
            }
            if object_type != "tree" {
                return Err(anyhow!(
                    "pin ref '{}' now targets a {object_type} object, not a tree; refusing to \
                     delete",
                    pin.ref_name
                ));
            }
            if object_name != pin.tree.0 {
                return Err(anyhow!(
                    "pin ref '{}' now points at {object_name}, not the inventoried {}; refusing \
                     to delete",
                    pin.ref_name,
                    pin.tree.0
                ));
            }
        }

        Ok(())
    }

    async fn head_exists(&self) -> Result<bool> {
        let output = self
            .cancellable_git_command(&["rev-parse", "--verify", "--quiet", "HEAD"], None)
            .output()
            .await
            .with_context(|| {
                format!(
                    "Failed to run git rev-parse --verify --quiet HEAD in '{}'",
                    self.repository_root.display()
                )
            })?;

        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if stderr.is_empty() { stdout } else { stderr };
                Err(anyhow!(
                    "git rev-parse --verify --quiet HEAD failed unexpectedly (status {:?}): {detail}",
                    output.status.code()
                ))
            }
        }
    }

    fn cancellable_git_command(&self, args: &[&str], index_file: Option<&Path>) -> Command {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(&self.repository_root)
            .env("GIT_DIR", &self.git_dir)
            .kill_on_drop(true);
        if let Some(index_file) = index_file {
            command.env("GIT_INDEX_FILE", index_file);
        }
        command
    }

    async fn run_git(&self, args: &[&str], index_file: Option<&Path>) -> Result<String> {
        let output = self
            .cancellable_git_command(args, index_file)
            .output()
            .await
            .with_context(|| {
                format!(
                    "Failed to run git command {:?} in '{}'",
                    args,
                    self.repository_root.display()
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            return Err(anyhow!("git {args:?} failed: {detail}"));
        }

        String::from_utf8(output.stdout)
            .with_context(|| format!("git {args:?} emitted invalid UTF-8"))
    }
}

/// One SCE-owned snapshot pin: a ref under
/// `refs/sce/mutation-cursor/<worktree-id>/` and the tree object it protects.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
pub struct PinnedRef {
    pub ref_name: String,
    pub tree: TreeId,
}

/// Why a worktree's pin inventory could not be produced.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
pub enum PinInventoryError {
    /// `git for-each-ref` itself failed to execute or exited non-zero.
    Git(anyhow::Error),
    /// A ref under the SCE namespace is not shaped like a `pin_tree` output: a
    /// symbolic ref, a non-tree target, a name/target SHA mismatch, an
    /// unparseable `for-each-ref` line, or an unexpected extra path segment.
    /// `reason` carries the specific discriminant for tests and `Display`.
    MalformedRef { ref_name: String, reason: String },
}

impl std::fmt::Display for PinInventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinInventoryError::Git(source) => write!(f, "{source}"),
            PinInventoryError::MalformedRef { ref_name, reason } => write!(
                f,
                "Malformed ref '{ref_name}' in the mutation-cursor snapshot namespace: {reason}"
            ),
        }
    }
}

impl std::error::Error for PinInventoryError {}

#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
fn parse_pin_line(line: &str, prefix: &str) -> std::result::Result<PinnedRef, PinInventoryError> {
    let fields: Vec<&str> = line.split('\0').collect();
    let [ref_name, object_name, object_type, symref] = fields.as_slice() else {
        return Err(PinInventoryError::MalformedRef {
            ref_name: fields
                .first()
                .map_or_else(|| line.to_string(), |field| (*field).to_string()),
            reason: format!(
                "git for-each-ref line did not have exactly four NUL-separated fields: '{line}'"
            ),
        });
    };

    if !symref.is_empty() {
        return Err(PinInventoryError::MalformedRef {
            ref_name: (*ref_name).to_string(),
            reason: format!(
                "ref is a symbolic ref pointing at '{symref}'; mutation-cursor pins must be direct \
                 refs to a tree object, and a symbolic ref inside the namespace is rejected rather \
                 than followed"
            ),
        });
    }

    if *object_type != "tree" {
        return Err(PinInventoryError::MalformedRef {
            ref_name: (*ref_name).to_string(),
            reason: format!("ref target is a {object_type} object, not a tree"),
        });
    }

    let Some(suffix) = ref_name.strip_prefix(prefix) else {
        return Err(PinInventoryError::MalformedRef {
            ref_name: (*ref_name).to_string(),
            reason: format!("ref name is not under the expected prefix '{prefix}'"),
        });
    };

    if suffix.is_empty() || suffix.contains('/') {
        return Err(PinInventoryError::MalformedRef {
            ref_name: (*ref_name).to_string(),
            reason: format!(
                "ref name has an unexpected path segment after the worktree prefix: '{suffix}'"
            ),
        });
    }

    if suffix != *object_name {
        return Err(PinInventoryError::MalformedRef {
            ref_name: (*ref_name).to_string(),
            reason: format!(
                "ref name suffix '{suffix}' disagrees with its target tree SHA '{object_name}'"
            ),
        });
    }

    Ok(PinnedRef {
        ref_name: (*ref_name).to_string(),
        tree: TreeId((*object_name).to_string()),
    })
}

#[allow(
    dead_code,
    reason = "ref reconciliation retained and unwired; see context/plans/mutation-cursor-ref-reconciliation.md"
)]
fn pin_ref_prefix(worktree_id: &WorktreeId) -> String {
    format!("{REF_NAMESPACE}/{}/", worktree_id.0)
}

fn pin_ref_name(worktree_id: &WorktreeId, tree: &TreeId) -> String {
    format!("{REF_NAMESPACE}/{}/{}", worktree_id.0, tree.0)
}

fn run_ref_mutation_blocking(
    repository_root: &Path,
    git_dir: &Path,
    args: &[String],
    stdin: Option<&[u8]>,
) -> Result<String> {
    let mut command = std::process::Command::new("git");
    command
        .args(args)
        .current_dir(repository_root)
        .env("GIT_DIR", git_dir);

    let output = match stdin {
        None => command.output().with_context(|| {
            format!(
                "Failed to run git command {args:?} in '{}'",
                repository_root.display()
            )
        })?,
        Some(payload) => {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .with_context(|| {
                    format!(
                        "Failed to run git command {args:?} in '{}'",
                        repository_root.display()
                    )
                })?;

            let write_result = match child.stdin.take() {
                Some(mut child_stdin) => child_stdin.write_all(payload),
                None => Err(std::io::Error::other("child stdin was not piped")),
            };
            let output = child.wait_with_output();

            write_result.with_context(|| format!("Failed to write stdin to git {args:?}"))?;
            output.with_context(|| format!("Failed to wait for git {args:?}"))?
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(anyhow!("git {args:?} failed: {detail}"));
    }

    String::from_utf8(output.stdout).with_context(|| format!("git {args:?} emitted invalid UTF-8"))
}

pub(crate) async fn resolve_git_dir(repository_root: &Path) -> Result<PathBuf> {
    let git_dir =
        PathBuf::from(run_rev_parse(repository_root, &["rev-parse", "--absolute-git-dir"]).await?);

    debug_assert!(
        git_dir.is_absolute(),
        "git rev-parse --absolute-git-dir should always return an absolute path, got '{}'",
        git_dir.display()
    );

    Ok(git_dir)
}

async fn resolve_git_common_dir(repository_root: &Path) -> Result<PathBuf> {
    let git_common_dir = PathBuf::from(
        run_rev_parse(
            repository_root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await?,
    );

    debug_assert!(
        git_common_dir.is_absolute(),
        "git rev-parse --path-format=absolute --git-common-dir should always return an absolute path, got '{}'",
        git_common_dir.display()
    );

    Ok(git_common_dir)
}

pub(crate) async fn resolve_worktree_root(repository_root: &Path) -> Result<PathBuf> {
    let reported_root = run_rev_parse(
        repository_root,
        &["rev-parse", "--path-format=absolute", "--show-toplevel"],
    )
    .await?;

    let root_path = PathBuf::from(reported_root);
    debug_assert!(
        root_path.is_absolute(),
        "git rev-parse --path-format=absolute --show-toplevel should always return an absolute path, got '{}'",
        root_path.display()
    );

    let canonical_root = tokio::fs::canonicalize(&root_path).await.with_context(|| {
        format!(
            "failed to canonicalize the worktree root '{}'",
            root_path.display()
        )
    })?;
    let is_directory = tokio::fs::metadata(&canonical_root)
        .await
        .is_ok_and(|metadata| metadata.is_dir());
    if !is_directory {
        bail!(
            "resolved worktree root '{}' is not a directory",
            canonical_root.display()
        );
    }

    Ok(canonical_root)
}

pub(crate) async fn resolve_worktree_id(repository_root: &Path) -> Result<WorktreeId> {
    let git_dir = resolve_git_dir(repository_root).await?;
    let git_common_dir = resolve_git_common_dir(repository_root).await?;

    if git_dir == git_common_dir {
        return Ok(WorktreeId("main".to_string()));
    }

    let worktree_name = git_dir.file_name().ok_or_else(|| {
        anyhow!(
            "linked worktree git dir '{}' has no final path component",
            git_dir.display()
        )
    })?;

    Ok(WorktreeId(format!(
        "worktrees/{}",
        sanitize_ref_component(&worktree_name.to_string_lossy())
    )))
}

fn sanitize_ref_component(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

async fn run_rev_parse(repository_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .kill_on_drop(true)
        .output()
        .await
        .with_context(|| {
            format!(
                "Failed to run git {args:?} in '{}'",
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(
            "git {args:?} failed in '{}': {}",
            repository_root.display(),
            stderr
        ));
    }

    Ok(String::from_utf8(output.stdout)
        .with_context(|| format!("git {args:?} emitted invalid UTF-8"))?
        .trim()
        .to_string())
}

struct TempIndexGuard {
    path: Option<PathBuf>,
}

impl TempIndexGuard {
    fn reserve(tmp_dir: &Path) -> TempIndexGuard {
        let path = tmp_dir.join(format!("index-{}", Uuid::new_v4()));
        TempIndexGuard { path: Some(path) }
    }

    fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("temporary index guard is still armed")
    }

    async fn cleanup(&mut self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let _ = tokio::fs::remove_file(path).await;
        self.path = None;
    }
}

impl Drop for TempIndexGuard {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            drop(handle.spawn_blocking(move || {
                let _ = std::fs::remove_file(path);
            }));
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;
    use std::time::Duration;

    use super::super::worktree_lock::{WorktreeLock, WorktreeLockError};
    use super::{
        resolve_git_dir, resolve_worktree_id, GitSnapshotService, PinnedRef, TempIndexGuard,
        SCE_RUNTIME_DIR, TMP_INDEX_DIR,
    };
    use crate::services::mutation_trace::types::WorktreeId;

    const LOCK_CONTENDED_TIMEOUT: Duration = Duration::from_millis(300);
    const LOCK_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);
    const GUARD_DROP_CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_committed_repository(root: &Path) {
        std::fs::create_dir_all(root).expect("repo dir");
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "snapshot@example.invalid"]);
        git(root, &["config", "user.name", "Snapshot Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("tracked.txt"), "tracked\n").expect("write tracked file");
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-q", "-m", "initial"]);
    }

    fn temp_index_files(git_dir: &Path) -> Vec<String> {
        let tmp_dir = git_dir.join(SCE_RUNTIME_DIR).join(TMP_INDEX_DIR);
        let mut names: Vec<String> = std::fs::read_dir(&tmp_dir)
            .expect("read temporary index dir")
            .map(|entry| {
                entry
                    .expect("temporary index dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn capture_tree_removes_temporary_index_before_returning_and_keeps_real_index() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo_root = dir.path().join("repo");
        init_committed_repository(&repo_root);
        std::fs::write(repo_root.join("untracked.txt"), "untracked\n").expect("write file");

        let git_dir = resolve_git_dir(&repo_root).await.expect("git dir");
        let real_index_before = std::fs::read(git_dir.join("index")).expect("read real index");

        let snapshot = GitSnapshotService::new(&repo_root)
            .await
            .expect("snapshot service");
        let tree = snapshot.capture_tree().await.expect("capture tree");

        assert!(temp_index_files(&git_dir).is_empty());
        assert_eq!(
            std::fs::read(git_dir.join("index")).expect("read real index after capture"),
            real_index_before
        );
        assert_eq!(
            snapshot
                .file_at_tree(&tree, "tracked.txt")
                .await
                .expect("read tracked file"),
            Some("tracked\n".to_string())
        );
        assert_eq!(
            snapshot
                .file_at_tree(&tree, "untracked.txt")
                .await
                .expect("read untracked file"),
            Some("untracked\n".to_string())
        );
    }

    #[tokio::test]
    async fn capture_tree_removes_temporary_index_and_keeps_original_git_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo_root = dir.path().join("repo");
        init_committed_repository(&repo_root);
        std::fs::write(repo_root.join("untracked.txt"), "untracked\n").expect("write file");

        let git_dir = resolve_git_dir(&repo_root).await.expect("git dir");
        let snapshot = GitSnapshotService::new(&repo_root)
            .await
            .expect("snapshot service");
        let mut index_name = None;

        let error = snapshot
            .capture_tree_inner(|index_file| {
                assert!(index_file.is_file(), "read-tree should create the index");
                let lock_file = index_file.with_file_name(format!(
                    "{}.lock",
                    index_file
                        .file_name()
                        .expect("index name")
                        .to_string_lossy()
                ));
                std::fs::write(&lock_file, "").expect("hold temporary index lock");
                index_name = Some(index_file.file_name().expect("index name").to_owned());
            })
            .await
            .expect_err("git add must fail while the index lock is held");

        let index_name = index_name.expect("hook ran").to_string_lossy().into_owned();
        let message = format!("{error:#}");
        assert!(
            message.starts_with("git [\"add\", \"-A\", \"--\", \".\"] failed:"),
            "unexpected error: {message}"
        );
        assert_eq!(
            temp_index_files(&git_dir),
            vec![format!("{index_name}.lock")]
        );
    }

    #[tokio::test]
    async fn dropped_temp_index_guard_schedules_cleanup_off_the_runtime_worker() {
        let dir = tempfile::tempdir().expect("temp dir");
        let guard = TempIndexGuard::reserve(dir.path());
        let index_file = guard.path().to_path_buf();
        std::fs::write(&index_file, "index").expect("create temporary index");

        drop(guard);

        assert_eq!(tokio::spawn(async { 7 }).await.expect("unrelated task"), 7);
        tokio::time::timeout(GUARD_DROP_CLEANUP_TIMEOUT, async {
            while tokio::fs::try_exists(&index_file)
                .await
                .expect("check temporary index")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropped guard should remove the temporary index");
    }

    #[tokio::test]
    async fn async_snapshot_service_captures_pins_inventories_and_deletes_tree() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo_root = dir.path().join("repo");
        init_committed_repository(&repo_root);
        std::fs::write(repo_root.join("untracked.txt"), "untracked\n").expect("write file");

        let snapshot = GitSnapshotService::new(&repo_root)
            .await
            .expect("snapshot service");
        let worktree_id = WorktreeId("main".to_string());

        let tree = snapshot.capture_tree().await.expect("capture tree");
        assert_eq!(
            snapshot
                .file_at_tree(&tree, "untracked.txt")
                .await
                .expect("read captured file"),
            Some("untracked\n".to_string())
        );

        let git_dir = resolve_git_dir(&repo_root).await.expect("git dir");
        let lock = WorktreeLock::acquire_async(&git_dir, LOCK_RELEASE_TIMEOUT)
            .await
            .expect("worktree lock");

        snapshot
            .pin_tree(lock.lease(), &worktree_id, &tree)
            .await
            .expect("pin tree");
        let pins = snapshot.list_pins(&worktree_id).await.expect("list pins");
        assert_eq!(
            pins,
            vec![PinnedRef {
                ref_name: format!("refs/sce/mutation-cursor/main/{}", tree.0),
                tree: tree.clone(),
            }]
        );

        snapshot
            .delete_pins(lock.lease(), &pins)
            .await
            .expect("delete pins");
        assert!(snapshot
            .list_pins(&worktree_id)
            .await
            .expect("list pins after delete")
            .is_empty());
    }

    #[tokio::test]
    async fn cancelled_pin_keeps_worktree_lock_until_git_ref_mutation_completes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo_root = dir.path().join("repo");
        init_committed_repository(&repo_root);
        std::fs::write(repo_root.join("untracked.txt"), "untracked\n").expect("write file");

        let git_dir = resolve_git_dir(&repo_root).await.expect("git dir");
        let worktree_id = WorktreeId("main".to_string());
        let snapshot = GitSnapshotService::new(&repo_root)
            .await
            .expect("snapshot service");
        let tree = snapshot.capture_tree().await.expect("capture tree");
        let pin_ref = format!("refs/sce/mutation-cursor/main/{}", tree.0);

        let lock_a = WorktreeLock::acquire_async(&git_dir, LOCK_RELEASE_TIMEOUT)
            .await
            .expect("worktree lock A");
        let lease = lock_a.lease();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

        let caller = tokio::spawn({
            let repo_root = repo_root.clone();
            let worktree_id = worktree_id.clone();
            let tree = tree.clone();
            async move {
                let _lock_a = lock_a;
                let snapshot = GitSnapshotService::new(&repo_root)
                    .await
                    .expect("caller snapshot service");
                snapshot
                    .pin_tree_inner(lease, &worktree_id, &tree, move || {
                        entered_tx.send(()).expect("signal worker entered");
                        release_rx.recv().expect("release worker");
                    })
                    .await
            }
        });

        entered_rx.await.expect("ref mutation worker entered");
        caller.abort();
        assert!(caller
            .await
            .expect_err("caller must be cancelled")
            .is_cancelled());

        assert!(matches!(
            WorktreeLock::acquire_async(&git_dir, LOCK_CONTENDED_TIMEOUT).await,
            Err(WorktreeLockError::TimedOut { .. })
        ));
        assert!(snapshot
            .list_pins(&worktree_id)
            .await
            .expect("list pins before release")
            .is_empty());

        release_tx.send(()).expect("release worker");

        let lock_b = WorktreeLock::acquire_async(&git_dir, LOCK_RELEASE_TIMEOUT)
            .await
            .expect("worktree lock B after the ref mutation completes");
        let pins = snapshot.list_pins(&worktree_id).await.expect("list pins");
        assert_eq!(
            pins,
            vec![PinnedRef {
                ref_name: pin_ref.clone(),
                tree: tree.clone(),
            }]
        );
        assert!(!git_dir.join(format!("{pin_ref}.lock")).exists());
        assert!(!git_dir.join("packed-refs.lock").exists());

        git(
            &repo_root,
            &["update-ref", "refs/sce-test/after-cancelled-pin", "HEAD"],
        );
        snapshot
            .delete_pins(lock_b.lease(), &pins)
            .await
            .expect("delete pins after cancelled pin");
        assert!(snapshot
            .list_pins(&worktree_id)
            .await
            .expect("list pins after delete")
            .is_empty());
    }

    #[tokio::test]
    async fn cancelled_delete_keeps_worktree_lock_until_git_ref_deletion_completes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo_root = dir.path().join("repo");
        init_committed_repository(&repo_root);
        std::fs::write(repo_root.join("untracked.txt"), "untracked\n").expect("write file");

        let git_dir = resolve_git_dir(&repo_root).await.expect("git dir");
        let worktree_id = WorktreeId("main".to_string());
        let snapshot = GitSnapshotService::new(&repo_root)
            .await
            .expect("snapshot service");
        let tree = snapshot.capture_tree().await.expect("capture tree");
        let pin_ref = format!("refs/sce/mutation-cursor/main/{}", tree.0);

        let pin_lock = WorktreeLock::acquire_async(&git_dir, LOCK_RELEASE_TIMEOUT)
            .await
            .expect("pin lock");
        snapshot
            .pin_tree(pin_lock.lease(), &worktree_id, &tree)
            .await
            .expect("pin orphan tree");
        drop(pin_lock);
        let pins = snapshot.list_pins(&worktree_id).await.expect("list pins");
        assert_eq!(
            pins,
            vec![PinnedRef {
                ref_name: pin_ref.clone(),
                tree: tree.clone(),
            }]
        );

        let lock_a = WorktreeLock::acquire_async(&git_dir, LOCK_RELEASE_TIMEOUT)
            .await
            .expect("worktree lock A");
        let lease = lock_a.lease();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

        let caller = tokio::spawn({
            let repo_root = repo_root.clone();
            let pins = pins.clone();
            async move {
                let _lock_a = lock_a;
                let snapshot = GitSnapshotService::new(&repo_root)
                    .await
                    .expect("caller snapshot service");
                snapshot
                    .delete_pins_inner(
                        lease,
                        &pins,
                        || {},
                        move || {
                            entered_tx.send(()).expect("signal worker entered");
                            release_rx
                                .recv_timeout(LOCK_RELEASE_TIMEOUT)
                                .expect("release worker");
                        },
                    )
                    .await
            }
        });

        entered_rx.await.expect("ref deletion worker entered");
        assert_eq!(
            snapshot.list_pins(&worktree_id).await.expect("list pins"),
            pins,
            "the parked worker must not have deleted the pin yet"
        );

        caller.abort();
        assert!(caller
            .await
            .expect_err("caller must be cancelled")
            .is_cancelled());

        assert!(matches!(
            WorktreeLock::acquire_async(&git_dir, LOCK_CONTENDED_TIMEOUT).await,
            Err(WorktreeLockError::TimedOut { .. })
        ));
        assert_eq!(
            snapshot
                .list_pins(&worktree_id)
                .await
                .expect("list pins while the worker is still parked"),
            pins
        );

        release_tx.send(()).expect("release worker");

        let lock_b = WorktreeLock::acquire_async(&git_dir, LOCK_RELEASE_TIMEOUT)
            .await
            .expect("worktree lock B after the ref deletion completes");
        assert!(snapshot
            .list_pins(&worktree_id)
            .await
            .expect("list pins after the released worker finished")
            .is_empty());
        assert!(!git_dir.join(format!("{pin_ref}.lock")).exists());
        assert!(!git_dir.join("packed-refs.lock").exists());

        snapshot
            .pin_tree(lock_b.lease(), &worktree_id, &tree)
            .await
            .expect("a legitimate pin proceeds after the cancelled deletion");
        assert_eq!(
            snapshot
                .list_pins(&worktree_id)
                .await
                .expect("list pins after re-pin"),
            pins
        );
    }

    #[tokio::test]
    async fn async_identity_resolution_distinguishes_main_and_linked_worktrees() {
        let dir = tempfile::tempdir().expect("temp dir");
        let main_root = dir.path().join("main");
        let linked_root = dir.path().join("linked");
        init_committed_repository(&main_root);
        git(
            &main_root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "linked-branch",
                linked_root.to_str().expect("utf-8 path"),
            ],
        );

        let main_git_dir = resolve_git_dir(&main_root).await.expect("main git dir");
        let linked_git_dir = resolve_git_dir(&linked_root).await.expect("linked git dir");
        assert_ne!(main_git_dir, linked_git_dir);

        assert_eq!(
            resolve_worktree_id(&main_root).await.expect("main id"),
            WorktreeId("main".to_string())
        );
        let linked_id = resolve_worktree_id(&linked_root).await.expect("linked id");
        assert!(
            linked_id.0.starts_with("worktrees/"),
            "unexpected linked worktree id {linked_id:?}"
        );
    }
}
