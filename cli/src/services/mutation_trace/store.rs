//! Domain<->SQL codecs for the mutation-cursor persistence layer.
//!
//! These codecs are the only translation between `super::types` domain
//! values and the `TEXT`/`BLOB` representations `cli/migrations/agent-trace-
//! repository/003_mutation_trace_protocol.sql` constrains those columns to.
//! Every codec here is an explicit function over a fixed set of variants — no
//! codec derives from `Debug` or a serde representation, so a variant rename
//! cannot silently change the durable encoding.
//!
//! This module carries no query, projection, or commit logic yet (see
//! `mutation-cursor-store-persistence` plan tasks T03+); it only establishes
//! the byte- and string-level encodings later tasks build on.

use anyhow::{bail, Result};

use super::types::{ActorKind, Attribution, Boundary, FailureKind, ScopeStatus};

/// Encodes a worktree/event revision as the 8-byte big-endian `BLOB` stored
/// by every `revision` column in migration `003`.
pub fn encode_revision(revision: u64) -> [u8; 8] {
    revision.to_be_bytes()
}

/// Decodes a worktree/event revision from the 8-byte big-endian `BLOB`
/// migration `003`'s `CHECK (typeof(revision) = 'blob' AND length(revision)
/// = 8)` constraint guarantees on every stored value.
pub fn decode_revision(blob: &[u8]) -> Result<u64> {
    let bytes: [u8; 8] = blob.try_into().map_err(|_| {
        anyhow::anyhow!("revision blob must be exactly 8 bytes, got {}", blob.len())
    })?;
    Ok(u64::from_be_bytes(bytes))
}

/// Encodes an [`ActorKind`] as the `mutation_trace_scopes.actor_kind` `TEXT`
/// value migration `003`'s `CHECK (actor_kind IN (...))` allow-list expects.
pub fn encode_actor_kind(actor_kind: ActorKind) -> &'static str {
    match actor_kind {
        ActorKind::ClaudeCode => "claude_code",
        ActorKind::Codex => "codex",
        ActorKind::OpenCode => "opencode",
        ActorKind::Pi => "pi",
    }
}

/// Decodes an [`ActorKind`] from `mutation_trace_scopes.actor_kind`.
pub fn decode_actor_kind(value: &str) -> Result<ActorKind> {
    match value {
        "claude_code" => Ok(ActorKind::ClaudeCode),
        "codex" => Ok(ActorKind::Codex),
        "opencode" => Ok(ActorKind::OpenCode),
        "pi" => Ok(ActorKind::Pi),
        other => bail!("unrecognized actor_kind: {other:?}"),
    }
}

/// Encodes a [`FailureKind`] as the `failure_kind` `TEXT` value migration
/// `003` constrains `mutation_trace_worktrees.failure_kind` and
/// `mutation_trace_events.failure_kind` to.
pub fn encode_failure_kind(failure_kind: FailureKind) -> &'static str {
    match failure_kind {
        FailureKind::Healthy => "healthy",
        FailureKind::SnapshotFailure => "snapshot_failure",
    }
}

/// Decodes a [`FailureKind`] from a `failure_kind` column.
pub fn decode_failure_kind(value: &str) -> Result<FailureKind> {
    match value {
        "healthy" => Ok(FailureKind::Healthy),
        "snapshot_failure" => Ok(FailureKind::SnapshotFailure),
        other => bail!("unrecognized failure_kind: {other:?}"),
    }
}

/// Encodes a [`ScopeStatus`] as the `mutation_trace_scopes.status` `TEXT`
/// value migration `003`'s `CHECK (status IN (...))` allow-list expects.
pub fn encode_scope_status(status: ScopeStatus) -> &'static str {
    match status {
        ScopeStatus::NeverSeen => "never_seen",
        ScopeStatus::Active => "active",
        ScopeStatus::Closed => "closed",
        ScopeStatus::Abandoned => "abandoned",
    }
}

