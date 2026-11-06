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
fn a_deleted_then_recreated_file_stays_tracked_and_takes_the_new_provenance() {
    let mut lineage = baseline(&[("plan.md", Some("a\n"))]);
    lineage
        .apply(
            &diff(
                "diff --git a/plan.md b/plan.md\ndeleted file mode 100644\n--- a/plan.md\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-a\n",
            ),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("delete");
    lineage
        .apply(
            &diff(
                "diff --git a/plan.md b/plan.md\nnew file mode 100644\n--- /dev/null\n+++ b/plan.md\n@@ -0,0 +1,1 @@\n+a\n",
            ),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("recreate");
    lineage
        .apply(
            &diff("diff --git a/plan.md b/plan.md\n--- a/plan.md\n+++ b/plan.md\n@@ -1,1 +1,2 @@\n a\n+  - Verify:\n"),
            &TransitionOrigin::MutationAi(scope("author")),
        )
        .expect("edit after recreation");

    assert_eq!(
        lineage.provenance_at("plan.md", 2, "  - Verify:"),
        LineProvenance::MutationAi {
            scope_id: scope("author")
        }
    );
    assert_eq!(
        lineage.provenance_at("plan.md", 1, "a"),
        LineProvenance::MutationNonAi
    );
}

#[test]
fn a_renamed_away_file_stays_tracked_as_absent() {
    let mut lineage = baseline(&[("old.md", Some("a\n")), ("new.md", None)]);
    lineage
        .apply(
            &diff(
                "diff --git a/old.md b/new.md\nsimilarity index 100%\nrename from old.md\nrename to new.md\n",
            ),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("rename");
    lineage
        .apply(
            &diff(
                "diff --git a/old.md b/old.md\nnew file mode 100644\n--- /dev/null\n+++ b/old.md\n@@ -0,0 +1,1 @@\n+b\n",
            ),
            &TransitionOrigin::MutationAi(scope("s1")),
        )
        .expect("recreate the renamed-away path");

    assert_eq!(
        lineage.provenance_at("old.md", 1, "b"),
        LineProvenance::MutationAi {
            scope_id: scope("s1")
        }
    );
}

#[test]
fn adding_a_file_that_is_tracked_with_content_is_a_lineage_error() {
    let mut lineage = baseline(&[("f.rs", Some("a\n"))]);
    let error = lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\nnew file mode 100644\n--- /dev/null\n+++ b/f.rs\n@@ -0,0 +1,1 @@\n+b\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect_err("a stale tracked file must fail closed");
    assert_eq!(error.path, "f.rs");
}
