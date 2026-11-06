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

fn parent(entries: &[(&str, Option<&str>)]) -> MutationLineage {
    let map: BTreeMap<String, Option<String>> = entries
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.map(str::to_owned)))
        .collect();
    MutationLineage::from_parent(&map)
}

fn id(path: &str, line_number: u64) -> ParentLineId {
    ParentLineId {
        path: path.to_owned(),
        line_number,
    }
}

fn ai(scope_id: &str) -> LineProvenance {
    LineProvenance::MutationAi {
        scope_id: scope(scope_id),
    }
}

fn edit(path: &str, hunks: &str) -> ParsedPatch {
    diff(&format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{hunks}"
    ))
}

#[test]
fn an_ai_exclusive_mutation_records_its_removal_of_one_parent_line() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\nc\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-b\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove b");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 2), ai("ai"))]
    );
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(2, "b")]),
        vec![ai("ai")]
    );
}

#[test]
fn a_non_exclusive_mutation_records_a_non_exclusive_removal() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\nc\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-b\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("remove b");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 2), LineProvenance::MutationNonAi)]
    );
}

#[test]
fn an_unobserved_transition_records_an_unknown_removal() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\nc\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-b\n"),
            &TransitionOrigin::Unobserved,
        )
        .expect("remove b");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 2), LineProvenance::Unknown)]
    );
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(2, "b")]),
        vec![LineProvenance::Unknown]
    );
}

#[test]
fn an_insertion_above_shifts_a_parent_line_without_changing_its_identity() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -1,2 +1,4 @@\n+x\n+y\n a\n b\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("insert above");
    lineage
        .apply(
            &edit("f.rs", "@@ -4,1 +3,0 @@\n-b\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove shifted b");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 2), ai("ai"))]
    );
}

#[test]
fn removals_in_one_hunk_keep_distinct_identities() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\nc\nd\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -1,4 +1,2 @@\n a\n-b\n-c\n d\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove b and c");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 2), ai("ai")), (id("f.rs", 3), ai("ai"))]
    );
}

#[test]
fn removals_in_separate_hunks_keep_their_own_identities() {
    let mut lineage = parent(&[("f.rs", Some("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-2\n@@ -8,1 +6,0 @@\n-8\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("remove 2 and 8");
    assert_eq!(
        lineage.removed_identities(),
        vec![
            (id("f.rs", 2), LineProvenance::MutationNonAi),
            (id("f.rs", 8), LineProvenance::MutationNonAi)
        ]
    );
}

#[test]
fn identical_text_at_two_parent_positions_keeps_two_identities() {
    let mut lineage = parent(&[("f.rs", Some("x\na\nx\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -3,1 +2,0 @@\n-x\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove the second x");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 3), ai("ai"))]
    );
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(3, "x")]),
        vec![ai("ai")]
    );
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(1, "x")]),
        vec![LineProvenance::Unknown],
        "a canonical diff aligned to the other x must not borrow its removal"
    );
}

#[test]
fn a_disagreeing_alignment_among_duplicates_leaves_every_such_removal_unknown() {
    let mut lineage = parent(&[("f.rs", Some("x\nx\na\nx\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -1,3 +1,1 @@\n-x\n-x\n a\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove the first two x");
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(2, "x"), (4, "x")]),
        vec![LineProvenance::Unknown, LineProvenance::Unknown]
    );
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(1, "x"), (2, "x")]),
        vec![ai("ai"), ai("ai")]
    );
}

#[test]
fn removing_a_line_added_after_the_parent_records_no_parent_removal() {
    let mut lineage = parent(&[("f.rs", Some("a\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -1,0 +2,1 @@\n+n\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("add n");
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-n\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove n");
    assert!(lineage.removed_identities().is_empty());
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(2, "n")]),
        vec![LineProvenance::Unknown]
    );
}

#[test]
fn restoring_removed_content_invalidates_the_removal() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\nc\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-b\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove b");
    lineage
        .apply(
            &edit("f.rs", "@@ -1,0 +2,1 @@\n+b\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("restore b");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 2), LineProvenance::Unknown)]
    );
    assert_eq!(
        lineage.provenance_at("f.rs", 2, "b"),
        LineProvenance::MutationNonAi,
        "the restored line is a new line, not the parent line"
    );
}

#[test]
fn a_restored_then_redeleted_line_never_resurrects_the_older_ai_removal() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\nc\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-b\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove b");
    lineage
        .apply(
            &edit("f.rs", "@@ -1,0 +2,1 @@\n+b\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("restore b");
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-b\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("remove b again");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 2), LineProvenance::Unknown)]
    );
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(2, "b")]),
        vec![LineProvenance::Unknown]
    );
}

