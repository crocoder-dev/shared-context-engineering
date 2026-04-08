# Plan: Agent trace removal with optional attribution gate

## Change summary
- Remove the current Agent Trace implementation from the CLI/runtime for this release window so the v0.3.0 redesign can start from a clean baseline.
- Keep the local database path behavior limited to creating/opening an empty database file with no schema bootstrap or trace tables.
- Keep hook-trigger entrypoints present, remove trace/persistence behavior, and preserve attribution-only behavior behind a new config/env gate that defaults to disabled.

## Success criteria
- Agent Trace persistence, schema migration, trace emission, retry, notes, and rewrite-handling behavior are removed from the current runtime surface.
- `local_db` still creates/opens the configured database file, but no schema tables or trace migrations are applied during setup/bootstrap.
- Hook entrypoints needed for attribution wiring still exist, but trace persistence/rewrite side effects are removed and the disabled-default path is a deterministic no-op.
- Attribution behavior tied to hooks is controlled by a config/env gate that defaults to disabled.
- When the attribution gate is explicitly enabled, hook-triggered attribution behavior still functions without reintroducing Agent Trace persistence, schema bootstrap, retry, notes, or rewrite flows.
- User-facing help, doctor/setup/config/runtime context, and generated/current-state docs no longer describe removed Agent Trace behavior as active.

## Constraints and non-goals
- Do not redesign the future v0.3.0 attribution/tracing architecture in this plan; this change is a rollback/reset to a minimal safe baseline.
- Do not introduce new tracing/persistence behavior behind the new config/env gate in this release; the gate enables attribution-only behavior while removed trace infrastructure stays absent.
- Keep hook command/installation surfaces stable enough that future work can reuse them without reintroducing removed Agent Trace implementation now.
- Treat code as source of truth if current context artifacts still describe removed tracing behavior; context repair is part of this plan.
- Preserve one-task/one-atomic-commit slicing; each executable task must land as one coherent commit.

## Task stack
- [x] T01: Remove Agent Trace persistence and local DB schema bootstrap (status:done)
  - Task ID: T01
  - Goal: Strip Agent Trace-specific local persistence behavior so the runtime only opens/creates the DB file without applying trace schema migrations or writing trace data.
  - Boundaries (in/out of scope): In scope: `cli/src/services/local_db.rs` and directly coupled persistence/bootstrap call sites for trace schema setup and trace-specific DB writes. Out of scope: hook command semantics, config/env gating, and broader doc cleanup beyond code comments tightly coupled to the removed persistence flow.
  - Done when: Agent Trace schema/bootstrap and trace-write paths are removed or disconnected; opening the local DB yields an empty database file with no trace tables; no runtime path still expects trace migrations to have run.
  - Verification notes (commands or checks): Inspect `local_db` ownership and call sites to confirm no trace schema bootstrap remains; run the narrowest relevant validation covering local DB compile/test paths, then include repo baseline checks during final validation.
  - Completed: 2026-04-08
  - Files changed: `cli/src/services/local_db.rs`, `cli/src/services/hooks.rs`
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`; `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T02: Keep attribution-only hooks behind a disabled-by-default gate (status:todo)
  - Task ID: T02
  - Goal: Keep the hook-trigger surface present while removing trace persistence, retry, rewrite handling, and other Agent Trace side effects; preserve attribution-only behavior behind a config/env gate that defaults to disabled.
  - Boundaries (in/out of scope): In scope: hook runtime entrypoints, config/env resolution, disabled-default no-op behavior, and enabled-path attribution behavior that does not depend on trace infrastructure. Out of scope: future v0.3.0 tracing redesign, new persistence behavior, and unrelated CLI command-surface redesign.
  - Done when: Hook commands/installed triggers remain invocable; default behavior is disabled/no-op; enabling the new config/env gate restores attribution-only behavior; no trace-related side effect runs in either mode; the new gate resolves through the existing precedence model and is documented in code/tests.
  - Verification notes (commands or checks): Inspect hook runtime and config resolution paths for disabled-default no-op behavior plus enabled attribution-only behavior; run targeted hook/config tests where available; include full repo validation in the final task.

- [ ] T03: Remove Agent Trace-specific command, doctor, setup, and path-surface behavior (status:todo)
  - Task ID: T03
  - Goal: Eliminate remaining active runtime references to Agent Trace from command help, doctor/setup readiness checks, default-path inventories, and related operator-facing surfaces while preserving the optional attribution-only hook baseline.
  - Boundaries (in/out of scope): In scope: command/help text, doctor checks, setup/install expectations, default-path/service references, and removal of dead trace-only seams exposed to users/operators. Out of scope: broad unrelated CLI polish and the future tracing redesign.
  - Done when: User-facing runtime/help/doctor/setup/path surfaces no longer present Agent Trace as an active supported feature; any retained hook trigger surface is described as attribution-only and disabled by default; removed trace-only code paths no longer drive warnings/dead branches.
  - Verification notes (commands or checks): Inspect command-surface, doctor, setup, and default-path outputs/contracts for removed Agent Trace references; run targeted checks for affected CLI modules, then defer full repo checks to the final task.

- [ ] T04: Sync current-state context for the trace-removal baseline (status:todo)
  - Task ID: T04
  - Goal: Update shared context to reflect that Agent Trace runtime behavior has been removed, hook attribution is optional and disabled by default, and local DB bootstrap is empty-file only.
  - Boundaries (in/out of scope): In scope: affected `context/overview.md`, `context/glossary.md`, `context/context-map.md`, and focused `context/sce/` artifacts describing tracing/hooks/local DB behavior. Out of scope: speculative v0.3.0 redesign docs or historical postmortems.
  - Done when: Current-state context no longer documents removed Agent Trace behavior as active, and retained hook attribution/local DB behavior is described accurately for future sessions.
  - Verification notes (commands or checks): Review all touched context files against code truth; ensure stale Agent Trace contract files are updated, replaced, or removed as appropriate.

- [ ] T05: Validation and cleanup (status:todo)
  - Task ID: T05
  - Goal: Run full repository validation, confirm removed tracing behavior stays absent while optional attribution behavior still works when enabled, and clean up any leftover dead references or temporary scaffolding.
  - Boundaries (in/out of scope): In scope: `nix run .#pkl-check-generated`, `nix flake check`, final targeted spot-checks for disabled-default noop hooks, enabled attribution-only behavior, empty DB expectations, and final context-sync verification. Out of scope: new feature work or redesign follow-ons.
  - Done when: Required validation passes, leftover dead/stale trace-removal artifacts are cleaned up, and plan/context state is ready for handoff completion.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted inspection that hooks are no-op by default, attribution works when enabled, and local DB bootstrap remains empty-file only.

## Open questions
- None.
