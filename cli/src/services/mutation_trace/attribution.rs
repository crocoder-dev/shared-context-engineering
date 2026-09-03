use std::collections::{BTreeSet, HashMap, HashSet};

use super::types::{Attribution, FailureKind};
use crate::services::patch::{
    ParsedPatch, PatchFileChange, PatchHunk, TouchedLine, TouchedLineKind,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationPatchEvidence {
    pub patch: ParsedPatch,
    pub tainted: bool,
    pub failure_kind: FailureKind,
    pub attribution: Attribution,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PatchLineLocation {
    pub file_index: usize,
    pub hunk_index: usize,
    pub line_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MutationLineMatch {
    pub mutation: PatchLineLocation,
    pub target: PatchLineLocation,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationAttributionResult {
    pub mutation_ai_patch: ParsedPatch,
    pub resolved_non_ai_patch: ParsedPatch,
    pub unresolved_patch: ParsedPatch,
}

#[must_use]
pub fn resolve_mutation_attribution(
    direct_coverage: &ParsedPatch,
    unresolved_patch: &ParsedPatch,
    mutation_evidence: &[MutationPatchEvidence],
) -> MutationAttributionResult {
    let direct_lines = direct_line_keys(direct_coverage);
    let target_locations = all_locations(unresolved_patch)
        .into_iter()
        .filter(|location| {
            let line = line_at(unresolved_patch, *location);
            !direct_lines.contains(&(
                logical_path(&unresolved_patch.files[location.file_index]).to_owned(),
                line.kind,
                line.line_number,
                line.content.clone(),
            ))
        })
        .collect::<BTreeSet<_>>();

    let mut remaining = target_locations;
    let mut mutation_ai = BTreeSet::new();
    let mut resolved_non_ai = BTreeSet::new();

    for evidence in mutation_evidence {
        if remaining.is_empty() {
            break;
        }

        let matches =
            strict_mutation_matches_for_locations(&evidence.patch, unresolved_patch, &remaining);
        if matches.is_empty() {
            continue;
        }

        let positive = !evidence.tainted
            && evidence.failure_kind == FailureKind::Healthy
            && matches!(evidence.attribution, Attribution::AiExclusive(_));

        for matched in matches {
            remaining.remove(&matched.target);
            if positive {
                mutation_ai.insert(matched.target);
            } else {
                resolved_non_ai.insert(matched.target);
            }
        }
    }

    MutationAttributionResult {
        mutation_ai_patch: patch_for_locations(unresolved_patch, &mutation_ai),
        resolved_non_ai_patch: patch_for_locations(unresolved_patch, &resolved_non_ai),
        unresolved_patch: patch_for_locations(unresolved_patch, &remaining),
    }
}

#[must_use]
pub fn strict_mutation_matches(
    mutation_patch: &ParsedPatch,
    target_patch: &ParsedPatch,
) -> Vec<MutationLineMatch> {
    let remaining = all_locations(target_patch);
    strict_mutation_matches_for_locations(mutation_patch, target_patch, &remaining)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ExactLineKey<'a> {
    kind: TouchedLineKind,
    line_number: u64,
    content: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct HistoricalLineKey<'a> {
    kind: TouchedLineKind,
    content: &'a str,
}

#[allow(clippy::too_many_lines)]
fn strict_mutation_matches_for_locations(
    mutation_patch: &ParsedPatch,
    target_patch: &ParsedPatch,
    remaining: &BTreeSet<PatchLineLocation>,
) -> Vec<MutationLineMatch> {
    let file_pairs = pair_files(mutation_patch, target_patch, remaining);
    let mut matches = Vec::new();

    for (mutation_file_index, target_file_index) in file_pairs {
        let mutation_lines = file_locations(
            mutation_file_index,
            &mutation_patch.files[mutation_file_index],
        );
        let target_lines =
            file_locations(target_file_index, &target_patch.files[target_file_index])
                .into_iter()
                .filter(|location| remaining.contains(location))
                .collect::<Vec<_>>();

        let mut mutation_used = BTreeSet::new();
        let mut target_used = BTreeSet::new();

        let target_exact_counts = counts_by(&target_lines, |location| {
            let line = line_at(target_patch, *location);
            ExactLineKey {
                kind: line.kind,
                line_number: line.line_number,
                content: &line.content,
            }
        });

        let mutation_exact_counts = counts_by(&mutation_lines, |location| {
            let line = line_at(mutation_patch, *location);
            ExactLineKey {
                kind: line.kind,
                line_number: line.line_number,
                content: &line.content,
            }
        });

        let mut mutation_exact = HashMap::new();
        for location in &mutation_lines {
            let line = line_at(mutation_patch, *location);
            let key = ExactLineKey {
                kind: line.kind,
                line_number: line.line_number,
                content: &line.content,
            };
            if mutation_exact_counts.get(&key) == Some(&1)
                && target_exact_counts.get(&key) == Some(&1)
            {
                mutation_exact.insert(key, *location);
            }
        }
        for location in &target_lines {
            let line = line_at(target_patch, *location);
            let key = ExactLineKey {
                kind: line.kind,
                line_number: line.line_number,
                content: &line.content,
            };
            if let Some(&mutation_location) = mutation_exact.get(&key) {
                mutation_used.insert(mutation_location);
                target_used.insert(*location);
                matches.push(MutationLineMatch {
                    mutation: mutation_location,
                    target: *location,
                });
            }
        }

        let remaining_mutation = mutation_lines
            .into_iter()
            .filter(|location| !mutation_used.contains(location))
            .collect::<Vec<_>>();
        let remaining_target = target_lines
            .into_iter()
            .filter(|location| !target_used.contains(location))
            .collect::<Vec<_>>();
        let mutation_historical_counts = counts_by(&remaining_mutation, |location| {
            let line = line_at(mutation_patch, *location);
            HistoricalLineKey {
                kind: line.kind,
                content: &line.content,
            }
        });
        let target_historical_counts = counts_by(&remaining_target, |location| {
            let line = line_at(target_patch, *location);
            HistoricalLineKey {
                kind: line.kind,
                content: &line.content,
            }
        });

        let mut mutation_historical = HashMap::new();
        for location in &remaining_mutation {
            let line = line_at(mutation_patch, *location);
            let key = HistoricalLineKey {
                kind: line.kind,
                content: &line.content,
            };
            if mutation_historical_counts.get(&key) == Some(&1)
                && target_historical_counts.get(&key) == Some(&1)
            {
                mutation_historical.insert(key, *location);
            }
        }
        for location in &remaining_target {
            let line = line_at(target_patch, *location);
            let key = HistoricalLineKey {
                kind: line.kind,
                content: &line.content,
            };
            if let Some(&mutation_location) = mutation_historical.get(&key) {
                matches.push(MutationLineMatch {
                    mutation: mutation_location,
                    target: *location,
                });
            }
        }
    }

    matches.sort_by_key(|matched| matched.target);
    matches
}

fn pair_files(
    mutation_patch: &ParsedPatch,
    target_patch: &ParsedPatch,
    remaining: &BTreeSet<PatchLineLocation>,
) -> Vec<(usize, usize)> {
    let active_targets = target_patch
        .files
        .iter()
        .enumerate()
        .filter(|(file_index, file)| {
            file_locations(*file_index, file)
                .into_iter()
                .any(|location| remaining.contains(&location))
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mutation_files = mutation_patch
        .files
        .iter()
        .enumerate()
        .filter(|(file_index, file)| !file_locations(*file_index, file).is_empty())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    let mut pairs = Vec::new();
    let mut used_mutation_files = BTreeSet::new();
    let mut used_target_files = BTreeSet::new();

    for &target_index in &active_targets {
        let target_logical_path = logical_path(&target_patch.files[target_index]);
        let exact = mutation_files
            .iter()
            .copied()
            .filter(|mutation_index| {
                !used_mutation_files.contains(mutation_index)
                    && logical_path(&mutation_patch.files[*mutation_index]) == target_logical_path
            })
            .collect::<Vec<_>>();
        let same_target_path_count = active_targets
            .iter()
            .filter(|other| logical_path(&target_patch.files[**other]) == target_logical_path)
            .count();
        if exact.len() == 1 && same_target_path_count == 1 {
            let mutation_index = exact[0];
            pairs.push((mutation_index, target_index));
            used_mutation_files.insert(mutation_index);
            used_target_files.insert(target_index);
        }
    }

    for &target_index in &active_targets {
        if used_target_files.contains(&target_index) {
            continue;
        }
        let target_logical_path = logical_path(&target_patch.files[target_index]);
        let candidates = mutation_files
            .iter()
            .copied()
            .filter(|mutation_index| {
                !used_mutation_files.contains(mutation_index)
                    && paths_have_normalized_suffix(
                        logical_path(&mutation_patch.files[*mutation_index]),
                        target_logical_path,
                    )
            })
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            continue;
        }
        let mutation_index = candidates[0];
        let reverse_targets = active_targets
            .iter()
            .filter(|other| {
                !used_target_files.contains(other)
                    && paths_have_normalized_suffix(
                        logical_path(&mutation_patch.files[mutation_index]),
                        logical_path(&target_patch.files[**other]),
                    )
            })
            .count();
        if reverse_targets == 1 {
            pairs.push((mutation_index, target_index));
            used_mutation_files.insert(mutation_index);
            used_target_files.insert(target_index);
        }
    }

    pairs.sort_by_key(|(_, target_index)| *target_index);
    pairs
}

fn counts_by<K, F>(locations: &[PatchLineLocation], mut key_for: F) -> HashMap<K, usize>
where
    K: Eq + std::hash::Hash,
    F: FnMut(&PatchLineLocation) -> K,
{
    let mut counts = HashMap::new();
    for location in locations {
        *counts.entry(key_for(location)).or_insert(0) += 1;
    }
    counts
}

fn direct_line_keys(direct_patch: &ParsedPatch) -> HashSet<(String, TouchedLineKind, u64, String)> {
    direct_patch
        .files
        .iter()
        .flat_map(|file| {
            let path = logical_path(file).to_owned();
            file.hunks.iter().flat_map(move |hunk| {
                let path = path.clone();
                hunk.lines.iter().map(move |line| {
                    (
                        path.clone(),
                        line.kind,
                        line.line_number,
                        line.content.clone(),
                    )
                })
            })
        })
        .collect()
}

fn all_locations(patch: &ParsedPatch) -> BTreeSet<PatchLineLocation> {
    patch
        .files
        .iter()
        .enumerate()
        .flat_map(|(file_index, file)| {
            file.hunks
                .iter()
                .enumerate()
                .flat_map(move |(hunk_index, hunk)| {
                    (0..hunk.lines.len()).map(move |line_index| PatchLineLocation {
                        file_index,
                        hunk_index,
                        line_index,
                    })
                })
        })
        .collect()
}

fn file_locations(file_index: usize, file: &PatchFileChange) -> Vec<PatchLineLocation> {
    file.hunks
        .iter()
        .enumerate()
        .flat_map(|(hunk_index, hunk)| {
            (0..hunk.lines.len()).map(move |line_index| PatchLineLocation {
                file_index,
                hunk_index,
                line_index,
            })
        })
        .collect()
}

fn line_at(patch: &ParsedPatch, location: PatchLineLocation) -> &TouchedLine {
    &patch.files[location.file_index].hunks[location.hunk_index].lines[location.line_index]
}

fn logical_path(file: &PatchFileChange) -> &str {
    if file.new_path.is_empty() {
        &file.old_path
    } else {
        &file.new_path
    }
}

fn normalized_components(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

fn paths_have_normalized_suffix(left: &str, right: &str) -> bool {
    let left = normalized_components(left);
    let right = normalized_components(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right
        || (left.len() > right.len() && left.ends_with(&right))
        || (right.len() > left.len() && right.ends_with(&left))
}

fn patch_for_locations(patch: &ParsedPatch, selected: &BTreeSet<PatchLineLocation>) -> ParsedPatch {
    let files = patch
        .files
        .iter()
        .enumerate()
        .filter_map(|(file_index, file)| {
            let hunks = file
                .hunks
                .iter()
                .enumerate()
                .filter_map(|(hunk_index, hunk)| {
                    let lines = hunk
                        .lines
                        .iter()
                        .enumerate()
                        .filter_map(|(line_index, line)| {
                            let location = PatchLineLocation {
                                file_index,
                                hunk_index,
                                line_index,
                            };
                            selected.contains(&location).then(|| line.clone())
                        })
                        .collect::<Vec<_>>();
                    (!lines.is_empty()).then(|| PatchHunk {
                        lines,
                        ..hunk.clone()
                    })
                })
                .collect::<Vec<_>>();
            (!hunks.is_empty()).then(|| PatchFileChange {
                hunks,
                ..file.clone()
            })
        })
        .collect();

    ParsedPatch { files }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::mutation_trace::types::ScopeId;
    use crate::services::patch::FileChangeKind;

    fn line(kind: TouchedLineKind, number: u64, content: &str) -> TouchedLine {
        TouchedLine {
            kind,
            line_number: number,
            content: content.to_owned(),
            session_id: None,
        }
    }

    fn patch(path: &str, lines: Vec<TouchedLine>) -> ParsedPatch {
        ParsedPatch {
            files: vec![PatchFileChange {
                old_path: path.to_owned(),
                new_path: path.to_owned(),
                kind: FileChangeKind::Modified,
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_count: 1,
                    new_start: 1,
                    new_count: 1,
                    model_id: None,
                    lines,
                }],
            }],
        }
    }

    fn evidence(patch: ParsedPatch, attribution: Attribution) -> MutationPatchEvidence {
        MutationPatchEvidence {
            patch,
            tainted: false,
            failure_kind: FailureKind::Healthy,
            attribution,
        }
    }

    fn exclusive() -> Attribution {
        Attribution::AiExclusive(ScopeId("scope".to_owned()))
    }

    fn locations(result: &ParsedPatch) -> Vec<(u64, String)> {
        result
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .map(|line| (line.line_number, line.content.clone()))
            .collect()
    }

    #[test]
    fn mutation_attribution_direct_lines_are_excluded_before_matching() {
        let direct = patch(
            "src/lib.rs",
            vec![line(TouchedLineKind::Added, 1, "direct")],
        );
        let unresolved = patch(
            "src/lib.rs",
            vec![
                line(TouchedLineKind::Added, 1, "direct"),
                line(TouchedLineKind::Added, 2, "mutation"),
            ],
        );
        let result = resolve_mutation_attribution(
            &direct,
            &unresolved,
            &[evidence(
                patch(
                    "src/lib.rs",
                    vec![
                        line(TouchedLineKind::Added, 1, "direct"),
                        line(TouchedLineKind::Added, 2, "mutation"),
                    ],
                ),
                exclusive(),
            )],
        );

        assert_eq!(
            locations(&result.mutation_ai_patch),
            vec![(2, "mutation".into())]
        );
        assert!(result.resolved_non_ai_patch.files.is_empty());
        assert!(result.unresolved_patch.files.is_empty());
    }

    #[test]
    fn mutation_attribution_newest_nonexclusive_match_blocks_older_evidence() {
        let unresolved = patch("src/lib.rs", vec![line(TouchedLineKind::Added, 8, "same")]);
        let newest = MutationPatchEvidence {
            tainted: false,
            failure_kind: FailureKind::Healthy,
            attribution: Attribution::AiContended,
            patch: patch("src/lib.rs", vec![line(TouchedLineKind::Added, 8, "same")]),
        };
        let older = evidence(
            patch("src/lib.rs", vec![line(TouchedLineKind::Added, 8, "same")]),
            exclusive(),
        );
        let result = resolve_mutation_attribution(
            &ParsedPatch { files: vec![] },
            &unresolved,
            &[newest, older],
        );

        assert!(result.mutation_ai_patch.files.is_empty());
        assert_eq!(
            locations(&result.resolved_non_ai_patch),
            vec![(8, "same".into())]
        );
        assert!(result.unresolved_patch.files.is_empty());
    }

    #[test]
    fn mutation_attribution_unrelated_newer_event_does_not_block_older_exclusive_match() {
        let unresolved = patch(
            "src/lib.rs",
            vec![line(TouchedLineKind::Added, 8, "target")],
        );
        let newest = evidence(
            patch(
                "src/lib.rs",
                vec![line(TouchedLineKind::Added, 20, "unrelated")],
            ),
            exclusive(),
        );
        let older = evidence(
            patch(
                "src/lib.rs",
                vec![line(TouchedLineKind::Added, 8, "target")],
            ),
            exclusive(),
        );
        let result = resolve_mutation_attribution(
            &ParsedPatch { files: vec![] },
            &unresolved,
            &[newest, older],
        );

        assert_eq!(
            locations(&result.mutation_ai_patch),
            vec![(8, "target".into())]
        );
        assert!(result.resolved_non_ai_patch.files.is_empty());
        assert!(result.unresolved_patch.files.is_empty());
    }

    #[test]
    fn mutation_attribution_exact_matching_precedes_unique_historical_fallback() {
        let target = patch(
            "src/lib.rs",
            vec![
                line(TouchedLineKind::Added, 10, "exact"),
                line(TouchedLineKind::Added, 20, "fallback"),
            ],
        );
        let mutation = patch(
            "src/lib.rs",
            vec![
                line(TouchedLineKind::Added, 10, "exact"),
                line(TouchedLineKind::Added, 99, "fallback"),
            ],
        );

        let matches = strict_mutation_matches(&mutation, &target);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].target.line_index, 0);
        assert_eq!(matches[1].target.line_index, 1);
    }

    #[test]
    fn mutation_attribution_duplicate_lines_and_ambiguous_paths_are_not_guessed() {
        let duplicate_target = patch(
            "src/lib.rs",
            vec![
                line(TouchedLineKind::Added, 1, "repeat"),
                line(TouchedLineKind::Added, 2, "repeat"),
            ],
        );
        let duplicate_mutation = patch(
            "src/lib.rs",
            vec![
                line(TouchedLineKind::Added, 9, "repeat"),
                line(TouchedLineKind::Added, 10, "repeat"),
            ],
        );
        assert!(strict_mutation_matches(&duplicate_mutation, &duplicate_target).is_empty());

        let ambiguous_target = ParsedPatch {
            files: vec![
                PatchFileChange {
                    old_path: "src/foo.rs".into(),
                    new_path: "src/foo.rs".into(),
                    ..duplicate_target.files[0].clone()
                },
                PatchFileChange {
                    old_path: "tests/foo.rs".into(),
                    new_path: "tests/foo.rs".into(),
                    ..duplicate_target.files[0].clone()
                },
            ],
        };
        let suffix_mutation = patch("foo.rs", vec![line(TouchedLineKind::Added, 1, "repeat")]);
        assert!(strict_mutation_matches(&suffix_mutation, &ambiguous_target).is_empty());
    }

    #[test]
    fn mutation_attribution_tainted_and_unhealthy_exclusive_matches_are_non_ai() {
        let unresolved = patch("src/lib.rs", vec![line(TouchedLineKind::Added, 1, "line")]);
        let mut tainted = evidence(
            patch("src/lib.rs", vec![line(TouchedLineKind::Added, 1, "line")]),
            exclusive(),
        );
        tainted.tainted = true;
        let result =
            resolve_mutation_attribution(&ParsedPatch { files: vec![] }, &unresolved, &[tainted]);
        assert!(result.mutation_ai_patch.files.is_empty());
        assert_eq!(
            locations(&result.resolved_non_ai_patch),
            vec![(1, "line".into())]
        );
    }

    #[test]
    fn mutation_attribution_every_nonpositive_state_resolves_as_non_ai() {
        let unresolved = patch("src/lib.rs", vec![line(TouchedLineKind::Added, 1, "line")]);
        let cases = [
            (Attribution::AiContended, FailureKind::Healthy, false),
            (Attribution::IneligibleUnscoped, FailureKind::Healthy, false),
            (exclusive(), FailureKind::SnapshotFailure, false),
            (exclusive(), FailureKind::Healthy, true),
        ];

        for (attribution, failure_kind, tainted) in cases {
            let mut event = evidence(
                patch("src/lib.rs", vec![line(TouchedLineKind::Added, 1, "line")]),
                attribution,
            );
            event.failure_kind = failure_kind;
            event.tainted = tainted;
            let result =
                resolve_mutation_attribution(&ParsedPatch { files: vec![] }, &unresolved, &[event]);
            assert!(result.mutation_ai_patch.files.is_empty());
            assert_eq!(
                locations(&result.resolved_non_ai_patch),
                vec![(1, "line".into())]
            );
            assert!(result.unresolved_patch.files.is_empty());
        }
    }
}
