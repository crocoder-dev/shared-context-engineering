use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use uuid::Uuid;

use crate::services::mutation_trace::types::{TreeId, WorktreeId};

const SCE_RUNTIME_DIR: &str = "sce";
const TMP_INDEX_DIR: &str = "tmp";
const REF_NAMESPACE: &str = "refs/sce/mutation-cursor";

pub struct GitSnapshotService {
    git_dir: PathBuf,
    repository_root: PathBuf,
}

impl GitSnapshotService {
    pub fn new(repository_root: &Path) -> Result<GitSnapshotService> {
        let git_dir = resolve_git_dir(repository_root)?;
        Ok(GitSnapshotService {
            git_dir,
            repository_root: repository_root.to_path_buf(),
        })
    }

    pub fn capture_tree(&self) -> Result<TreeId> {
        let tmp_dir = self.git_dir.join(SCE_RUNTIME_DIR).join(TMP_INDEX_DIR);
        std::fs::create_dir_all(&tmp_dir).with_context(|| {
            format!(
                "Failed to create temporary index directory '{}'",
                tmp_dir.display()
            )
        })?;
        let index_guard = TempIndexGuard::reserve(&tmp_dir);

        if self.head_exists()? {
            self.run_git(&["read-tree", "HEAD"], Some(&index_guard.path))?;
        } else {
            self.run_git(&["read-tree", "--empty"], Some(&index_guard.path))?;
        }

        self.run_git(&["add", "-A", "--", "."], Some(&index_guard.path))?;

        let tree_sha = self.run_git(&["write-tree"], Some(&index_guard.path))?;
        Ok(TreeId(tree_sha.trim().to_string()))
    }

    pub fn pin_tree(&self, worktree_id: &WorktreeId, tree: &TreeId) -> Result<()> {
        let ref_name = pin_ref_name(worktree_id, tree);
        self.run_git(&["update-ref", &ref_name, &tree.0], None)?;
        Ok(())
    }

    pub fn diff_trees(&self, before: &TreeId, after: &TreeId) -> Result<String> {
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
    }

