# Plan: agent-trace-export-readers

## Change summary

Add incremental local Agent Trace export readers for the four capture streams
(`messages`, `parts`, `diff_traces`, `agent_traces`) stored in the
repository-scoped `RepositoryAgentTraceDb` established by the `source-instance`
plan (PR #197). This establishes the local read/export boundary — cursor in,
owned wire-compatible rows out — that the next PR will compose with a
control-plane HTTP client and `sce trace sync` orchestration.

This plan adds a new `AgentTraceExportReader` seam plus four owned,
`serde::Serialize`-derived export-row DTOs with camelCase JSON matching the
already-shipped control-plane ingestion contract. Each reader method takes a
`cursor: i64` (last server-accepted `table.id`) and a `limit: usize`, runs
`SELECT ... WHERE id > ?1 ORDER BY id ASC LIMIT ?2`, validates cursor/limit/
JS-safe-integer bounds, and returns a fully materialized `Vec<...>` — no open
transaction, no iterator, no borrowed row state, no network, no local cursor,
no auth. `post_commit_patch_intersections` is not exported.

This is purely additive: no existing writer, schema, or hook behavior changes.

## Acceptance criteria

- [ ] AC1: `AgentTraceExportReader::read_messages_after(cursor, limit)` returns owned `AgentTraceMessageExportRow` values for `messages.id > cursor`, ordered by `id ASC`, capped at `limit`, with `sourceRowId` equal to the local `id` unmodified.
  - Validate: `cargo test -p shared-context-engineering --lib services::agent_trace_export`
- [ ] AC2: The same contract holds for `read_parts_after`, `read_diff_traces_after`, and `read_agent_traces_after`, including exact column mapping, nullable-field preservation as `Option<T>` → JSON `null`, and no gap/contiguity assumption.
  - Validate: `cargo test -p shared-context-engineering --lib services::agent_trace_export`
- [ ] AC3: Every export row type serializes via `serde_json` to the exact camelCase shape already shipped by the control-plane ingestion contract (field names and value shapes as specified in this plan's DTO sections).
  - Validate: `cargo test -p shared-context-engineering --lib services::agent_trace_export::tests` (serialization contract tests)
- [ ] AC4: Readers reject `cursor < 0`, `limit == 0`, and `limit > AGENT_TRACE_EXPORT_BATCH_SIZE` (500) with a clear error and no query execution; readers reject rows whose exportable numeric fields fall outside `0..=9_007_199_254_740_991` with a clear export error instead of truncating/casting.
  - Validate: `cargo test -p shared-context-engineering --lib services::agent_trace_export::tests` (validation tests)
- [ ] AC5: Invoking any reader method performs no database mutation (no inserts, no cursor table, no metadata writes).
  - Validate: inspection of `AgentTraceExportReader` (read-only `SELECT` methods only, no `INSERT`/`UPDATE`/`DDL`) plus a test asserting row counts/`repository_metadata` are unchanged after a read.
- [ ] AC6: A `RepositoryAgentTraceDb` opened through the existing repository storage resolver (from PR #197) can be passed directly into `AgentTraceExportReader::new(&storage.db)` and read at least one stream, proving the composition point without the reader owning or generating `source_instance_id`.
  - Validate: `cargo test -p shared-context-engineering --lib services::agent_trace_export::tests::source_instance_integration` (or equivalently named integration test)

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/agent-trace-db.md` and/or a new `context/sce/agent-trace-export-readers.md` documenting the reader boundary, the four stream queries, and the explicit no-local-cursor / no-sync-db statement.
- `context/context-map.md` entry for the new domain file if a dedicated file is created.

## Constraints and non-goals

- **In scope:** a new `cli/src/services/agent_trace_export/` (or `agent_trace_export.rs`) module; four export DTOs; `AgentTraceExportReader` with four `read_*_after` methods; the `AGENT_TRACE_EXPORT_BATCH_SIZE` constant; cursor/limit/safe-integer validation; reader unit + integration tests; durable context documentation.
- **Out of scope:** `sce trace sync`, any HTTP client or control-plane request/response types beyond the export row DTOs, WorkOS/auth changes, server cursor fetching, `POST /agent-trace/ingestion/batch`, retry/backoff against a remote server, local cursor persistence, any new database or table, patch parsing/normalization, `code_changes` derivation, analytics.
- **Constraints:** no new local sync database; no bridge lock; no changes to `RepositoryAgentTraceDb` write semantics, migrations, or schema; must not derive or accept a source identity other than the existing `RepositoryMetadata.source_instance_id`; must not UUID-parse `source_instance_id`; JSON field names are camelCase and must match the already-shipped control-plane contract exactly as specified in this plan.
- **Non-goal:** building any part of the future `sce trace sync` command, config, or CLI surface. This plan produces a library-level reader only; nothing here is user-invocable.

## Task stack

- [x] T01: `Add export batch-size constant, error handling for cursor/limit, and safe-integer validation helper` (status:done)
  - Task ID: T01
  - Goal: Establish the shared validation primitives every reader method depends on: `pub const AGENT_TRACE_EXPORT_BATCH_SIZE: usize = 500;`, a cursor validator rejecting `cursor < 0`, a limit validator rejecting `limit == 0` and `limit > AGENT_TRACE_EXPORT_BATCH_SIZE`, and a JS-safe-integer validator rejecting values outside `0..=9_007_199_254_740_991` (`Number.MAX_SAFE_INTEGER`), all returning `anyhow::Result` with clear, distinct error messages per failure mode (matching this repository's established `anyhow` error convention).
  - Boundaries (in/out of scope): In — the constant, validator functions/helpers, and their unit tests in a new `cli/src/services/agent_trace_export/mod.rs`. Out — any reader method, any DTO, any SQL.
  - Dependencies: none
  - Done when: `cargo test -p shared-context-engineering --lib services::agent_trace_export` passes with unit tests for cursor `< 0` rejection, limit `0`/`501` rejection, limit `500` acceptance, and safe-integer boundary acceptance/rejection at `0`, `9_007_199_254_740_991`, and `9_007_199_254_740_992`.
  - Verification notes (commands or checks): `cargo test -p shared-context-engineering --lib services::agent_trace_export`
  - Evidence: Added `cli/src/services/agent_trace_export/mod.rs` with `AGENT_TRACE_EXPORT_BATCH_SIZE = 500`, `JS_MAX_SAFE_INTEGER`, and `validate_cursor`/`validate_limit`/`validate_js_safe_integer` (each returning `anyhow::Result<()>` with a distinct `bail!` message per failure mode). Registered the module in `cli/src/services/mod.rs` with `#[allow(dead_code)]` (unused until later tasks wire it up). 9 unit tests cover cursor `<0` rejection and `>=0` acceptance, limit `0`/`501` rejection and `1`/`500` acceptance, and safe-integer rejection at `-1`/`9_007_199_254_740_992` with acceptance at `0`/`9_007_199_254_740_991`.
  - Verification run: `nix flake check` (repository policy blocks direct `cargo test`; the `cli-tests` flake check runs the full workspace suite including the new `services::agent_trace_export` tests) — all 3 checks (`cli-tests`, `cli-clippy`, `cli-fmt`) passed.
  - Deviations: none.

- [x] T02: `Define the four export DTOs with camelCase Serialize and serialization contract tests` (status:done)
  - Task ID: T02
  - Goal: Define `AgentTraceMessageExportRow`, `AgentTracePartExportRow`, `AgentTraceDiffTraceExportRow`, and `AgentTraceAgentTraceExportRow` as owned structs deriving `serde::Serialize` with `#[serde(rename_all = "camelCase")]`, matching field-for-field the JSON shapes specified in the change request (messages: `sourceRowId, sessionId, messageId, role, generatedAtUnixMs`; parts: `sourceRowId, sessionId, messageId, type, text, generatedAtUnixMs`; diff_traces: `sourceRowId, sessionId, timeMs, patch, modelId, toolName, toolVersion, payloadType` with `modelId`/`toolName`/`toolVersion` as `Option<String>`; agent_traces: `sourceRowId, agentTraceId, commitId, commitTimeMs, traceJson, url, remoteUrl` with `remoteUrl: Option<String>`). Reuse the existing `agent_trace_db::MessageRole` enum for `role` by adding a `Serialize` derive with lowercase rename (`user`/`assistant`) rather than introducing a parallel role type; do not export local `created_at`/`updated_at`.
  - Boundaries (in/out of scope): In — the four struct definitions, `MessageRole`'s added `Serialize` derive, and `serde_json`-based serialization contract tests asserting exact JSON shape for representative rows of each type (including `null` for `None` fields). Out — any reader method, any SQL, any DB access.
  - Dependencies: T01
  - Done when: `cargo test` for the new serialization contract tests passes, asserting JSON output byte-for-byte (via `serde_json::json!` comparison or exact string) for one representative row per stream, including a diff_trace row with all-`None` nullable fields and one with all populated, and an agent_trace row with `remoteUrl: null`.
  - Verification notes (commands or checks): `cargo test -p shared-context-engineering --lib services::agent_trace_export`
  - Evidence: Added `AgentTraceMessageExportRow`, `AgentTracePartExportRow`, `AgentTraceDiffTraceExportRow`, and `AgentTraceAgentTraceExportRow` to `cli/src/services/agent_trace_export/mod.rs`, each `#[derive(Clone, Debug, PartialEq, Serialize)]` with `#[serde(rename_all = "camelCase")]` (the `parts` row's `type` field uses `#[serde(rename = "type")]` since `type` is a Rust keyword). Added `#[derive(..., serde::Serialize)]` with `#[serde(rename_all = "lowercase")]` to `MessageRole` in `cli/src/services/agent_trace_db/mod.rs`, reused directly as the `role` field type — no parallel role type introduced. Added 8 serialization contract tests via `serde_json::to_value`/`json!` comparison covering: message row with `assistant` role, message row with `user` role (lowercase check), part row full shape, diff_trace row with all nullable fields populated, diff_trace row with all nullable fields `None` → JSON `null`, agent_trace row with `remoteUrl: null`, and agent_trace row with `remoteUrl` populated.
  - Verification run: `nix flake check` — all 3 checks (`cli-tests`, `cli-clippy`, `cli-fmt`) passed.
  - Deviations: none.

- [ ] T03: `Implement AgentTraceExportReader::read_messages_after and read_parts_after with full test coverage` (status:todo)
  - Task ID: T03
  - Goal: Implement `pub struct AgentTraceExportReader<'a> { db: &'a RepositoryAgentTraceDb }` with `pub fn new(db: &'a RepositoryAgentTraceDb) -> Self`, `read_messages_after(&self, cursor: i64, limit: usize) -> Result<Vec<AgentTraceMessageExportRow>>`, and `read_parts_after(&self, cursor: i64, limit: usize) -> Result<Vec<AgentTracePartExportRow>>`. Each validates cursor/limit via T01 helpers, runs `SELECT id, session_id, message_id, role, generated_at_unix_ms FROM messages WHERE id > ?1 ORDER BY id ASC LIMIT ?2` (and the equivalent for `parts`) via the existing `TursoDb` query API, validates safe-integer bounds on `sourceRowId`/`generatedAtUnixMs` per row before returning, and materializes fully into owned `Vec<...>` before returning (no borrowed row/iterator/transaction survives the call).
  - Boundaries (in/out of scope): In — the reader struct, `new`, `read_messages_after`, `read_parts_after`, and their tests (incremental read with a gap, limit truncation plus follow-up read, empty result at/above max ID, invalid-cursor test, invalid-limit tests, safe-integer rejection test, no-mutation test). Out — `read_diff_traces_after`, `read_agent_traces_after`, any HTTP/network code.
  - Dependencies: T02
  - Done when: tests seed a temporary `RepositoryAgentTraceDb` (following the existing `unique_test_db_path`/`RepositoryAgentTraceDb::new_at` pattern in `cli/src/services/agent_trace_db/repository.rs`), insert messages/parts via the existing `insert_message`/`insert_messages`/`insert_part`/`insert_parts` helpers or direct SQL to control exact `id` values, and assert: seeded IDs `1,2,3` with `cursor=1` returns `[2,3]` in order with exact field mapping; a non-contiguous ID set (e.g. `11,15,19,30` after `cursor=10`) returns all four; `limit=3` over 10 rows returns IDs `1..3` and a follow-up read from `cursor=3` continues correctly; `cursor` at/beyond the max ID returns an empty `Vec`; `cursor=-1` errors with no query executed; `limit=0` and `limit=501` error; a row with `generated_at_unix_ms > 9_007_199_254_740_991` (seeded via direct SQL) errors; and read calls leave row counts and `repository_metadata` unchanged.
  - Verification notes (commands or checks): `cargo test -p shared-context-engineering --lib services::agent_trace_export`

- [ ] T04: `Implement AgentTraceExportReader::read_diff_traces_after and read_agent_traces_after with full test coverage` (status:todo)
  - Task ID: T04
  - Goal: Implement `read_diff_traces_after(&self, cursor: i64, limit: usize) -> Result<Vec<AgentTraceDiffTraceExportRow>>` and `read_agent_traces_after(&self, cursor: i64, limit: usize) -> Result<Vec<AgentTraceAgentTraceExportRow>>` on `AgentTraceExportReader`, following the same cursor/limit/safe-integer validation and materialization discipline as T03. `read_diff_traces_after` selects `id, session_id, time_ms, patch, model_id, tool_name, tool_version, payload_type` from `diff_traces` and preserves `patch`/`payload_type` raw and unmodified (no patch parsing, no normalizer call). `read_agent_traces_after` selects `id, agent_trace_id, commit_id, commit_time_ms, trace_json, url, remote_url` from `agent_traces` and preserves `trace_json` as the exact raw string from SQLite (no parse/reserialize).
  - Boundaries (in/out of scope): In — the two reader methods and their tests (nullable `model_id`/`tool_name`/`tool_version` both populated and `NULL`; raw `patch`/`payload_type` passthrough; `remote_url = NULL` → JSON `null`; `trace_json` byte-for-byte passthrough; gap/limit/empty/invalid-cursor/invalid-limit/safe-integer/no-mutation tests mirroring T03's coverage for these two streams). Out — `read_messages_after`, `read_parts_after` (T03), any HTTP/network code, any use of `cli/src/services/patch.rs` or `structured_patch.rs`.
  - Dependencies: T03
  - Done when: all diff_traces/agent_traces reader tests pass per the Goal, and `AgentTraceExportReader` now exposes all four `read_*_after` methods with the complete `pub` API shape from the change request (`reader.read_messages_after(cursor, limit)`, etc.), ready for direct use by a future `sce trace sync` command.
  - Verification notes (commands or checks): `cargo test -p shared-context-engineering --lib services::agent_trace_export`

- [ ] T05: `Add source-instance storage-resolver integration test and document the export reader boundary` (status:todo)
  - Task ID: T05
  - Goal: Add one integration-style test that resolves `ResolvedAgentTraceStorage` through the existing PR #197 storage resolver (`resolve_agent_trace_storage_at_state_root` or equivalent test entrypoint), asserts `storage.metadata.repository_id` and `storage.metadata.source_instance_id` remain available, constructs `AgentTraceExportReader::new(&storage.db)`, and successfully reads at least one stream — proving the `ResolvedAgentTraceStorage → metadata + db → AgentTraceExportReader` composition point without the reader generating or owning `source_instance_id`. Then document the new boundary: create or extend Agent Trace context documentation describing the layering (`SCE local source DB → incremental export reader → future control-plane client`), the fact that `repository_id`/`source_instance_id` identify the source while `table.id` is the per-stream progress marker, the exact `WHERE id > cursor ORDER BY id ASC LIMIT batch_size` query shape for all four streams, and an explicit statement that there is no local sync cursor, no `agent-trace-sync.db`, no Turso Sync, no ETL, and no DWH. Update `context/context-map.md` if a new domain file is added.
  - Boundaries (in/out of scope): In — the one integration test, and durable-context documentation edits/additions. Out — any further reader behavior changes; any control-plane client code.
  - Dependencies: T04
  - Done when: the integration test passes, durable context documents the export-reader boundary and the four exact stream queries, and `nix run .#pkl-check-generated` plus `nix flake check` both pass.
  - Verification notes (commands or checks): `cargo test -p shared-context-engineering --lib services::agent_trace_export`; `nix run .#pkl-check-generated`; `nix flake check`

## Open questions

None. The change request is fully specified end-to-end (module boundary, DTO shapes, query semantics, validation rules, and test coverage), and it does not duplicate or extend PR #197's identity work — it consumes `RepositoryMetadata` as-is and adds a strictly read-only, additive seam.
