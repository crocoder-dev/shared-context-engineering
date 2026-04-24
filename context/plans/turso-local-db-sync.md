# Plan: Add Turso Local DB for Agent Traces

## Change Summary
Implement a local Turso database for storing agent traces. This includes adding the `turso` dependency, creating SQL migrations embedded in the CLI, implementing a local DB adapter service (`cli/src/services/local_db.rs`), and exposing initialization through a `sync` command (local-only, no cloud sync in this phase).

## Success Criteria
- `turso` is added as a dependency and compiles successfully
- SQL migrations are embedded in the CLI under `cli/migrations/`
- `cli/src/services/local_db.rs` initializes a local Turso DB and runs migrations
- `cli/src/services/sync.rs` implements the `sync` command that initializes the DB
- The `sync` command is wired up and accessible via `sce sync`
- Context files are updated to reflect the new state
- `nix flake check` passes

## Constraints and Non-Goals
- **Local DB only**: No cloud sync implementation in this plan
- **Agent traces only**: Store only agent trace data (no other SCE state)
- **Embedded migrations**: Migrations are embedded at compile time using `include_str!` from `cli/migrations/`
- **Neutral visibility**: The `sync` command is user-invocable but hidden from top-level `sce --help` initially (similar to `auth` and `hooks`)
- **Schema approach**: Store agent traces as JSON blobs in a simple table, leveraging the existing `AgentTrace` serde serialization

## Task Stack

- [x] T01: Add turso dependency to cli/Cargo.toml (status:done)
  - Task ID: T01
  - Goal: Add the `turso` crate as a dependency to the CLI package
  - Boundaries (in/out of scope): In - Cargo.toml edit, verify compilation. Out - code changes, migration files, service implementation
  - Done when: `turso` is in Cargo.toml dependencies, `nix develop -c sh -c 'cd cli && cargo check'` succeeds
  - Verification notes (commands or checks): `grep -q '^turso' cli/Cargo.toml`, `nix develop -c sh -c 'cd cli && cargo check'`
  - **Completed:** 2026-04-24
  - **Files changed:** cli/Cargo.toml, cli/Cargo.lock
  - **Evidence:** `nix flake check` passes, turso 0.1.5 in Cargo.toml and Cargo.lock

- [x] T02: Create cli/migrations/ with initial agent traces schema (status:done)
  - Task ID: T02
  - Goal: Create the migrations directory and initial SQL migration file for agent traces
  - Boundaries (in/out of scope): In - cli/migrations/001_create_agent_traces.sql with table definition. Out - embedding code, Rust code changes
  - Done when: cli/migrations/001_create_agent_traces.sql exists with CREATE TABLE statement for agent_traces
  - Verification notes (commands or checks): `test -f cli/migrations/001_create_agent_traces.sql`, verify SQL syntax is valid SQLite/Turso SQL
  - **Completed:** 2026-04-24
  - **Files changed:** cli/migrations/001_create_agent_traces.sql
  - **Evidence:** Migration file created with agent_traces table (id INTEGER PRIMARY KEY AUTOINCREMENT, trace_json TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))

- [x] T03: Implement cli/src/services/local_db.rs - Turso adapter (status:done)
  - Task ID: T03
  - Goal: Create the local DB module that initializes Turso, runs embedded migrations, and provides query/execute methods
  - Boundaries (in/out of scope): In - local_db.rs with Turso Builder, migration runner using embedded SQL via `include_str!`, basic error handling with `anyhow`. Out - cloud sync, agent trace serialization/deserialization
  - Done when: local_db.rs exists, can initialize a DB at a path, runs migrations on startup, has basic query/execute methods
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`, unit tests for local_db if applicable
  - **Completed:** 2026-04-24
  - **Files changed:** cli/src/services/local_db.rs, cli/src/services/mod.rs, cli/src/services/default_paths.rs, flake.nix
  - **Evidence:** `nix flake check` passes, LocalDb struct with new/execute/query methods, embedded migrations from cli/migrations/, local_db_path() in default_paths.rs

- [ ] T04: Implement cli/src/services/sync.rs - sync command (status:todo)
  - Task ID: T04
  - Goal: Implement the sync command that initializes the local DB and reports status
  - Boundaries (in/out of scope): In - sync.rs with SyncRequest struct, calls local_db to init/status, prints DB path and status. Out - cloud sync, agent trace insertion
  - Done when: sync.rs exists with command implementation, `sce sync` initializes the DB and prints status
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo run -- sync'`, verify DB file is created at expected path

- [ ] T05: Wire up sync command in cli_schema.rs and app.rs (status:todo)
  - Task ID: T05
  - Goal: Add Sync variant to Commands enum and wire up the RuntimeCommand execution
  - Boundaries (in/out of scope): In - cli_schema.rs Commands enum addition (hidden from help), app.rs convert_clap_command and SyncCommand RuntimeCommand implementation. Out - help text changes, top-level visibility changes
  - Done when: `sce sync` invokes the sync service, `sce sync --help` shows help
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo run -- sync --help'`, verify command executes

- [ ] T06: Update context files (status:todo)
  - Task ID: T06
  - Goal: Update overview.md, glossary.md, and context-map.md to reflect the new local DB and sync command state
  - Boundaries (in/out of scope): In - context/overview.md (update CLI dependencies, sync command description), context/glossary.md (add/modify local_db and sync entries), context/context-map.md if needed. Out - other context files, plan files
  - Done when: Context files accurately describe the implemented local DB and sync command
  - Verification notes (commands or checks): Review updated context files for accuracy, verify glossary entries match implementation

- [ ] T07: Validation and cleanup (status:todo)
  - Task ID: T07
  - Goal: Run full validation suite and clean up any issues
  - Boundaries (in/out of scope): In - nix flake check, build verification, context sync. Out - additional features, cloud sync
  - Done when: `nix flake check` passes, `nix run .#pkl-check-generated` passes
  - Verification notes (commands or checks): `nix flake check`, `nix run .#pkl-check-generated`

## Open Questions
None - all clarification questions were resolved before planning.

## Assumptions
- The `turso` Rust crate provides a compatible API with `turso::Builder::new_local(path)` for local database creation
- Agent traces will be stored as JSON blobs using the existing `AgentTrace` serde serialization
- The local DB path follows the existing default path catalog in `cli/src/services/default_paths.rs` (e.g., `$XDG_STATE_HOME/sce/local.db`)
- Migration numbering starts at 001 and uses sequential numbering