    fn head_exists(&self) -> Result<bool> {
        let output = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", "HEAD"])
            .current_dir(&self.repository_root)
            .env("GIT_DIR", &self.git_dir)
            .output()
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

    fn run_git(&self, args: &[&str], index_file: Option<&Path>) -> Result<String> {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(&self.repository_root)
            .env("GIT_DIR", &self.git_dir);
        if let Some(index_file) = index_file {
            command.env("GIT_INDEX_FILE", index_file);
        }

        let output = command.output().with_context(|| {
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

fn pin_ref_name(worktree_id: &WorktreeId, tree: &TreeId) -> String {
    format!("{REF_NAMESPACE}/{}/{}", worktree_id.0, tree.0)
}

fn resolve_git_dir(repository_root: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .current_dir(repository_root)
        .output()
        .with_context(|| {
            format!(
                "Failed to run git rev-parse --absolute-git-dir in '{}'",
                repository_root.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(
            "git rev-parse --absolute-git-dir failed in '{}': {}",
            repository_root.display(),
            stderr
        ));
    }

    let git_dir = PathBuf::from(
        String::from_utf8(output.stdout)
            .with_context(|| "git rev-parse --absolute-git-dir emitted invalid UTF-8")?
            .trim(),
    );

    debug_assert!(
        git_dir.is_absolute(),
        "git rev-parse --absolute-git-dir should always return an absolute path, got '{}'",
        git_dir.display()
    );

    Ok(git_dir)
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
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_REPO_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_test_repo(label: &str) -> PathBuf {
        let id = NEXT_TEST_REPO_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "sce-git-snapshot-{label}-{}-{id}",
            std::process::id()
        ))
    }

    fn init_repo(repo_root: &Path) {
        std::fs::create_dir_all(repo_root).expect("repo root should be created");
        run(repo_root, &["init", "--quiet"]);
        run(repo_root, &["config", "user.email", "test@example.com"]);
        run(repo_root, &["config", "user.name", "Test"]);
    }

    fn run(repo_root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("git output should be valid UTF-8")
    }

    fn commit_all(repo_root: &Path, message: &str) {
        run(repo_root, &["add", "-A"]);
        run(repo_root, &["commit", "--quiet", "-m", message]);
    }

    fn remove_test_repo(repo_root: &Path) {
        let _ = std::fs::remove_dir_all(repo_root);
    }

    fn worktree_id() -> WorktreeId {
        WorktreeId("test-worktree".to_string())
    }

    #[test]
    fn capture_preserves_real_index_and_working_tree_state() {
        let repo_root = unique_test_repo("preserves-index");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("committed.txt"), b"original\n")
            .expect("committed file should be writable");
        commit_all(&repo_root, "initial commit");

        std::fs::write(repo_root.join("staged.txt"), b"staged\n")
            .expect("staged file should be writable");
        run(&repo_root, &["add", "staged.txt"]);
        std::fs::write(repo_root.join("committed.txt"), b"modified\n")
            .expect("unstaged modification should be writable");
        std::fs::write(repo_root.join("untracked.txt"), b"untracked\n")
            .expect("untracked file should be writable");

        let status_before = run(&repo_root, &["status", "--porcelain"]);
        let diff_before = run(&repo_root, &["diff"]);
        let diff_cached_before = run(&repo_root, &["diff", "--cached"]);

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let tree = service
            .capture_tree()
            .expect("capture should succeed with mixed staged/unstaged/untracked state");
        assert!(!tree.0.is_empty());

        let status_after = run(&repo_root, &["status", "--porcelain"]);
        let diff_after = run(&repo_root, &["diff"]);
        let diff_cached_after = run(&repo_root, &["diff", "--cached"]);

        assert_eq!(status_before, status_after);
        assert_eq!(diff_before, diff_after);
        assert_eq!(diff_cached_before, diff_cached_after);

        let ls_tree = run(&repo_root, &["ls-tree", "-r", "--name-only", &tree.0]);
        assert!(ls_tree.contains("staged.txt"));
        assert!(ls_tree.contains("untracked.txt"));
        let committed_blob = run(&repo_root, &["show", &format!("{}:committed.txt", tree.0)]);
        assert_eq!(committed_blob, "modified\n");

        remove_test_repo(&repo_root);
    }

    #[test]
    fn capture_excludes_ignored_files() {
        let repo_root = unique_test_repo("ignored-files");
        init_repo(&repo_root);
        std::fs::write(repo_root.join(".gitignore"), b"ignored.txt\n")
            .expect(".gitignore should be writable");
        commit_all(&repo_root, "add gitignore");
        std::fs::write(repo_root.join("ignored.txt"), b"should not appear\n")
            .expect("ignored file should be writable");

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let tree = service
            .capture_tree()
            .expect("capture should succeed with an ignored file present");

        let ls_tree = run(&repo_root, &["ls-tree", "-r", "--name-only", &tree.0]);
        assert!(!ls_tree.contains("ignored.txt"));

        remove_test_repo(&repo_root);
    }

    #[test]
    fn capture_reflects_deletion_of_a_committed_file() {
        let repo_root = unique_test_repo("deletion");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("to-delete.txt"), b"will be removed\n")
            .expect("file should be writable");
        commit_all(&repo_root, "add file to delete");
        std::fs::remove_file(repo_root.join("to-delete.txt")).expect("file should be removable");

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let tree = service
            .capture_tree()
            .expect("capture should succeed with a deletion pending");

        let ls_tree = run(&repo_root, &["ls-tree", "-r", "--name-only", &tree.0]);
        assert!(!ls_tree.contains("to-delete.txt"));

        remove_test_repo(&repo_root);
    }

    #[test]
    fn capture_on_unborn_head_with_a_file_produces_a_valid_tree() {
        let repo_root = unique_test_repo("unborn-head-with-file");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("untracked.txt"), b"before any commit\n")
            .expect("untracked file should be writable");

        let head_probe = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", "HEAD"])
            .current_dir(&repo_root)
            .output()
            .expect("git rev-parse should spawn");
        assert!(
            !head_probe.status.success(),
            "HEAD should be unborn in a freshly initialized repository"
        );

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let tree = service
            .capture_tree()
            .expect("capture should succeed on an unborn HEAD with a file present");

        let ls_tree = run(&repo_root, &["ls-tree", "-r", "--name-only", &tree.0]);
        assert!(ls_tree.contains("untracked.txt"));

        remove_test_repo(&repo_root);
    }

    #[test]
    fn an_unexpected_head_probe_failure_propagates_instead_of_using_read_tree_empty() {
        let repo_root = unique_test_repo("head-probe-failure");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("file.txt"), b"content\n").expect("file should be writable");
        commit_all(&repo_root, "initial commit");

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");

        std::fs::remove_file(service.git_dir.join("HEAD"))
            .expect("HEAD file should be removable to simulate an unexpected failure");

        let result = service.capture_tree();
        assert!(
            result.is_err(),
            "an unexpected HEAD-probe failure must surface as an error, not a false empty-baseline capture"
        );

        remove_test_repo(&repo_root);
    }

