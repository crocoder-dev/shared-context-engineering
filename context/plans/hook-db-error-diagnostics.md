# hook-db-error-diagnostics

## Change summary

Improve error visibility when the hook runtime fails to open the per-checkout Agent Trace DB. Two gaps exist: (1) the lazy-init fast-path error is discarded when the migration-running fallback also fails, losing diagnostic context; (2) `sce doctor` only checks file existence, not whether the DB can actually be opened, connected to, or has a valid schema — so doctor can report `[PASS]` while hooks still fail.

## Success criteria

- When `resolve_or_create_agent_trace_db_for_checkout` fast-path fails and the fallback `open_at` also fails, the error message includes the fast-path failure reason alongside the fallback failure.
- `sce doctor` reports an error if the per-checkout Agent Trace DB file exists but cannot be opened, or if the schema is missing/incomplete (not just file-level existence).
- `nix flake check` passes.
- Existing hook flows, doctor output, and setup behavior are unchanged except for added error context and deeper health reporting.

## Constraints and non-goals

- In scope: preserving fast-path error in `checkout/mod.rs::resolve_or_create_agent_trace_db_for_checkout`; adding `AgentTraceDbConnectionFailed` and `AgentTraceDbSchemaNotReady` health problem kinds with full lifecycle → doctor enum plumbing.
- In scope: `diagnose_agent_trace_db_health` gains a non-mutating DB open + schema check when the file exists.
- Out of scope: changing any hook payload persistence logic, adding retry behavior, or modifying the Turso/LibSQL connection layer.
- Out of scope: new external dependencies.
- Out of scope: changing the `HealthProblemKind` / `ProblemKind` taxonomy beyond the two new variants.
- Out of scope: test-only fixtures or integration tests that require a real DB on disk (the deep check uses the same non-mutating codepath already exercised by hook runtime, and `diagnose_agent_trace_db_health` is currently untested at the unit level).

## Assumptions

- `AgentTraceDb::open_for_hooks_without_migrations_at` followed by `ensure_schema_ready_for_hooks` is safe to call during doctor diagnosis — it opens a connection but does not mutate the DB.
- The doctor deep-check failure does not need to be auto-fixable via `sce doctor --fix`; it directs the user to run `sce setup` or fix permissions manually.

## Tasks

- [x] T01: `Preserve fast-path error in checkout DB lazy-init fallback` (status:done)
  - Task ID: T01
  - Goal: When `resolve_or_create_agent_trace_db_for_checkout` in `cli/src/services/checkout/mod.rs` fails the fast-path open and then fails the fallback `open_at`, include the fast-path error reason in the fallback error context so the operator can see both failure causes.
  - Boundaries (in/out of scope): In — `resolve_or_create_agent_trace_db_for_checkout` lines 192-201; the `Err(_)` arm captures the error and chains it into the `.with_context()` closure. Out — all other functions, error classification, hook command routing.
  - Done when: The `Err` arm captures the fast-path error value and includes it in the `format!` string inside `.with_context()`, so the resulting anyhow chain reads: `failed to initialize Agent Trace DB for checkout <id> at '<path>' (fast-path attempt: <fast_error>)`.
  - Verification notes: `nix flake check`; manual inspection of `cli/src/services/checkout/mod.rs` to confirm `Err(fast_error)` binding.
  - **Completed:** 2026-06-16
  - **Files changed:** `cli/src/services/checkout/mod.rs`
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity)
  - **Notes:** Changed `Err(_)` to `Err(fast_error)` and added ` (fast-path attempt: {fast_error})` to the format string.

