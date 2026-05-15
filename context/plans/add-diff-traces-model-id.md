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

- [x] T03: `Update Rust SQL constants, DiffTraceInsert, and insert logic` (status:done)
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
  - Execution record:
    - Status: done
    - Completed: 2026-05-15
    - Files changed: `cli/src/services/agent_trace_db/mod.rs`
    - Evidence: `git diff -- cli/src/services/agent_trace_db/mod.rs` confirms migration `005_add_diff_traces_model_id` is registered, `DiffTraceInsert` includes `model_id`, `INSERT_DIFF_TRACE_SQL` writes `model_id` as `?4`, and `insert_diff_trace_with` passes the 4th parameter.
    - Check evidence: `nix develop -c sh -c 'cd cli && cargo check'` attempted and failed before completing because `hooks/mod.rs` still constructs `DiffTraceInsert` without `model_id`; `nix run .#pkl-check-generated` also failed at the same Nix package build dependency for the same compile error; user approved stopping at the T03 boundary because that call-site/payload parsing work is T04 scope.
    - Context sync classification: localized Rust DB insert-path change; root shared context expected verify-only, with Agent Trace DB context checked for drift.

- [x] T04: `Update Rust DiffTracePayload struct and parsing in hooks/mod.rs` (status:done)
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
  - Execution record:
    - Status: done
    - Completed: 2026-05-15
    - Files changed: `cli/src/services/hooks/mod.rs`
    - Evidence: `DiffTracePayload` now includes `model_id`; `parse_diff_trace_payload` reads required non-empty `model_id`; `persist_diff_trace_payload_to_agent_trace_db_with` passes `&payload.model_id` into `DiffTraceInsert`.
    - Check evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
    - Context sync classification: important localized hook runtime-contract change; root shared context and Agent Trace hook/runtime docs require drift check/update so `diff-trace` no longer documents ignored `model_id` payload fields.

- [x] T05: `Validation and cleanup` (status:done)
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
  - Execution record:
    - Status: done
    - Completed: 2026-05-15
    - Files changed: none (verification + cleanup only)
    - Evidence:
      - `nix run .#pkl-check-generated` passed: "Generated outputs are up to date."
      - `nix flake check` passed: "all checks passed!" (evaluated packages, checks, apps, devShells — all 15 checks passed)
      - `context/tmp/` cleaned: stale JSON artifacts and sce.log removed, only .gitignore remains
      - No new test files or test cases found from this plan
    - Context sync classification: verify-only (no root edits expected; combined result of T01–T04 confirmed)

## Validation Report

### Commands run
| Command | Exit code | Key output |
|---|---|---|
| `nix run .#pkl-check-generated` | 0 | "Generated outputs are up to date." |
| `nix flake check` | 0 | "all checks passed!" (evaluated packages, checks, apps, devShells — 15 checks: cli-tests, cli-clippy, cli-fmt, integrations-install-*, pkl-parity, npm-*, config-lib-*) |
| `rm -rf context/tmp/*.json context/tmp/sce.log` | 0 | Cleanup completed; only `.gitignore` remains in `context/tmp/` |
| `git diff --name-only HEAD~4..HEAD \| grep -i test` | 1 (no matches) | No new test files found across the plan's commits |

### Success-criteria verification
- [x] **New migration `005_add_diff_traces_model_id.sql`** adds `model_id TEXT` (nullable) to `diff_traces` — T02 confirmed via file content check: `ALTER TABLE diff_traces ADD COLUMN model_id TEXT;`
- [x] **`extractDiffTracePayload` returns `model_id`** constructed as `providerID/modelID` — T01 confirmed via TypeScript typecheck + pkl parity + payload inspection
- [x] **`DiffTracePayload` struct** parses `model_id` as a required non-empty string — T04 confirmed via `nix develop -c sh -c 'cd cli && cargo check'` and `nix flake check`
- [x] **`DiffTraceInsert` includes `model_id`** and INSERT SQL writes it — T03 confirmed via git diff of `cli/src/services/agent_trace_db/mod.rs`
- [x] **Existing SELECT queries unchanged** — T03/T04 boundaries explicitly excluded SELECT queries; confirmed via git inspection
- [x] **`nix flake check` passes** — exit 0, all 15 checks passed

### Temporary scaffolding removed
- `context/tmp/` cleaned: 157 stale `*diff-trace.json`, `*post-commit.json`, and `sce.log` files deleted

### Residual risks
- None identified. All three layers (TypeScript plugin → Rust hook → Rust DB) are wired end-to-end, all checks pass, context is synced.

### Plan completion summary
All 5 tasks (T01–T05) are complete. The `model_id` column is now:
1. **Emitted** by the TypeScript agent-trace plugin (`extractDiffTracePayload` constructs `providerID/modelID`)
2. **Parsed** by the Rust hook handler (`DiffTracePayload.model_id` validated as required non-empty string)
3. **Persisted** via migration `005_add_diff_traces_model_id.sql` and updated `INSERT_DIFF_TRACE_SQL` with `DiffTraceInsert.model_id`

## Open questions

None — all clarifications resolved during intake.
