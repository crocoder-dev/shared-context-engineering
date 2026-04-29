# Plan: Persist diff-trace payloads in local DB

## Change summary

Extend the current `sce hooks diff-trace` JSON-file persistence path with local Turso DB persistence. The incoming payload remains the current OpenCode handoff shape:

```json
{
  "sessionID": "ses_22a776106ffekgqm1lBkVsV9rT",
  "diff": "Index: ...",
  "time": 1777403999227
}
```

The existing `cli/migrations/001_create_agent_traces.sql` migration is a placeholder and should be replaced with a first migration that creates a `diff_traces` table. Runtime persistence should store the payload fields as timestamp/session/patch data in the local DB while preserving the current `context/tmp/*-diff-trace.json` artifact creation for accepted `diff-trace` payloads.

## Success criteria

- The placeholder `agent_traces` migration is removed/replaced in source.
- The embedded migration set creates a `diff_traces` table, not an `agent_traces` placeholder table.
- `diff_traces` stores:
  - `time_ms` as the incoming `time` Unix epoch millisecond integer
  - `session_id` as the incoming `sessionID` string
  - `patch` as the incoming `diff` string
  - a deterministic DB-owned insertion timestamp such as `created_at`
- `LocalDb::new()` still opens/creates the canonical local DB and runs embedded migrations during setup/doctor/bootstrap flows.
- `sce hooks diff-trace` keeps the current STDIN JSON validation contract for `sessionID`, `diff`, and `time`.
- Accepted `sce hooks diff-trace` payloads are persisted to the local DB and continue to create the current collision-safe `context/tmp/*-diff-trace.json` artifact.
- Existing non-diff hook trace behavior is not broadened or redesigned by this change.
- Current-state context files accurately describe DB-backed diff-trace persistence plus retained JSON artifact creation.
- `nix run .#pkl-check-generated` and `nix flake check` pass.

## Constraints and non-goals

- Use the existing local Turso adapter and canonical local DB path; do not introduce a new database engine or dependency.
- Use SQL identifier `diff_traces` rather than a hyphenated table name.
- Treat the current `001_create_agent_traces.sql` migration as source-level placeholder work and replace it with a new `001_create_diff_traces.sql` migration rather than adding a forward-only `002` migration.
- Do not implement cloud sync, retry queues, git notes, hosted ingestion, or Agent Trace generation in this plan.
- Do not change the OpenCode plugin payload shape or `sce hooks diff-trace` invocation shape.
- Do not persist raw full-event OpenCode snapshots in the local DB as part of this change.
- Do not remove existing `context/tmp/*-diff-trace.json` creation; DB persistence is additive for this path.
- Planning does not approve implementation; each task still requires `/next-task` execution approval.

## Task stack

- [x] T01: `Replace placeholder migration with diff_traces schema` (status:done)
  - Task ID: T01
  - Goal: Replace the source-level placeholder migration with an embedded migration that creates the `diff_traces` table.
  - Boundaries (in/out of scope): In - remove/rename `cli/migrations/001_create_agent_traces.sql`, create `cli/migrations/001_create_diff_traces.sql`, update `cli/src/services/local_db.rs` embedded migration reference and comments. Out - hook runtime DB insertion, OpenCode plugin changes, cloud sync, migration version tracking redesign.
  - Done when: The only `001` migration source creates `diff_traces` with `id`, `time_ms`, `session_id`, `patch`, and `created_at`; `LocalDb::new()` still embeds and runs migration `001`; no source reference to `001_create_agent_traces.sql` remains.
  - Verification notes (commands or checks): Inspect `cli/migrations/` and `cli/src/services/local_db.rs`; run `nix develop -c sh -c 'cd cli && cargo check'` or the broader `nix flake check` if practical.
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/migrations/001_create_agent_traces.sql` removed, `cli/migrations/001_create_diff_traces.sql` added, `cli/src/services/local_db.rs`
  - **Evidence:** Inspected migration and embed path; `grep` confirmed no code/source references to `001_create_agent_traces.sql`; `nix develop -c sh -c 'cd cli && cargo check'` passed after temporarily staging the new migration so the Git flake source included it, then unstaging it; `nix run .#pkl-check-generated` passed.
  - **Notes:** T01 only replaces the embedded schema migration; runtime diff-trace insertion remains for T02/T03. Context sync updated the local DB domain file plus root glossary/context-map references for the new migration/table name.

