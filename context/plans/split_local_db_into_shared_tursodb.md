# Plan: Split local_db into shared TursoDb + agent_trace_db

## Change summary

Extract the duplicated Turso database infrastructure (runtime bridging, connection management, migration execution) from `local_db` into a shared generic `TursoDb<M: DbSpec>` struct. Create a new `agent_trace_db` service that uses the same shared infrastructure with its own migration (`diff_traces` table in `cli/migrations/agent-trace/`). Implement `DiffTraceInsert<'a>` and `insert_diff_trace()` as domain-specific additions on `AgentTraceDb`. Remove the old `agent_traces` migration from `local_db`.

## Success criteria

- `TursoDb<M: DbSpec>` generic struct handles: tokio runtime creation, Turso connection, `execute`/`query` methods, and migration execution
- `DbSpec` trait provides: `db_name()`, `db_path()`, `migrations()` — implemented per database
- `LocalDb` becomes a thin type alias over `TursoDb<LocalDbSpec>` with zero migrations
- `AgentTraceDb` is a thin type alias over `TursoDb<AgentTraceDbSpec>` with `DiffTraceInsert<'a>` and `insert_diff_trace()` method
- `agent_trace_db` has its own `lifecycle.rs` following the same `ServiceLifecycle` pattern
- `agent-trace.db` is created at `<state_root>/sce/agent-trace.db`
- `lifecycle_providers()` includes `AgentTraceDbLifecycle` in orchestration order
- `nix flake check` passes

## Constraints and non-goals

- **In scope**: shared `TursoDb` struct, `DbSpec` trait, `LocalDb` refactor, `AgentTraceDb` creation with `DiffTraceInsert` + `insert_diff_trace()`, lifecycle integration, path resolution, migration files
- **Out of scope**: query/retrieval logic for diff traces, agent-traces table schema, changes to `agent_trace.rs` domain model, `hooks.rs` integration (existing code already imports from `agent_trace_db`, will be wired in a follow-up)
- No new dependencies — reuse existing `turso`, `tokio`, `anyhow`
- Follow existing `local_db` patterns exactly (naming, error messages, lifecycle structure)
- Migrations for `agent_trace_db` live in `cli/migrations/agent-trace/` (separate subdirectory)

## Task stack

- [x] T01: `Create shared db module with TursoDb generic struct and DbSpec trait` (status:done)
  - Task ID: T01
  - Goal: Create `cli/src/services/db/mod.rs` containing a `DbSpec` trait and a generic `TursoDb<M: DbSpec>` struct that encapsulates the runtime, connection, execute/query methods, and migration execution logic currently in `local_db/mod.rs`.
  - Boundaries (in/out of scope): In — `DbSpec` trait (`fn db_name() -> &'static str`, `fn db_path() -> Result<PathBuf>`, `fn migrations() -> &'static [(&'static str, &'static str)]`), `TursoDb<M>` struct with `new()`, `execute()`, `query()`, `run_migrations()`. Out — no lifecycle code, no concrete DB specs, no domain-specific insert methods.
  - Done when: `TursoDb<M: DbSpec>` compiles, provides the same sync API as current `LocalDb`, and migration execution works generically via the trait.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'` — no compile errors.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/db/mod.rs`, `cli/src/services/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed.
  - Notes: Added the approved minimal `pub mod db;` export in T01 so the new shared module is compile-checked; broader `agent_trace_db` export/wiring remains in later tasks.

- [x] T02: `Refactor local_db to use TursoDb<LocalDbSpec> with empty migrations` (status:done)
  - Task ID: T02
  - Goal: Replace the existing `LocalDb` struct in `cli/src/services/local_db/mod.rs` with `pub type LocalDb = TursoDb<LocalDbSpec>` and implement `DbSpec` for `LocalDbSpec` (returns `local_db_path()`, empty `&[]` migrations). Delete `cli/migrations/001_create_agent_traces.sql`.
  - Boundaries (in/out of scope): In — `local_db/mod.rs` refactor, `LocalDbSpec` impl, migration file deletion. Out — lifecycle changes (covered in T04), any new tables.
  - Done when: `LocalDb::new()` still works identically (opens DB at same path, runs zero migrations), old migration file is removed, `local_db/mod.rs` is a thin wrapper (~15 lines).
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`; verify `local_db/mod.rs` is minimal.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/local_db/mod.rs`, `cli/migrations/001_create_agent_traces.sql`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed.
  - Notes: `LocalDb` is now a `TursoDb<LocalDbSpec>` alias with zero migrations and the old `agent_traces` migration file removed; lifecycle changes remain T04 scope.

