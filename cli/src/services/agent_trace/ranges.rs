use std::collections::BTreeMap;

use super::{Contributor, ContributorType, LineAttribution};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum RangeAttribution {
    Ai {
        session_id: Option<String>,
        model_id: Option<String>,
    },
    NonExclusive,
    Unknown,
}

impl RangeAttribution {
    pub(super) fn from_decision(decision: Option<&LineAttribution>) -> Self {
        match decision {
            Some(LineAttribution::Ai(line)) => Self::Ai {
                session_id: line
                    .provenance
                    .session_id
                    .clone()
                    .filter(|session_id| !session_id.is_empty()),
                model_id: line.provenance.model_id.clone(),
            },
            Some(LineAttribution::NonExclusive) => Self::NonExclusive,
            Some(LineAttribution::Unknown) | None => Self::Unknown,
        }
    }

    pub(super) fn contributor(&self) -> Contributor {
        match self {
            Self::Ai { model_id, .. } => Contributor {
                kind: ContributorType::Ai,
                model_id: model_id.clone(),
            },
            Self::NonExclusive | Self::Unknown => Contributor {
                kind: ContributorType::Unknown,
                model_id: None,
            },
        }
    }

    fn conversation_key(&self) -> ConversationKey {
        match self {
            Self::Ai {
                session_id: Some(session_id),
                ..
            } => ConversationKey::Session(session_id.clone()),
            Self::Ai {
                session_id: None,
                model_id,
            } => ConversationKey::Sessionless {
                model_id: model_id.clone(),
            },
            Self::NonExclusive => ConversationKey::NonExclusive,
            Self::Unknown => ConversationKey::Unknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AttributedLine {
    pub(super) line_number: u64,
    pub(super) attribution: RangeAttribution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AttributedRange {
    pub(super) start_line: u64,
    pub(super) end_line: u64,
    pub(super) attribution: RangeAttribution,
}

pub(super) fn coalesce_ranges(lines: &[AttributedLine]) -> Vec<AttributedRange> {
    let mut ranges: Vec<AttributedRange> = Vec::new();
    for line in lines {
        match ranges.last_mut() {
            Some(range)
                if range.end_line + 1 == line.line_number
                    && range.attribution == line.attribution =>
            {
                range.end_line = line.line_number;
            }
            _ => ranges.push(AttributedRange {
                start_line: line.line_number,
                end_line: line.line_number,
                attribution: line.attribution.clone(),
            }),
        }
    }
    ranges
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ConversationKey {
    Session(String),
    Sessionless { model_id: Option<String> },
    NonExclusive,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConversationGroup {
    key: ConversationKey,
    pub(super) ranges: Vec<AttributedRange>,
}

impl ConversationGroup {
    pub(super) fn session_id(&self) -> Option<&str> {
        match &self.key {
            ConversationKey::Session(session_id) => Some(session_id),
            ConversationKey::Sessionless { .. }
            | ConversationKey::NonExclusive
            | ConversationKey::Unknown => None,
        }
    }

    fn is_homogeneous(&self) -> bool {
        self.ranges
            .windows(2)
            .all(|pair| pair[0].attribution == pair[1].attribution)
    }

    pub(super) fn contributor(&self) -> Option<Contributor> {
        let first = self.ranges.first()?;
        if self.is_homogeneous() {
            return Some(first.attribution.contributor());
        }
        let kind = first.attribution.contributor().kind;
        self.ranges
            .iter()
            .all(|range| range.attribution.contributor().kind == kind)
            .then_some(Contributor {
                kind,
                model_id: None,
            })
    }

    pub(super) fn range_contributor(&self, range: &AttributedRange) -> Option<Contributor> {
        (!self.is_homogeneous()).then(|| range.attribution.contributor())
    }
}

pub(super) fn group_conversations(ranges: Vec<AttributedRange>) -> Vec<ConversationGroup> {
    let mut groups: BTreeMap<ConversationKey, Vec<AttributedRange>> = BTreeMap::new();
    for range in ranges {
        groups
            .entry(range.attribution.conversation_key())
            .or_default()
            .push(range);
    }
    let mut groups: Vec<ConversationGroup> = groups
        .into_iter()
        .map(|(key, ranges)| ConversationGroup { key, ranges })
        .collect();
    groups.sort_by_key(|group| group.ranges.first().map(|range| range.start_line));
    groups
}
