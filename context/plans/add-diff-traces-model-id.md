# Plan: Add `model_id` column to `diff_traces` table

## Change summary

Add a `model_id` text column to the `diff_traces` table to track which AI model generated each diff trace. The value is constructed from the OpenCode event's model info as `${providerID}/${modelID}`.

Three layers need updating:
1. **TypeScript agent-trace plugin** — emit `model_id` in the `DiffTracePayload` sent to the Rust hook
2. **Rust hook handler** — parse `model_id` from the payload and pass it through to the DB layer
3. **Rust DB layer** — migration to add the column, updated insert SQL, and updated insert struct

## Success criteria

- New migration `005_add_diff_traces_model_id.sql` adds `model_id TEXT` (nullable) to `diff_traces`
- `extractDiffTracePayload` returns `model_id` constructed as `providerID/modelID`
- `DiffTracePayload` struct in Rust parses `model_id` as a required non-empty string
- `DiffTraceInsert` includes `model_id` and the INSERT SQL writes it
- Existing SELECT queries for recent patches remain unchanged
- `nix flake check` passes

## Constraints and non-goals

- The `model_id` column is **nullable** (per user choice); existing rows get `NULL`.
- The SELECT query for recent diff trace patches (`SELECT_RECENT_DIFF_TRACE_PATCHES_SQL`) is **not** updated — the model_id is for storage/audit only.
- `DiffTracePatchRow`, `ParsedDiffTracePatch`, and `SkippedDiffTracePatch` structs are **not** changed.
- No changes to the post-commit intersection or agent_traces flows.
- Do not add new tests for this plan; use existing checks, typechecking/build checks, and inspection.

## Task stack

- [x] T01: `Add model_id to TypeScript DiffTracePayload and extractDiffTracePayload` (status:done)
  - Task ID: T01
  - Goal: Update the agent-trace plugin to construct and emit `model_id` in the diff trace payload.
  - Boundaries (in/out of scope):
    - In — `DiffTracePayload` type gains `model_id: string`; `extractDiffTracePayload` extracts `model.providerID` and `model.modelID` from the event info and joins them with `/`
    - Out — Changes to the OpenCode event type definitions; changes to how model info is typed; new test files or new test cases
  - Done when:
    - `extractDiffTracePayload` returns `model_id` field constructed from `input.event.properties.info.model.providerID` + `/` + `input.event.properties.info.model.modelID`
    - If `model` object or its sub-fields are missing, falls back to `"unknown/unknown"`
    - `buildTrace` passes the payload as before
  - Verification notes (commands or checks):
    - Visual inspection of the returned payload shape
    - `nix develop -c tsc --noEmit -p config/lib/agent-trace-plugin/tsconfig.json`
    - `nix run .#pkl-check-generated`
  - Execution record:
    - Status: done
    - Completed: 2026-05-15
    - Files changed: `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`, generated `config/.opencode/plugins/sce-agent-trace.ts`, generated `config/automated/.opencode/plugins/sce-agent-trace.ts`
    - Evidence: `nix develop -c tsc --noEmit -p config/lib/agent-trace-plugin/tsconfig.json` passed; `nix run .#pkl-check-generated` passed; payload shape inspected directly
    - Cleanup completed: previously added `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.test.ts` removed; no new tests remain from T01
    - Context sync classification: important localized runtime-contract change; synced `context/sce/opencode-agent-trace-plugin-runtime.md`, discoverability/root summaries, and Rust hook docs to distinguish plugin-emitted `model_id` from pending Rust validation/storage support

