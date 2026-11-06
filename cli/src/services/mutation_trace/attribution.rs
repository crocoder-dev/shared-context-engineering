use std::collections::{BTreeMap, BTreeSet};

use crate::services::patch::{ParsedPatch, PatchFileChange, PatchHunk};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchLineProvenance {
    pub session_id: Option<String>,
    pub model_id: Option<String>,
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

pub fn patch_for_locations_with_provenance(
    patch: &ParsedPatch,
    selected: &BTreeMap<PatchLineLocation, PatchLineProvenance>,
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
                            let provenance = selected.get(&location)?;
                            let mut line = line.clone();
                            line.session_id.clone_from(&provenance.session_id);
                            Some(line)
                        })
                        .collect::<Vec<_>>();
                    (!lines.is_empty()).then(|| PatchHunk {
                        model_id: agreed_model(selected, file_index, hunk_index),
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

fn agreed_model(
    selected: &BTreeMap<PatchLineLocation, PatchLineProvenance>,
    file_index: usize,
    hunk_index: usize,
) -> Option<String> {
    let mut model: Option<&str> = None;
    for (location, provenance) in selected {
        if location.file_index != file_index || location.hunk_index != hunk_index {
            continue;
        }
        let candidate = provenance.model_id.as_deref()?;
        match model {
            None => model = Some(candidate),
            Some(existing) if existing == candidate => {}
            Some(_) => return None,
        }
    }
    model.map(ToOwned::to_owned)
}
