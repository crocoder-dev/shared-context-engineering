# Agent Trace Hook No-Hot-Path Migrations Plan

## Change summary

Stop high-frequency Agent Trace hook invocations from running Agent Trace DB migrations during normal trace persistence. `sce setup` and lifecycle/doctor-owned setup remain responsible for schema initialization and upgrades. All Agent Trace hook paths must open the database for runtime reads/writes without migration execution, verify that the required schema is already present, and fail with clear setup/doctor guidance when the schema is missing.

This targets transient `database is locked` failures caused by hook processes racing through migration metadata setup (`__sce_migrations`) while preserving explicit schema initialization through setup/doctor flows.

## Success criteria

- `sce hooks conversation-trace`, `sce hooks diff-trace`, and `sce hooks post-commit` no longer run Agent Trace DB migrations as part of their high-frequency runtime path.
- Agent Trace DB migrations still run through setup/lifecycle initialization, including `sce setup` and existing `AgentTraceDbLifecycle::setup` behavior.
- Hook runtime paths fail with actionable guidance when required Agent Trace DB schema is absent, directing the operator to run `sce setup` or `sce doctor --fix`.
- Runtime query/write retry behavior remains available for hook database operations.
- Existing hook behavior and output contracts remain unchanged except for clearer missing-schema failures and reduced migration-lock exposure.
- Tests cover no-migration hook DB opening/schema readiness behavior and the missing-schema guidance path.
- Context files are synced to describe the resulting current-state architecture.

## Constraints and non-goals

- Do not add new database libraries or external services.
- Do not remove migrations from setup, lifecycle, or explicit database initialization flows.
- Do not opportunistically run migrations from Agent Trace hook paths when schema is missing.
- Do not add retry/backfill queues or `context/tmp` artifacts for `conversation-trace`.
- Do not change Agent Trace DB schema shape as part of this plan unless strictly required for readiness checks.
- Do not broaden hook command surface or top-level CLI help visibility.

## Assumptions

- Scope includes all high-frequency Agent Trace hook paths: `conversation-trace`, `diff-trace`, and `post-commit`.
- Missing hook schema should fail fast with clear guidance, not run migrations as a fallback.
- `sce doctor --fix` may remain limited to safe parent-directory repair; if it cannot apply migrations today, guidance may include `sce setup` as the canonical schema initialization command.

## Task stack

- [x] T01: `Add no-migration Agent Trace DB open path` (status:done)
  - Task ID: T01
  - Goal: Introduce an explicit Agent Trace DB construction/open path for runtime hooks that opens/connects the database without running embedded migrations while preserving existing retry-backed query/write methods.
  - Boundaries (in/out of scope): In - adapter/API seam needed by Agent Trace hook code, tests proving migration execution is bypassed for this new path, preservation of existing `AgentTraceDb::new()` migration behavior for setup/lifecycle. Out - changing schema definitions, changing auth/local DB behavior, changing setup/doctor command output.
  - Done when: A named no-migration Agent Trace DB open API exists; existing setup/lifecycle code still uses migration-running initialization; tests distinguish migration-running initialization from no-migration hook open behavior.
  - Verification notes (commands or checks): Run the narrow Rust tests for the DB adapter/Agent Trace DB module through Nix, then include them in final `nix flake check`.
  - Completed: 2026-06-09
  - Files changed: `cli/src/services/db/mod.rs`, `cli/src/services/agent_trace_db/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt' && nix flake check` passed. Initial narrow `cargo test agent_trace_db` command was blocked by repo bash policy in favor of `nix flake check`.
  - Notes: Added shared `TursoDb::<M>::open_without_migrations()` plus Agent Trace-specific `AgentTraceDb::open_for_hooks_without_migrations()` for later hook routing. `TursoDb::<M>::new()` still runs migrations. Focused Agent Trace DB tests distinguish migration-running initialization from no-migration opening.

