# Decision: Separate Agent Trace DWH destination schema with a source-instance-scoped identity contract

Date: 2026-08-08
Status: Accepted
Plan: `context/plans/agent-trace-dwh-schema-identity-contract.md`
Task: T01, T02

## Context

Agent Trace data currently lives only in the repository-scoped `agent-trace.db`
live-capture schema (`cli/migrations/agent-trace-repository/`), written directly
by hook runtime. A future ETL consumer needs a separate, append-oriented
destination for extracted conversation, trace, and code-change data that can be
re-ingested from independently created source databases (for example after a
machine reset or checkout re-clone) without producing duplicate rows, and that
must tolerate out-of-order or partial batch ingestion across fact tables. No
destination schema, adapter boundary, or identity contract for this existed
before this plan.

## Decision

Introduce a dedicated Agent Trace DWH destination schema
(`cli/migrations/agent-trace-dwh/001_dwh_schema.sql`) and a separate
explicit-path-only `AgentTraceDwhDb = TursoDb<AgentTraceDwhDbSpec>` adapter,
distinct from the repository-scoped source schema, with two deliberately
different uniqueness scopes: deterministic logical identities (`messages` on
`(repository_id, session_id, message_id)`, `agent_traces` on `(repository_id,
agent_trace_id)`) exclude `source_instance_id` so re-ingesting the same logical
event from an independently created source database stays idempotent, while raw
local autoincrement source row IDs (`message_parts.source_part_id`,
`code_changes.source_diff_trace_id`) are scoped by `(repository_id,
source_instance_id, <local id>)` since those integers are not stable across
independently created source databases.

## Rationale

Splitting source and destination schemas keeps the live-capture path free of
ETL/warehouse concerns and lets the DWH schema evolve independently. Scoping
identity by whether a value is deterministically reproducible (session/message
IDs, a derived `agent_trace_id`) versus a raw local autoincrement integer is the
only way to get both idempotent re-ingestion of logical events and safe
coexistence of unrelated local IDs across independently created source
databases and repositories, without introducing foreign keys that would block
out-of-order or partial ingestion.

## Alternatives considered

- **Single shared schema for source and destination** — would couple live-capture
  hook write paths to future ETL/warehouse concerns and force one physical
  schema to serve two different consistency and ingestion-order requirements.
- **Uniform identity scoping (always including `source_instance_id`)** — simpler,
  but would make idempotent re-ingestion from an independently created source
  database impossible for deterministic logical events, producing duplicate
  messages/traces on every source-database recreation.
- **Foreign keys enforcing ingestion order across fact tables** — would provide
  referential integrity but blocks the required out-of-order/partial batch
  ingestion across independently created source databases, so it was rejected.

## Compatibility and risks

- The DWH schema and adapter are net-new and not wired into any lifecycle
  provider, doctor/setup flow, or CLI command, so this decision has no runtime
  compatibility impact yet.
- Risk: a future ETL implementation could violate the identity contract (for
  example scoping a deterministic identity by `source_instance_id`) and silently
  reintroduce duplicate rows on re-ingestion; mitigated by the schema-level
  unique indexes enforcing both scopes and by the identity contract being
  recorded in durable context for future ETL work to consult.

## Guardrails

- Hash columns (`text_sha256`, `trace_json_sha256`, `patch_sha256`) store
  integrity hashes without this decision defining how they are computed.
- No foreign keys constrain ingestion order between DWH fact tables; this must
  hold for any future table added to the DWH schema.
- The DWH adapter must not own a sync URL, credentials, ETL state transitions,
  bridge locking, or CLI lifecycle behavior; that remains out of scope until a
  future ETL/wiring plan addresses it explicitly.

## Consequences

- Future ETL implementation can re-run extraction from a recreated source
  database for the same repository without producing duplicate logical rows.
- The same local source integer ID (a `parts.id` or `diff_traces.id`) is
  expected and allowed to coexist across different source instances and
  repositories in the DWH.
- Any future DWH table must decide up front which of the two identity scopes
  its natural key follows, since the schema has no generic fallback.

## Follow-up

None.

## References

- Plan: [`agent-trace-dwh-schema-identity-contract`](../plans/agent-trace-dwh-schema-identity-contract.md)
- Task: T01, T02
- Current-state context: [`agent-trace-dwh-db.md`](../sce/agent-trace-dwh-db.md), [`agent-trace-db.md`](../sce/agent-trace-db.md), [`shared-turso-db.md`](../sce/shared-turso-db.md)
- Evidence: [`001_dwh_schema.sql`](../../cli/migrations/agent-trace-dwh/001_dwh_schema.sql), [`agent_trace_dwh_db/mod.rs`](../../cli/src/services/agent_trace_dwh_db/mod.rs)
