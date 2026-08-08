# Agent Trace DWH Database (Destination Schema)

The Agent Trace DWH is a separate, append-oriented destination schema for a future ETL consumer of repository-scoped Agent Trace data. It is a distinct database boundary from the repository-scoped `agent-trace.db` source schema (see [agent-trace-db.md](agent-trace-db.md)): the DWH is never written by hooks, `sce trace`, or any live capture path, and this repository does not yet extract, transform, hash, or sync into it.

## Adapter

`cli/src/services/agent_trace_dwh_db/mod.rs` defines:

- `AgentTraceDwhDbSpec: DbSpec` — like `RepositoryAgentTraceDbSpec`, `db_path()` bails: this adapter still has no canonical spec path and callers must use the explicit-path `TursoDb` constructors. A canonical local sync replica now exists as a *separate* boundary — `AgentTraceDwhReplica` (see [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md)) — which opens a `AgentTraceDwhDb` over a Turso Sync connection rather than through this spec's own constructors. `db_config_key()` reuses `"agent_trace_db"` rather than adding new retry configuration surface. `migrations()` returns the build-time generated `generated_migrations::AGENT_TRACE_DWH_MIGRATIONS`, discovered from `cli/migrations/agent-trace-dwh/` the same way as every other `DbSpec` (see [shared-turso-db.md](shared-turso-db.md)).
- `pub type AgentTraceDwhDb = TursoDb<AgentTraceDwhDbSpec>` — a fourth concrete `TursoDb` wrapper alongside `LocalDb`, `AuthDb`, and `RepositoryAgentTraceDb`.
- `AgentTraceDwhDb::ensure_dwh_schema_ready()` — non-mutating readiness check delegating to the shared `TursoDb::ensure_schema_ready()`.
- `AgentTraceDwhDb::classify_schema_state()` — non-mutating `AgentTraceDwhSchemaState` classification (`Ready`, `Empty`, `Incompatible(String)`) built on `TursoDb::migration_metadata_problems()` plus a `sqlite_master` scan for any user-defined table. `Empty` requires no `__sce_migrations` table and no other user-defined table at all (including the seven DWH contract tables); every other non-ready case — an unrelated schema, a partial DWH schema, or a migration ledger with unexpected entries — classifies `Incompatible`. The `sqlite_master` scan excludes Turso Sync's own internal bookkeeping tables (`turso_cdc`, `turso_cdc_version`, and any table prefixed `__turso_internal`), which a freshly bootstrapped Turso Sync database carries even before any user-defined schema exists; without this exclusion a genuinely empty Turso Sync remote/replica would misclassify as `Incompatible`. `AgentTraceDwhReplica::open()` drives its empty-remote auto-initialization state machine directly off this classification (see [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md)).

The module is not registered with any lifecycle provider, doctor/setup flow, or CLI command; it is `#[allow(dead_code)]` at the `cli/src/services/mod.rs` registration until an ETL consumer exists.

## Schema

`cli/migrations/agent-trace-dwh/001_dwh_schema.sql` is one fresh multi-statement baseline (migration ID `001_dwh_schema`) creating exactly seven tables, with no foreign keys anywhere in the schema:

- `repositories`, `source_instances` — lineage dimension tables, unique on `repository_id` and `(repository_id, source_instance_id)` respectively.
- `etl_watermarks` — extraction progress, independently keyed and unique on `(repository_id, source_instance_id, source_table)`; `source_table` is free text, not a database enum, so new source tables need no schema change.
- `messages`, `message_parts`, `agent_traces`, `code_changes` — fact tables. Every fact table carries `repository_id` and `source_instance_id` as plain `TEXT` lineage columns (never a foreign key), so ingestion can proceed independently and out of order across fact tables and across independently created source databases.

## Identity and uniqueness contract

Two different uniqueness scopes are used, chosen by whether the source identity is deterministic:

- **Deterministic logical identity excludes `source_instance_id`.** `messages` is unique on `(repository_id, session_id, message_id)`; `agent_traces` is unique on `(repository_id, agent_trace_id)`. Both `session_id`/`message_id` and `agent_trace_id` are expected to be reproduced identically if the same logical event is re-ingested from an independently created source database for the same repository, so excluding `source_instance_id` from uniqueness keeps re-ingestion idempotent across repositories and independently created source databases — duplicate inserts fail with a `UNIQUE` constraint violation regardless of which source instance they came from.
- **Raw local source row IDs are scoped by source instance.** `message_parts` is unique on `(repository_id, source_instance_id, source_part_id)`, where `source_part_id` is the source `parts.id` local autoincrement value; `code_changes` is unique on `(repository_id, source_instance_id, source_diff_trace_id)`, where `source_diff_trace_id` is the source `diff_traces.id` local autoincrement value. Local autoincrement IDs are not stable across independently created source databases, so the same local integer ID is expected — and allowed — to coexist across different source instances and repositories.

`message_parts` also carries `idx_dwh_message_parts_order` on `(repository_id, session_id, message_id, generated_at_unix_ms, source_part_id)`, so deterministic message-part reconstruction orders by source timestamp and falls back to `source_part_id` when multiple parts share the same `generated_at_unix_ms`.

## Data preservation and hashing

`message_parts.text` and `agent_traces.trace_json` store complete source text/JSON verbatim, with no truncation or normalization columns. Source event timestamps (`generated_at_unix_ms`, `commit_time_ms`, `time_ms`) are preserved as integer milliseconds, matching the source schema; only DWH-local metadata timestamps (`first_seen_at`, `updated_at`, `ingested_at`) use the shared UTC text default `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`. Integrity hash columns (`message_parts.text_sha256`, `agent_traces.trace_json_sha256`, `code_changes.patch_sha256`) exist as storage for a future hashing implementation; this schema does not compute or populate them.

See also: [agent-trace-db.md](agent-trace-db.md), [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md), [shared-turso-db.md](shared-turso-db.md), [../context-map.md](../context-map.md)
