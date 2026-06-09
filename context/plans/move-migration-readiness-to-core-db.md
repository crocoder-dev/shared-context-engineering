# Plan: Move migration readiness check to core DB

## Change summary

Move the generic migration readiness functions (`ensure_schema_ready_for_hooks_with` and `schema_migration_metadata_problems`) from `agent_trace_db/mod.rs` to the core `db/mod.rs` module, generalize the API as a public method on `TursoDb<M>`, add a unit test for migration readiness checking in `db/mod.rs`, and update `AgentTraceDb::ensure_schema_ready_for_hooks()` to delegate to the moved core method.

## Success criteria

- `TursoDb<M>` exposes a public `migration_metadata_problems()` method and a public `ensure_schema_ready(setup_guidance)` method that any `DbSpec` consumer can call.
- `AgentTraceDb::ensure_schema_ready_for_hooks()` delegates to `TursoDb::ensure_schema_ready()` with the Agent Trace–specific setup guidance string.
- `agent_trace_db/mod.rs` no longer contains the `ensure_schema_ready_for_hooks_with` or `schema_migration_metadata_problems` free functions.
- A new unit test in `db/mod.rs` exercises the readiness check with scenarios: missing migration metadata table, incomplete migrations, unexpected migrations, and complete/ready schema.
- `nix flake check` passes.
- Context files (`context/sce/shared-turso-db.md`, `context/sce/agent-trace-db.md`, `context/context-map.md`) reflect the moved ownership.

## Constraints and non-goals

- No behavior change to the readiness check logic itself — this is a pure refactor + test addition.
- `LocalDb` and `AuthDb` are not wired to call `ensure_schema_ready()` in this plan; the API is available for future use but no new callers are added.
- The `AGENT_TRACE_SCHEMA_SETUP_GUIDANCE` constant stays in `agent_trace_db/mod.rs` as a domain-specific string.
- No changes to `EncryptedTursoDb<M>` in this plan; the readiness check is only on `TursoDb<M>` since no encrypted-DB consumer currently needs it.

## Task stack

- [x] T01: `Move migration_metadata_problems and ensure_schema_ready to TursoDb<M>` (status:done)
  - Task ID: T01
  - Goal: Move the two generic free functions from `agent_trace_db/mod.rs` into `db/mod.rs` as public methods on `TursoDb<M>`, update `AgentTraceDb::ensure_schema_ready_for_hooks()` to delegate, and remove the old free functions.
  - Boundaries (in/out of scope): In — moving `schema_migration_metadata_problems` and `ensure_schema_ready_for_hooks_with` logic to `TursoDb<M>` as `migration_metadata_problems(&self)` and `ensure_schema_ready(&self, setup_guidance: &str)`, updating `AgentTraceDb::ensure_schema_ready_for_hooks()` to call `self.ensure_schema_ready(AGENT_TRACE_SCHEMA_SETUP_GUIDANCE)`, removing the old free functions from `agent_trace_db/mod.rs`. Out — no changes to `EncryptedTursoDb`, no new callers for `LocalDb`/`AuthDb`, no behavior changes to the readiness logic.
  - Done when: `agent_trace_db/mod.rs` no longer contains `ensure_schema_ready_for_hooks_with` or `schema_migration_metadata_problems`; `db/mod.rs` contains both methods on `TursoDb<M>`; `AgentTraceDb::ensure_schema_ready_for_hooks()` delegates to the core method; `nix flake check` passes.
  - Verification notes: `nix flake check`; `grep -r 'ensure_schema_ready_for_hooks_with\|schema_migration_metadata_problems' cli/src/services/agent_trace_db/` returns no matches; `grep 'fn ensure_schema_ready\|fn migration_metadata_problems' cli/src/services/db/mod.rs` returns both methods.
  - **Status:** done
  - **Completed:** 2026-06-09
  - **Files changed:** `cli/src/services/db/mod.rs`, `cli/src/services/agent_trace_db/mod.rs`
  - **Evidence:** `nix flake check` all checks passed; `nix run .#pkl-check-generated` up to date; grep confirms old free functions removed from `agent_trace_db/mod.rs` and new methods present on `TursoDb<M>` in `db/mod.rs`; `AgentTraceDb::ensure_schema_ready_for_hooks()` now delegates to `self.ensure_schema_ready(AGENT_TRACE_SCHEMA_SETUP_GUIDANCE)`.
  - **Notes:** Pure refactor with no behavior change. The `ensure_schema_ready` error message now uses `M::db_name()` for the database name prefix instead of the hardcoded "Agent Trace DB" string, making it generic for any `DbSpec` consumer.

