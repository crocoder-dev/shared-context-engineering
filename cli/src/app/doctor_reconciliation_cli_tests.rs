use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::run_with_dependency_check_and_streams;
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::default_paths::agent_trace_db_path_for_repository_at;
use crate::services::repository_identity::resolve::resolve_repository_identity;

const ARGS_ENV: &str = "SCE_DOCTOR_RECONCILIATION_CLI_ARGS";
const OUT_ENV: &str = "SCE_DOCTOR_RECONCILIATION_CLI_OUT";
const CHILD_TEST: &str = "app::doctor_reconciliation_cli_tests::doctor_reconciliation_cli_child";
const STATE_FILE: &str = "ref-maintenance.json";
const LOCK_FILE: &str = "mutation-cursor.lock";
const RECONCILIATION_DETAIL_PREFIX: &str = "Snapshot ref reconciliation";

struct CliRun {
    stdout: String,
    stderr: String,
}

impl CliRun {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|error| panic!("doctor stdout is not JSON ({error}):\n{}", self.stdout))
    }
}

struct Sandbox {
    dir: tempfile::TempDir,
    repo: PathBuf,
    repository_id: String,
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

impl Sandbox {
    fn new(remote: &str) -> Self {
        let dir = tempfile::tempdir().expect("sandbox");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@example.invalid"]);
        git(&repo, &["config", "user.name", "T"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["remote", "add", "origin", remote]);
        std::fs::write(repo.join("tracked.txt"), "tracked\n").expect("tracked file");
        git(&repo, &["add", "tracked.txt"]);
        git(&repo, &["commit", "-q", "-m", "initial"]);
        let repository_id = resolve_repository_identity(&repo, None, "origin")
            .expect("identity")
            .identity
            .repository_id;
        Self {
            dir,
            repo,
            repository_id,
        }
    }

    fn state_root(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    fn db_path(&self) -> PathBuf {
        agent_trace_db_path_for_repository_at(&self.state_root(), &self.repository_id)
            .expect("db path")
    }

    fn sce_dir(&self) -> PathBuf {
        self.repo.join(".git").join("sce")
    }

    fn state_file(&self) -> PathBuf {
        self.sce_dir().join(STATE_FILE)
    }

    async fn create_db(&self, repository_id: &str) {
        let db = RepositoryAgentTraceDb::new_at(self.db_path())
            .await
            .expect("create db");
        db.verify_or_initialize_repository_metadata(repository_id)
            .await
            .expect("initialize metadata");
    }

    fn orphan_pin(&self) -> String {
        let tree = git(&self.repo, &["rev-parse", "HEAD^{tree}"]);
        git(
            &self.repo,
            &[
                "update-ref",
                &format!("refs/sce/mutation-cursor/main/{tree}"),
                &tree,
            ],
        );
        tree
    }

    fn refs(&self) -> String {
        git(
            &self.repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/sce/",
            ],
        )
    }

    fn state_bytes(&self) -> Option<Vec<u8>> {
        std::fs::read(self.state_file()).ok()
    }

    fn state_json(&self) -> Value {
        serde_json::from_slice(&self.state_bytes().expect("maintenance state was written"))
            .expect("state JSON")
    }

    fn run(&self, args: &[&str]) -> CliRun {
        let out = self.dir.path().join(format!("out-{}", args.join("-")));
        std::fs::create_dir_all(&out).expect("out dir");
        let output = Command::new(std::env::current_exe().expect("current exe"))
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .current_dir(&self.repo)
            .env(ARGS_ENV, args.join(" "))
            .env(OUT_ENV, &out)
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_STATE_HOME", self.state_root())
            .env("XDG_CACHE_HOME", self.dir.path().join("cache"))
            .env("NO_COLOR", "1")
            .env_remove("SCE_CONFIG_FILE")
            .output()
            .expect("child runs");
        assert!(
            output.status.success(),
            "child harness failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        CliRun {
            stdout: std::fs::read_to_string(out.join("stdout")).expect("stdout"),
            stderr: std::fs::read_to_string(out.join("stderr")).expect("stderr"),
        }
    }
}

fn non_reconciliation_fix_results(report: &Value, sandbox_root: &Path) -> Vec<Value> {
    let root = sandbox_root.to_string_lossy().into_owned();
    report["fix_results"]
        .as_array()
        .expect("fix_results array")
        .iter()
        .filter(|entry| {
            !entry["detail"]
                .as_str()
                .expect("detail")
                .starts_with(RECONCILIATION_DETAIL_PREFIX)
        })
        .map(|entry| {
            let mut entry = entry.clone();
            let detail = entry["detail"]
                .as_str()
                .expect("detail")
                .replace(&root, "<sandbox>");
            entry["detail"] = Value::String(detail);
            entry
        })
        .collect()
}

fn reconciliation_fix_result(report: &Value) -> Value {
    let matches: Vec<Value> = report["fix_results"]
        .as_array()
        .expect("fix_results array")
        .iter()
        .filter(|entry| {
            entry["detail"]
                .as_str()
                .expect("detail")
                .starts_with(RECONCILIATION_DETAIL_PREFIX)
        })
        .cloned()
        .collect();
    assert_eq!(matches.len(), 1, "reconciliation reports exactly once");
    matches.into_iter().next().expect("one entry")
}

#[tokio::test(flavor = "multi_thread")]
async fn doctor_reconciliation_cli_child() {
    let (Some(args), Some(out)) = (
        std::env::var_os(ARGS_ENV),
        std::env::var_os(OUT_ENV).map(PathBuf::from),
    ) else {
        return;
    };
    let mut command_line = vec!["sce".to_string()];
    command_line.extend(
        args.to_string_lossy()
            .split(' ')
            .filter(|part| !part.is_empty())
            .map(str::to_string),
    );
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let _ = Box::pin(run_with_dependency_check_and_streams(
        command_line,
        || Ok(()),
        &mut stdout,
        &mut stderr,
    ))
    .await;
    std::fs::write(out.join("stdout"), stdout).expect("stdout");
    std::fs::write(out.join("stderr"), stderr).expect("stderr");
}

#[tokio::test(flavor = "multi_thread")]
async fn doctor_inspection_is_read_only_and_fix_reconciles_the_orphan_pin_once() {
    let sandbox = Sandbox::new("https://example.invalid/org/repo-a.git");
    sandbox.create_db(&sandbox.repository_id).await;
    let tree = sandbox.orphan_pin();
    let refs_before = sandbox.refs();
    assert!(refs_before.contains(&tree));

    let inspection = sandbox.run(&["doctor", "--format", "json"]);
    let report = inspection.json();
    assert!(
        report["ref_reconciliation_fix"].is_null(),
        "plain doctor never reconciles: {}",
        inspection.stderr
    );
    assert_eq!(sandbox.refs(), refs_before);
    assert_eq!(sandbox.state_bytes(), None);

    let fix = sandbox.run(&["doctor", "--fix", "--format", "json"]);
    let report = fix.json();
    let reconciliation = &report["ref_reconciliation_fix"];
    assert_eq!(reconciliation["outcome"], "completed", "{}", fix.stdout);
    assert_eq!(reconciliation["counts"]["deleted"], 1);
    assert_eq!(reconciliation["counts"]["retained"], 0);
    assert_eq!(reconciliation["counts"]["local_required"], 0);
    assert!(reconciliation["failure"].is_null());
    assert!(reconciliation["state_warning"].is_null());
    assert_eq!(sandbox.refs(), "");

    let entry = reconciliation_fix_result(&report);
    assert_eq!(entry["outcome"], "fixed");
    assert_eq!(entry["category"], "mutation_scope_health");
    assert_eq!(
        entry["detail"],
        "Snapshot ref reconciliation completed: deleted 1, retained 0, locally required 0."
    );

    let state = sandbox.state_json();
    assert!(state["last_success"].is_number());
    assert_eq!(state["last_report"]["deleted"], 1);
    assert!(state.get("consecutive_failures").is_none_or(|v| v == 0));

    let again = sandbox.run(&["doctor", "--fix", "--format", "json"]).json();
    assert_eq!(again["ref_reconciliation_fix"]["counts"]["deleted"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn doctor_fix_reports_missing_database_without_creating_it_or_deleting_refs() {
    let sandbox = Sandbox::new("https://example.invalid/org/repo-a.git");
    sandbox.orphan_pin();
    let refs_before = sandbox.refs();
    let db_path = sandbox.db_path();

    let fix = sandbox.run(&["doctor", "--fix", "--format", "json"]);
    let report = fix.json();
    let reconciliation = &report["ref_reconciliation_fix"];
    assert_eq!(reconciliation["outcome"], "failed", "{}", fix.stdout);
    assert_eq!(reconciliation["failure"]["kind"], "agent_trace_db_missing");
    assert!(reconciliation["counts"].is_null());
    assert_eq!(sandbox.refs(), refs_before);
    assert!(!db_path.exists());
    let bootstrap: Vec<String> = non_reconciliation_fix_results(&report, sandbox.dir.path())
        .iter()
        .map(|entry| entry["detail"].as_str().expect("detail").to_string())
        .filter(|detail| detail.starts_with("Agent trace DB parent directory bootstrapped"))
        .collect();
    assert_eq!(
        bootstrap.len(),
        1,
        "the parent directory comes from the separate global-state repair, not from reconciliation"
    );
    for suffix in ["-wal", "-shm", "-tshm"] {
        let mut sidecar = db_path.clone().into_os_string();
        sidecar.push(suffix);
        assert!(!PathBuf::from(sidecar).exists());
    }

    let entry = reconciliation_fix_result(&report);
    assert_eq!(entry["outcome"], "failed");
    assert!(entry["detail"]
        .as_str()
        .expect("detail")
        .starts_with("Snapshot ref reconciliation failed (agent_trace_db_missing):"));

    let state = sandbox.state_json();
    assert_eq!(state["consecutive_failures"], 1);
    assert!(state.get("last_success").is_none());
    assert!(report["fix_results"].is_array());
}

#[tokio::test(flavor = "multi_thread")]
async fn doctor_fix_reports_repository_mismatch_without_touching_the_database_or_refs() {
    let sandbox = Sandbox::new("https://example.invalid/org/repo-a.git");
    let foreign = "some-other-repository";
    sandbox.create_db(foreign).await;
    sandbox.orphan_pin();
    let refs_before = sandbox.refs();

    let fix = sandbox.run(&["doctor", "--fix", "--format", "json"]);
    let report = fix.json();
    let reconciliation = &report["ref_reconciliation_fix"];
    assert_eq!(reconciliation["outcome"], "failed", "{}", fix.stdout);
    assert_eq!(
        reconciliation["failure"]["kind"],
        "agent_trace_db_repository_mismatch"
    );
    assert!(reconciliation["failure"]["message"]
        .as_str()
        .expect("message")
        .contains(foreign));
    assert_eq!(sandbox.refs(), refs_before);
    assert_eq!(sandbox.state_json()["consecutive_failures"], 1);

    let (_, metadata) =
        RepositoryAgentTraceDb::open_verified_existing_at(sandbox.db_path(), foreign)
            .await
            .expect("the foreign repository identity is untouched");
    assert_eq!(metadata.repository_id, foreign);
}

#[tokio::test(flavor = "multi_thread")]
async fn doctor_fix_reports_busy_when_the_worktree_lock_is_held_and_keeps_other_repairs() {
    let reference = Sandbox::new("https://example.invalid/org/repo-a.git");
    reference.create_db(&reference.repository_id).await;
    reference.orphan_pin();
    let reference_report = reference
        .run(&["doctor", "--fix", "--format", "json"])
        .json();

    let sandbox = Sandbox::new("https://example.invalid/org/repo-a.git");
    sandbox.create_db(&sandbox.repository_id).await;
    sandbox.orphan_pin();
    let refs_before = sandbox.refs();
    std::fs::create_dir_all(sandbox.sce_dir()).expect("sce dir");
    let held = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(sandbox.sce_dir().join(LOCK_FILE))
        .expect("lock file");
    held.try_lock().expect("hold the worktree lock externally");

    let fix = sandbox.run(&["doctor", "--fix", "--format", "json"]);
    let report = fix.json();
    drop(held);

    let reconciliation = &report["ref_reconciliation_fix"];
    assert_eq!(reconciliation["outcome"], "busy", "{}", fix.stdout);
    assert!(reconciliation["counts"].is_null());
    assert!(reconciliation["failure"].is_null());
    assert_eq!(sandbox.refs(), refs_before);
    assert_eq!(sandbox.state_bytes(), None);

    let entry = reconciliation_fix_result(&report);
    assert_eq!(entry["outcome"], "skipped");
    assert!(entry["detail"]
        .as_str()
        .expect("detail")
        .starts_with("Snapshot ref reconciliation skipped:"));
    assert_eq!(
        non_reconciliation_fix_results(&report, sandbox.dir.path()),
        non_reconciliation_fix_results(&reference_report, reference.dir.path()),
        "independent doctor repairs are preserved"
    );
}
