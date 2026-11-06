use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::services::mutation_trace::attribution::{
    logical_path, patch_for_locations_with_provenance, PatchLineLocation, PatchLineProvenance,
};
use crate::services::patch::{paths_refer_to_same_file, ParsedPatch, TouchedLineKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DirectLineEvidence {
    Unique(PatchLineProvenance),
    Conflicting,
}

type ExactLineKey<'a> = (TouchedLineKind, u64, &'a str);

pub(crate) fn direct_line_evidence(
    candidates: &[ParsedPatch],
    target: &ParsedPatch,
) -> BTreeMap<PatchLineLocation, DirectLineEvidence> {
    let mut evidence = BTreeMap::new();

    for (file_index, target_file) in target.files.iter().enumerate() {
        let target_path = logical_path(target_file);
        let mut by_line: HashMap<ExactLineKey<'_>, Vec<PatchLineProvenance>> = HashMap::new();
        for candidate_file in candidates.iter().flat_map(|patch| patch.files.iter()) {
            if !paths_refer_to_same_file(logical_path(candidate_file), target_path) {
                continue;
            }
            for hunk in &candidate_file.hunks {
                for line in &hunk.lines {
                    by_line
                        .entry((line.kind, line.line_number, line.content.as_str()))
                        .or_default()
                        .push(PatchLineProvenance {
                            session_id: line.session_id.clone(),
                            model_id: hunk.model_id.clone(),
                        });
                }
            }
        }
        if by_line.is_empty() {
            continue;
        }

        for (hunk_index, hunk) in target_file.hunks.iter().enumerate() {
            for (line_index, line) in hunk.lines.iter().enumerate() {
                let Some(sources) =
                    by_line.get(&(line.kind, line.line_number, line.content.as_str()))
                else {
                    continue;
                };
                evidence.insert(
                    PatchLineLocation {
                        file_index,
                        hunk_index,
                        line_index,
                    },
                    resolve_sources(sources),
                );
            }
        }
    }

    evidence
}

fn resolve_sources(sources: &[PatchLineProvenance]) -> DirectLineEvidence {
    let sessions: BTreeSet<Option<&str>> = sources
        .iter()
        .map(|source| source.session_id.as_deref())
        .collect();
    if sessions.len() != 1 {
        return DirectLineEvidence::Conflicting;
    }

    let models: BTreeSet<Option<&str>> = sources
        .iter()
        .map(|source| source.model_id.as_deref())
        .collect();
    let model_id = match models.into_iter().collect::<Vec<_>>().as_slice() {
        [Some(model)] => Some((*model).to_owned()),
        _ => None,
    };

    DirectLineEvidence::Unique(PatchLineProvenance {
        session_id: sources[0].session_id.clone(),
        model_id,
    })
}

pub(crate) fn attributable_direct_evidence(
    candidates: &[ParsedPatch],
    target: &ParsedPatch,
) -> BTreeMap<PatchLineLocation, PatchLineProvenance> {
    direct_line_evidence(candidates, target)
        .into_iter()
        .filter_map(|(location, evidence)| match evidence {
            DirectLineEvidence::Unique(provenance) => Some((location, provenance)),
            DirectLineEvidence::Conflicting => None,
        })
        .collect()
}

pub(crate) fn direct_evidence_patch(
    candidates: &[ParsedPatch],
    target: &ParsedPatch,
) -> ParsedPatch {
    patch_for_locations_with_provenance(target, &attributable_direct_evidence(candidates, target))
}

pub(crate) fn has_direct_evidence(candidates: &[ParsedPatch], target: &ParsedPatch) -> bool {
    !direct_line_evidence(candidates, target).is_empty()
}
