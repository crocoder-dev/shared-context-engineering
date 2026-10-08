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
        let tmp_dir = self.git_dir.join(SCE_RUNTIME_DIR).join(TMP_INDEX_DIR);
        std::fs::create_dir_all(&tmp_dir).with_context(|| {
            format!(
                "Failed to create temporary index directory '{}'",
                tmp_dir.display()
            )
        })?;
        let index_guard = TempIndexGuard::reserve(&tmp_dir);

        if self.head_exists().await? {
            self.run_git(&["read-tree", "HEAD"], Some(&index_guard.path))
                .await?;
        } else {
            self.run_git(&["read-tree", "--empty"], Some(&index_guard.path))
                .await?;
        }

        self.run_git(&["add", "-A", "--", "."], Some(&index_guard.path))
            .await?;

        let tree_sha = self
            .run_git(&["write-tree"], Some(&index_guard.path))
            .await?;
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

    /// Delete exactly `pins` in one atomic, no-dereference
    /// `git update-ref --no-deref --stdin` transaction, each `delete`
    /// conditioned on the tree SHA recorded in the [`PinnedRef`].
    ///
    /// Two independent safety properties:
    ///
    /// - **Atomic** — `git update-ref --stdin` commits every command together
    ///   at end of input; if any command fails (including a failed old-value
    ///   check) the whole transaction aborts and no ref is changed.
    /// - **No dereference** — `--no-deref` makes every `delete` operate on the
    ///   exact ref name given, never on a ref reached by resolving a symbolic
    ///   ref. Combined with a fail-closed re-check (below), a
    ///   direct-ref → symbolic-ref race between inventory and deletion can
    ///   never cause this call to touch the symref's target (for example a ref
    ///   owned by another worktree).
    ///
    /// Before issuing the transaction, each supplied ref is re-inventoried: it
    /// must still exist, still be a direct ref to a tree, and still point at
    /// the inventoried SHA. If any has changed — deleted, retargeted, or turned
    /// into a symbolic ref — this returns `Err` and deletes nothing, preferring
    /// failure over acting on unexpected namespace state. An empty slice is a
    /// successful no-op.
    pub async fn delete_pins(&self, lease: WorktreeLockLease, pins: &[PinnedRef]) -> Result<()> {
        self.delete_pins_inner(lease, pins, || {}).await
    }

    /// Body of [`delete_pins`] with a deterministic test seam that fires
    /// **after** the fail-closed preflight re-inventory and **before** the
    /// `git update-ref --no-deref --stdin` transaction is spawned. Production
    /// calls it with a no-op hook; the inline atomicity test uses the hook to
    /// mutate a ref *after* it has passed preflight, so the transaction is
    /// actually issued and the per-`delete` expected-old-value check — not the
    /// preflight — is what aborts the batch. This is the only proof that the
    /// Git transaction itself is atomic; the preflight proves a different
    /// property (unexpected ref state before the transaction is even attempted).
    async fn delete_pins_inner(
        &self,
        lease: WorktreeLockLease,
        pins: &[PinnedRef],
        after_preflight: impl FnOnce(),
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

        self.run_ref_mutation(
            lease,
            vec![
                "update-ref".to_string(),
                "--no-deref".to_string(),
                "--stdin".to_string(),
            ],
            Some(stdin_payload.into_bytes()),
        )
        .await?;

        Ok(())
    }

    async fn run_ref_mutation(
        &self,
        lease: WorktreeLockLease,
        args: Vec<String>,
        stdin: Option<Vec<u8>>,
    ) -> Result<String> {
        self.run_ref_mutation_inner(lease, args, stdin, || {}).await
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
pub struct PinnedRef {
    pub ref_name: String,
    pub tree: TreeId,
}

/// Why a worktree's pin inventory could not be produced.
#[derive(Debug)]
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

    let canonical_root = std::fs::canonicalize(&root_path).with_context(|| {
        format!(
            "failed to canonicalize the worktree root '{}'",
            root_path.display()
        )
    })?;
    if !canonical_root.is_dir() {
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
    path: PathBuf,
}

impl TempIndexGuard {
    fn reserve(tmp_dir: &Path) -> TempIndexGuard {
        let path = tmp_dir.join(format!("index-{}", Uuid::new_v4()));
        TempIndexGuard { path }
    }
}

impl Drop for TempIndexGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;
    use std::time::Duration;

    use super::super::worktree_lock::{WorktreeLock, WorktreeLockError};
    use super::{resolve_git_dir, resolve_worktree_id, GitSnapshotService, PinnedRef};
    use crate::services::mutation_trace::types::WorktreeId;

    const LOCK_CONTENDED_TIMEOUT: Duration = Duration::from_millis(300);
    const LOCK_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);

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
