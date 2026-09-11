use std::collections::{BTreeSet, HashSet};

use crate::services::patch::{
    ParsedPatch, PatchFileChange, PatchHunk, TouchedLine, TouchedLineKind,
};

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationAttributionResult {
    pub mutation_ai_patch: ParsedPatch,
    pub resolved_non_ai_patch: ParsedPatch,
    pub unresolved_patch: ParsedPatch,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PatchLineLocation {
    pub file_index: usize,
    pub hunk_index: usize,
    pub line_index: usize,
}

#[must_use]
pub fn exclude_direct_coverage(
    target_patch: &ParsedPatch,
    direct_coverage: &ParsedPatch,
) -> ParsedPatch {
    let direct_lines = direct_line_keys(direct_coverage);
    let selected: BTreeSet<PatchLineLocation> = all_locations(target_patch)
        .into_iter()
        .filter(|location| {
            let line = line_at(target_patch, *location);
            !direct_lines.contains(&(
                logical_path(&target_patch.files[location.file_index]).to_owned(),
                line.kind,
                line.line_number,
                line.content.clone(),
            ))
        })
        .collect();
    patch_for_locations(target_patch, &selected)
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

fn line_at(patch: &ParsedPatch, location: PatchLineLocation) -> &TouchedLine {
    &patch.files[location.file_index].hunks[location.hunk_index].lines[location.line_index]
}

pub(crate) fn logical_path(file: &PatchFileChange) -> &str {
    if file.new_path.is_empty() {
        &file.old_path
    } else {
        &file.new_path
    }
}

pub fn patch_for_locations(
    patch: &ParsedPatch,
    selected: &BTreeSet<PatchLineLocation>,
) -> ParsedPatch {
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

    fn contents(result: &ParsedPatch) -> Vec<(u64, String)> {
        result
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
            .map(|line| (line.line_number, line.content.clone()))
            .collect()
    }

    #[test]
    fn exclude_direct_coverage_removes_exactly_the_directly_covered_lines() {
        let direct = patch(
            "src/lib.rs",
            vec![line(TouchedLineKind::Added, 1, "direct")],
        );
        let target = patch(
            "src/lib.rs",
            vec![
                line(TouchedLineKind::Added, 1, "direct"),
                line(TouchedLineKind::Added, 2, "mutation"),
            ],
        );

        let remaining = exclude_direct_coverage(&target, &direct);
        assert_eq!(contents(&remaining), vec![(2, "mutation".to_owned())]);
    }

    #[test]
    fn exclude_direct_coverage_keeps_everything_when_direct_is_empty() {
        let target = patch("src/lib.rs", vec![line(TouchedLineKind::Added, 1, "x")]);
        let remaining = exclude_direct_coverage(&target, &ParsedPatch { files: vec![] });
        assert_eq!(contents(&remaining), vec![(1, "x".to_owned())]);
    }

    #[test]
    fn exclude_direct_coverage_matches_on_content_not_only_position() {
        let direct = patch("src/lib.rs", vec![line(TouchedLineKind::Added, 1, "kept")]);
        let target = patch(
            "src/lib.rs",
            vec![line(TouchedLineKind::Added, 1, "different")],
        );
        let remaining = exclude_direct_coverage(&target, &direct);
        assert_eq!(contents(&remaining), vec![(1, "different".to_owned())]);
    }
}
