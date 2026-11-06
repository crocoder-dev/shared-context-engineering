use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::repository_path::{RepositoryPath, RepositoryRoot};
use super::AgentTraceTool;
use crate::services::agent_trace_db::ParsedDiffTracePatch;
use crate::services::mutation_trace::attribution::{
    logical_path, patch_for_locations_with_provenance, PatchLineLocation, PatchLineProvenance,
};
use crate::services::patch::{ParsedPatch, PatchFileChange, TouchedLineKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectEvidenceSource<'a> {
    pub patch: &'a ParsedPatch,
    pub base_dir: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub tool_version: Option<&'a str>,
}

#[cfg(test)]
impl<'a> DirectEvidenceSource<'a> {
    pub fn repository_relative(patch: &'a ParsedPatch) -> Self {
        DirectEvidenceSource {
            patch,
            base_dir: None,
            tool_name: None,
            tool_version: None,
        }
    }
}

impl<'a> From<&'a ParsedDiffTracePatch> for DirectEvidenceSource<'a> {
    fn from(row: &'a ParsedDiffTracePatch) -> Self {
        DirectEvidenceSource {
            patch: &row.patch,
            base_dir: row.path_base_dir.as_deref(),
            tool_name: row.tool_name.as_deref(),
            tool_version: row.tool_version.as_deref(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedDirectLine {
    pub(crate) provenance: PatchLineProvenance,
    pub(crate) sources: BTreeSet<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DirectLineEvidence {
    Accepted(AcceptedDirectLine),
    Conflicting,
}

type ExactLineKey<'a> = (TouchedLineKind, u64, &'a str);

pub(crate) fn direct_line_evidence(
    sources: &[DirectEvidenceSource<'_>],
    target: &ParsedPatch,
    repository_root: &RepositoryRoot,
) -> BTreeMap<PatchLineLocation, DirectLineEvidence> {
    let candidate_files: Vec<(usize, &PatchFileChange, RepositoryPath)> = sources
        .iter()
        .enumerate()
        .flat_map(|(source_index, source)| {
            source.patch.files.iter().filter_map(move |file| {
                repository_root
                    .repository_path(logical_path(file), source.base_dir)
                    .map(|path| (source_index, file, path))
            })
        })
        .collect();

    let mut evidence = BTreeMap::new();
    for (file_index, target_file) in target.files.iter().enumerate() {
        let Some(target_path) = repository_root.repository_path(logical_path(target_file), None)
        else {
            continue;
        };
        let mut by_line: HashMap<ExactLineKey<'_>, Vec<(usize, PatchLineProvenance)>> =
            HashMap::new();
        for (source_index, candidate_file, _) in candidate_files
            .iter()
            .filter(|(_, _, candidate_path)| *candidate_path == target_path)
        {
            for hunk in &candidate_file.hunks {
                for line in &hunk.lines {
                    by_line
                        .entry((line.kind, line.line_number, line.content.as_str()))
                        .or_default()
                        .push((
                            *source_index,
                            PatchLineProvenance {
                                session_id: line.session_id.clone(),
                                model_id: hunk.model_id.clone(),
                            },
                        ));
                }
            }
        }
        if by_line.is_empty() {
            continue;
        }

        for (hunk_index, hunk) in target_file.hunks.iter().enumerate() {
            for (line_index, line) in hunk.lines.iter().enumerate() {
                let Some(candidates) =
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
                    resolve_candidates(candidates),
                );
            }
        }
    }

    evidence
}

fn resolve_candidates(candidates: &[(usize, PatchLineProvenance)]) -> DirectLineEvidence {
    let sessions: BTreeSet<Option<&str>> = candidates
        .iter()
        .map(|(_, provenance)| provenance.session_id.as_deref())
        .collect();
    if sessions.len() != 1 {
        return DirectLineEvidence::Conflicting;
    }

    let models: BTreeSet<Option<&str>> = candidates
        .iter()
        .map(|(_, provenance)| provenance.model_id.as_deref())
        .collect();
    let model_id = match models.into_iter().collect::<Vec<_>>().as_slice() {
        [Some(model)] => Some((*model).to_owned()),
        _ => None,
    };

    DirectLineEvidence::Accepted(AcceptedDirectLine {
        provenance: PatchLineProvenance {
            session_id: candidates[0].1.session_id.clone(),
            model_id,
        },
        sources: candidates.iter().map(|(index, _)| *index).collect(),
    })
}

pub(crate) fn accepted_direct_evidence(
    sources: &[DirectEvidenceSource<'_>],
    target: &ParsedPatch,
    repository_root: &RepositoryRoot,
) -> BTreeMap<PatchLineLocation, AcceptedDirectLine> {
    direct_line_evidence(sources, target, repository_root)
        .into_iter()
        .filter_map(|(location, evidence)| match evidence {
            DirectLineEvidence::Accepted(accepted) => Some((location, accepted)),
            DirectLineEvidence::Conflicting => None,
        })
        .collect()
}

pub(crate) fn direct_evidence_patch(
    sources: &[DirectEvidenceSource<'_>],
    target: &ParsedPatch,
    repository_root: &RepositoryRoot,
) -> ParsedPatch {
    let provenance = accepted_direct_evidence(sources, target, repository_root)
        .into_iter()
        .map(|(location, accepted)| (location, accepted.provenance))
        .collect();
    patch_for_locations_with_provenance(target, &provenance)
}

pub(crate) fn has_direct_evidence(
    sources: &[DirectEvidenceSource<'_>],
    target: &ParsedPatch,
    repository_root: &RepositoryRoot,
) -> bool {
    !accepted_direct_evidence(sources, target, repository_root).is_empty()
}

pub(crate) fn accepted_direct_tool(
    sources: &[DirectEvidenceSource<'_>],
    accepted_sources: &BTreeSet<usize>,
) -> Option<AgentTraceTool> {
    let accepted: Vec<&DirectEvidenceSource<'_>> = accepted_sources
        .iter()
        .filter_map(|index| sources.get(*index))
        .collect();
    let name = single_value(accepted.iter().map(|source| source.tool_name))?;
    Some(AgentTraceTool {
        name: Some(name.to_owned()),
        version: single_value(accepted.iter().map(|source| source.tool_version))
            .map(ToOwned::to_owned),
    })
}

fn single_value<'a>(values: impl Iterator<Item = Option<&'a str>>) -> Option<&'a str> {
    let distinct: BTreeSet<Option<&str>> = values.collect();
    match distinct.into_iter().collect::<Vec<_>>().as_slice() {
        [Some(value)] => Some(value),
        _ => None,
    }
}