- [ ] T02: `Add Agent Trace schema readiness checks` (status:todo)
  - Task ID: T02
  - Goal: Add a deterministic schema readiness check for the Agent Trace DB tables/indexes required by all active Agent Trace hook writers/readers.
  - Boundaries (in/out of scope): In - readiness helper(s) for required objects such as `diff_traces`, `post_commit_patch_intersections`, `agent_traces`, `messages`, `parts`, and required indexes/columns where needed; actionable error type/message for missing schema. Out - running migrations, repairing schema, backfilling legacy DBs.
  - Done when: The readiness check can identify an uninitialized or incomplete Agent Trace DB before hook persistence proceeds and returns a stable error with `Run 'sce setup' or 'sce doctor --fix'` style guidance.
  - Verification notes (commands or checks): Unit tests cover ready schema, empty DB, and at least one partially missing required object case.

- [ ] T03: `Route all Agent Trace hooks through no-migration readiness-gated DB access` (status:todo)
  - Task ID: T03
  - Goal: Update `conversation-trace`, `diff-trace`, and `post-commit` Agent Trace DB access to use the no-migration open path plus schema readiness checks before runtime reads/writes.
  - Boundaries (in/out of scope): In - hook DB construction call sites, preserving existing parser/accounting/output behavior, ensuring missing-schema DB failures are command-failing where current DB open failures are command-failing. Out - changing generated OpenCode plugin behavior, changing attribution-only `commit-msg`, changing no-op `pre-commit`/`post-rewrite`.
  - Done when: No active Agent Trace hook path calls the migration-running constructor during runtime persistence; missing schema produces clear runtime guidance; normal ready-schema paths retain existing persisted data behavior.
  - Verification notes (commands or checks): Focused hook tests for `conversation-trace`, `diff-trace`, and `post-commit` cover ready-schema behavior and missing-schema failure guidance where feasible.

- [ ] T04: `Keep setup and doctor lifecycle as schema initialization owners` (status:todo)
  - Task ID: T04
  - Goal: Verify and, if needed, adjust setup/lifecycle/doctor documentation and tests so schema initialization remains owned by setup/lifecycle flows, not hook runtime flows.
  - Boundaries (in/out of scope): In - tests or assertions around `AgentTraceDbLifecycle::setup` using migration-running initialization; operator guidance consistency between hook missing-schema errors, `sce setup`, and doctor reporting. Out - broad doctor repair expansion beyond existing safe behavior unless required to make guidance truthful.
  - Done when: Setup/lifecycle initialization remains the tested path for applying Agent Trace migrations; hook missing-schema guidance aligns with available commands.
  - Verification notes (commands or checks): Run focused lifecycle/setup/doctor tests relevant to Agent Trace DB initialization and health reporting.

- [ ] T05: `Sync current-state context for Agent Trace DB runtime migration policy` (status:todo)
  - Task ID: T05
  - Goal: Update durable context to describe the new split between migration-running setup/lifecycle initialization and no-migration hook runtime access.
  - Boundaries (in/out of scope): In - current-state updates to `context/sce/shared-turso-db.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, and root context files/glossary if the architecture terminology changes. Out - completed-work summaries, historical narration, unrelated context churn.
  - Done when: Context accurately states that high-frequency Agent Trace hook paths do not run migrations and instead require pre-initialized schema with clear setup/doctor guidance on failure.
  - Verification notes (commands or checks): Review context against code truth; run generated-output parity if generated docs/config are touched.

- [ ] T06: `Validation and cleanup` (status:todo)
  - Task ID: T06
  - Goal: Run full validation, remove temporary scaffolding, and confirm all success criteria are met.
  - Boundaries (in/out of scope): In - full repo validation, targeted manual/automated command checks as appropriate, final plan evidence capture. Out - new behavior changes beyond fixes required by validation failures.
  - Done when: `nix flake check` passes; `nix run .#pkl-check-generated` passes; targeted hook/DB tests pass; no temporary test files or debug instrumentation remain; plan status/evidence is updated.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; any focused Rust test commands used during earlier tasks.

## Open questions

- None. User confirmed all Agent Trace hook paths are in scope and missing schema should fail with clear guidance instead of running migrations.