- [x] T02: `Create DB migration 005_add_diff_traces_model_id.sql` (status:done)
  - Task ID: T02
  - Goal: Create a new SQL migration file that adds the `model_id` column to the `diff_traces` table.
  - Boundaries (in/out of scope):
    - In — Create `cli/migrations/agent-trace/005_add_diff_traces_model_id.sql` with `ALTER TABLE diff_traces ADD COLUMN model_id TEXT;`
    - Out — Changes to any other table; changes to migration runner
  - Done when:
    - Migration file exists at `cli/migrations/agent-trace/005_add_diff_traces_model_id.sql`
    - Contains `ALTER TABLE diff_traces ADD COLUMN model_id TEXT;`
  - Verification notes (commands or checks):
    - File exists: `ls cli/migrations/agent-trace/005_add_diff_traces_model_id.sql`
  - Execution record:
    - Status: done
    - Completed: 2026-05-15
    - Files changed: `cli/migrations/agent-trace/005_add_diff_traces_model_id.sql`
    - Evidence: migration file read successfully; `test -f "cli/migrations/agent-trace/005_add_diff_traces_model_id.sql" && test "$(tr -d '\n' < "cli/migrations/agent-trace/005_add_diff_traces_model_id.sql")" = "ALTER TABLE diff_traces ADD COLUMN model_id TEXT;"` passed
    - Context sync classification: localized migration artifact change; root shared context verify-only; synced `context/sce/agent-trace-db.md` and `context/context-map.md` to document the checked-in but not-yet-registered migration file

- [ ] T03: `Update Rust SQL constants, DiffTraceInsert, and insert logic` (status:todo)
  - Task ID: T03
  - Goal: Update `agent_trace_db/mod.rs` to wire the new `model_id` column through the insert path.
  - Boundaries (in/out of scope):
    - In — Register `005_add_diff_traces_model_id` in `AGENT_TRACE_MIGRATIONS`; add `model_id: &'a str` to `DiffTraceInsert`; update `INSERT_DIFF_TRACE_SQL` to include `model_id` as `?4`; update `insert_diff_trace_with` to pass `input.model_id`
    - Out — Changes to `DiffTracePatchRow`, `ParsedDiffTracePatch`, `SkippedDiffTracePatch`, or any SELECT/query code
  - Done when:
    - New migration registered in the migrations list
    - `DiffTraceInsert` has `model_id: &'a str` field
    - `INSERT_DIFF_TRACE_SQL` is `INSERT INTO diff_traces (time_ms, session_id, patch, model_id) VALUES (?1, ?2, ?3, ?4)`
    - `insert_diff_trace_with` passes the 4th parameter
  - Verification notes (commands or checks):
    - `nix develop -c sh -c 'cd cli && cargo check'`
    - `nix flake check`

- [ ] T04: `Update Rust DiffTracePayload struct and parsing in hooks/mod.rs` (status:todo)
  - Task ID: T04
  - Goal: Parse `model_id` from the STDIN payload and pass it through to the DB insert.
  - Boundaries (in/out of scope):
    - In — Add `model_id: String` to `DiffTracePayload`; parse `model_id` via `required_non_empty_string_field` in `parse_diff_trace_payload`; pass `&payload.model_id` in `persist_diff_trace_payload_to_agent_trace_db_with` into `DiffTraceInsert`
    - Out — Changes to JSON serialization format or payload validation
  - Done when:
    - `DiffTracePayload` has `model_id` field with appropriate serde rename
    - `parse_diff_trace_payload` reads `model_id` from JSON
    - `persist_diff_trace_payload_to_agent_trace_db_with` supplies `model_id` to `DiffTraceInsert`
  - Verification notes (commands or checks):
    - `nix develop -c sh -c 'cd cli && cargo check'`
    - `nix flake check`

- [ ] T05: `Validation and cleanup` (status:todo)
  - Task ID: T05
  - Goal: Verify full pipeline compiles, existing tests pass, and no regressions.
  - Boundaries (in/out of scope):
    - In — Run `nix flake check`; run generated-output parity/type/build checks; confirm new migration is loadable; confirm no new tests were added by this plan
    - Out — Any code changes beyond verification
  - Done when:
    - `nix flake check` passes
    - Generated-output parity passes
    - No new test files or test cases remain from this plan
    - No stale artifacts left in `context/tmp/`
  - Verification notes (commands or checks):
    - `nix flake check`
    - `nix run .#pkl-check-generated`
    - `nix develop -c sh -c 'cd cli && cargo check'`

## Open questions

None — all clarifications resolved during intake.