- [x] T03: `Create agent_trace_db service with DiffTraceInsert and insert_diff_trace` (status:done)
  - Task ID: T03
  - Goal: Create `cli/src/services/agent_trace_db/mod.rs` with: `AgentTraceDbSpec` implementing `DbSpec` (path: `agent_trace_db_path()` → `<state_root>/sce/agent-trace.db`, migrations loaded from `cli/migrations/agent-trace/001_create_diff_traces.sql`), `pub type AgentTraceDb = TursoDb<AgentTraceDbSpec>`, `DiffTraceInsert<'a>` struct (`time_ms: i64`, `session_id: &'a str`, `patch: &'a str`), `INSERT_DIFF_TRACE_SQL` constant, and `impl AgentTraceDb { fn insert_diff_trace(&self, input: DiffTraceInsert<'_>) -> Result<u64> }`. Create `cli/migrations/agent-trace/001_create_diff_traces.sql`. Add `agent_trace_db_path()` to `default_paths.rs`.
  - Boundaries (in/out of scope): In — new module, new migration SQL, new path function, type alias, domain struct, insert method. Out — lifecycle code (T04), query/retrieval methods.
  - Done when: `AgentTraceDb::new()` creates `agent-trace.db` at correct path with `diff_traces` table; `AgentTraceDb::insert_diff_trace()` inserts a row via parameterized SQL; `agent_trace_db_path()` resolves correctly.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`; verify migration SQL matches spec (`diff_traces` table with `id`, `time_ms`, `session_id`, `patch`, `created_at`).
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/agent_trace_db/mod.rs`, `cli/migrations/agent-trace/001_create_diff_traces.sql`, `cli/src/services/default_paths.rs`, `cli/src/services/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed; migration SQL verified with `diff_traces` columns `id`, `time_ms`, `session_id`, `patch`, and `created_at`.
  - Notes: Added minimal `pub mod agent_trace_db;` wiring so the new module is compiled during T03 checks; lifecycle registration remains T04 scope.

- [x] T04: `Add AgentTraceDbLifecycle and integrate into lifecycle_providers` (status:done)
  - Task ID: T04
  - Goal: Create `cli/src/services/agent_trace_db/lifecycle.rs` implementing `ServiceLifecycle` for `AgentTraceDbLifecycle` (diagnose path health, fix parent dir, setup via `AgentTraceDb::new()`). Extract shared lifecycle helper functions from `local_db/lifecycle.rs` into the shared `db` module (`collect_db_path_health()`, `bootstrap_db_parent()`) so both lifecycle impls reuse them. Register `AgentTraceDbLifecycle` in `lifecycle_providers()`.
  - Boundaries (in/out of scope): In — `agent_trace_db/lifecycle.rs`, shared helpers in `db/`, `lifecycle_providers()` update, `local_db/lifecycle.rs` refactor to use shared helpers. Out — changes to `ServiceLifecycle` trait itself.
  - Done when: `sce doctor` checks both DB paths; `sce doctor --fix` can bootstrap both parent dirs; `sce setup` initializes both databases; lifecycle provider order is config → local_db → agent_trace_db → hooks.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`; verify `lifecycle_providers(true)` returns 4 providers.
  - Completed: 2026-04-30
  - Files changed: `cli/src/services/agent_trace_db/lifecycle.rs`, `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/db/mod.rs`, `cli/src/services/local_db/lifecycle.rs`, `cli/src/services/lifecycle.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed; `nix flake check` passed, including CLI tests/clippy/fmt and provider-count test coverage; `nix run .#pkl-check-generated` passed after context sync.
  - Notes: Added shared DB path-health/bootstrap helpers, reused them from local DB lifecycle, added agent trace DB lifecycle setup/diagnose/fix, and registered provider order as config → local_db → agent_trace_db → hooks. Context sync classified this as an important lifecycle/architecture change and updated root/domain context. Direct targeted `cargo test lifecycle_providers_include_agent_trace_db -- --exact` was blocked by the repository bash policy in favor of `nix flake check`.

