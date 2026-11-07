use std::path::Path;
use std::process::{Command, Stdio};

use std::io::Write as _;

use super::{
    parse_ls_tree_entry, CommittedFile, CommittedFileReadError, CommittedFileReader,
    GitCommittedFiles, GitEntryKind,
};

const REGULAR_PATH: &str = "src/lib.rs";
const EXECUTABLE_PATH: &str = "bin/run.sh";
const SYMLINK_PATH: &str = "src/link.rs";
const GITLINK_PATH: &str = "vendor/sub";
const DIRECTORY_PATH: &str = "src";
const REGULAR_BYTES: &[u8] = b"pub fn f() {}\n";
const EXECUTABLE_BYTES: &[u8] = b"#!/bin/sh\necho hi\n";
const OBJECT_ID: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

fn git_with_input(root: &Path, args: &[&str], input: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "@1793960000 +0000")
        .env("GIT_COMMITTER_DATE", "@1793960000 +0000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git should start");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .expect("write stdin");
    let output = child.wait_with_output().expect("git should finish");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .trim()
        .to_owned()
}

fn git(root: &Path, args: &[&str]) -> String {
    git_with_input(root, args, b"")
}

fn hash_object(root: &Path, bytes: &[u8]) -> String {
    git_with_input(root, &["hash-object", "-w", "--stdin"], bytes)
}

fn commit_index(root: &Path) -> String {
    let tree = git(root, &["write-tree"]);
    git(
        root,
        &[
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit-tree",
            &tree,
            "-m",
            "entries",
        ],
    )
}

struct EntryRepo {
    dir: tempfile::TempDir,
    revision: String,
}

impl EntryRepo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        git(root, &["init", "-q"]);
        let regular = hash_object(root, REGULAR_BYTES);
        let executable = hash_object(root, EXECUTABLE_BYTES);
        let link_target = hash_object(root, b"lib.rs");
        let submodule_commit = {
            let empty_tree = git_with_input(root, &["mktree"], b"");
            git(
                root,
                &[
                    "-c",
                    "user.email=t@example.invalid",
                    "-c",
                    "user.name=t",
                    "commit-tree",
                    &empty_tree,
                    "-m",
                    "sub",
                ],
            )
        };
        for (mode, object, path) in [
            ("100644", &regular, REGULAR_PATH),
            ("100755", &executable, EXECUTABLE_PATH),
            ("120000", &link_target, SYMLINK_PATH),
            ("160000", &submodule_commit, GITLINK_PATH),
        ] {
            git(
                root,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("{mode},{object},{path}"),
                ],
            );
        }
        let revision = commit_index(root);
        EntryRepo { dir, revision }
    }

    fn reader(&self) -> GitCommittedFiles<'_> {
        GitCommittedFiles::new(self.dir.path(), &self.revision)
    }
}

#[test]
fn regular_and_executable_files_are_read_as_blobs() {
    let repo = EntryRepo::new();
    assert_eq!(
        repo.reader().read_committed_file(REGULAR_PATH),
        Ok(CommittedFile::Blob(REGULAR_BYTES.to_vec()))
    );
    assert_eq!(
        repo.reader().read_committed_file(EXECUTABLE_PATH),
        Ok(CommittedFile::Blob(EXECUTABLE_BYTES.to_vec()))
    );
}

#[test]
fn symlinks_gitlinks_and_directories_are_not_line_addressable() {
    let repo = EntryRepo::new();
    for path in [SYMLINK_PATH, GITLINK_PATH, DIRECTORY_PATH] {
        assert_eq!(
            repo.reader().read_committed_file(path),
            Ok(CommittedFile::NotLineAddressable),
            "{path}"
        );
    }
}

#[test]
fn absent_paths_are_missing_and_never_matched_by_prefix_or_suffix() {
    let repo = EntryRepo::new();
    for path in [
        "src/absent.rs",
        "lib.rs",
        "src/lib",
        "SRC/lib.rs",
        "src/*.rs",
    ] {
        assert_eq!(
            repo.reader().read_committed_file(path),
            Ok(CommittedFile::Missing),
            "{path}"
        );
    }
}

#[test]
fn a_failed_tree_lookup_is_an_error_not_a_missing_file() {
    let repo = EntryRepo::new();
    let unknown = "0".repeat(40);
    let error = GitCommittedFiles::new(repo.dir.path(), &unknown)
        .read_committed_file(REGULAR_PATH)
        .expect_err("unknown revision must fail");
    assert!(
        matches!(&error, CommittedFileReadError::TreeLookupFailed { path, revision, .. }
            if path == REGULAR_PATH && *revision == unknown),
        "{error:?}"
    );

    let not_a_repository = tempfile::tempdir().expect("plain dir");
    assert!(matches!(
        GitCommittedFiles::new(not_a_repository.path(), &repo.revision)
            .read_committed_file(REGULAR_PATH),
        Err(CommittedFileReadError::TreeLookupFailed { .. })
    ));
}