#[test]
fn matching_content_added_to_an_unrelated_file_is_not_a_restoration() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\n")), ("g.rs", Some("z\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-b\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove b");
    lineage
        .apply(
            &edit("g.rs", "@@ -1,0 +2,1 @@\n+b\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("add b elsewhere");
    assert_eq!(
        lineage.removed_identities(),
        vec![(id("f.rs", 2), ai("ai"))]
    );
}

#[test]
fn deleting_a_whole_file_removes_every_parent_line() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\ndeleted file mode 100644\n--- a/f.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("delete");
    assert_eq!(
        lineage.committed_removals("f.rs", "f.rs", &[(1, "a"), (2, "b")]),
        vec![ai("ai"), ai("ai")]
    );
}

#[test]
fn recreating_a_deleted_file_invalidates_only_the_restored_content() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\n"))]);
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\ndeleted file mode 100644\n--- a/f.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("delete");
    lineage
        .apply(
            &diff("diff --git a/f.rs b/f.rs\nnew file mode 100644\n--- /dev/null\n+++ b/f.rs\n@@ -0,0 +1,2 @@\n+a\n+c\n"),
            &TransitionOrigin::MutationNonAi,
        )
        .expect("recreate");
    assert_eq!(
        lineage.removed_identities(),
        vec![
            (id("f.rs", 1), LineProvenance::Unknown),
            (id("f.rs", 2), ai("ai"))
        ]
    );
}