/// Decodes a [`ScopeStatus`] from `mutation_trace_scopes.status`.
pub fn decode_scope_status(value: &str) -> Result<ScopeStatus> {
    match value {
        "never_seen" => Ok(ScopeStatus::NeverSeen),
        "active" => Ok(ScopeStatus::Active),
        "closed" => Ok(ScopeStatus::Closed),
        "abandoned" => Ok(ScopeStatus::Abandoned),
        other => bail!("unrecognized scope status: {other:?}"),
    }
}

/// [`Attribution`]'s discriminant, decoupled from its `AiExclusive` payload
/// (`ScopeId`). Reconstructing a full [`Attribution`] from a persisted row
/// also needs `attribution_scope_id`, which is a `mutation_trace_events`
/// query concern owned by a later task, not by this codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttributionKind {
    IneligibleUnscoped,
    AiExclusive,
    AiContended,
}

/// The discriminant of an [`Attribution`] value.
pub fn attribution_kind(attribution: &Attribution) -> AttributionKind {
    match attribution {
        Attribution::IneligibleUnscoped => AttributionKind::IneligibleUnscoped,
        Attribution::AiExclusive(_) => AttributionKind::AiExclusive,
        Attribution::AiContended => AttributionKind::AiContended,
    }
}

/// Encodes an [`AttributionKind`] as the
/// `mutation_trace_events.attribution_kind` `TEXT` value migration `003`'s
/// `CHECK (attribution_kind IN (...))` allow-list expects.
pub fn encode_attribution_kind(kind: AttributionKind) -> &'static str {
    match kind {
        AttributionKind::IneligibleUnscoped => "ineligible_unscoped",
        AttributionKind::AiExclusive => "ai_exclusive",
        AttributionKind::AiContended => "ai_contended",
    }
}

/// Decodes an [`AttributionKind`] from `mutation_trace_events.attribution_kind`.
pub fn decode_attribution_kind(value: &str) -> Result<AttributionKind> {
    match value {
        "ineligible_unscoped" => Ok(AttributionKind::IneligibleUnscoped),
        "ai_exclusive" => Ok(AttributionKind::AiExclusive),
        "ai_contended" => Ok(AttributionKind::AiContended),
        other => bail!("unrecognized attribution_kind: {other:?}"),
    }
}

/// [`Boundary`]'s discriminant, decoupled from its `scope`/`event`/`worktree`
/// payload. Reconstructing a full [`Boundary`] from a persisted row also
/// needs `boundary_scope_id`/`boundary_event_id`, which is a
/// `mutation_trace_events` query concern owned by a later task, not by this
/// codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryKind {
    Start,
    Advance,
    Close,
    Flush,
}

/// The discriminant of a [`Boundary`] value.
pub fn boundary_kind(boundary: &Boundary) -> BoundaryKind {
    match boundary {
        Boundary::Start { .. } => BoundaryKind::Start,
        Boundary::Advance { .. } => BoundaryKind::Advance,
        Boundary::Close { .. } => BoundaryKind::Close,
        Boundary::Flush { .. } => BoundaryKind::Flush,
    }
}

/// Encodes a [`BoundaryKind`] as the `mutation_trace_events.boundary_kind`
/// `TEXT` value migration `003`'s `CHECK (boundary_kind IN (...))`
/// allow-list expects.
pub fn encode_boundary_kind(kind: BoundaryKind) -> &'static str {
    match kind {
        BoundaryKind::Start => "start",
        BoundaryKind::Advance => "advance",
        BoundaryKind::Close => "close",
        BoundaryKind::Flush => "flush",
    }
}

