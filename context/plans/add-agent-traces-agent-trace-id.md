# Plan: Add `agent_trace_id` column to `agent_traces` table

## Change summary

The `agent_traces` table currently stores the UUIDv7 agent trace ID only inside `trace_json` (as part of the serialized `AgentTrace` JSON payload). Add a dedicated `agent_trace_id TEXT` column so the UUIDv7 identifier is queryable without parsing JSON.

The `generate_agent_trace_id()` function in `agent_trace.rs` already produces a UUIDv7 string. No uuid v4 usage exists anywhere in the codebase — confirmed via grep, nothing to remove.

Three layers need updating:
1. **SQL migration** — add the column to the `agent_traces` table
2. **Rust DB layer** — register migration, update insert SQL and `AgentTraceInsert` struct
3. **Rust hook caller** — extract `agent_trace.id` from the built `AgentTrace` and pass it through

## Success criteria

- New migration `006_add_agent_traces_agent_trace_id.sql` adds `agent_trace_id TEXT` (nullable) to `agent_traces`
- Migration is registered in `AGENT_TRACE_MIGRATIONS`
- `INSERT_AGENT_TRACE_SQL` includes `agent_trace_id` as a parameter
- `AgentTraceInsert` carries `agent_trace_id: &'a str`
- Caller in `hooks/mod.rs` extracts `agent_trace.id` and passes it in the insert
- `nix flake check` passes

## Constraints and non-goals

- The `agent_trace_id` column is **nullable** (existing rows get `NULL`).
- No changes to `agent_trace.rs` — `generate_agent_trace_id` and `AgentTrace.id` are unchanged.
- No changes to the TypeScript plugin, payload parsing, or any other table.
- No new tests — existing build/tests cover the change.
- uuid v4 removal is not needed — confirmed zero usage across the codebase.

## Task stack

- [x] T01: `Create migration 006_add_agent_traces_agent_trace_id.sql` (status:done)
  - Task ID: T01
  - Goal: Create the SQL migration file that adds the `agent_trace_id` column.
  - Boundaries (in/out of scope):
    - In — Create `cli/migrations/agent-trace/006_add_agent_traces_agent_trace_id.sql` with `ALTER TABLE agent_traces ADD COLUMN agent_trace_id TEXT;`
    - Out — Changes to any other table, migration runner, or existing files
  - Done when:
    - Migration file exists at `cli/migrations/agent-trace/006_add_agent_traces_agent_trace_id.sql`
    - Contains `ALTER TABLE agent_traces ADD COLUMN agent_trace_id TEXT;`
  - Verification notes (commands or checks):
    - `test -f cli/migrations/agent-trace/006_add_agent_traces_agent_trace_id.sql` ✅
    - File content inspection ✅
    - `nix develop -c sh -c 'cd cli && cargo check'` ✅
  - **Completed:** 2026-05-19
  - **Files changed:** `cli/migrations/agent-trace/006_add_agent_traces_agent_trace_id.sql` (new)
  - **Evidence:** File exists with correct ALTER TABLE content; `cargo check` passes (0.52s)

