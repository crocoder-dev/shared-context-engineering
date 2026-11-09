use std::collections::{BTreeMap, BTreeSet};

use crate::services::mutation_trace::types::ScopeId;
use crate::services::patch::{
    FileChangeKind, ParsedPatch, PatchFileChange, PatchHunk, TouchedLine, TouchedLineKind,
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
    fn provenance(&self) -> LineProvenance {
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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ParentLineId {
    pub path: String,
    pub line_number: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProvenanceLine {
    content: String,
    provenance: LineProvenance,
    parent_identity: Option<ParentLineId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RemovalRecord {
    content: String,
    provenance: LineProvenance,
}

#[derive(Debug, Default)]
struct TransitionEffects {
    removed: Vec<(ParentLineId, String)>,
    added: Vec<(String, String)>,
}

impl TransitionEffects {
    fn record_removed(&mut self, region: &[ProvenanceLine], offsets: &BTreeSet<usize>) {
        for offset in offsets {
            let line = &region[*offset];
            if let Some(identity) = &line.parent_identity {
                self.removed.push((identity.clone(), line.content.clone()));
            }
        }
    }

    fn record_added(&mut self, path: &str, content: &str) {
        self.added.push((path.to_owned(), content.to_owned()));
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MutationLineage {
    files: BTreeMap<String, Vec<ProvenanceLine>>,
    removals: BTreeMap<ParentLineId, RemovalRecord>,
    parent_paths: BTreeMap<String, String>,
}

fn unknown_lines(content: Option<&str>) -> Vec<ProvenanceLine> {
    match content {
        None => Vec::new(),
        Some(text) => text
            .lines()
            .map(|line| ProvenanceLine {
                content: line.to_owned(),
                provenance: LineProvenance::Unknown,
                parent_identity: None,
            })
            .collect(),
    }
}

fn parent_lines(path: &str, content: Option<&str>) -> Vec<ProvenanceLine> {
    match content {
        None => Vec::new(),
        Some(text) => text
            .lines()
            .zip(1_u64..)
            .map(|(line, line_number)| ProvenanceLine {
                content: line.to_owned(),
                provenance: LineProvenance::Unknown,
                parent_identity: Some(ParentLineId {
                    path: path.to_owned(),
                    line_number,
                }),
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
            removals: BTreeMap::new(),
            parent_paths: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn from_parent(files: &BTreeMap<String, Option<String>>) -> Self {
        MutationLineage {
            files: files
                .iter()
                .map(|(path, content)| (path.clone(), parent_lines(path, content.as_deref())))
                .collect(),
            removals: BTreeMap::new(),
            parent_paths: files
                .iter()
                .filter(|(_, content)| content.is_some())
                .map(|(path, _)| (path.clone(), path.clone()))
                .collect(),
        }
    }

    pub fn reset_all(&mut self, files: &BTreeMap<String, Option<String>>) {
        *self = MutationLineage::from_baseline(files);
    }

    pub fn apply(
        &mut self,
        patch: &ParsedPatch,
        origin: &TransitionOrigin,
    ) -> Result<(), LineageError> {
        let mut effects = TransitionEffects::default();
        for file in &patch.files {
            self.apply_file(file, origin, &mut effects)?;
        }
        self.invalidate_restored_removals(&effects.added);
        for (identity, content) in effects.removed {
            self.removals.insert(
                identity,
                RemovalRecord {
                    content,
                    provenance: origin.provenance(),
                },
            );
        }
        Ok(())
    }

    fn invalidate_restored_removals(&mut self, added: &[(String, String)]) {
        if self.removals.is_empty() {
            return;
        }
        let mut restorable: BTreeSet<(&str, &str)> = BTreeSet::new();
        for (path, content) in added {
            restorable.insert((path, content));
            if let Some(parent_path) = self.parent_paths.get(path) {
                restorable.insert((parent_path, content));
            }
        }
        for (identity, record) in &mut self.removals {
            if restorable.contains(&(identity.path.as_str(), record.content.as_str())) {
                record.provenance = LineProvenance::Unknown;
            }
        }
    }

    fn apply_file(
        &mut self,
        file: &PatchFileChange,
        origin: &TransitionOrigin,
        effects: &mut TransitionEffects,
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

        if file.kind == FileChangeKind::Added && !old.is_empty() {
            return Err(LineageError {
                path: logical_path,
                reason: String::from("added file is already tracked with content"),
            });
        }

        let new = apply_hunks(&logical_path, &old, file, origin, effects)?;

        let renamed = !file.old_path.is_empty()
            && !file.new_path.is_empty()
            && file.old_path != file.new_path;
        if renamed {
            match self.parent_paths.remove(&file.old_path) {
                Some(parent_path) if tracks_source => {
                    self.parent_paths.insert(file.new_path.clone(), parent_path);
                }
                _ => {
                    self.parent_paths.remove(&file.new_path);
                }
            }
        }
        if tracks_source && file.old_path != file.new_path {
            self.files.insert(file.old_path.clone(), Vec::new());
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

    pub fn tracked_paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    #[must_use]
    pub fn matches_content(&self, path: &str, content: Option<&str>) -> bool {
        let Some(tracked) = self.files.get(path) else {
            return false;
        };
        let expected: Vec<&str> = content
            .map(|text| text.lines().collect())
            .unwrap_or_default();
        tracked.len() == expected.len()
            && tracked
                .iter()
                .zip(&expected)
                .all(|(line, expected)| line.content == *expected)
    }

    pub fn distrust_removals(&mut self, path: &str) {
        let parent_path = self.parent_paths.get(path).cloned();
        for (identity, record) in &mut self.removals {
            if identity.path == path || parent_path.as_deref() == Some(identity.path.as_str()) {
                record.provenance = LineProvenance::Unknown;
            }
        }
    }

    #[must_use]
    pub fn parent_path_of(&self, path: &str) -> Option<&str> {
        self.parent_paths.get(path).map(String::as_str)
    }

    #[cfg(test)]
    #[must_use]
    pub fn removed_identities(&self) -> Vec<(ParentLineId, LineProvenance)> {
        self.removals
            .iter()
            .map(|(identity, record)| (identity.clone(), record.provenance.clone()))
            .collect()
    }

    fn removals_of(&self, parent_path: &str) -> BTreeMap<u64, &RemovalRecord> {
        self.removals
            .iter()
            .filter(|(identity, _)| identity.path == parent_path)
            .map(|(identity, record)| (identity.line_number, record))
            .collect()
    }

    #[must_use]
    pub fn committed_removals(
        &self,
        current_path: &str,
        parent_path: &str,
        committed: &[(u64, &str)],
    ) -> Vec<LineProvenance> {
        if self.parent_path_of(current_path) != Some(parent_path) {
            return vec![LineProvenance::Unknown; committed.len()];
        }
        let recorded = self.removals_of(parent_path);
        let mut committed_numbers: BTreeSet<u64> = BTreeSet::new();
        let mut repeated: BTreeSet<u64> = BTreeSet::new();
        for (line_number, _) in committed {
            if !committed_numbers.insert(*line_number) {
                repeated.insert(*line_number);
            }
        }

        let mut ambiguous: BTreeSet<&str> = BTreeSet::new();
        for (line_number, record) in &recorded {
            if !committed_numbers.contains(line_number) {
                ambiguous.insert(&record.content);
            }
        }
        for (line_number, content) in committed {
            if !recorded.contains_key(line_number) {
                ambiguous.insert(content);
            }
        }

        committed
            .iter()
            .map(|(line_number, content)| match recorded.get(line_number) {
                Some(record)
                    if record.content == *content
                        && !ambiguous.contains(content)
                        && !repeated.contains(line_number) =>
                {
                    record.provenance.clone()
                }
                _ => LineProvenance::Unknown,
            })
            .collect()
    }
}

fn apply_hunks(
    path: &str,
    old: &[ProvenanceLine],
    file: &PatchFileChange,
    origin: &TransitionOrigin,
    effects: &mut TransitionEffects,
) -> Result<Vec<ProvenanceLine>, LineageError> {
    let mut hunks: Vec<&PatchHunk> = file.hunks.iter().collect();
    hunks.sort_by_key(|hunk| (hunk.old_start, hunk.new_start));

    let mut new: Vec<ProvenanceLine> = Vec::new();
    let mut cursor: usize = 0;

    for hunk in hunks {
        cursor = apply_one_hunk(path, old, hunk, origin, &mut new, cursor, effects)?;
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
    effects: &mut TransitionEffects,
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
    effects.record_removed(region, &removed_indices);

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
                provenance: origin.provenance(),
                parent_identity: None,
            });
            effects.record_added(path, content);
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
