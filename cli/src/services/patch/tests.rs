use std::fmt::Write as _;

use super::{parse_patch, FileChangeKind, PatchFileChange};

fn quote(path: &str) -> String {
    let mut quoted = String::from("\"");
    for byte in path.bytes() {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            b'\t' => quoted.push_str("\\t"),
            b'\n' => quoted.push_str("\\n"),
            0x20..=0x7e => quoted.push(char::from(byte)),
            _ => write!(quoted, "\\{byte:03o}").expect("write to string"),
        }
    }
    quoted.push('"');
    quoted
}

fn needs_quoting(path: &str) -> bool {
    path.bytes()
        .any(|byte| matches!(byte, b'"' | b'\\') || !(0x20..=0x7e).contains(&byte))
}

fn git_name(prefix: &str, path: &str) -> String {
    let name = format!("{prefix}{path}");
    if needs_quoting(&name) {
        quote(&name)
    } else {
        name
    }
}

fn marker_name(prefix: &str, path: &str) -> String {
    let name = git_name(prefix, path);
    if name.contains(' ') {
        format!("{name}\t")
    } else {
        name
    }
}

fn modified_patch(path: &str) -> String {
    format!(
        "diff --git {} {}\nindex 1111111..2222222 100644\n--- {}\n+++ {}\n@@ -1 +1 @@\n-old\n+new\n",
        git_name("a/", path),
        git_name("b/", path),
        marker_name("a/", path),
        marker_name("b/", path),
    )
}

fn single_file(text: &str) -> PatchFileChange {
    let mut patch = parse_patch(text, None).expect("patch parses");
    assert_eq!(patch.files.len(), 1, "{text}");
    patch.files.remove(0)
}

const PATHS: [&str; 10] = [
    "src/café.rs",
    "src/file name.rs",
    "src/file\tname.rs",
    "src/file\nname.rs",
    "src/file\"name.rs",
    "src/file\\name.rs",
    "src/x b/y.rs",
    "src/trailing ",
    "a/b/nested.rs",
    "src/🦀.rs",
];

#[test]
fn git_paths_decode_to_their_exact_identity() {
    for path in PATHS {
        let file = single_file(&modified_patch(path));
        assert_eq!(file.old_path, path, "{path:?}");
        assert_eq!(file.new_path, path, "{path:?}");
        assert_eq!(file.kind, FileChangeKind::Modified, "{path:?}");
        assert_eq!(file.hunks[0].lines.len(), 2);
    }
}

#[test]
fn header_paths_stand_alone_when_no_markers_follow() {
    for path in PATHS {
        let text = format!(
            "diff --git {} {}\nold mode 100644\nnew mode 100755\n",
            git_name("a/", path),
            git_name("b/", path),
        );
        let file = single_file(&text);
        assert_eq!(
            (file.old_path.as_str(), file.new_path.as_str()),
            (path, path)
        );
    }
}

#[test]
fn created_and_deleted_quoted_files_keep_dev_null_semantics() {
    let path = "src/café.rs";
    let created = format!(
        "diff --git {a} {b}\nnew file mode 100644\n--- /dev/null\n+++ {b}\n@@ -0,0 +1 @@\n+x\n",
        a = git_name("a/", path),
        b = git_name("b/", path),
    );
    let file = single_file(&created);
    assert_eq!(file.kind, FileChangeKind::Added);
    assert_eq!((file.old_path.as_str(), file.new_path.as_str()), ("", path));

    let deleted = format!(
        "diff --git {a} {b}\ndeleted file mode 100644\n--- {a}\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n",
        a = git_name("a/", path),
        b = git_name("b/", path),
    );
    let file = single_file(&deleted);
    assert_eq!(file.kind, FileChangeKind::Deleted);
    assert_eq!((file.old_path.as_str(), file.new_path.as_str()), (path, ""));
}

#[test]
fn renames_take_their_paths_from_quoted_metadata() {
    let text = format!(
        "diff --git a/src/x b/y.rs b/src/x b/z.rs\nsimilarity index 100%\nrename from {}\nrename to {}\n",
        "src/x b/y.rs",
        quote("src/x b/z\u{e9}.rs"),
    );
    let file = single_file(&text);
    assert_eq!(file.kind, FileChangeKind::Renamed);
    assert_eq!(file.old_path, "src/x b/y.rs");
    assert_eq!(file.new_path, "src/x b/zé.rs");
}

#[test]
fn ambiguous_headers_without_resolving_lines_are_rejected() {
    let error = parse_patch(
        "diff --git a/x b/y b/z\nold mode 100644\nnew mode 100755\n",
        None,
    )
    .expect_err("ambiguous header must not be guessed");
    assert!(error.message.contains("ambiguous header"), "{error}");
}

#[test]
fn malformed_quoted_paths_fail_explicitly() {
    for text in [
        "diff --git \"a/caf\\q.rs\" \"b/caf\\q.rs\"\n",
        "diff --git a/x b/x\n--- \"a/caf\\351.rs\"\n+++ b/x\n",
        "diff --git a/x b/x\n--- a/x\n+++ \"b/unterminated\n",
        "diff --git a/x b/x\nrename to \"b\\000\"\n",
    ] {
        let error = parse_patch(text, None).expect_err("malformed path must fail");
        assert!(error.message.starts_with("invalid path in"), "{error}");
    }
}

#[test]
fn index_style_patches_still_parse() {
    let text = "Index: src/lib.rs\n===================================================================\n--- src/lib.rs\t(revision 1)\n+++ src/lib.rs\t(working copy)\n@@ -1 +1 @@\n-old\n+new\n";
    let file = single_file(text);
    assert_eq!(
        (file.old_path.as_str(), file.new_path.as_str()),
        ("src/lib.rs", "src/lib.rs")
    );
    assert_eq!(file.kind, FileChangeKind::Modified);
}
