use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::services::agent_trace::{agent_trace_session_url, validate_agent_trace_value};
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
use crate::services::agent_trace_db::{DiffTraceInsert, PAYLOAD_TYPE_PATCH};
use crate::services::mutation_trace::attribution::MutationAttributionResult;
use crate::services::patch::{ParsedPatch, TouchedLineKind};

use super::{
    capture_post_commit_patch_from_git, run_post_commit_agent_trace_flow_with,
    run_post_commit_intersection_flow_with, PostCommitIntersectionFlowResult,
};

const REMOTE_URL: &str = "https://example.invalid/repo.git";
const BASE_TIME_S: i64 = 1_793_960_000;
const COMMIT_TIME_S: i64 = BASE_TIME_S + 1_000;
const SESSION: &str = "cc_session-unicode";
const MODEL: &str = "model-unicode";
const TOOL: &str = "claude";

const UNICODE_PATH: &str = "src/café.rs";
const PLAIN_PATH: &str = "src/plain.rs";
const EXECUTABLE_PATH: &str = "bin/run.sh";
const SYMLINK_PATH: &str = "src/link.rs";
const SEPARATOR_PATH: &str = "src/x b/y.rs";
const TRAILING_PATH: &str = "src/trailing ";
const ESCAPED_PATHS: [&str; 5] = [
    "src/file name.rs",
    "src/file\tname.rs",
    "src/file\nname.rs",
    "src/file\"name.rs",
    "src/file\\name.rs",
];
const UNICODE_LINES: [&str; 3] = ["fn café() {", "    \"ünïcødé ✓\"", "}"];

const fn ms(seconds: i64) -> i64 {
    seconds * 1_000
}

fn git(root: &Path, args: &[&str], time_s: Option<i64>) -> String {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(time_s) = time_s {
        let date = format!("@{time_s} +0000");
        command
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date);
    }
    let output = command.output().expect("git command should run");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .trim()
        .to_owned()
}

fn committed_blob(root: &Path, path: &str) -> Vec<u8> {
    let output = Command::new("git")
        .args(["cat-file", "blob", &format!("HEAD:{path}")])
        .current_dir(root)
        .output()
        .expect("git cat-file should run");
    assert!(output.status.success(), "no committed blob for {path:?}");
    output.stdout
}

fn independent_hash(bytes: &[u8]) -> String {
    let hash = murmur3::murmur3_32(&mut Cursor::new(bytes), 0).expect("murmur3");
    format!("murmur3:{hash:08x}")
}

fn source_lines(path: &str) -> Vec<String> {
    if path == UNICODE_PATH {
        return UNICODE_LINES
            .iter()
            .map(|line| (*line).to_owned())
            .collect();
    }
    vec![
        format!("const PATH: &str = {path:?};"),
        String::from("pub fn f() {}"),
    ]
}

fn source_text(path: &str) -> String {
    let mut text = source_lines(path).join("\n");
    text.push('\n');
    text
}

fn line_addressable_paths() -> Vec<&'static str> {
    let mut paths = vec![
        UNICODE_PATH,
        PLAIN_PATH,
        EXECUTABLE_PATH,
        SEPARATOR_PATH,
        TRAILING_PATH,
    ];
    paths.extend(ESCAPED_PATHS);
    paths.sort_unstable();
    paths
}

struct QuotedPathRepo {
    dir: tempfile::TempDir,
}

impl QuotedPathRepo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        git(root, &["init", "-q", "-b", "main"], None);
        for (key, value) in [
            ("user.email", "paths@example.invalid"),
            ("user.name", "Path Identity Test"),
            ("commit.gpgsign", "false"),
            ("core.quotePath", "true"),
            ("core.symlinks", "true"),
        ] {
            git(root, &["config", key, value], None);
        }
        std::fs::write(root.join("README"), "base\n").expect("write base");
        git(root, &["add", "-A"], None);
        git(root, &["commit", "-q", "-m", "base"], Some(BASE_TIME_S));

        for path in line_addressable_paths() {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
            std::fs::write(file, source_text(path)).expect("write source");
        }
        std::os::unix::fs::symlink("plain.rs", root.join(SYMLINK_PATH)).expect("symlink");
        git(root, &["add", "-A"], None);
        git(
            root,
            &["update-index", "--chmod=+x", "--", EXECUTABLE_PATH],
            None,
        );
        git(root, &["commit", "-q", "-m", "paths"], Some(COMMIT_TIME_S));
        QuotedPathRepo { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }
}

