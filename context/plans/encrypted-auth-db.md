# Plan: Encrypted auth DB module

## Change summary

Create a new `auth_db` service for encrypted local persistence of auth tokens and related user information. The service will mirror the structure and conventions of `local_db` and `agent_trace_db`, use the shared `EncryptedTursoDb<M: DbSpec>` adapter, embed ordered SQL migrations from a new auth migration directory, and expose lifecycle setup/doctor integration through `lifecycle.rs`.

## Success criteria

- `cli/src/services/auth_db/mod.rs` exists and follows the thin database-wrapper pattern used by `local_db` and `agent_trace_db`.
- `AuthDb` is a type alias for `EncryptedTursoDb<AuthDbSpec>`.
- `AuthDbSpec` implements `DbSpec` with a canonical auth DB path, diagnostic name, and ordered embedded migrations.
- A new auth migration directory exists under `cli/migrations/auth/`.
- The baseline migration creates a single `auth_tokens` table with:
  - `id` required primary key, without `AUTOINCREMENT`
  - `access_token` required
  - `token_type` required
  - `expires_in` required
  - `refresh_token` required
  - `scope` optional
  - `stored_at_unix_seconds` required
  - `email` required
  - `created_at` required
- All `auth_tokens` columns are `NOT NULL` except `scope`.
- An index exists on `auth_tokens(email)`.
- `cli/src/services/auth_db/lifecycle.rs` follows the existing `ServiceLifecycle` pattern for path diagnosis, parent bootstrap, and setup initialization through `AuthDb::new()`.
- Required module/path/lifecycle wiring compiles without changing runtime auth-token read/write behavior yet.
- `nix flake check` passes.

## Constraints and non-goals

- **In scope**: new `auth_db` module files, new auth migration SQL files, canonical path resolver, service export, lifecycle provider registration, and context sync for the new current-state DB surface.
- **Out of scope**: replacing existing auth token storage, adding auth runtime reads/writes, token refresh behavior, token encryption/key management beyond using the existing `EncryptedTursoDb` adapter and `SCE_DB_ENCRYPTION_KEY`, cloud sync behavior, and any schema beyond the requested single `auth_tokens` table plus email index.
- Reuse existing dependencies and database infrastructure; do not add a new database library.
- Follow existing naming, migration embedding, lifecycle, and error-context conventions from `local_db` and `agent_trace_db`.
- Use forward-only embedded migrations consistent with current database modules.

## Assumptions

- The module path is `cli/src/services/auth_db/{mod.rs,lifecycle.rs}`.
- The table name is `auth_tokens`.
- The canonical database path should be added to the shared default-path catalog as `<state_root>/sce/auth.db`, unless implementation review identifies an already-approved auth DB path in the PR.
- “Wiring” means the minimal non-runtime integration needed for the new module to compile and participate in setup/doctor lifecycle flows: `services/mod.rs`, `default_paths.rs`, and `services/lifecycle.rs` updates. It does not include changing auth command/token-storage behavior.

## Task stack

- [x] T01: `Add auth DB path and migration files` (status:done)
  - Task ID: T01
  - Goal: Add the canonical auth DB path resolver and auth migration SQL files that define the requested encrypted database schema.
  - Boundaries (in/out of scope): In — add `auth_db_path()` to `cli/src/services/default_paths.rs`; create `cli/migrations/auth/001_create_auth_tokens.sql`; create `cli/migrations/auth/002_create_auth_tokens_email_index.sql` or equivalent ordered split migrations. Out — no Rust `auth_db` module implementation yet, no auth runtime writes, no lifecycle provider registration.
  - Done when: The path resolver returns `<state_root>/sce/auth.db`; the baseline table migration creates `auth_tokens` with the requested columns, `id` primary key without `AUTOINCREMENT`, and all columns `NOT NULL` except `scope`; the email index migration creates an index on `email`; migration SQL is idempotent in the same style as Agent Trace DB migrations.
  - Verification notes (commands or checks): Inspect SQL for schema compliance; run targeted Rust compile/format checks during implementation if path changes require compile validation.
  - Completed: 2026-05-25
  - Files changed: `cli/src/services/default_paths.rs`, `cli/migrations/auth/001_create_auth_tokens.sql`, `cli/migrations/auth/002_create_auth_tokens_email_index.sql`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed; SQL inspection confirmed the required `auth_tokens` table columns/constraints and `idx_auth_tokens_email` idempotent index.
  - Context sync classification: localized implementation change with default-path/current-state drift repaired in root and domain context during `sce-context-sync`.

