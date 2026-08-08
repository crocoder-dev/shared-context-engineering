# Agent Trace DWH Database (Destination Schema)

The Agent Trace DWH is a separate, append-oriented destination schema for the Agent Trace ETL consumer of repository-scoped Agent Trace data. It is a distinct database boundary from the repository-scoped `agent-trace.db` source schema (see [agent-trace-db.md](agent-trace-db.md)): the DWH is never written by hooks, `sce trace`, or any live capture path. The current ETL slices transform and atomically load `agent_traces`, logical `messages`, source-lineage-scoped `message_parts`, and source-lineage-scoped `code_changes` through independent table runners; transport synchronization remains a separate caller concern.

## Adapter

`cli/src/services/agent_trace_dwh_db/mod.rs` defines:

- `AgentTraceDwhDbSpec: DbSpec` — like `RepositoryAgentTraceDbSpec`, `db_path()` bails: this adapter still has no canonical spec path and callers must use the explicit-path `TursoDb` constructors. A canonical local sync replica now exists as a *separate* boundary — `AgentTraceDwhReplica` (see [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md)) — which opens a `AgentTraceDwhDb` over a Turso Sync connection rather than through this spec's own constructors. `db_config_key()` reuses `"agent_trace_db"` rather than adding new retry configuration surface. `migrations()` returns the build-time generated `generated_migrations::AGENT_TRACE_DWH_MIGRATIONS`, discovered from `cli/migrations/agent-trace-dwh/` the same way as every other `DbSpec` (see [shared-turso-db.md](shared-turso-db.md)).
- `pub type AgentTraceDwhDb = TursoDb<AgentTraceDwhDbSpec>` — a fourth concrete `TursoDb` wrapper alongside `LocalDb`, `AuthDb`, and `RepositoryAgentTraceDb`.
- `AgentTraceDwhDb::ensure_dwh_schema_ready()` — non-mutating readiness check delegating to the shared `TursoDb::ensure_schema_ready()`.
- `AgentTraceDwhDb::classify_schema_state()` — non-mutating `AgentTraceDwhSchemaState` classification (`Ready`, `Empty`, `Incompatible(String)`) built on `TursoDb::migration_metadata_problems()` plus a `sqlite_master` scan for any user-defined table. `Empty` requires no `__sce_migrations` table and no other user-defined table at all (including the seven DWH contract tables); every other non-ready case — an unrelated schema, a partial DWH schema, or a migration ledger with unexpected entries — classifies `Incompatible`. The `sqlite_master` scan excludes Turso Sync's own internal bookkeeping tables (`turso_cdc`, `turso_cdc_version`, and any table prefixed `__turso_internal`), which a freshly bootstrapped Turso Sync database carries even before any user-defined schema exists; without this exclusion a genuinely empty Turso Sync remote/replica would misclassify as `Incompatible`. `AgentTraceDwhReplica::open()` drives its empty-remote auto-initialization state machine directly off this classification (see [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md)).

The module is not registered with any lifecycle provider, doctor/setup flow, or CLI command; it remains `#[allow(dead_code)]` at the `cli/src/services/mod.rs` registration because the ETL API is a CLI-independent service boundary.

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

## Code-change loading

`load_code_change_batch()` in `cli/src/services/code_changes_etl.rs` transforms all source rows before opening the destination transaction. The loader ensures repository/source dimensions, looks up each `(repository_id, source_instance_id, source_diff_trace_id)`, compares session/time/attribution/payload/metrics/hash content, inserts missing rows without conflict-ignore or overwrite behavior, and counts identical rows as already present. A mismatch is an integrity error. The same transaction upserts the `diff_traces` watermark only after every row succeeds, so conflicts or other destination failures roll back dimensions, facts, and progress together.

`CodeChangesEtl` runs the source `diff_traces` bridge in bounded ordered batches. It extracts the exact eight-column source projection in a short plain read transaction, strictly normalizes supported `patch` and `structured` payloads through the canonical parser before destination work, preserves source metadata, counts parsed files and touched lines with checked integer conversion, and hashes exact source payload bytes. A null source `tool_name` is rejected because the destination contract requires it. Its replica-owned runner is `AgentTraceDwhReplica::run_code_changes_etl()`; neither it nor the destination adapter performs pull/push or credential handling.

## Session-only conversation relationship

`code_changes.session_id` is the only supported relationship from code changes to conversation facts. Queries may join code changes with `messages` and `message_parts` for the same `(repository_id, session_id)`, but `code_changes` has no `message_id` and the ETL never infers one. This proves session membership only, not causality to an individual message.

## Data preservation and hashing

`message_parts.text` and `agent_traces.trace_json` store complete source text/JSON verbatim, with no truncation or normalization columns. Source event timestamps (`generated_at_unix_ms`, `commit_time_ms`, `time_ms`) are preserved as integer milliseconds, matching the source schema; only DWH-local metadata timestamps (`first_seen_at`, `updated_at`, `ingested_at`) use the shared UTC text default `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`. Integrity hash columns (`message_parts.text_sha256`, `agent_traces.trace_json_sha256`, `code_changes.patch_sha256`) are schema storage; the `agent_traces` ETL slice computes and populates the lowercase SHA-256 of exact source JSON bytes, the `PartsEtl` slice computes and populates the lowercase SHA-256 of exact source part text bytes, and code-change loading computes the lowercase SHA-256 of exact source `diff_traces.patch` bytes for both supported payload types. Message rows require no content hash.

See also: [agent-trace-db.md](agent-trace-db.md), [agent-trace-etl.md](agent-trace-etl.md), [conversation-parts-etl.md](conversation-parts-etl.md), [agent-trace-dwh-replica.md](agent-trace-dwh-replica.md), [shared-turso-db.md](shared-turso-db.md), [../context-map.md](../context-map.md)