- [x] T05: `Update services/mod.rs exports and verify module wiring` (status:done)
  - Task ID: T05
  - Goal: Add `pub mod db;` and `pub mod agent_trace_db;` to `cli/src/services/mod.rs`. Ensure all public types (`AgentTraceDb`, `DiffTraceInsert`, `LocalDb`) are correctly re-exported.
  - Boundaries (in/out of scope): In — `mod.rs` exports, compilation verification. Out — any behavioral changes.
  - Done when: `nix develop -c sh -c 'cd cli && cargo check'` passes with no errors or warnings related to the new modules.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`.
  - Completed: 2026-04-30
  - Files changed: No code files changed; `cli/src/services/mod.rs` already exported `db` and `agent_trace_db`, and public types remain reachable through their public modules.
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; existing manifest metadata warnings only (`bin.0.categories`, `bin.0.include`, `bin.0.keywords`) and no module-wiring errors.
  - Notes: T05 was a verification-only task because T01/T03 had already added the minimal module exports required for compilation.

- [x] T06: `Final validation and context sync` (status:done)
  - Task ID: T06
  - Goal: Run full repo verification, remove any temporary scaffolding, and confirm all success criteria are met.
  - Boundaries (in/out of scope): In — `nix flake check`, clippy, format verification, context sync. Out — new feature work.
  - Done when: `nix flake check` passes; `nix develop -c sh -c 'cd cli && cargo clippy'` has no warnings; `context/` files reflect the new architecture.
  - Verification notes (commands or checks): `nix flake check`; `nix develop -c sh -c 'cd cli && cargo clippy'`; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'`.
  - Completed: 2026-04-30
  - Files changed: `context/plans/split_local_db_into_shared_tursodb.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed; `nix run .#pkl-check-generated` passed; `nix develop -c sh -c 'cd cli && cargo clippy'` passed with only existing Cargo manifest metadata warnings (`bin.0.categories`, `bin.0.include`, `bin.0.keywords`); `nix flake check` passed.
  - Notes: No task-owned temporary scaffolding was found. Existing `context/tmp/` diff-trace/post-commit runtime artifacts were left unchanged. Current context already reflects the shared Turso DB plus Agent Trace DB architecture; final validation remains captured below by the final-task validation pass.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` -> exit 0; Rust formatting check passed.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix develop -c sh -c 'cd cli && cargo clippy'` -> exit 0; no clippy warnings. Cargo emitted existing manifest metadata warnings for `bin.0.categories`, `bin.0.include`, and `bin.0.keywords`.
- `nix flake check` -> exit 0; all flake checks passed (`cli-tests`, `cli-clippy`, `cli-fmt`, integration install checks, `pkl-parity`, npm JS checks, and config-lib JS checks).

### Temporary scaffolding

- No task-owned temporary scaffolding was found or removed.
- Existing tracked/ignored runtime artifacts under `context/tmp/` were not introduced by this final validation task and were left unchanged.

### Success-criteria verification

- [x] `TursoDb<M: DbSpec>` generic struct handles tokio runtime creation, Turso connection, `execute`/`query`, and migration execution — confirmed in `cli/src/services/db/mod.rs`; `nix flake check` passed.
- [x] `DbSpec` provides `db_name()`, `db_path()`, and `migrations()` per database — confirmed in `cli/src/services/db/mod.rs`, `cli/src/services/local_db/mod.rs`, and `cli/src/services/agent_trace_db/mod.rs`.
- [x] `LocalDb` is a thin alias over `TursoDb<LocalDbSpec>` with zero migrations — confirmed in `cli/src/services/local_db/mod.rs`.
- [x] `AgentTraceDb` is a thin alias over `TursoDb<AgentTraceDbSpec>` with `DiffTraceInsert<'a>` and `insert_diff_trace()` — confirmed in `cli/src/services/agent_trace_db/mod.rs`.
- [x] `agent_trace_db` has its own `lifecycle.rs` following `ServiceLifecycle` — confirmed in `cli/src/services/agent_trace_db/lifecycle.rs`.
- [x] `agent-trace.db` is created at `<state_root>/sce/agent-trace.db` — confirmed by `agent_trace_db_path()` in `cli/src/services/default_paths.rs` and `AgentTraceDbSpec::db_path()`.
- [x] `lifecycle_providers()` includes `AgentTraceDbLifecycle` in orchestration order — confirmed in `cli/src/services/lifecycle.rs` (`config → local_db → agent_trace_db → hooks`).
- [x] `nix flake check` passes — confirmed exit 0.
- [x] Context reflects final implemented behavior — confirmed by `context/sce/shared-turso-db.md`, `context/sce/local-db.md`, `context/sce/agent-trace-db.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, and `context/context-map.md`.

### Failed checks and follow-ups

- None.

### Residual risks

- Hook-runtime writes to `AgentTraceDb` and diff-trace query/retrieval APIs remain explicit follow-up scope, as planned.

## Open questions

None — all requirements resolved via clarification.
