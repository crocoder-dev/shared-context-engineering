# Split local.db into local.db + agent-trace.db

## Change summary

Separate the current single Turso database (`local.db`) into two databases:
- `local.db` — remains as a database with an empty migration (no tables), reserved for future SCE runtime data.
- `agent-trace.db` — new database that owns the `diff_traces` table and all diff-trace persistence logic.

A new service module `agent_trace_db.rs` is created following the same structural pattern as `local_db.rs`. The `hooks.rs` module is updated to use `AgentTraceDb` instead of `LocalDb` for diff-trace persistence.

## Success criteria

- `agent_trace_db.rs` exists and mirrors the `LocalDb` pattern (Turso connection, tokio runtime, migrations, `execute`, `query`, `insert_diff_trace`).
- `agent-trace.db` is created at `<state_root>/sce/agent-trace.db` and contains the `diff_traces` table.
- `local.db` migration list is empty (no tables created).
- `hooks.rs` persists diff-trace payloads to `AgentTraceDb` instead of `LocalDb`.
- `default_paths.rs` exposes `agent_trace_db_path()`.
- `mod.rs` registers the new `agent_trace_db` module.
- `nix flake check` passes (build, clippy, fmt, tests).

## Constraints and non-goals

- **In scope**: new `agent_trace_db.rs` service, new migration SQL, `default_paths.rs` addition, `local_db.rs` migration cleanup, `hooks.rs` import/update, `mod.rs` registration.
- **Out of scope**: schema changes to `diff_traces` table, new tables, new features, changes to file-based trace persistence in `hooks.rs`.
- Follow existing `local_db.rs` patterns exactly (struct shape, runtime setup, error messages, `#[allow(dead_code)]` usage).
- No new dependencies; reuse `turso`, `tokio`, `anyhow`.

## Task stack

- [x] T01: `Add agent_trace_db_path() to default_paths.rs` (status:done)
  - Task ID: T01
  - Goal: Add a `agent_trace_db_path()` function that returns `<state_root>/sce/agent-trace.db`, mirroring the existing `local_db_path()` pattern.
  - Boundaries (in/out of scope): In — one new public function in `default_paths.rs`. Out — no other changes to `default_paths.rs`.
  - Done when: `agent_trace_db_path()` compiles and returns the correct path under the SCE state root.
  - Verification notes (commands or checks): `nix flake check` (compilation + clippy).
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/services/default_paths.rs`
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all clean)
  - **Notes:** Added `#[allow(dead_code)]` since function is consumed in later tasks

- [x] T02: `Create migration 001 for agent-trace.db with diff_traces table` (status:done)
  - Task ID: T02
  - Goal: Create `cli/migrations/agent-trace/001_create_diff_traces.sql` containing the existing `diff_traces` table DDL (copied verbatim from the current `001_create_diff_traces.sql`).
  - Boundaries (in/out of scope): In — new SQL file under a dedicated `agent-trace/` subdirectory. Out — no changes to existing `cli/migrations/001_create_diff_traces.sql` (handled in T04).
  - Done when: SQL file exists with correct `CREATE TABLE IF NOT EXISTS diff_traces` statement.
  - Verification notes (commands or checks): File exists and content matches the current migration DDL.
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/migrations/agent-trace/001_create_diff_traces.sql`
  - **Evidence:** `nix flake check` passed (all checks clean)

- [x] T03: `Create agent_trace_db.rs service module` (status:done)
  - Task ID: T03
  - Goal: Create `cli/src/services/agent_trace_db.rs` following the `local_db.rs` pattern: `AgentTraceDb` struct with `conn: turso::Connection` and `runtime: tokio::runtime::Runtime`, `new()` constructor using `agent_trace_db_path()`, `execute()`, `query()`, `insert_diff_trace()`, `DiffTraceInsert` struct, embedded migrations list, and `run_migrations()`.
  - Boundaries (in/out of scope): In — full service module mirroring `local_db.rs` structure but pointing at `agent-trace.db` and loading migrations from `cli/migrations/agent-trace/`. Out — no changes to `local_db.rs` or `hooks.rs` yet.
  - Done when: `agent_trace_db.rs` compiles, `AgentTraceDb::new()` opens/creates `agent-trace.db`, runs migrations, and `insert_diff_trace()` works.
  - Verification notes (commands or checks): `nix flake check` (compilation + clippy).
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/services/agent_trace_db.rs`, `cli/src/services/mod.rs`, `cli/migrations/agent-trace/001_create_diff_traces.sql`, `flake.nix`
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity all clean)
  - **Notes:** Followed `local_db.rs` pattern exactly. Added `pub mod agent_trace_db;` to `mod.rs` (part of T07, but needed for compilation verification). Added `cli/src/services/agent_trace_db.rs` to `flake.nix` fileset for Nix build.