    fn relative_path_from(base: &Path, target: &Path) -> PathBuf {
        let base_components: Vec<_> = base.components().collect();
        let target_components: Vec<_> = target.components().collect();

        let common_len = base_components
            .iter()
            .zip(target_components.iter())
            .take_while(|(a, b)| a == b)
            .count();

        let mut relative = PathBuf::new();
        for _ in common_len..base_components.len() {
            relative.push("..");
        }
        for component in &target_components[common_len..] {
            relative.push(component);
        }
        relative
    }

    #[test]
    fn resolves_an_absolute_git_dir_from_a_relative_repository_root() {
        let repo_root_abs = unique_test_repo("relative-root");
        init_repo(&repo_root_abs);
        std::fs::write(repo_root_abs.join("file.txt"), b"relative content\n")
            .expect("file should be writable");

        let cwd = std::env::current_dir().expect("current dir should be readable");
        let canonical_repo_root = repo_root_abs
            .canonicalize()
            .expect("temp repo root should canonicalize");
        let relative_repo_root = relative_path_from(&cwd, &canonical_repo_root);
        assert!(
            relative_repo_root.is_relative(),
            "test setup should produce a genuinely relative repository root"
        );

        let service = GitSnapshotService::new(&relative_repo_root)
            .expect("service should resolve git-dir from a relative repository root");
        assert!(
            service.git_dir.is_absolute(),
            "GitSnapshotService must always store an absolute git_dir, even from a relative repository_root"
        );

        let tree = service
            .capture_tree()
            .expect("capture should succeed with a relative repository root");
        let ls_tree = run(&repo_root_abs, &["ls-tree", "-r", "--name-only", &tree.0]);
        assert!(ls_tree.contains("file.txt"));

        service
            .pin_tree(&worktree_id(), &tree)
            .expect("pin should succeed with a relative repository root");
        let diff = service
            .diff_trees(&tree, &tree)
            .expect("diff should succeed with a relative repository root");
        assert!(diff.is_empty());

        remove_test_repo(&repo_root_abs);
    }

    #[test]
    fn capture_on_unborn_head_with_no_files_produces_an_empty_tree() {
        let repo_root = unique_test_repo("unborn-head-no-files");
        init_repo(&repo_root);

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let tree = service
            .capture_tree()
            .expect("capture should succeed on an unborn HEAD with no files at all");

        let ls_tree = run(&repo_root, &["ls-tree", "-r", "--name-only", &tree.0]);
        assert_eq!(ls_tree.trim(), "");

        remove_test_repo(&repo_root);
    }

    #[test]
    fn snapshot_survives_a_fresh_process_and_temp_index_deletion() {
        let repo_root = unique_test_repo("survives-process-exit");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("file.txt"), b"content\n").expect("file should be writable");

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let tree = service.capture_tree().expect("capture should succeed");

        let tmp_dir = service.git_dir.join(SCE_RUNTIME_DIR).join(TMP_INDEX_DIR);
        assert!(
            std::fs::read_dir(&tmp_dir).map_or(true, |mut entries| entries.next().is_none()),
            "temp index directory should not retain the completed capture's index file"
        );

        let resolved = run(&repo_root, &["cat-file", "-t", &tree.0]);
        assert_eq!(resolved.trim(), "tree");

