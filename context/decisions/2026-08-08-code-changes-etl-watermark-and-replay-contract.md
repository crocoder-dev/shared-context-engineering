# Decision: Use source-lineage ID watermarks with atomic strict replay for code-change ETL

Date: 2026-08-08
Status: Accepted
Plan: `context/plans/incremental-code-changes-etl.md`
Task: T01, T02, T03, T04, T05

## Context

Repository-scoped Agent Trace `diff_traces` rows use local integer IDs and may
be copied into the append-oriented DWH from independently created source
instances. The code-change bridge must preserve source lineage, avoid holding
source snapshots during transformation, and make failed batches replayable
without duplicate facts or silently overwriting an existing identity.

## Decision

The code-change ETL uses an independent `(repository_id, source_instance_id,
diff_traces)` watermark over ascending `diff_traces.id`; it strictly transforms
an entire bounded batch before destination work, then validates or inserts
`code_changes` rows and advances that watermark in one destination transaction.

## Rationale

Integer source IDs provide deterministic, lossless progress without timestamp
ambiguity, while source-instance scoping allows equal local IDs from separate
source databases to coexist. Transforming before opening the destination
transaction prevents malformed payloads from creating partial state. Comparing
all synchronized and derived fields on replay makes conflicts explicit, and the
single transaction preserves dimensions, facts, and progress as one replayable
unit.

## Alternatives considered

- **Timestamp-based progress** — not selected because timestamps are not unique
  progress identifiers and can miss or reorder source rows.
- **Conflict-ignore or overwrite loading** — not selected because either hides
  source inconsistencies or mutates an established fact without evidence.
- **Advancing the watermark per row** — not selected because a later batch
  failure would leave partial facts and progress that cannot be replayed as one
  unit.

## Compatibility and risks

- The contract is additive to the existing source capture schema and DWH
  destination; hooks and transport synchronization remain outside this ETL.
- A malformed or unsupported source payload blocks the batch until the source
  row is corrected, which is intentional strict behavior; the unchanged
  watermark permits safe replay.
- The local source ID is not globally unique, so every code-change identity and
  watermark lookup must retain `source_instance_id` as a guardrail.

## Guardrails

- Extract only the authored ordered `diff_traces` projection and never use a
  timestamp or separate `MAX(id)` query for progress.
- End the plain source read transaction before parsing, hashing, metrics, or
  destination work; retry only the shared transient source contention cases.
- Use exactly `(repository_id, source_instance_id, source_diff_trace_id)` for
  code-change identity and compare every synchronized and derived field before
  counting a replay.
- Keep pull/push, credentials, CLI orchestration, and message-level attribution
  outside the ETL boundary; `session_id` is the only conversation relationship.

## Consequences

- Code-change ingestion is deterministic, independently watermarked, and safe
  to rerun after transformation, integrity, or destination failures.
- Source writers can continue while ETL transforms extracted owned values, and
  failed batches leave dimensions, facts, and progress unchanged.
- Future changes to source update semantics or message-level attribution need a
  separate design rather than weakening this append-only contract.

## Follow-up

None.

## References

- Plan: [`incremental-code-changes-etl`](../plans/incremental-code-changes-etl.md)
- Task: `T01, T02, T03, T04, T05`
- Current-state context: [`code-changes-etl.md`](../sce/code-changes-etl.md), [`agent-trace-etl.md`](../sce/agent-trace-etl.md), [`agent-trace-dwh-db.md`](../sce/agent-trace-dwh-db.md)
- Evidence: [`code_changes_etl.rs`](../../cli/src/services/code_changes_etl.rs), [`code_changes_etl` tests](../../cli/src/services/code_changes_etl.rs)
- Related decision: [`agent-trace-dwh-schema-identity-contract`](2026-08-08-agent-trace-dwh-schema-identity-contract.md)
- Related decision: [`agent-trace-dwh-turso-sync-replica-ownership`](2026-08-08-agent-trace-dwh-turso-sync-replica-ownership.md)