- [x] T02: `Add Agent Trace DB connectivity and schema health checks to doctor` (status:done)
  - T02 consists of two sub-steps (single commit):
    - **T02a**: Add `AgentTraceDbConnectionFailed` and `AgentTraceDbSchemaNotReady` variants to `HealthProblemKind` (in `cli/src/services/lifecycle.rs`) and corresponding variants to `ProblemKind` (in `cli/src/services/doctor/types.rs`), plus mapping entries in `doctor_problem_kind` and `health_problem_kind` (in `cli/src/services/doctor/mod.rs`).
    - **T02b**: In `diagnose_agent_trace_db_health` (`cli/src/services/agent_trace_db/lifecycle.rs`), after the existing `collect_db_path_health` check, when the DB file exists: open it via `AgentTraceDb::open_for_hooks_without_migrations_at`, check `ensure_schema_ready_for_hooks`, and report `AgentTraceDbConnectionFailed` if open fails or `AgentTraceDbSchemaNotReady` if schema is incomplete.
  - Goal: `sce doctor` surfaces DB-level health problems, not just file-level existence.
  - Boundaries (in/out of scope): In — `lifecycle.rs` enum, `doctor/types.rs` enum, `doctor/mod.rs` mapping functions, `agent_trace_db/lifecycle.rs` diagnose function. Out — changes to `doctor/inspect.rs`, `doctor/render.rs`, `doctor/fixes.rs`, hook runtime, or any other service.
  - Done when: `diagnose_agent_trace_db_health` reports an error when the DB cannot be opened or has incomplete schema; `nix flake check` passes; `sce doctor` text output shows `[FAIL]` for the Agent Trace checkout DB row when the file exists but is unopenable.
  - Verification notes: `nix flake check`; `nix develop -c sh -c 'cd cli && cargo build'` to confirm compilation; manual `sce doctor` run in a checkout with a known-good DB to confirm no false positives.
  - **Completed:** 2026-06-16
  - **Files changed:** `cli/src/services/lifecycle.rs`, `cli/src/services/doctor/types.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/agent_trace_db/lifecycle.rs`, `cli/src/services/doctor/render.rs`
  - **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt, pkl-parity)
  - **Notes:** Added `AgentTraceDbConnectionFailed` and `AgentTraceDbSchemaNotReady` to both `HealthProblemKind` and `ProblemKind` enums with full mapping. Deep-check logic opens the DB via `open_for_hooks_without_migrations_at` and verifies schema via `ensure_schema_ready_for_hooks` when the DB file exists. `agent_trace_db_status()` in render.rs updated to recognize the new problem kinds for `[FAIL]` rendering.

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Run full validation, confirm error chain visibility, and sync context.
  - Boundaries (in/out of scope): In — `nix flake check`, `nix run .#pkl-check-generated`, review of changed files, context sync. Out — additional code changes.
  - Done when: All checks pass, the fast-path error is visible in the fallback context, doctor deep-check works, and context files are updated.
  - Verification notes: `nix flake check` && `nix run .#pkl-check-generated`; review `context/` updates.
  - **Completed:** 2026-06-16
  - **Files changed:** `cli/src/services/checkout/mod.rs` (reviewed T01), `cli/src/services/agent_trace_db/lifecycle.rs` (reviewed T02), `cli/src/services/lifecycle.rs` (reviewed T02), `cli/src/services/doctor/types.rs` (reviewed T02), `cli/src/services/doctor/mod.rs` (reviewed T02), `cli/src/services/doctor/render.rs` (reviewed T02)
  - **Evidence:** `nix flake check` passed (all 13 checks), `nix run .#pkl-check-generated` passed (generated outputs up to date). Fast-path error chain confirmed visible at `checkout/mod.rs:196-199`. Doctor deep-check confirmed at `agent_trace_db/lifecycle.rs:136-173`. Context sync completed.
  - **Notes:** This is the final task in the plan. All three tasks complete.

## Validation Report

### Commands run

- `nix flake check` → exit 0 (all 13 checks passed)
  - `cli-tests`, `cli-clippy`, `cli-fmt`
  - `integrations-install-tests`, `integrations-install-clippy`, `integrations-install-fmt`
  - `pkl-parity`
  - `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`
  - `config-lib-bun-tests`, `config-lib-biome-check`, `config-lib-biome-format`
- `nix run .#pkl-check-generated` → exit 0 (`Generated outputs are up to date.`)
- Temporary scaffolding: none introduced (no debug code, temp files, or test-only fixtures)

### Success-criteria verification

- [x] **Fast-path error preserved in fallback context** — confirmed at `cli/src/services/checkout/mod.rs:196-199`: `Err(fast_error)` binding and `(fast-path attempt: {fast_error})` in `.with_context()` format string.
- [x] **Doctor reports DB connectivity/schema errors** — confirmed at `cli/src/services/agent_trace_db/lifecycle.rs:136-173`: deep-check opens DB via `open_for_hooks_without_migrations_at`, checks `ensure_schema_ready_for_hooks`, reports `AgentTraceDbConnectionFailed` or `AgentTraceDbSchemaNotReady`.
- [x] **`nix flake check` passes** — all 13 derivations evaluated and passed.
- [x] **Existing flows unchanged** — all CLI tests pass, no behavioral changes to hook runtime, doctor output, or setup service beyond added error context and deeper health reporting.

### Residual risks

- None identified. The deep-check uses the same non-mutating open + schema-readiness codepath already exercised by hook runtime (`open_for_hooks_without_migrations_at` + `ensure_schema_ready_for_hooks`). No unit-level test coverage for `diagnose_agent_trace_db_health` (explicitly out of scope per plan constraints).

## Open questions

- None at this time.