#[test]
fn a_missing_blob_object_is_a_blob_read_error() {
    let repo = EntryRepo::new();
    let root = repo.dir.path();
    let absent_blob = "1".repeat(40);
    let tree = git_with_input(
        root,
        &["mktree", "--missing"],
        format!("100644 blob {absent_blob}\tghost.rs\n").as_bytes(),
    );
    let revision = git(
        root,
        &[
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit-tree",
            &tree,
            "-m",
            "ghost",
        ],
    );
    let error = GitCommittedFiles::new(root, &revision)
        .read_committed_file("ghost.rs")
        .expect_err("absent blob must fail");
    assert!(
        matches!(&error, CommittedFileReadError::BlobReadFailed { object_id, .. }
            if *object_id == absent_blob),
        "{error:?}"
    );
}

#[test]
fn modes_and_object_types_must_agree() {
    let accepted = [
        ("100644", "blob", GitEntryKind::RegularFile),
        ("100755", "blob", GitEntryKind::RegularFile),
        ("120000", "blob", GitEntryKind::Symlink),
        ("160000", "commit", GitEntryKind::Gitlink),
        ("040000", "tree", GitEntryKind::Directory),
    ];
    for (mode, object_type, kind) in accepted {
        assert_eq!(GitEntryKind::classify(mode, object_type), Some(kind));
    }
    for (mode, object_type) in [
        ("100644", "tree"),
        ("120000", "commit"),
        ("160000", "blob"),
        ("040000", "blob"),
        ("100664", "blob"),
        ("100644", "tag"),
    ] {
        assert_eq!(GitEntryKind::classify(mode, object_type), None);
    }
}

fn listing(records: &[&str]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| record.bytes().chain([0]))
        .collect()
}

#[test]
fn malformed_or_ambiguous_listings_are_rejected() {
    let malformed = [
        listing(&["100644 blob"]),
        listing(&[&format!("100644 blob {OBJECT_ID} src/lib.rs")]),
        listing(&[&format!("100644 {OBJECT_ID}\tsrc/lib.rs")]),
        listing(&["100644 blob not-hex\tsrc/lib.rs"]),
        listing(&[&format!("100644 tree {OBJECT_ID}\tsrc/lib.rs")]),
        listing(&[&format!("100644  blob {OBJECT_ID}\tsrc/lib.rs")]),
        listing(&[
            &format!("100644 blob {OBJECT_ID}\tsrc/lib.rs"),
            &format!("100755 blob {OBJECT_ID}\tsrc/lib.rs"),
        ]),
    ];
    for bytes in malformed {
        assert!(
            parse_ls_tree_entry(&bytes, REGULAR_PATH).is_err(),
            "{:?}",
            String::from_utf8_lossy(&bytes)
        );
    }
}

#[test]
fn listings_match_paths_exactly() {
    let bytes = listing(&[&format!("100644 blob {OBJECT_ID}\tsrc/caf\u{e9}.rs")]);
    assert_eq!(
        parse_ls_tree_entry(&bytes, "src/caf\u{e9}.rs").map(|entry| entry.map(|entry| entry.kind)),
        Ok(Some(GitEntryKind::RegularFile))
    );
    assert_eq!(parse_ls_tree_entry(&bytes, "caf\u{e9}.rs"), Ok(None));
    assert_eq!(parse_ls_tree_entry(b"", REGULAR_PATH), Ok(None));
}

