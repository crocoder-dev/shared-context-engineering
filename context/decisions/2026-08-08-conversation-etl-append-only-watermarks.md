# Decision: Treat conversation source fields as append-only under table-specific ID watermarks

Date: 2026-08-08
Status: Accepted
Plan: `context/plans/incremental-conversation-etl.md`
Task: T02, T03, T04, T05

## Context

The repository-scoped Agent Trace source stores conversation messages and parts
that are incrementally copied into the Agent Trace DWH. The source uses local
integer IDs, while the DWH must preserve source lineage, tolerate parts arriving
before messages, and allow messages and parts to advance independently. The
completed ETL implementation has no safe way to discover historical edits after
an integer-ID watermark, and source inspection found only schema-maintenance
`updated_at` trigger bodies issuing `UPDATE messages` or `UPDATE parts`; active
capture uses insert helpers.

## Decision

Treat the synchronized message role/timestamp and part session/message/type/text/
hash/timestamp fields as append-only and immutable for ETL purposes, using
independent integer-ID watermarks for the `messages` and `parts` source tables;
update CDC is deliberately out of scope.

## Rationale

Table-specific watermarks let either pipeline progress without coupling its
transaction or progress state to the other table, while append-only semantics
make the cursor complete and deterministic. The source audit supports this
assumption, and an intentional future source update must be addressed by an
explicit CDC design or architecture decision rather than silently being missed.

## Alternatives considered

- **Add update CDC or rescan rows at or below the watermark** — not selected
  because no update requirement exists in the current capture path and it would
  add a separate change-tracking contract to this append-oriented bridge.
- **Use one conversation-level watermark or transaction** — not selected
  because messages and parts have different source tables, local IDs, and valid
  out-of-order ingestion requirements.
- **Use timestamp cursors** — not selected because timestamps are not unique and
  cannot provide deterministic, lossless progress.

## Compatibility and risks

- Existing source writers remain compatible because they append messages and
  parts; the DWH contract does not alter the live-capture schema or writers.
- A future intentional update to synchronized fields would be ignored after its
  row ID is watermarked unless update CDC or an explicit replay strategy is
  designed; this risk is mitigated by documenting the assumption and requiring
  an architectural decision before changing it.

## Guardrails

- Keep `messages` and `parts` watermarks separate and keyed by repository,
  source instance, and source table.
- Preserve source row ordering by integer ID, and do not replace it with a
  timestamp-only cursor.
- Do not add a parent-message foreign key or couple parts ingestion to message
  existence.
- Do not introduce update CDC implicitly; future source updates require an
  explicit design and validation contract.

## Consequences

- Message and part ETL runs can progress independently and replay complete
  failed batches atomically.
- Parts may be loaded before their parent message, and source-lineage local IDs
  remain safe across independently created source databases.
- Historical updates to rows at or below a committed watermark are outside the
  current ETL guarantee.

## Follow-up

None.

## References

- Plan: [`incremental-conversation-etl`](../plans/incremental-conversation-etl.md)
- Task: `T02, T03, T04, T05`
- Current-state context: [`agent-trace-etl.md`](../sce/agent-trace-etl.md), [`conversation-messages-etl.md`](../sce/conversation-messages-etl.md), [`conversation-parts-etl.md`](../sce/conversation-parts-etl.md), [`conversation-etl.md`](../sce/conversation-etl.md)
- Evidence: [`incremental-conversation-etl.md`](../plans/incremental-conversation-etl.md), [`conversation_messages_etl.rs`](../../cli/src/services/conversation_messages_etl.rs), [`conversation_parts_etl.rs`](../../cli/src/services/conversation_parts_etl.rs)
- Related decision: [`Separate Agent Trace DWH destination schema with a source-instance-scoped identity contract`](2026-08-08-agent-trace-dwh-schema-identity-contract.md)
- Related decision: [`Single-owner, disposable Turso Sync replica boundary for the Agent Trace DWH`](2026-08-08-agent-trace-dwh-turso-sync-replica-ownership.md)