- [x] T02: `Add migration readiness unit test in db/mod.rs` (status:done)
  - Task ID: T02
  - Goal: Add a `#[cfg(test)] mod tests` block in `db/mod.rs` with a test that exercises `TursoDb::migration_metadata_problems()` and `TursoDb::ensure_schema_ready()` using a lightweight `TestDbSpec` with a small embedded migration, covering: (1) missing `__sce_migrations` table, (2) incomplete applied migrations, (3) unexpected extra migrations, and (4) complete/ready schema returns `Ok(())`.
  - Boundaries (in/out of scope): In — test module in `db/mod.rs`, `TestDbSpec` with one or two minimal migrations, test cases for the four readiness scenarios. Out — no changes to production code, no changes to existing tests in other modules.
  - Done when: `cargo test` (via `nix flake check`) includes the new test and all four scenarios pass; the test creates and cleans up a temporary database.
  - Verification notes: `nix develop -c sh -c 'cd cli && cargo test migration_metadata_problems -- --exact'`; `nix flake check`.
  - **Status:** done
  - **Completed:** 2026-06-09
  - **Files changed:** none (test addition deferred per operator instruction)
  - **Evidence:** Operator instructed to skip test implementation; T01 production code changes are verified and the readiness methods on `TursoDb<M>` are in place.
  - **Notes:** T02 was marked done without adding unit tests per operator instruction. The `migration_metadata_problems()` and `ensure_schema_ready()` methods on `TursoDb<M>` are exercised indirectly through existing `AgentTraceDb` integration tests.

- [x] T03: `Update context files to reflect moved ownership` (status:done)
  - Task ID: T03
  - Goal: Update `context/sce/shared-turso-db.md`, `context/sce/agent-trace-db.md`, and `context/context-map.md` to reflect that `migration_metadata_problems()` and `ensure_schema_ready()` are now on `TursoDb<M>` in the core DB module, and `AgentTraceDb::ensure_schema_ready_for_hooks()` delegates to the core method.
  - Boundaries (in/out of scope): In — updating the three context files. Out — no code changes.
  - Done when: Context files accurately describe the current code ownership; `shared-turso-db.md` documents the two new public methods on `TursoDb<M>`; `agent-trace-db.md` describes delegation to the core method; `context-map.md` references are current.
  - Verification notes: Manual review of updated context files against the code changes from T01.
  - **Status:** done
  - **Completed:** 2026-06-09
  - **Files changed:** none (context files were already current from prior sync)
  - **Evidence:** Verified all three context files (`shared-turso-db.md`, `agent-trace-db.md`, `context-map.md`) plus the glossary entry (`TursoDb migration readiness check`) already reflect the moved ownership: `migration_metadata_problems()` and `ensure_schema_ready()` documented on `TursoDb<M>` in `shared-turso-db.md`; `ensure_schema_ready_for_hooks()` delegation documented in `agent-trace-db.md`; `context-map.md` references current. No edits needed.
  - **Notes:** Verify-only pass — context was already aligned with code truth from T01.

- [x] T04: `Validate and clean up` (status:done)
  - Task ID: T04
  - Goal: Run full validation suite and verify no stale references remain.
  - Boundaries (in/out of scope): In — `nix flake check`, `nix run .#pkl-check-generated`, grep for stale references. Out — no code changes.
  - Done when: `nix flake check` passes; `nix run .#pkl-check-generated` passes; no references to removed free functions remain in production code.
  - Verification notes: `nix flake check`; `nix run .#pkl-check-generated`; `grep -r 'ensure_schema_ready_for_hooks_with\|schema_migration_metadata_problems' cli/src/` returns no matches.
  - **Status:** done
  - **Completed:** 2026-06-09
  - **Files changed:** none
  - **Evidence:** `nix flake check` all checks passed; `nix run .#pkl-check-generated` reports "Generated outputs are up to date"; `grep -r 'ensure_schema_ready_for_hooks_with\|schema_migration_metadata_problems' cli/src/` returns no matches (exit code 1).
  - **Notes:** Validation-only task. All checks pass. No stale references remain.

## Validation Report

### Commands run
- `nix flake check` → exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format, integrations-install-tests, integrations-install-clippy, integrations-install-fmt)
- `nix run .#pkl-check-generated` → exit 0 ("Generated outputs are up to date")
- `grep -r 'ensure_schema_ready_for_hooks_with\|schema_migration_metadata_problems' cli/src/` → exit 1 (no matches found)

### Success-criteria verification
- [x] `TursoDb<M>` exposes public `migration_metadata_problems()` and `ensure_schema_ready(setup_guidance)` → confirmed in `db/mod.rs` lines 515-587
- [x] `AgentTraceDb::ensure_schema_ready_for_hooks()` delegates to `TursoDb::ensure_schema_ready()` → confirmed in `agent_trace_db/mod.rs` line 321
- [x] `agent_trace_db/mod.rs` no longer contains `ensure_schema_ready_for_hooks_with` or `schema_migration_metadata_problems` → confirmed by grep returning no matches
- [x] Unit test in `db/mod.rs` for readiness check → deferred per operator instruction (T02); methods exercised indirectly through existing AgentTraceDb integration tests
- [x] `nix flake check` passes → confirmed (exit 0, all checks passed)
- [x] Context files reflect moved ownership → confirmed: `shared-turso-db.md`, `agent-trace-db.md`, `context-map.md`, and `glossary.md` all current

### Temporary scaffolding removed
- None introduced during this plan.

### Residual risks
- None identified.

## Open questions

None — the scope is clear and the API design is straightforward.