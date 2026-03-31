# Plan: Centralize CLI path definitions in the default path service

## Change summary
- Refactor `cli/` so production path definitions are owned by `cli/src/services/default_paths.rs` instead of being hardcoded across service modules.
- Expand the current default-path seam beyond per-user persisted locations so it can also supply repo-relative and install/runtime path definitions used by the CLI.
- Leave test-only path literals out of scope; the production rule is that non-test code in `cli/` must not hardcode path strings outside the default path service.

## Success criteria
- No non-test production file under `cli/` hardcodes path literals or path segments outside `cli/src/services/default_paths.rs`.
- The default path service exposes canonical accessors for every currently implemented production path shape used by the CLI, including per-user default locations and repo-relative/runtime paths.
- Config discovery, doctor checks, setup flows, hook/runtime entrypoints, and any other current path consumers in `cli/` resolve their paths through the shared default path service.
- Current CLI behavior stays functionally equivalent except for the internal source of path definitions and any intentionally clarified path-reporting wording.
- Tests cover the expanded path service contract and the highest-risk migrated consumers.
- A regression guard exists so future production path literals in `cli/` are caught automatically.
- Current-state context reflects the broadened ownership of the default path service.

## Constraints and non-goals
- Scope is limited to `cli/` production code plus required context updates; no application behavior redesign beyond path centralization.
- Test files may keep explicit path literals when needed for focused fixtures and assertions.
- User-supplied paths, environment-provided paths, and externally returned filesystem paths are not “hardcoded paths” for this plan.
- Do not introduce fallback behavior, migration logic, or speculative new storage locations just to satisfy the refactor.
- Do not move implementation into a new service unless the final canonical owner remains `cli/src/services/default_paths.rs`.

## Task stack
- [ ] T01: `Expand default_paths.rs into the canonical CLI path catalog` (status:todo)
  - Task ID: T01
  - Goal: Define the complete production path-ownership contract in `cli/src/services/default_paths.rs` so all currently used hardcoded CLI paths have named accessors/types in one canonical location.
  - Boundaries (in/out of scope): In - inventorying current non-test path literals in `cli/`, designing accessor families for per-user, repo-relative, and runtime/install paths, and adding focused unit tests for the new contract. Out - migrating all callers; that lands in later tasks.
  - Done when: `default_paths.rs` exposes the canonical accessors needed for the current CLI path surface and no new production path owner is introduced elsewhere.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T02: `Migrate config and doctor path consumers to the shared path catalog` (status:todo)
  - Task ID: T02
  - Goal: Replace hardcoded production paths in config/discovery/doctor flows with accessors from `default_paths.rs`.
  - Boundaries (in/out of scope): In - repo-local config path resolution, doctor local/global config path reporting/validation, and related helper seams/tests in `config.rs` and `doctor.rs`. Out - setup/install target paths and hook runtime path literals.
  - Done when: Config and doctor production code no longer assembles hardcoded path strings directly and their path-sensitive tests still pass against the centralized seam.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T03: `Migrate setup and hook-related production paths to the shared catalog` (status:todo)
  - Task ID: T03
  - Goal: Route setup/install/runtime path definitions for generated targets, hook files, and related repo-relative locations through `default_paths.rs`.
  - Boundaries (in/out of scope): In - production path definitions used by `setup.rs`, hook-facing runtime helpers, and any shared app wiring that still hardcodes hook/message-file paths. Out - test fixture literals and unrelated business logic changes.
  - Done when: Setup and hook production code no longer owns hardcoded path strings outside `default_paths.rs`, and runtime/help behavior remains deterministic.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T04: `Eliminate remaining production path literals and add regression guards` (status:todo)
  - Task ID: T04
  - Goal: Remove any remaining non-test CLI hardcoded paths not covered in earlier tasks and add automated coverage that fails if new production path literals are introduced outside `default_paths.rs`.
  - Boundaries (in/out of scope): In - final sweep of remaining production modules, regression-test or lint-style guard coverage inside the Rust test surface, and stabilization of any shared helper APIs needed by the guard. Out - context updates and final validation reporting.
  - Done when: The implementation has an enforceable regression guard for the “default path service is the only path owner” rule and the remaining production path literals have been removed.
  - Verification notes (commands or checks): `nix flake check`.

- [ ] T05: `Sync context for centralized CLI path ownership` (status:todo)
  - Task ID: T05
  - Goal: Update durable context to describe `default_paths.rs` as the canonical owner of production CLI path definitions, not only per-user persisted defaults.
  - Boundaries (in/out of scope): In - focused updates to `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`, and any directly relevant CLI/SCE path-contract docs. Out - historical summaries or unrelated context churn.
  - Done when: Durable context reflects the broadened path-ownership contract and no stale docs imply that other production modules should define their own paths.
  - Verification notes (commands or checks): Manual review against code truth.

- [ ] T06: `Validation and cleanup` (status:todo)
  - Task ID: T06
  - Goal: Run the full verification pass, confirm the path-centralization contract is enforced, remove temporary scaffolding, and leave the plan ready for completion tracking.
  - Boundaries (in/out of scope): In - final repo validation, generated-output parity check if touched, plan status updates, and cleanup of any temporary helper scaffolding introduced during implementation. Out - new refactors or follow-on behavior changes.
  - Done when: Required validation passes, no temporary scaffolding remains, and the plan file accurately reflects the final execution state.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated` (if generated outputs change); `nix flake check`.

## Open questions
- None.