fn unicode_direct_patch() -> String {
    let quoted = r#""a/src/caf\303\251.rs""#;
    let quoted_new = r#""b/src/caf\303\251.rs""#;
    let mut text = format!(
        "diff --git {quoted} {quoted_new}\nnew file mode 100644\n--- /dev/null\n+++ {quoted_new}\n@@ -0,0 +1,{} @@\n",
        UNICODE_LINES.len()
    );
    for line in UNICODE_LINES {
        text.push('+');
        text.push_str(line);
        text.push('\n');
    }
    text
}

async fn open_db() -> (tempfile::TempDir, RepositoryAgentTraceDb) {
    let db_dir = tempfile::tempdir().expect("db dir");
    let db = RepositoryAgentTraceDb::new_at(db_dir.path().join("agent-trace.db"))
        .await
        .expect("open agent trace db");
    (db_dir, db)
}

async fn insert_unicode_direct_evidence(db: &RepositoryAgentTraceDb) {
    db.insert_diff_trace(DiffTraceInsert {
        time_ms: ms(COMMIT_TIME_S) - 500,
        session_id: SESSION,
        patch: &unicode_direct_patch(),
        model_id: Some(MODEL),
        tool_name: TOOL,
        tool_version: None,
        payload_type: PAYLOAD_TYPE_PATCH,
    })
    .await
    .expect("insert diff trace");
}

async fn intersection_flow(
    repo: &QuotedPathRepo,
    db: &RepositoryAgentTraceDb,
) -> PostCommitIntersectionFlowResult {
    run_post_commit_intersection_flow_with(
        repo.root(),
        capture_post_commit_patch_from_git,
        || Ok(ms(COMMIT_TIME_S) + 500),
        async |cutoff_ms, end_ms| db.recent_diff_trace_patches(cutoff_ms, end_ms).await,
        async |insert| {
            db.insert_post_commit_patch_intersection(insert)
                .await
                .map(|_| ())
        },
    )
    .await
    .expect("intersection flow")
}

fn no_mutation() -> MutationAttributionResult {
    let empty = ParsedPatch { files: Vec::new() };
    MutationAttributionResult {
        mutation_ai_patch: empty.clone(),
        resolved_non_ai_patch: empty.clone(),
        unresolved_patch: empty,
    }
}

async fn persist(
    db: &mut RepositoryAgentTraceDb,
    flow: &PostCommitIntersectionFlowResult,
) -> Result<()> {
    run_post_commit_agent_trace_flow_with(
        flow,
        None,
        REMOTE_URL,
        &no_mutation(),
        None,
        |value| validate_agent_trace_value(value).map_err(|error| anyhow!(error.to_string())),
        async |insert| db.insert_agent_trace(insert).await.map(|_| ()),
    )
    .await
    .map(|_| ())
}

async fn persisted_traces(db: &RepositoryAgentTraceDb) -> Vec<String> {
    db.query_map("SELECT trace_json FROM agent_traces", (), |row| {
        Ok(row.get(0)?)
    })
    .await
    .expect("read agent traces")
}

fn added_lines(patch: &ParsedPatch) -> u64 {
    patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind == TouchedLineKind::Added)
        .count() as u64
}