        remove_test_repo(&repo_root);
    }

    #[test]
    fn pinned_snapshot_survives_git_gc_prune_now() {
        let repo_root = unique_test_repo("gc-prune-now");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("pinned.txt"), b"pinned content\n")
            .expect("file should be writable");

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let pinned_tree = service.capture_tree().expect("capture should succeed");
        service
            .pin_tree(&worktree_id(), &pinned_tree)
            .expect("pin should succeed");

        std::fs::write(
            repo_root.join("unreachable.txt"),
            b"distinct unreachable content\n",
        )
        .expect("file should be writable");
        let unpinned_tree = service.capture_tree().expect("capture should succeed");
        assert_ne!(pinned_tree.0, unpinned_tree.0);

        run(&repo_root, &["gc", "--prune=now"]);

        let pinned_resolved = run(&repo_root, &["cat-file", "-t", &pinned_tree.0]);
        assert_eq!(pinned_resolved.trim(), "tree");

        let unpinned_probe = Command::new("git")
            .args(["cat-file", "-t", &unpinned_tree.0])
            .current_dir(&repo_root)
            .output()
            .expect("git cat-file should spawn");
        assert!(
            !unpinned_probe.status.success(),
            "an unpinned, unreachable tree should be reclaimed by git gc --prune=now"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn pinned_snapshot_survives_git_prune_expire_now() {
        let repo_root = unique_test_repo("prune-expire-now");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("pinned.txt"), b"pinned content\n")
            .expect("file should be writable");

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let pinned_tree = service.capture_tree().expect("capture should succeed");
        service
            .pin_tree(&worktree_id(), &pinned_tree)
            .expect("pin should succeed");

        std::fs::write(
            repo_root.join("unreachable-2.txt"),
            b"another distinct unreachable content\n",
        )
        .expect("file should be writable");
        let unpinned_tree = service.capture_tree().expect("capture should succeed");
        assert_ne!(pinned_tree.0, unpinned_tree.0);

        run(&repo_root, &["prune", "--expire=now"]);

        let pinned_resolved = run(&repo_root, &["cat-file", "-t", &pinned_tree.0]);
        assert_eq!(pinned_resolved.trim(), "tree");

        let unpinned_probe = Command::new("git")
            .args(["cat-file", "-t", &unpinned_tree.0])
            .current_dir(&repo_root)
            .output()
            .expect("git cat-file should spawn");
        assert!(
            !unpinned_probe.status.success(),
            "an unpinned, unreachable tree should be reclaimed by git prune --expire=now"
        );

        remove_test_repo(&repo_root);
    }

    #[test]
    fn pin_tree_is_idempotent_for_the_same_worktree_and_tree() {
        let repo_root = unique_test_repo("pin-idempotent");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("file.txt"), b"content\n").expect("file should be writable");

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let tree = service.capture_tree().expect("capture should succeed");

        service
            .pin_tree(&worktree_id(), &tree)
            .expect("first pin should succeed");
        service
            .pin_tree(&worktree_id(), &tree)
            .expect("second, identical pin should be a harmless no-op");

        let ref_name = pin_ref_name(&worktree_id(), &tree);
        let resolved = run(&repo_root, &["rev-parse", &ref_name]);
        assert_eq!(resolved.trim(), tree.0);

        remove_test_repo(&repo_root);
    }

    #[test]
    fn diff_trees_returns_parseable_git_diff_output() {
        let repo_root = unique_test_repo("diff-trees");
        init_repo(&repo_root);
        std::fs::write(repo_root.join("file.txt"), b"before\n").expect("file should be writable");
        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let before = service.capture_tree().expect("capture should succeed");

        std::fs::write(repo_root.join("file.txt"), b"after\n").expect("file should be writable");
        let after = service.capture_tree().expect("capture should succeed");

        let diff = service
            .diff_trees(&before, &after)
            .expect("diff should succeed");
        assert!(diff.contains("file.txt"));
        assert!(diff.contains("-before"));
        assert!(diff.contains("+after"));

        remove_test_repo(&repo_root);
    }

    #[test]
    fn capture_and_pin_work_against_a_sha256_repository_when_supported() {
        let repo_root = unique_test_repo("sha256");
        std::fs::create_dir_all(&repo_root).expect("repo root should be created");
        let init = Command::new("git")
            .args(["init", "--quiet", "--object-format=sha256"])
            .current_dir(&repo_root)
            .output()
            .expect("git init should spawn");
        if !init.status.success() {
            remove_test_repo(&repo_root);
            return;
        }
        run(&repo_root, &["config", "user.email", "test@example.com"]);
        run(&repo_root, &["config", "user.name", "Test"]);
        std::fs::write(repo_root.join("file.txt"), b"content\n").expect("file should be writable");

        let service = GitSnapshotService::new(&repo_root).expect("service should resolve git-dir");
        let tree = service
            .capture_tree()
            .expect("capture should succeed against a SHA-256 repository");
        service
            .pin_tree(&worktree_id(), &tree)
            .expect("pin should succeed against a SHA-256 repository");

        let ls_tree = run(&repo_root, &["ls-tree", "-r", "--name-only", &tree.0]);
        assert!(ls_tree.contains("file.txt"));

        remove_test_repo(&repo_root);
    }
}
