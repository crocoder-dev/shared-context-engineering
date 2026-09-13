use super::*;
use crate::services::patch::parse_patch;

fn scope(id: &str) -> ScopeId {
    ScopeId(id.to_owned())
}

fn baseline(entries: &[(&str, Option<&str>)]) -> MutationLineage {
    let map: BTreeMap<String, Option<String>> = entries
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.map(str::to_owned)))
        .collect();
    MutationLineage::from_baseline(&map)
}

fn diff(text: &str) -> ParsedPatch {
    parse_patch(text, None).expect("diff should parse")
}

#[test]
fn an_added_ai_line_carries_ai_provenance() {
    let mut lineage = baseline(&[("f.rs", Some("a\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n"),
            &TransitionOrigin::MutationAi(scope("s1")),
        )
        .expect("apply");
    assert_eq!(
        lineage.provenance_at("f.rs", 2, "foo"),
        LineProvenance::MutationAi {
            scope_id: scope("s1")
        }
    );
    assert_eq!(
        lineage.provenance_at("f.rs", 1, "a"),
        LineProvenance::Unknown
    );
}

#[test]
fn a_removed_line_loses_its_provenance_permanently() {
    let mut lineage = baseline(&[("f.rs", Some("a\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n"),
            &TransitionOrigin::MutationAi(scope("s1")),
        )
        .expect("add");
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,1 @@\n a\n-foo\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("remove");
    assert_eq!(
        lineage.provenance_at("f.rs", 2, "foo"),
        LineProvenance::Unknown
    );
    assert_eq!(lineage.tracked_paths().count(), 1);
}

#[test]
fn identical_remove_then_readd_takes_the_new_transition_provenance() {
    let mut lineage = baseline(&[("f.rs", Some("a\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("add");
    lineage
        .apply(
            &diff(
                "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-foo\n+foo\n",
            ),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("replace");
    assert_eq!(
        lineage.provenance_at("f.rs", 2, "foo"),
        LineProvenance::MutationNonAi
    );
}

#[test]
fn context_provenance_survives_line_number_movement() {
    let mut lineage = baseline(&[("f.rs", Some("b\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-b\n+B\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("seed B as AI");
    lineage
        .apply(
            &diff(
                "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,4 @@\n+x\n+y\n+z\n B\n",
            ),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("insert above");
    assert_eq!(
        lineage.provenance_at("f.rs", 4, "B"),
        LineProvenance::MutationAi {
            scope_id: scope("ai")
        }
    );
}

#[test]
fn unobserved_transition_introduces_unknown_lines() {
    let mut lineage = baseline(&[("f.rs", Some("a\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n"),
            &TransitionOrigin::Unobserved,
        )
        .expect("apply");
    assert_eq!(
        lineage.provenance_at("f.rs", 2, "foo"),
        LineProvenance::Unknown
    );
}

#[test]
fn a_mismatched_removed_line_is_a_lineage_error() {
    let mut lineage = baseline(&[("f.rs", Some("a\n"))]);
    let error = lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-different\n+new\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect_err("content mismatch must fail closed");
    assert_eq!(error.path, "f.rs");
}

#[test]
fn duplicate_lines_do_not_let_provenance_jump_between_occurrences() {
    let mut lineage = baseline(&[("f.rs", Some("foo\nfoo\nfoo\n"))]);
    lineage
        .apply(
            &diff(
                "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-foo\n+foo\n",
            ),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("replace middle");
    assert_eq!(
        lineage.provenance_at("f.rs", 1, "foo"),
        LineProvenance::Unknown
    );
    assert_eq!(
        lineage.provenance_at("f.rs", 2, "foo"),
        LineProvenance::MutationAi {
            scope_id: scope("ai")
        }
    );
    assert_eq!(
        lineage.provenance_at("f.rs", 3, "foo"),
        LineProvenance::Unknown
    );
}

#[test]
fn a_deleted_file_drops_out_of_the_lineage() {
    let mut lineage = baseline(&[("f.rs", Some("a\nb\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\ndeleted file mode 100644\n--- a/f.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("delete");
    assert_eq!(lineage.tracked_paths().count(), 0);
}

#[test]
fn reset_file_returns_a_file_to_a_conservative_baseline() {
    let mut lineage = baseline(&[("f.rs", Some("a\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,2 @@\n a\n+foo\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("add");
    lineage.reset_file("f.rs", Some("a\nfoo\n"));
    assert_eq!(
        lineage.provenance_at("f.rs", 2, "foo"),
        LineProvenance::Unknown
    );
}