/// Decodes a [`BoundaryKind`] from `mutation_trace_events.boundary_kind`.
pub fn decode_boundary_kind(value: &str) -> Result<BoundaryKind> {
    match value {
        "start" => Ok(BoundaryKind::Start),
        "advance" => Ok(BoundaryKind::Advance),
        "close" => Ok(BoundaryKind::Close),
        "flush" => Ok(BoundaryKind::Flush),
        other => bail!("unrecognized boundary_kind: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::mutation_trace::types::{EventId, ScopeId};

    #[test]
    fn revision_round_trips_at_boundary_values() {
        for revision in [0u64, 1, i64::MAX as u64, (i64::MAX as u64) + 1, u64::MAX] {
            let encoded = encode_revision(revision);
            assert_eq!(encoded.len(), 8);
            assert_eq!(decode_revision(&encoded).unwrap(), revision);
        }
    }

    #[test]
    fn decode_revision_rejects_wrong_length() {
        assert!(decode_revision(&[0u8; 7]).is_err());
        assert!(decode_revision(&[0u8; 9]).is_err());
    }

    #[test]
    fn actor_kind_round_trips_every_variant() {
        for actor_kind in [
            ActorKind::ClaudeCode,
            ActorKind::Codex,
            ActorKind::OpenCode,
            ActorKind::Pi,
        ] {
            let encoded = encode_actor_kind(actor_kind);
            assert_eq!(decode_actor_kind(encoded).unwrap(), actor_kind);
        }
    }

    #[test]
    fn decode_actor_kind_rejects_unknown_value() {
        assert!(decode_actor_kind("unknown").is_err());
    }

    #[test]
    fn failure_kind_round_trips_every_variant() {
        for failure_kind in [FailureKind::Healthy, FailureKind::SnapshotFailure] {
            let encoded = encode_failure_kind(failure_kind);
            assert_eq!(decode_failure_kind(encoded).unwrap(), failure_kind);
        }
    }

    #[test]
    fn decode_failure_kind_rejects_unknown_value() {
        assert!(decode_failure_kind("unknown").is_err());
    }

    #[test]
    fn scope_status_round_trips_every_variant() {
        for status in [
            ScopeStatus::NeverSeen,
            ScopeStatus::Active,
            ScopeStatus::Closed,
            ScopeStatus::Abandoned,
        ] {
            let encoded = encode_scope_status(status);
            assert_eq!(decode_scope_status(encoded).unwrap(), status);
        }
    }

    #[test]
    fn decode_scope_status_rejects_unknown_value() {
        assert!(decode_scope_status("unknown").is_err());
    }

    #[test]
    fn attribution_kind_round_trips_every_variant() {
        let ineligible = Attribution::IneligibleUnscoped;
        let exclusive = Attribution::AiExclusive(ScopeId("scope-1".to_string()));
        let contended = Attribution::AiContended;

        for attribution in [&ineligible, &exclusive, &contended] {
            let kind = attribution_kind(attribution);
            let encoded = encode_attribution_kind(kind);
            assert_eq!(decode_attribution_kind(encoded).unwrap(), kind);
        }

        assert_eq!(attribution_kind(&exclusive), AttributionKind::AiExclusive);
    }

    #[test]
    fn decode_attribution_kind_rejects_unknown_value() {
        assert!(decode_attribution_kind("unknown").is_err());
    }

    #[test]
    fn boundary_kind_round_trips_every_variant() {
        let start = Boundary::Start {
            scope: ScopeId("scope-1".to_string()),
            event: EventId("event-1".to_string()),
        };
        let advance = Boundary::Advance {
            scope: ScopeId("scope-1".to_string()),
            event: EventId("event-2".to_string()),
        };
        let close = Boundary::Close {
            scope: ScopeId("scope-1".to_string()),
            event: EventId("event-3".to_string()),
        };
        let flush = Boundary::Flush {
            worktree: crate::services::mutation_trace::types::WorktreeId("wt-1".to_string()),
        };

        for boundary in [&start, &advance, &close, &flush] {
            let kind = boundary_kind(boundary);
            let encoded = encode_boundary_kind(kind);
            assert_eq!(decode_boundary_kind(encoded).unwrap(), kind);
        }
    }

    #[test]
    fn decode_boundary_kind_rejects_unknown_value() {
        assert!(decode_boundary_kind("unknown").is_err());
    }
}