- [x] T02: `Register migration and update Rust DB insert path` (status:done)
  - Task ID: T02
  - Goal: Wire the new migration into the Rust DB layer and update the insert SQL + struct.
  - Boundaries (in/out of scope):
    - In — Register `006_add_agent_traces_agent_trace_id` in `AGENT_TRACE_MIGRATIONS` with corresponding `include_str!` constant; update `INSERT_AGENT_TRACE_SQL` to include `agent_trace_id`; add `agent_trace_id: &'a str` to `AgentTraceInsert`
    - Out — Changes to any other SQL statements, query logic, or structs (`DiffTraceInsert`, `PostCommitPatchIntersectionInsert`, etc.)
  - Done when:
    - Migration `006_add_agent_traces_agent_trace_id` registered in the migrations list
    - `AgentTraceInsert` has `agent_trace_id: &'a str` field
    - `INSERT_AGENT_TRACE_SQL` is `INSERT INTO agent_traces (commit_id, commit_time_ms, trace_json, agent_trace_id) VALUES (?1, ?2, ?3, ?4)`
    - Build compiles (caller not yet updated, so `AgentTraceInsert` construction may still fail — acceptable at T02 boundary)
  - Verification notes (commands or checks):
    - `nix develop -c sh -c 'cd cli && cargo check'` (expect compile errors in hooks/mod.rs — that's T03 scope)
    - Visual inspection of the migration constant, SQL, and struct
  - **Completed:** 2026-05-19
  - **Files changed:** `cli/src/services/agent_trace_db/mod.rs`
  - **Evidence:** Migration registered in list; SQL updated to 4 params; struct has `agent_trace_id`; only expected error in `hooks/mod.rs` (T03 scope); test assertion updated for 6 migrations; `cargo check` reports only the expected caller error

- [x] T03: `Pass agent_trace_id from hooks/mod.rs caller` (status:done)
  - Task ID: T03
  - Goal: Extract the UUIDv7 ID from the built `AgentTrace` struct and pass it in the insert input.
  - Boundaries (in/out of scope):
    - In — Modify the insert-input construction in `hooks/mod.rs` (around line 533) to include `agent_trace_id: &agent_trace.id`
    - Out — Changes to `agent_trace.rs`, validation logic, or any other flow
  - Done when:
    - `AgentTraceInsert` in `hooks/mod.rs` includes `agent_trace_id: &agent_trace.id`
    - `nix develop -c sh -c 'cd cli && cargo check'` passes
  - Verification notes (commands or checks):
    - `nix develop -c sh -c 'cd cli && cargo check'`
    - `nix flake check`
  - **Completed:** 2026-05-19
  - **Files changed:** `cli/src/services/hooks/mod.rs`
  - **Evidence:** `cargo check` passes; `nix flake check` passes; `agent_trace_id: &agent_trace.id` added to `AgentTraceInsert` constructor

- [x] T04: `Validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Verify full pipeline compiles, existing tests pass, and no regressions.
  - Boundaries (in/out of scope):
    - In — Run `nix flake check`; confirm new migration is loadable; confirm applied migration ID assertions include the new migration; confirm no stale artifacts in `context/tmp/`
    - Out — Any code changes beyond verification
  - Done when:
    - `nix flake check` passes
    - No stale artifacts left in `context/tmp/`
  - Verification notes (commands or checks):
    - `nix flake check`
    - `nix develop -c sh -c 'cd cli && cargo check'`
  - **Completed:** 2026-05-19
  - **Files changed:** none (verification-only)
  - **Evidence:** Migration file exists at `cli/migrations/agent-trace/006_add_agent_traces_agent_trace_id.sql` with correct ALTER TABLE content; migration registered in AGENT_TRACE_MIGRATIONS; INSERT_AGENT_TRACE_SQL includes `agent_trace_id`; AgentTraceInsert has `agent_trace_id: &'a str`; test assertions cover all 6 migration IDs; stale artifacts cleaned from `context/tmp/`; `nix flake check` passes

## Validation Report

### Commands run
- `nix flake check` -> exit 0 — all checks evaluated and built cleanly (pkl-parity built and passed)
- `rm -f context/tmp/2026-*.json context/tmp/sce.log` — 138 stale JSON artifacts + sce.log removed; only `.gitignore` remains
- Verified migration file: `test -f cli/migrations/agent-trace/006_add_agent_traces_agent_trace_id.sql` ✅
- Verified migration content: `ALTER TABLE agent_traces ADD COLUMN agent_trace_id TEXT;` ✅

### Success-criteria verification
- [x] New migration `006_add_agent_traces_agent_trace_id.sql` adds `agent_trace_id TEXT` (nullable) — confirmed file exists with correct SQL
- [x] Migration registered in `AGENT_TRACE_MIGRATIONS` — confirmed line 44 of `agent_trace_db/mod.rs`
- [x] `INSERT_AGENT_TRACE_SQL` includes `agent_trace_id` as 4th parameter — confirmed line 74
- [x] `AgentTraceInsert` carries `agent_trace_id: &'a str` — confirmed line 168
- [x] Caller in `hooks/mod.rs` passes `agent_trace_id: &agent_trace.id` — confirmed line 537
- [x] `nix flake check` passes — confirmed 2026-05-19

### Residual risks
- None identified. The `agent_trace_id` column is nullable, so existing rows with NULL remain valid. No schema changes affect other tables or operations.

## Open questions

None — all clarifications resolved during intake.