- [x] T02: `Create auth_db mod.rs using EncryptedTursoDb` (status:done)
  - Task ID: T02
  - Goal: Create `cli/src/services/auth_db/mod.rs` as the encrypted database wrapper for auth token persistence.
  - Boundaries (in/out of scope): In — define `AuthDbSpec`, `pub type AuthDb = EncryptedTursoDb<AuthDbSpec>`, embed ordered auth migrations with `include_str!`, implement `DbSpec` using `auth_db_path()`, and expose `pub mod lifecycle;`. Out — no domain-specific insert/query helpers unless needed for compile-only tests, no auth command integration, no token-storage replacement.
  - Done when: `AuthDb::new()` would open the encrypted auth DB through `EncryptedTursoDb`, require `SCE_DB_ENCRYPTION_KEY` via the shared adapter, and run the auth migrations through `__sce_migrations`.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`; inspect that the module mirrors `local_db`/`agent_trace_db` naming and migration style.
  - Completed: 2026-05-25
  - Files changed: `cli/src/services/auth_db/mod.rs`, `cli/src/services/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo fmt -- --check'` passed; inspection confirmed `AuthDbSpec` uses `auth_db_path()`, migration IDs remain ordered, and `AuthDb` aliases `EncryptedTursoDb<AuthDbSpec>`.
  - Notes: `pub mod lifecycle;` and `cli/src/services/auth_db/lifecycle.rs` remain deferred to T03 per readiness decision; `AuthDb` is marked `#[allow(dead_code)]` until lifecycle/runtime wiring consumes it.
  - Context sync classification: important localized DB-service state change; updated auth DB and shared Turso/domain context plus root discoverability/current-state references.

- [x] T03: `Add auth DB lifecycle integration` (status:done)
  - Task ID: T03
  - Goal: Create `cli/src/services/auth_db/lifecycle.rs` and wire the provider into the shared lifecycle catalog.
  - Boundaries (in/out of scope): In — implement `AuthDbLifecycle` with `diagnose`, `fix`, and `setup` following `LocalDbLifecycle` and `AgentTraceDbLifecycle`; use shared `collect_db_path_health()` and `bootstrap_db_parent()` helpers; add a `LifecycleProviderId::AuthDb`; register `AuthDbLifecycle` in deterministic provider order; export `pub mod auth_db;` from `services/mod.rs`. Out — no doctor renderer redesign, no setup output shape changes beyond existing lifecycle aggregation behavior.
  - Done when: Setup initializes the encrypted auth DB through lifecycle aggregation, doctor/fix can diagnose/bootstrap the auth DB parent path, and provider order remains deterministic with auth DB placed alongside the other DB providers.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`; inspect `lifecycle_providers(include_hooks)` order and provider ID coverage.
  - Completed: 2026-05-25
  - Files changed: `cli/src/services/auth_db/lifecycle.rs`, `cli/src/services/auth_db/mod.rs`, `cli/src/services/lifecycle.rs`
  - Evidence: `cargo check` passed; `cargo fmt -- --check` passed; provider order confirmed: config → `local_db` → `auth_db` → `agent_trace_db` → hooks; `LifecycleProviderId::AuthDb` added to enum.
  - Context sync classification: important change — new lifecycle provider and enum variant affect cross-cutting service lifecycle behavior.

- [ ] T04: `Add focused tests for auth DB schema and lifecycle wiring` (status:todo)
  - Task ID: T04
  - Goal: Add narrow tests that prove the new auth DB migration list and lifecycle wiring stay deterministic.
  - Boundaries (in/out of scope): In — module-level tests or existing lifecycle tests verifying migration IDs/order, `auth_tokens` schema/index SQL presence, path resolver behavior if covered by existing default-path tests, and lifecycle provider inclusion/order. Out — integration tests that require real auth login, WorkOS calls, or production token data.
  - Done when: Tests fail if the auth DB provider is not registered, migration ordering changes unexpectedly, or the required table/index SQL is missing required constraints.
  - Verification notes (commands or checks): Prefer targeted test commands during implementation, then rely on `nix flake check` for final coverage.

- [ ] T05: `Sync context for encrypted auth DB current state` (status:todo)
  - Task ID: T05
  - Goal: Update durable context so future sessions know the auth DB module, encrypted adapter usage, migration schema, path, and lifecycle registration exist.
  - Boundaries (in/out of scope): In — update focused context files such as `context/sce/shared-turso-db.md`, a new or existing auth DB context file, `context/context-map.md`, and glossary/overview entries only if the change is important at those scopes. Out — completed-work narration in durable context, unrelated Agent Trace or local DB rewrites.
  - Done when: Context describes the resulting current state rather than task history, and no stale statements conflict with the new auth DB surface.
  - Verification notes (commands or checks): Review context files against code truth after implementation; ensure `context/plans/encrypted-auth-db.md` remains the active execution artifact.

- [ ] T06: `Final validation and cleanup` (status:todo)
  - Task ID: T06
  - Goal: Run the full repo validation pass, remove temporary scaffolding, and confirm all plan success criteria are met.
  - Boundaries (in/out of scope): In — full test/lint/format validation, generated-output parity, temporary-file cleanup, and success-criteria evidence capture in this plan. Out — new auth DB runtime features or schema expansion.
  - Done when: `nix flake check` passes, `nix run .#pkl-check-generated` passes, no task-owned temporary scaffolding remains, and this plan records validation evidence for every success criterion.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions

None for planning. The implementation should call out before coding if the current PR already introduced a canonical auth DB path that differs from the `<state_root>/sce/auth.db` assumption.
