#![allow(dead_code)]

use std::fmt;

pub const OTEL_TARGET: &str = "sce::otel";
pub const OTEL_LABEL_MAX_LEN: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtelName {
    Command,
    MutationScopeCoordinate,
    WorktreeLock,
    Reconciliation,
    GitSnapshot,
    DbOperation,
    Sync,
}

impl OtelName {
    pub const ALL: [Self; 7] = [
        Self::Command,
        Self::MutationScopeCoordinate,
        Self::WorktreeLock,
        Self::Reconciliation,
        Self::GitSnapshot,
        Self::DbOperation,
        Self::Sync,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "sce.command",
            Self::MutationScopeCoordinate => "sce.mutation_scope.coordinate",
            Self::WorktreeLock => "sce.worktree.lock",
            Self::Reconciliation => "sce.reconciliation",
            Self::GitSnapshot => "sce.git.snapshot",
            Self::DbOperation => "sce.db.operation",
            Self::Sync => "sce.sync",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtelAttributeKey {
    CommandName,
    Outcome,
    ErrorCategory,
    OperationType,
    DurationMs,
    Count,
    Attempts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OtelValueKind {
    Label,
    Number,
}

impl OtelAttributeKey {
    pub const ALL: [Self; 7] = [
        Self::CommandName,
        Self::Outcome,
        Self::ErrorCategory,
        Self::OperationType,
        Self::DurationMs,
        Self::Count,
        Self::Attempts,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CommandName => "sce.command.name",
            Self::Outcome => "sce.outcome",
            Self::ErrorCategory => "sce.error.category",
            Self::OperationType => "sce.operation.type",
            Self::DurationMs => "sce.duration_ms",
            Self::Count => "sce.count",
            Self::Attempts => "sce.attempts",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == key)
    }

    fn kind(self) -> OtelValueKind {
        match self {
            Self::CommandName | Self::Outcome | Self::ErrorCategory | Self::OperationType => {
                OtelValueKind::Label
            }
            Self::DurationMs | Self::Count | Self::Attempts => OtelValueKind::Number,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtelRawValue<'a> {
    Number(u64),
    Text(&'a str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtelLabel(String);

impl OtelLabel {
    pub fn parse(value: &str) -> Option<Self> {
        let valid = !value.is_empty()
            && value.len() <= OTEL_LABEL_MAX_LEN
            && value
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'));
        valid.then(|| Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OtelLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OtelValue {
    Number(u64),
    Label(OtelLabel),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtelRecord {
    pub name: OtelName,
    pub attributes: Vec<(OtelAttributeKey, OtelValue)>,
}

pub fn admit_otel_attribute(
    key: &str,
    value: OtelRawValue<'_>,
) -> Option<(OtelAttributeKey, OtelValue)> {
    let key = OtelAttributeKey::parse(key)?;
    let value = match (key.kind(), value) {
        (OtelValueKind::Number, OtelRawValue::Number(number)) => OtelValue::Number(number),
        (OtelValueKind::Label, OtelRawValue::Text(text)) => {
            OtelValue::Label(OtelLabel::parse(text)?)
        }
        _ => return None,
    };
    Some((key, value))
}

pub fn admit_otel_record(
    target: &str,
    name: &str,
    attributes: &[(&str, OtelRawValue<'_>)],
) -> Option<OtelRecord> {
    if target != OTEL_TARGET {
        return None;
    }

    let name = OtelName::parse(name)?;
    let attributes = attributes
        .iter()
        .filter_map(|(key, value)| admit_otel_attribute(key, *value))
        .collect();
    Some(OtelRecord { name, attributes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::observability::tracing_boundary::SCE_TRACING_TARGET;

    const SECRET: &str = "sk-live-SECRET-TOKEN-9f8e7d";
    const SENSITIVE_PATH: &str = "/home/victim/.ssh/id_ed25519";

    #[test]
    fn tracing_boundary_b_admits_only_otel_target_and_enumerated_names() {
        let ok = admit_otel_record(
            OTEL_TARGET,
            "sce.command",
            &[
                ("sce.outcome", OtelRawValue::Text("success")),
                ("sce.duration_ms", OtelRawValue::Number(12)),
            ],
        )
        .expect("admitted");
        assert_eq!(ok.name, OtelName::Command);
        assert_eq!(ok.attributes.len(), 2);

        assert!(admit_otel_record(SCE_TRACING_TARGET, "sce.command", &[]).is_none());
        assert!(admit_otel_record("sce::services::resilience", "sce.command", &[]).is_none());
        assert!(admit_otel_record(OTEL_TARGET, "sce.arbitrary", &[]).is_none());
        assert!(admit_otel_record(OTEL_TARGET, SECRET, &[]).is_none());
    }

    #[test]
    fn tracing_boundary_b_drops_disallowed_keys_and_unbounded_or_mistyped_values() {
        let oversized = "a".repeat(OTEL_LABEL_MAX_LEN + 1);
        let record = admit_otel_record(
            OTEL_TARGET,
            "sce.db.operation",
            &[
                ("exception.message", OtelRawValue::Text("boom")),
                ("exception.stacktrace", OtelRawValue::Text("at main")),
                ("path", OtelRawValue::Text(SENSITIVE_PATH)),
                ("sce.outcome", OtelRawValue::Text(&oversized)),
                ("sce.error.category", OtelRawValue::Text(SENSITIVE_PATH)),
                ("sce.operation.type", OtelRawValue::Text("Has Space")),
                ("sce.duration_ms", OtelRawValue::Text("12")),
                ("sce.count", OtelRawValue::Number(3)),
                ("sce.operation.type", OtelRawValue::Text("db.execute_query")),
            ],
        )
        .expect("admitted");

        let keys: Vec<_> = record.attributes.iter().map(|(key, _)| *key).collect();
        assert_eq!(
            keys,
            vec![OtelAttributeKey::Count, OtelAttributeKey::OperationType]
        );
    }

    #[test]
    fn tracing_boundary_b_policy_names_and_keys_have_stable_shapes() {
        for name in OtelName::ALL {
            assert_eq!(OtelName::parse(name.as_str()), Some(name));
        }
        for key in OtelAttributeKey::ALL {
            assert_eq!(OtelAttributeKey::parse(key.as_str()), Some(key));
            assert!(!key.as_str().contains("exception"));
        }
    }
}