- [x] T02: `Add LocalDb diff-trace insertion seam` (status:done)
  - Task ID: T02
  - Goal: Add a focused local DB API for inserting a validated diff-trace payload without exposing table SQL throughout hook runtime code.
  - Boundaries (in/out of scope): In - local DB method or small typed input for `time_ms`, `session_id`, and `patch`; parameterized SQL insert into `diff_traces`; focused tests or compile-time coverage as practical. Out - hook command behavior changes, schema migrations beyond T01, sync/cloud behavior.
  - Done when: Callers can persist one diff trace through `LocalDb` using typed values; SQL uses parameters rather than string interpolation; insertion errors carry actionable local-DB context.
  - Verification notes (commands or checks): Run a targeted Rust check/test through Nix if available; otherwise run `nix develop -c sh -c 'cd cli && cargo check'`; final validation remains `nix flake check`.
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/services/local_db.rs`
  - **Evidence:** Targeted `cargo test local_db` through Nix was blocked by the repository bash policy requiring `nix flake check` instead of direct Cargo tests; `nix develop -c sh -c 'cd cli && cargo check'` and `nix develop -c sh -c 'cd cli && cargo check --tests'` passed after temporarily staging `cli/migrations/001_create_diff_traces.sql` so the Git-backed Nix source included the untracked migration, then unstaging it; `nix develop -c sh -c 'cd cli && cargo fmt'` ran through the same temporary staging flow; `nix run .#pkl-check-generated` passed.
  - **Notes:** Added `DiffTraceInsert` plus `LocalDb::insert_diff_trace` as the typed parameterized insertion seam. Context sync updated local DB/domain references plus stale root pointers that explicitly listed the local DB service API.

- [x] T03: `Persist sce hooks diff-trace payloads to DB and JSON` (status:done)
  - Task ID: T03
  - Goal: Extend the accepted `sce hooks diff-trace` path so it writes both the existing `context/tmp` JSON artifact and a `LocalDb` row.
  - Boundaries (in/out of scope): In - `cli/src/services/hooks.rs` diff-trace runtime path, success text update, keep existing diff-trace JSON persistence, focused behavioral coverage as practical. Out - payload validation contract changes, OpenCode TypeScript changes, post-commit/post-rewrite/commit-msg behavior redesign, local DB schema changes beyond T01.
  - Done when: `run_diff_trace_subcommand_from_payload` validates the existing payload shape, creates the existing collision-safe `context/tmp/*-diff-trace.json` artifact, and inserts `time`/`sessionID`/`diff` into `diff_traces`; failure logging still reports persistence errors through the existing hook error path.
  - Verification notes (commands or checks): Exercise `sce hooks diff-trace` with a representative payload and verify both a DB row and a diff-trace JSON artifact exist; run targeted Rust tests/checks through Nix, then rely on final `nix flake check`.
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/services/hooks.rs`, `cli/src/services/local_db.rs`
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo fmt'` passed; direct targeted `cargo test` through Nix was blocked by the repository bash policy requiring `nix flake check`; `nix develop -c sh -c 'cd cli && cargo check --tests'` passed; representative `sce hooks diff-trace` execution passed and Python/SQLite verification confirmed exactly one retained `context/tmp/*-diff-trace.json` artifact plus one `diff_traces` row with `time_ms`, `session_id`, and `patch`; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - **Notes:** Verification commands that build through the Git-backed Nix flake temporarily staged `cli/migrations/001_create_diff_traces.sql` so the existing untracked migration from T01/T02 was visible to `include_str!`, then unstaged it afterward. T03 only wires accepted diff-trace payloads into the existing `LocalDb::insert_diff_trace` seam and keeps the JSON artifact path intact. Context sync updated current-state docs for the new DB+JSON runtime truth; T04 remains the next plan task for any dedicated context-only review/cleanup the human still wants.

- [x] T04: `Sync current-state context for DB-backed diff traces` (status:done)
  - Task ID: T04
  - Goal: Update durable context so future sessions know diff-trace persistence is DB-backed in addition to the existing `context/tmp` JSON artifact.
  - Boundaries (in/out of scope): In - focused updates to `context/sce/local-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/overview.md`, `context/glossary.md`, and `context/context-map.md` if needed. Out - completed-work narration in core context, unrelated Agent Trace historical docs, plan history cleanup.
  - Done when: Context describes `diff_traces` schema ownership, additive DB persistence for `sce hooks diff-trace`, the retained `context/tmp/*-diff-trace.json` artifact behavior, and the retained OpenCode plugin handoff shape.
  - Verification notes (commands or checks): Review context files against code truth; run `nix run .#pkl-check-generated` if generated outputs were not intentionally changed.
  - **Completed:** 2026-04-29
  - **Files changed:** `context/context-map.md`, `context/plans/diff-trace-local-db-persistence.md`
  - **Evidence:** Reviewed `cli/src/services/local_db.rs`, `cli/migrations/001_create_diff_traces.sql`, and `cli/src/services/hooks.rs` against `context/sce/local-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/opencode-agent-trace-plugin-runtime.md`, `context/overview.md`, `context/glossary.md`, and `context/context-map.md`; the durable context already describes `diff_traces`, additive DB insertion for accepted `sce hooks diff-trace` payloads, retained collision-safe `context/tmp/*-diff-trace.json` artifacts, and retained OpenCode `{ sessionID, diff, time }` handoff. `nix run .#pkl-check-generated` passed after temporarily staging `cli/migrations/001_create_diff_traces.sql` so the Git-backed Nix flake source included the untracked migration, then unstaging it.
  - **Notes:** Important-change classification for this execution: verify-only root context pass. The target current-state docs already matched code truth from the prior sync; this task only refreshed the context-map summary for the OpenCode plugin runtime link and recorded T04 status/evidence.