#[test]
fn an_observed_rename_keeps_parent_identity_under_the_old_path() {
    let mut lineage = parent(&[("old.md", Some("a\nb\nc\n")), ("new.md", None)]);
    lineage
        .apply(
            &diff("diff --git a/old.md b/new.md\nsimilarity index 66%\nrename from old.md\nrename to new.md\n--- a/old.md\n+++ b/new.md\n@@ -1,3 +1,2 @@\n a\n-b\n c\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("rename and remove b");
    assert_eq!(lineage.parent_path_of("new.md"), Some("old.md"));
    assert_eq!(lineage.parent_path_of("old.md"), None);
    assert_eq!(
        lineage.committed_removals("new.md", "old.md", &[(2, "b")]),
        vec![ai("ai")]
    );
}

#[test]
fn a_delete_and_add_pair_does_not_establish_a_rename() {
    let mut lineage = parent(&[("old.md", Some("a\nb\n")), ("new.md", None)]);
    lineage
        .apply(
            &diff("diff --git a/old.md b/old.md\ndeleted file mode 100644\n--- a/old.md\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n\
                   diff --git a/new.md b/new.md\nnew file mode 100644\n--- /dev/null\n+++ b/new.md\n@@ -0,0 +1,1 @@\n+a\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("delete and add");
    assert_eq!(lineage.parent_path_of("new.md"), None);
    assert_eq!(
        lineage.committed_removals("new.md", "old.md", &[(2, "b")]),
        vec![LineProvenance::Unknown]
    );
}

#[test]
fn a_baseline_reset_drops_every_removal_claim() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\n"))]);
    lineage
        .apply(
            &edit("f.rs", "@@ -2,1 +1,0 @@\n-b\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove b");
    let map: BTreeMap<String, Option<String>> =
        [("f.rs".to_owned(), Some("a\n".to_owned()))].into();
    lineage.reset_all(&map);
    assert!(lineage.removed_identities().is_empty());
    lineage
        .apply(
            &edit("f.rs", "@@ -1,1 +0,0 @@\n-a\n"),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove a after the reset");
    assert!(
        lineage.removed_identities().is_empty(),
        "lines after a reset carry no parent identity"
    );
    assert_eq!(lineage.parent_path_of("f.rs"), None);
}

#[test]
fn distrusting_a_path_makes_its_removals_unknown() {
    let mut lineage = parent(&[("f.rs", Some("a\nb\n")), ("g.rs", Some("c\nd\n"))]);
    lineage
        .apply(
            &diff(
                "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +1,0 @@\n-b\n\
                   diff --git a/g.rs b/g.rs\n--- a/g.rs\n+++ b/g.rs\n@@ -2,1 +1,0 @@\n-d\n",
            ),
            &TransitionOrigin::MutationAi(scope("ai")),
        )
        .expect("remove b and d");
    lineage.distrust_removals("f.rs");
    assert_eq!(
        lineage.removed_identities(),
        vec![
            (id("f.rs", 2), LineProvenance::Unknown),
            (id("g.rs", 2), ai("ai"))
        ]
    );
}

/// Deterministic randomized checks of parent-line identity and removal
/// provenance against an independent reference model.
///
/// The reference applies each generated operation directly to lines whose
/// parent identity is known by construction; the lineage only ever sees the
/// unified diff of that operation. The canonical committed diff is derived by
/// an independent LCS, so it can align duplicate content differently from the
/// replayed history.
mod randomized_tests {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;

    use crate::services::mutation_trace::lineage::{
        LineProvenance, MutationLineage, ParentLineId, TransitionOrigin,
    };
    use crate::services::mutation_trace::types::ScopeId;
    use crate::services::patch::parse_patch;

    const PATH: &str = "f.rs";
    const SEEDS: u64 = 600;
    const ALPHABET: [&str; 4] = ["a", "b", "c", "d"];
    const MAX_PARENT_LINES: u64 = 8;
    const MAX_OPERATIONS: u64 = 10;

    struct SplitMix(u64);

    impl SplitMix {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }

        fn index(&mut self, len: usize) -> usize {
            usize::try_from(self.below(len as u64)).expect("index fits")
        }

        fn content(&mut self) -> String {
            ALPHABET[self.index(ALPHABET.len())].to_owned()
        }

        fn origin(&mut self) -> TransitionOrigin {
            match self.below(4) {
                0 => TransitionOrigin::MutationAi(ScopeId("scope-1".to_owned())),
                1 => TransitionOrigin::MutationAi(ScopeId("scope-2".to_owned())),
                2 => TransitionOrigin::MutationNonAi,
                _ => TransitionOrigin::Unobserved,
            }
        }
    }

    /// The reference's own mapping from transition to effective provenance.
    fn expected_provenance(origin: &TransitionOrigin) -> LineProvenance {
        match origin {
            TransitionOrigin::MutationAi(scope_id) => LineProvenance::MutationAi {
                scope_id: scope_id.clone(),
            },
            TransitionOrigin::MutationNonAi => LineProvenance::MutationNonAi,
            TransitionOrigin::Unobserved => LineProvenance::Unknown,
        }
    }

    #[derive(Clone)]
    struct ReferenceLine {
        content: String,
        parent: Option<u64>,
        provenance: LineProvenance,
    }

    struct Reference {
        lines: Vec<ReferenceLine>,
        removals: BTreeMap<u64, (String, LineProvenance)>,
    }

    impl Reference {
        fn new(parent: &[String]) -> Self {
            Reference {
                lines: parent
                    .iter()
                    .zip(1_u64..)
                    .map(|(content, line_number)| ReferenceLine {
                        content: content.clone(),
                        parent: Some(line_number),
                        provenance: LineProvenance::Unknown,
                    })
                    .collect(),
                removals: BTreeMap::new(),
            }
        }

        /// Restoring content that an earlier transition removed makes that
        /// removal unprovable; removals of the current transition are untouched.
        fn restore_check(&mut self, content: &str) {
            for (removed, provenance) in self.removals.values_mut() {
                if removed == content {
                    *provenance = LineProvenance::Unknown;
                }
            }
        }

        fn remove(&mut self, position: usize, origin: &TransitionOrigin) -> ReferenceLine {
            let line = self.lines.remove(position);
            if let Some(parent) = line.parent {
                self.removals
                    .insert(parent, (line.content.clone(), expected_provenance(origin)));
            }
            line
        }

        fn insert(&mut self, position: usize, content: &str, origin: &TransitionOrigin) {
            self.lines.insert(
                position,
                ReferenceLine {
                    content: content.to_owned(),
                    parent: None,
                    provenance: expected_provenance(origin),
                },
            );
        }

        fn gap(&mut self) {
            self.removals.clear();
            for line in &mut self.lines {
                line.parent = None;
                line.provenance = LineProvenance::Unknown;
            }
        }

        fn text(&self) -> String {
            lines_text(self.lines.iter().map(|line| line.content.as_str()))
        }
    }

    fn lines_text<'a>(lines: impl Iterator<Item = &'a str>) -> String {
        lines.fold(String::new(), |mut text, line| {
            text.push_str(line);
            text.push('\n');
            text
        })
    }

    fn file_diff(hunk: &str) -> String {
        format!("diff --git a/{PATH} b/{PATH}\n--- a/{PATH}\n+++ b/{PATH}\n{hunk}")
    }

    /// A whole-file hunk, as Git emits it, that removes `old[position]` when
    /// `removed` and then inserts `added` at `position`.
    fn whole_file_hunk(
        old: &[String],
        position: usize,
        removed: bool,
        added: Option<&str>,
    ) -> String {
        let mut body = String::new();
        for (index, line) in old.iter().enumerate() {
            if index == position {
                if removed {
                    writeln!(body, "-{line}").expect("write hunk line");
                }
                if let Some(added) = added {
                    writeln!(body, "+{added}").expect("write hunk line");
                }
                if removed {
                    continue;
                }
            }
            writeln!(body, " {line}").expect("write hunk line");
        }
        if position == old.len() {
            if let Some(added) = added {
                writeln!(body, "+{added}").expect("write hunk line");
            }
        }
        let new_len = old.len() - usize::from(removed) + usize::from(added.is_some());
        let start = |len: usize| usize::from(len > 0);
        format!(
            "@@ -{},{} +{},{} @@\n{body}",
            start(old.len()),
            old.len(),
            start(new_len),
            new_len
        )
    }

    /// Old-side line numbers an LCS diff from `old` to `new` removes.
    fn lcs_removed(old: &[&str], new: &[&str]) -> Vec<u64> {
        let mut table = vec![vec![0_usize; new.len() + 1]; old.len() + 1];
        for i in (0..old.len()).rev() {
            for j in (0..new.len()).rev() {
                table[i][j] = if old[i] == new[j] {
                    table[i + 1][j + 1] + 1
                } else {
                    table[i + 1][j].max(table[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        let mut removed = Vec::new();
        while i < old.len() {
            if j < new.len() && old[i] == new[j] && table[i][j] == table[i + 1][j + 1] + 1 {
                i += 1;
                j += 1;
            } else if j >= new.len() || table[i + 1][j] >= table[i][j + 1] {
                removed.push(i as u64 + 1);
                i += 1;
            } else {
                j += 1;
            }
        }
        removed
    }

    /// After a gap, no parent line can resolve as removed, whichever lines the
    /// canonical diff names.
    fn committed_removal_results_are_unknown(lineage: &MutationLineage, parent: &[String]) -> bool {
        let every_parent_line: Vec<(u64, &str)> = parent
            .iter()
            .zip(1_u64..)
            .map(|(content, line_number)| (line_number, content.as_str()))
            .collect();
        lineage
            .committed_removals(PATH, PATH, &every_parent_line)
            .iter()
            .all(|provenance| *provenance == LineProvenance::Unknown)
    }

    #[derive(Default)]
    struct Coverage {
        ai_removals: usize,
        non_ai_removals: usize,
        unknown_removals: usize,
        invalidated_restorations: usize,
        gaps: usize,
    }

    #[allow(clippy::too_many_lines)]
    fn run_seed(seed: u64, coverage: &mut Coverage) {
        let mut rng = SplitMix(seed);
        let parent_len = rng.below(MAX_PARENT_LINES) + 1;
        let parent: Vec<String> = (0..parent_len).map(|_| rng.content()).collect();
        let parent_text = lines_text(parent.iter().map(String::as_str));
        let files: BTreeMap<String, Option<String>> =
            [(PATH.to_owned(), Some(parent_text.clone()))].into();

        let mut lineage = MutationLineage::from_parent(&files);
        let mut added_only = MutationLineage::from_baseline(&files);
        let mut reference = Reference::new(&parent);
        let mut gapped = false;

        for _ in 0..=rng.below(MAX_OPERATIONS) {
            if rng.below(12) == 0 {
                let current: BTreeMap<String, Option<String>> =
                    [(PATH.to_owned(), Some(reference.text()))].into();
                lineage.reset_all(&current);
                added_only.reset_all(&current);
                reference.gap();
                gapped = true;
                coverage.gaps += 1;
                continue;
            }

            let origin = rng.origin();
            let len = reference.lines.len();
            let old: Vec<String> = reference
                .lines
                .iter()
                .map(|line| line.content.clone())
                .collect();
            let hunk = match rng.below(4) {
                0 if len > 0 => {
                    let position = rng.index(len);
                    reference.remove(position, &origin);
                    whole_file_hunk(&old, position, true, None)
                }
                1 if len > 0 => {
                    let position = rng.index(len);
                    let content = rng.content();
                    reference.restore_check(&content);
                    reference.remove(position, &origin);
                    reference.insert(position, &content, &origin);
                    whole_file_hunk(&old, position, true, Some(&content))
                }
                2 if !reference.removals.is_empty() => {
                    let removed: Vec<String> = reference
                        .removals
                        .values()
                        .map(|(content, _)| content.clone())
                        .collect();
                    let content = removed[rng.index(removed.len())].clone();
                    let position = rng.index(len + 1);
                    if reference.removals.values().any(|(removed, provenance)| {
                        *removed == content && *provenance != LineProvenance::Unknown
                    }) {
                        coverage.invalidated_restorations += 1;
                    }
                    reference.restore_check(&content);
                    reference.insert(position, &content, &origin);
                    whole_file_hunk(&old, position, false, Some(&content))
                }
                _ => {
                    let position = rng.index(len + 1);
                    let content = rng.content();
                    reference.restore_check(&content);
                    reference.insert(position, &content, &origin);
                    whole_file_hunk(&old, position, false, Some(&content))
                }
            };
            let patch = parse_patch(&file_diff(&hunk), None).expect("generated diff parses");
            lineage.apply(&patch, &origin).unwrap_or_else(|error| {
                panic!("seed {seed}: lineage rejected {hunk:?}: {error:?}")
            });
            added_only
                .apply(&patch, &origin)
                .expect("added-line lineage applies the same diff");
        }

        let expected_removals: Vec<(ParentLineId, LineProvenance)> = reference
            .removals
            .iter()
            .map(|(line_number, (_, provenance))| {
                (
                    ParentLineId {
                        path: PATH.to_owned(),
                        line_number: *line_number,
                    },
                    provenance.clone(),
                )
            })
            .collect();
        assert_eq!(
            lineage.removed_identities(),
            expected_removals,
            "seed {seed}: removal identities"
        );
        for (identity, _) in &expected_removals {
            assert!(identity.line_number >= 1 && identity.line_number <= parent_len);
        }
        if gapped {
            assert!(
                committed_removal_results_are_unknown(&lineage, &parent),
                "seed {seed}: a gap must not preserve removal ownership"
            );
        }

        for (index, line) in reference.lines.iter().enumerate() {
            let line_number = index as u64 + 1;
            assert_eq!(
                lineage.provenance_at(PATH, line_number, &line.content),
                line.provenance,
                "seed {seed}: added-line provenance at {line_number}"
            );
            assert_eq!(
                added_only.provenance_at(PATH, line_number, &line.content),
                line.provenance,
                "seed {seed}: identity tracking changed added-line provenance"
            );
        }

        let old: Vec<&str> = parent.iter().map(String::as_str).collect();
        let new: Vec<&str> = reference
            .lines
            .iter()
            .map(|line| line.content.as_str())
            .collect();
        let committed: Vec<(u64, &str)> = lcs_removed(&old, &new)
            .into_iter()
            .map(|line_number| {
                let index = usize::try_from(line_number - 1).expect("index fits");
                (line_number, old[index])
            })
            .collect();
        let results = lineage.committed_removals(PATH, PATH, &committed);
        assert_eq!(results.len(), committed.len());

        let surviving_parents: Vec<u64> = reference
            .lines
            .iter()
            .filter_map(|line| line.parent)
            .collect();
        for ((line_number, content), result) in committed.iter().zip(&results) {
            match result {
                LineProvenance::Unknown => coverage.unknown_removals += 1,
                resolved => {
                    let (removed_content, provenance) =
                        reference.removals.get(line_number).unwrap_or_else(|| {
                            panic!("seed {seed}: {line_number} resolved without removal")
                        });
                    assert_eq!(removed_content, content, "seed {seed}");
                    assert_eq!(provenance, resolved, "seed {seed}: removal provenance");
                    assert!(
                        !surviving_parents.contains(line_number),
                        "seed {seed}: a surviving parent line was attributed as removed"
                    );
                    if matches!(resolved, LineProvenance::MutationAi { .. }) {
                        coverage.ai_removals += 1;
                    } else {
                        coverage.non_ai_removals += 1;
                    }
                }
            }
        }
    }

    #[test]
    fn randomized_histories_match_the_reference_removal_model() {
        let mut coverage = Coverage::default();
        for seed in 0..SEEDS {
            run_seed(seed, &mut coverage);
        }
        assert!(coverage.ai_removals > 0, "no AI removal was exercised");
        assert!(
            coverage.non_ai_removals > 0,
            "no non-AI removal was exercised"
        );
        assert!(
            coverage.unknown_removals > 0,
            "no unknown removal was exercised"
        );
        assert!(
            coverage.invalidated_restorations > 0,
            "no restoration of an attributed removal was exercised"
        );
        assert!(coverage.gaps > 0, "no gap was exercised");
    }

    #[test]
    fn lcs_reference_diff_removes_only_unmatched_old_lines() {
        assert_eq!(lcs_removed(&["a", "b", "c"], &["a", "c"]), vec![2]);
        assert_eq!(lcs_removed(&["a", "b"], &[]), vec![1, 2]);
        assert_eq!(lcs_removed(&["a"], &["b", "a"]), Vec::<u64>::new());
    }
}