fn added_file_patch(path: &str, mode: &str, lines: &[&str]) -> String {
    let mut text = format!(
        "diff --git a/{path} b/{path}\nnew file mode {mode}\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    );
    for line in lines {
        text.push('+');
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn entry_patch() -> crate::services::patch::ParsedPatch {
    let text = [
        added_file_patch(REGULAR_PATH, "100644", &["pub fn f() {}"]),
        added_file_patch(EXECUTABLE_PATH, "100755", &["#!/bin/sh", "echo hi"]),
        added_file_patch(SYMLINK_PATH, "120000", &["lib.rs"]),
        added_file_patch(GITLINK_PATH, "160000", &["Subproject commit 0000"]),
    ]
    .concat();
    crate::services::patch::parse_patch(&text, None).expect("entry patch parses")
}

fn build_entry_trace(
    patch: &crate::services::patch::ParsedPatch,
    reader: &dyn CommittedFileReader,
) -> anyhow::Result<crate::services::agent_trace::AgentTrace> {
    use crate::services::agent_trace::{
        build_agent_trace_from_evidence, AgentTraceEvidence, AgentTraceMetadataInput,
        RepositoryRoot,
    };
    let empty = crate::services::patch::ParsedPatch { files: Vec::new() };
    build_agent_trace_from_evidence(
        AgentTraceEvidence {
            repository_root: &RepositoryRoot::new(Path::new("/workspace/project")),
            direct_sources: &[],
            mutation_ai_patch: &empty,
            mutation_non_ai_patch: &empty,
        },
        patch,
        AgentTraceMetadataInput {
            commit_timestamp: "2026-11-06T10:00:00Z",
            commit_revision: OBJECT_ID,
            vcs_type: None,
        },
        reader,
    )
}

#[test]
fn only_regular_git_entries_receive_ranges_while_counters_cover_every_entry() {
    let repo = EntryRepo::new();
    let trace = build_entry_trace(&entry_patch(), &repo.reader()).expect("trace builds");
    let value = serde_json::to_value(&trace).expect("serialize");
    crate::services::agent_trace::validate_agent_trace_value(&value).expect("schema");

    let ranges: Vec<(&str, u64, u64, Option<&str>)> = trace
        .files
        .iter()
        .flat_map(|file| {
            file.conversations
                .iter()
                .flat_map(|conversation| conversation.ranges.iter())
                .map(|range| {
                    (
                        file.path.as_str(),
                        range.start_line,
                        range.end_line,
                        range.content_hash.as_deref(),
                    )
                })
        })
        .collect();
    let executable_hash = super::content_hash(EXECUTABLE_BYTES);
    let regular_hash = super::content_hash(REGULAR_BYTES);
    assert_eq!(
        ranges,
        vec![
            (REGULAR_PATH, 1, 1, Some(regular_hash.as_str())),
            (EXECUTABLE_PATH, 1, 2, Some(executable_hash.as_str())),
        ]
    );
    assert_eq!(trace.metadata.sce.line_changes.unknown.added, 5);
    assert_eq!(trace.metadata.sce.line_changes.unknown.removed, 0);
}

struct FailingReader(CommittedFileReadError);

impl CommittedFileReader for FailingReader {
    fn read_committed_file(&self, _path: &str) -> super::CommittedFileRead {
        Err(self.0.clone())
    }
}

struct FixedReader(CommittedFile);

impl CommittedFileReader for FixedReader {
    fn read_committed_file(&self, _path: &str) -> super::CommittedFileRead {
        Ok(self.0.clone())
    }
}

#[test]
fn missing_paths_and_read_errors_never_become_hashless_ranges() {
    let patch = crate::services::patch::parse_patch(
        &added_file_patch(REGULAR_PATH, "100644", &["pub fn f() {}"]),
        None,
    )
    .expect("patch parses");
    let detail = String::from("fatal: boom");
    let failures: [(&dyn CommittedFileReader, &str); 4] = [
        (
            &FixedReader(CommittedFile::Missing),
            "is absent from revision",
        ),
        (
            &FailingReader(CommittedFileReadError::TreeLookupFailed {
                path: REGULAR_PATH.to_owned(),
                revision: OBJECT_ID.to_owned(),
                detail: detail.clone(),
            }),
            "with 'git ls-tree': fatal: boom",
        ),
        (
            &FailingReader(CommittedFileReadError::MalformedTreeEntry {
                path: REGULAR_PATH.to_owned(),
                revision: OBJECT_ID.to_owned(),
                detail: detail.clone(),
            }),
            "Unexpected 'git ls-tree' entry",
        ),
        (
            &FailingReader(CommittedFileReadError::BlobReadFailed {
                path: REGULAR_PATH.to_owned(),
                revision: OBJECT_ID.to_owned(),
                object_id: OBJECT_ID.to_owned(),
                detail,
            }),
            "with 'git cat-file': fatal: boom",
        ),
    ];
    for (reader, expected) in failures {
        let error = build_entry_trace(&patch, reader).expect_err("must fail closed");
        let chain = format!("{error:#}");
        assert!(chain.contains(expected), "{chain}");
        assert!(chain.contains(REGULAR_PATH), "{chain}");
        assert!(chain.contains(OBJECT_ID), "{chain}");
    }

    let unverified =
        build_entry_trace(&patch, &FixedReader(CommittedFile::Unverified)).expect("best effort");
    let range = &unverified.files[0].conversations[0].ranges[0];
    assert_eq!((range.start_line, range.end_line), (1, 1));
    assert_eq!(range.content_hash, None);
}