- [x] T05: `Validation and cleanup` (status:done)
  - Task ID: T05
  - Goal: Run full repo validation and remove incidental scaffolding or stale references introduced during implementation.
  - Boundaries (in/out of scope): In - formatting/lint/test validation, generated-output parity, search for stale `agent_traces` references and incorrect DB-only persistence claims, update this plan with validation evidence. Out - new feature work, cloud sync, broad refactors.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass or failures are captured with concrete follow-up; no unintended generated-output drift remains; plan task statuses/evidence are current.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect changed files to confirm `context/tmp/*-diff-trace.json` persistence remains documented and implemented alongside DB insertion.
  - **Completed:** 2026-04-29
  - **Files changed:** `context/plans/diff-trace-local-db-persistence.md`
  - **Evidence:** `nix run .#pkl-check-generated` passed; `nix flake check` passed; searched source/current-state docs for stale `agent_traces` references and incorrect DB-only persistence claims; inspected `cli/migrations/001_create_diff_traces.sql`, hook/local-DB references, and durable context references for retained `context/tmp/*-diff-trace.json` artifact behavior alongside additive `diff_traces` insertion.
  - **Notes:** Validation used temporary staging of `cli/migrations/001_create_diff_traces.sql` so Git-backed Nix flake checks included the new untracked migration, then restored the index afterward. No in-scope cleanup edits were needed beyond recording final validation evidence; remaining `agent_traces` matches are historical/plan-context references rather than active source/current-state drift. Important-change classification for this execution: verify-only root context pass.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0; output ended with `Generated outputs are up to date.`
- `nix flake check` -> exit 0; output ended with `all checks passed!`

### Cleanup and drift checks

- Temporary staging was used only so the Git-backed Nix flake source included `cli/migrations/001_create_diff_traces.sql`; the index was restored afterward.
- `context/tmp/` contains pre-existing ignored log files only (`sce-agent-trace.log`, `sce.log`); no validation-introduced scaffolding was left behind.
- Active CLI source search for `001_create_agent_traces`/`agent_traces` found no stale migration/table references; active matches are `diff_traces` only.
- Current-state context describes additive local DB insertion and retained `context/tmp/*-diff-trace.json` artifacts for accepted `sce hooks diff-trace` payloads.

### Success-criteria verification

- [x] Placeholder `agent_traces` migration is removed/replaced in source: `cli/migrations/001_create_agent_traces.sql` is deleted and `cli/migrations/001_create_diff_traces.sql` is present.
- [x] Embedded migration creates `diff_traces`: `cli/src/services/local_db.rs` embeds `001_create_diff_traces.sql`, whose SQL creates `diff_traces`.
- [x] `diff_traces` columns store incoming `time_ms`, `session_id`, `patch`, and DB-owned `created_at`: confirmed in `cli/migrations/001_create_diff_traces.sql`.
- [x] `LocalDb::new()` still opens/creates the canonical DB and runs embedded migrations: confirmed in `cli/src/services/local_db.rs`.
- [x] `sce hooks diff-trace` keeps existing STDIN validation for `sessionID`, `diff`, and `time`: confirmed in `cli/src/services/hooks.rs`.
- [x] Accepted `sce hooks diff-trace` payloads persist to both local DB and JSON artifact: confirmed in `cli/src/services/hooks.rs` via `persist_diff_trace_payload(...)` plus `persist_diff_trace_payload_to_local_db(...)`.
- [x] Non-diff hook behavior was not broadened: `pre-commit`, `post-commit`, and `post-rewrite` remain deterministic no-op paths; `commit-msg` remains attribution-gated.
- [x] Current-state context reflects DB-backed diff-trace persistence plus retained JSON artifact creation: confirmed in `context/sce/local-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/opencode-agent-trace-plugin-runtime.md`, and root context pointers.
- [x] Required validation passed: `nix run .#pkl-check-generated` and `nix flake check` both passed.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.

## Open questions

None. Clarified before planning:
- `sce hooks diff-trace` should use additive DB persistence while keeping the current `context/tmp/*-diff-trace.json` artifact creation.
- The placeholder `001_create_agent_traces.sql` migration should be replaced as `001_create_diff_traces.sql`, not followed by a `002` migration.
- The current payload shape is `{ "sessionID": string, "diff": string, "time": number }`; DB columns should preserve that information as session, patch/diff text, and timestamp milliseconds.

## Assumptions

- Local development DB instances with the placeholder `agent_traces` table do not require data migration because the existing migration is explicitly treated as placeholder source work.
- `patch` is the table column name for the incoming `diff` string because the persisted content is a unified patch/diff body.
- `created_at` is useful operational metadata and does not replace the incoming `time_ms` event timestamp.