#[cfg(unix)]
#[tokio::test]
async fn quoted_unicode_and_escaped_paths_persist_exact_identities_and_verified_hashes() {
    let repo = QuotedPathRepo::new();
    let (_db_dir, mut db) = open_db().await;
    insert_unicode_direct_evidence(&db).await;

    let flow = intersection_flow(&repo, &db).await;
    let mut parsed_paths: Vec<&str> = flow
        .post_commit_data
        .parsed_patch
        .files
        .iter()
        .map(|file| file.new_path.as_str())
        .collect();
    parsed_paths.sort_unstable();
    let mut expected_parsed = line_addressable_paths();
    expected_parsed.push(SYMLINK_PATH);
    expected_parsed.sort_unstable();
    assert_eq!(parsed_paths, expected_parsed);

    persist(&mut db, &flow).await.expect("agent trace flow");
    let traces = persisted_traces(&db).await;
    assert_eq!(traces.len(), 1);
    let value: Value = serde_json::from_str(&traces[0]).expect("trace JSON");
    validate_agent_trace_value(&value).expect("persisted trace validates");

    let files: BTreeMap<String, &Value> = value["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| (file["path"].as_str().expect("path").to_owned(), file))
        .collect();
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        line_addressable_paths()
    );

    for (path, file) in &files {
        let blob = committed_blob(repo.root(), path);
        assert_eq!(blob, source_text(path).into_bytes(), "{path:?}");
        let ranges: Vec<&Value> = file["conversations"]
            .as_array()
            .expect("conversations")
            .iter()
            .flat_map(|conversation| conversation["ranges"].as_array().expect("ranges"))
            .collect();
        assert_eq!(ranges.len(), 1, "{path:?}");
        assert_eq!(ranges[0]["start_line"], 1);
        assert_eq!(ranges[0]["end_line"], source_lines(path).len());
        assert_eq!(
            ranges[0]["content_hash"].as_str(),
            Some(independent_hash(&blob).as_str()),
            "{path:?}"
        );
    }

    let unicode = &files[UNICODE_PATH]["conversations"][0];
    assert_eq!(unicode["contributor"]["type"], "ai");
    assert_eq!(unicode["contributor"]["model_id"], MODEL);
    assert_eq!(
        unicode["related"][0]["url"].as_str(),
        Some(agent_trace_session_url(SESSION).as_str())
    );
    for (path, file) in &files {
        if path != UNICODE_PATH {
            assert_eq!(file["conversations"][0]["contributor"]["type"], "unknown");
        }
    }

    let total_added = added_lines(&flow.post_commit_data.parsed_patch);
    let expected_added: u64 = line_addressable_paths()
        .iter()
        .map(|path| source_lines(path).len() as u64)
        .sum::<u64>()
        + 1;
    assert_eq!(total_added, expected_added);
    let line_changes = &value["metadata"]["sce"]["line_changes"];
    assert_eq!(line_changes["ai"]["added"], UNICODE_LINES.len());
    assert_eq!(
        line_changes["unknown"]["added"],
        expected_added - UNICODE_LINES.len() as u64
    );
    assert_eq!(line_changes["non_exclusive"]["added"], 0);
    for bucket in ["ai", "non_exclusive", "unknown"] {
        assert_eq!(line_changes[bucket]["removed"], 0);
    }
}

async fn assert_flow_persists_nothing(
    repo: &QuotedPathRepo,
    edit: impl FnOnce(&mut PostCommitIntersectionFlowResult),
    expected: &str,
) {
    let (_db_dir, mut db) = open_db().await;
    let mut flow = intersection_flow(repo, &db).await;
    edit(&mut flow);
    let error = persist(&mut db, &flow)
        .await
        .expect_err("unverifiable committed content must fail the flow");
    let chain = format!("{error:#}");
    assert!(chain.contains(expected), "{chain}");
    assert!(persisted_traces(&db).await.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn a_path_missing_from_the_recorded_revision_persists_no_trace() {
    let repo = QuotedPathRepo::new();
    assert_flow_persists_nothing(
        &repo,
        |flow| {
            let file = flow
                .post_commit_data
                .parsed_patch
                .files
                .iter_mut()
                .find(|file| file.new_path == UNICODE_PATH)
                .expect("unicode file");
            file.new_path = String::from("src/cafe\u{301}.rs");
        },
        "is absent from revision",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_tree_lookup_persists_no_trace() {
    let repo = QuotedPathRepo::new();
    assert_flow_persists_nothing(
        &repo,
        |flow| flow.post_commit_data.commit_oid = "0".repeat(40),
        "with 'git ls-tree'",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_missing_blob_object_persists_no_trace() {
    let repo = QuotedPathRepo::new();
    let blob = git(
        repo.root(),
        &["rev-parse", &format!("HEAD:{PLAIN_PATH}")],
        None,
    );
    assert_flow_persists_nothing(
        &repo,
        |flow| {
            let object = flow
                .repository_root
                .join(".git/objects")
                .join(&blob[..2])
                .join(&blob[2..]);
            std::fs::remove_file(object).expect("remove loose blob");
        },
        "with 'git cat-file'",
    )
    .await;
}