- [x] T04: `Empty local.db migrations` (status:done)
  - Task ID: T04
  - Goal: Remove the `diff_traces` migration from `local_db.rs` — clear the `MIGRATIONS` list to empty and remove the `MIGRATION_001` constant, `INSERT_DIFF_TRACE_SQL` constant, `DiffTraceInsert` struct, and `insert_diff_trace()` method. Keep `LocalDb` struct, `new()`, `execute()`, `query()`, and `run_migrations()` (which becomes a no-op).
  - Boundaries (in/out of scope): In — strip diff-trace-specific code from `local_db.rs`. Out — do not remove `LocalDb` itself; it stays as a valid empty database adapter.
  - Done when: `local_db.rs` compiles with an empty migrations list and no diff-trace methods; `LocalDb::new()` creates `local.db` with no tables.
  - Verification notes (commands or checks): `nix flake check` (compilation + clippy).
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/services/local_db.rs`; synced current-state context in `context/sce/local-db.md`, `context/sce/agent-trace-db.md`, `context/architecture.md`, `context/overview.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, and `context/cli/cli-command-surface.md`.
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix flake check` passed (all checks clean); `nix run .#pkl-check-generated` passed after context sync.
  - **Blocked:** 2026-04-29
  - **Blocked evidence:** First attempt applied the scoped `local_db.rs` cleanup, then `nix develop -c sh -c 'cd cli && cargo check'` failed because `hooks.rs` still imported `local_db::DiffTraceInsert` and called `LocalDb::insert_diff_trace()` at that time.
  - **Unblocked:** 2026-04-29 by T08 completion.
  - **Notes:** User chose to revert the scoped code edit rather than expand into T08 during the first T04 attempt. T08 was completed first, then T04 removed the local-db diff-trace migration/API while preserving the empty `LocalDb` adapter.

- [x] T05: `Add bootstrap_agent_trace_db() to setup.rs` (status:done)
  - Task ID: T05
  - Goal: Add a `bootstrap_agent_trace_db()` function to `setup.rs` that calls `AgentTraceDb::new()`, mirroring the existing `bootstrap_local_db()` pattern.
  - Boundaries (in/out of scope): In — one new public function in `setup.rs`. Out — no changes to `app.rs` yet.
  - Done when: `bootstrap_agent_trace_db()` compiles and creates `agent-trace.db` with migrations applied.
  - Verification notes (commands or checks): `nix flake check` (compilation + clippy).
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/services/setup.rs`; synced current-state context in `context/sce/agent-trace-db.md`, `context/sce/setup-repo-local-config-bootstrap.md`, and `context/context-map.md`.
  - **Evidence:** Initial `nix develop -c sh -c 'cd cli && cargo check'` failed because the new helper is intentionally unused until T06 and warnings are denied; added scoped `#[allow(dead_code)]`, then `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo clippy'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed after context sync. Direct `cargo fmt --check` was blocked by SCE bash-tool policy in favor of `nix flake check`, whose `cli-fmt` check passed.
  - **Notes:** Added the setup bootstrap helper only; app setup-flow invocation remains deferred to T06.

- [x] T06: `Call bootstrap_agent_trace_db() in app.rs setup flow` (status:done)
  - Task ID: T06
  - Goal: Add `services::setup::bootstrap_agent_trace_db()` call in `app.rs` right after the existing `bootstrap_local_db()` call in the `SetupCommand::execute` method.
  - Boundaries (in/out of scope): In — one new line in `app.rs` setup flow. Out — no other changes to `app.rs`.
  - Done when: `sce setup` creates both `local.db` and `agent-trace.db`.
  - Verification notes (commands or checks): `nix flake check` (compilation + clippy).
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/app.rs`; synced current-state context in `context/sce/agent-trace-db.md`, `context/sce/setup-repo-local-config-bootstrap.md`, `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/cli/cli-command-surface.md`.
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo clippy'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed after context sync (cli-tests, cli-clippy, cli-fmt, pkl-parity all clean).
  - **Notes:** Setup dispatch now bootstraps `agent-trace.db` immediately after the existing `local.db` bootstrap; context sync classified this as a root-visible setup behavior update and refreshed affected setup/DB docs.

- [x] T07: `Register agent_trace_db in mod.rs` (status:done)
  - Task ID: T07
  - Goal: Add `pub mod agent_trace_db;` to `cli/src/services/mod.rs`.
  - Boundaries (in/out of scope): In — one line addition to `mod.rs`. Out — no other module changes.
  - Done when: `mod.rs` includes `agent_trace_db` and the project compiles.
  - Verification notes (commands or checks): `nix flake check` (compilation).
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/services/mod.rs`
  - **Evidence:** `cli/src/services/mod.rs` includes `pub mod agent_trace_db;`; `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix run .#pkl-check-generated` passed; `nix flake check` passed.
  - **Notes:** Plan status reconciled with code truth; the module registration was introduced during earlier service-module work and remained present before this task execution.

