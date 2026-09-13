use std::collections::{BTreeMap, BTreeSet};

use crate::services::mutation_trace::types::ScopeId;
use crate::services::patch::{
    ParsedPatch, PatchFileChange, PatchHunk, TouchedLine, TouchedLineKind,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineProvenance {
    Unknown,
    MutationAi { scope_id: ScopeId },
    MutationNonAi,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionOrigin {
    MutationAi(ScopeId),
    MutationNonAi,
    Unobserved,
}

impl TransitionOrigin {
    fn added_provenance(&self) -> LineProvenance {
        match self {
            TransitionOrigin::MutationAi(scope_id) => LineProvenance::MutationAi {
                scope_id: scope_id.clone(),
            },
            TransitionOrigin::MutationNonAi => LineProvenance::MutationNonAi,
            TransitionOrigin::Unobserved => LineProvenance::Unknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageError {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProvenanceLine {
    content: String,
    provenance: LineProvenance,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MutationLineage {
    files: BTreeMap<String, Vec<ProvenanceLine>>,
}

fn unknown_lines(content: Option<&str>) -> Vec<ProvenanceLine> {
    match content {
        None => Vec::new(),
        Some(text) => text
            .lines()
            .map(|line| ProvenanceLine {
                content: line.to_owned(),
                provenance: LineProvenance::Unknown,
            })
            .collect(),
    }
}

impl MutationLineage {
    #[must_use]
    pub fn from_baseline(files: &BTreeMap<String, Option<String>>) -> Self {
        MutationLineage {
            files: files
                .iter()
                .map(|(path, content)| (path.clone(), unknown_lines(content.as_deref())))
                .collect(),
        }
    }

    pub fn reset_file(&mut self, path: &str, content: Option<&str>) {
        self.files.insert(path.to_owned(), unknown_lines(content));
    }

    pub fn reset_all(&mut self, files: &BTreeMap<String, Option<String>>) {
        self.files = files
            .iter()
            .map(|(path, content)| (path.clone(), unknown_lines(content.as_deref())))
            .collect();
    }

    pub fn tracked_paths(&self) -> impl Iterator<Item = &String> {
        self.files.keys()
    }

    pub fn apply(
        &mut self,
        patch: &ParsedPatch,
        origin: &TransitionOrigin,
    ) -> Result<(), LineageError> {
        for file in &patch.files {
            self.apply_file(file, origin)?;
        }
        Ok(())
    }

    fn apply_file(
        &mut self,
        file: &PatchFileChange,
        origin: &TransitionOrigin,
    ) -> Result<(), LineageError> {
        let tracks_source = !file.old_path.is_empty() && self.files.contains_key(&file.old_path);
        let tracks_dest = !file.new_path.is_empty() && self.files.contains_key(&file.new_path);
        if !tracks_source && !tracks_dest {
            return Ok(());
        }

        let logical_path = if file.new_path.is_empty() {
            file.old_path.clone()
        } else {
            file.new_path.clone()
        };

        let old = self
            .files
            .get(&file.old_path)
            .or_else(|| {
                if file.new_path.is_empty() {
                    None
                } else {
                    self.files.get(&file.new_path)
                }
            })
            .cloned()
            .unwrap_or_default();

        let new = apply_hunks(&logical_path, &old, file, origin)?;

        if !file.old_path.is_empty() {
            self.files.remove(&file.old_path);
        }
        if !file.new_path.is_empty() {
            self.files.insert(file.new_path.clone(), new);
        }
        Ok(())
    }

    #[must_use]
    pub fn provenance_at(&self, path: &str, line_number: u64, content: &str) -> LineProvenance {
        let Some(lines) = self.files.get(path) else {
            return LineProvenance::Unknown;
        };
        let Some(index) = line_number
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
        else {
            return LineProvenance::Unknown;
        };
        match lines.get(index) {
            Some(line) if line.content == content => line.provenance.clone(),
            _ => LineProvenance::Unknown,
        }
    }
}

fn apply_hunks(
    path: &str,
    old: &[ProvenanceLine],
    file: &PatchFileChange,
    origin: &TransitionOrigin,
) -> Result<Vec<ProvenanceLine>, LineageError> {
    let mut hunks: Vec<&PatchHunk> = file.hunks.iter().collect();
    hunks.sort_by_key(|hunk| (hunk.old_start, hunk.new_start));

    let mut new: Vec<ProvenanceLine> = Vec::new();
    let mut cursor: usize = 0;

    for hunk in hunks {
        cursor = apply_one_hunk(path, old, hunk, origin, &mut new, cursor)?;
    }

    new.extend_from_slice(&old[cursor..]);
    Ok(new)
}

fn apply_one_hunk(
    path: &str,
    old: &[ProvenanceLine],
    hunk: &PatchHunk,
    origin: &TransitionOrigin,
    new: &mut Vec<ProvenanceLine>,
    mut cursor: usize,
) -> Result<usize, LineageError> {
    let fail = |reason: &str| LineageError {
        path: path.to_owned(),
        reason: reason.to_owned(),
    };

    let old_count = usize::try_from(hunk.old_count).map_err(|_| fail("old_count overflow"))?;
    let new_count = usize::try_from(hunk.new_count).map_err(|_| fail("new_count overflow"))?;

    let prefix_end = if old_count == 0 {
        usize::try_from(hunk.old_start).map_err(|_| fail("old_start overflow"))?
    } else {
        usize::try_from(hunk.old_start)
            .map_err(|_| fail("old_start overflow"))?
            .checked_sub(1)
            .ok_or_else(|| fail("old_start below 1 for a non-empty hunk"))?
    };

    if prefix_end < cursor {
        return Err(fail("hunks overlap or are out of order"));
    }
    if prefix_end > old.len() {
        return Err(fail("hunk starts past end of file"));
    }
    new.extend_from_slice(&old[cursor..prefix_end]);
    cursor = prefix_end;

    if cursor + old_count > old.len() {
        return Err(fail("hunk old region extends past end of file"));
    }
    let region = &old[cursor..cursor + old_count];
    cursor += old_count;

    let removed: Vec<&TouchedLine> = hunk
        .lines
        .iter()
        .filter(|line| line.kind == TouchedLineKind::Removed)
        .collect();
    let added: Vec<&TouchedLine> = hunk
        .lines
        .iter()
        .filter(|line| line.kind == TouchedLineKind::Added)
        .collect();

    if removed.len() > old_count {
        return Err(fail("more removed lines than the hunk's old region"));
    }
    if added.len() > new_count {
        return Err(fail("more added lines than the hunk's new region"));
    }
    if new_count - added.len() != old_count - removed.len() {
        return Err(fail("hunk context lengths are inconsistent"));
    }

    let mut removed_indices: BTreeSet<usize> = BTreeSet::new();
    for line in &removed {
        let offset = line
            .line_number
            .checked_sub(hunk.old_start)
            .and_then(|offset| usize::try_from(offset).ok())
            .filter(|offset| *offset < old_count)
            .ok_or_else(|| fail("removed line falls outside the hunk old region"))?;
        if region[offset].content != line.content {
            return Err(fail("removed line content does not match the tracked line"));
        }
        if !removed_indices.insert(offset) {
            return Err(fail("the same old line is removed twice"));
        }
    }

    let mut carried: Vec<&ProvenanceLine> = region
        .iter()
        .enumerate()
        .filter(|(index, _)| !removed_indices.contains(index))
        .map(|(_, line)| line)
        .collect();
    carried.reverse();

    let added_by_number: BTreeMap<u64, &str> = added
        .iter()
        .map(|line| (line.line_number, line.content.as_str()))
        .collect();
    if added_by_number.len() != added.len() {
        return Err(fail("two added lines share a line number"));
    }

    for position in 0..new_count {
        let line_number = hunk
            .new_start
            .checked_add(position as u64)
            .ok_or_else(|| fail("new line number overflow"))?;
        if let Some(content) = added_by_number.get(&line_number) {
            new.push(ProvenanceLine {
                content: (*content).to_owned(),
                provenance: origin.added_provenance(),
            });
        } else {
            let carried_line = carried
                .pop()
                .ok_or_else(|| fail("ran out of carried context lines"))?;
            new.push(carried_line.clone());
        }
    }
    if !carried.is_empty() {
        return Err(fail("carried context lines left unplaced"));
    }

    Ok(cursor)
}

#[cfg(test)]
#[path = "lineage/tests.rs"]
mod tests;