- [x] T08: `Update hooks.rs to use AgentTraceDb` (status:done)
  - Task ID: T08
  - Goal: Update `cli/src/services/hooks.rs` to import `AgentTraceDb` and `DiffTraceInsert` from `agent_trace_db` instead of `local_db`, and change `persist_diff_trace_payload_to_local_db` to use `AgentTraceDb::new()` and `insert_diff_trace()`.
  - Boundaries (in/out of scope): In — import change and DB adapter swap in `persist_diff_trace_payload_to_local_db`. Out — no changes to payload parsing, file-based persistence, or other hook subcommands.
  - Done when: `hooks.rs` compiles, diff-trace payloads are written to `agent-trace.db` instead of `local.db`.
  - Verification notes (commands or checks): `nix flake check` (compilation + clippy + tests).
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `cli/src/services/hooks.rs`
  - **Evidence:** `nix develop -c sh -c 'cd cli && cargo check'` passed; first `nix flake check` surfaced rustfmt-only formatting drift, `nix develop -c sh -c 'cd cli && cargo fmt'` fixed it, then `nix flake check` passed.
  - **Notes:** `sce hooks diff-trace` now opens `AgentTraceDb` for DB persistence while retaining existing payload parsing and `context/tmp` artifact persistence.

- [x] T09: `Validation and cleanup` (status:done)
  - Task ID: T09
  - Goal: Run full repo validation, verify both databases are created correctly during `sce setup`, and sync context.
  - Boundaries (in/out of scope): In — `nix flake check`, verify DB creation, context sync. Out — no code changes.
  - Done when: `nix flake check` passes; `local.db` exists with no tables; `agent-trace.db` exists with `diff_traces` table; context files updated if needed.
  - Verification notes (commands or checks): `nix flake check`; optionally inspect DBs with `sqlite3` to confirm table presence/absence.
  - **Status:** done
  - **Completed:** 2026-04-29
  - **Files changed:** `context/plans/split-local-db-agent-trace-db.md`
  - **Evidence:** `nix run .#pkl-check-generated` passed; `nix flake check` passed; isolated `sce setup --both --hooks` validation passed with `local_db_tables: []` and `agent_trace_db_tables: ["diff_traces"]`.
  - **Notes:** No application code changes were required. Temporary validation artifacts under `context/tmp/t09-split-local-db-agent-trace-db-20260429165139` were removed after inspection.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0; all evaluated checks passed.
- Isolated setup/database validation -> exit 0; ran `sce setup --both --hooks` in a temporary git repository with isolated `XDG_STATE_HOME`/`XDG_CONFIG_HOME`, then inspected the created database files with Python `sqlite3`.
- Cleanup -> exit 0; removed temporary validation artifacts from `context/tmp/t09-split-local-db-agent-trace-db-20260429165139`.

### Success-criteria verification

- [x] `agent_trace_db.rs` exists and mirrors the `LocalDb` pattern: confirmed by code/context sync and successful full flake checks.
- [x] `agent-trace.db` is created at `<state_root>/sce/agent-trace.db` and contains `diff_traces`: isolated validation output reported `agent_trace_db_exists: true` and `agent_trace_db_tables: ["diff_traces"]`.
- [x] `local.db` migration list is empty and creates no tables: isolated validation output reported `local_db_exists: true` and `local_db_tables: []`.
- [x] `hooks.rs` persists diff-trace payloads to `AgentTraceDb`: confirmed by current-state context sync and successful compile/test checks.
- [x] `default_paths.rs` exposes `agent_trace_db_path()`: confirmed by current-state code/context and successful compile checks.
- [x] `mod.rs` registers the new `agent_trace_db` module: confirmed in T07 and successful compile checks.
- [x] `nix flake check` passes: command completed with exit 0.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.

## Open questions

None — all requirements are clear from the existing codebase patterns.
