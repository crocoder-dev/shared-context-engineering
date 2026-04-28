# Plan: Local DB lifecycle

## Change summary

Implement a local DB lifecycle service following the established hooks lifecycle pattern, so that doctor's local DB health diagnosis and parent-directory bootstrap repair are owned by a `DiagnosticLifecycle` and `FixLifecycle` capability behind the `LifecycleRegistry`, and doctor resolves the local DB service by service ID instead of owning DB health logic directly. This is the second production lifecycle service implementation after hooks.

## Success criteria

1. A `local_db_lifecycle.rs` module exists under `cli/src/services/` and implements `DiagnosticLifecycle` and `FixLifecycle` for the `local_db` service ID, owning local DB path resolution, parent-directory readiness, DB existence/health, and migration-readiness diagnosis plus parent-directory bootstrap repair.
2. The `LifecycleRegistry` registers the local DB diagnostic and fix lifecycle facets for the `local_db` service ID, and registry lookup tests cover successful lookup and missing-capability handling for the new service.
3. Doctor's local DB health inspection consumes the local DB diagnostic lifecycle capability through the registry, and `doctor/inspect.rs` no longer directly owns local DB domain inspection logic beyond report assembly/mapping.
4. Doctor's local DB parent-directory bootstrap repair consumes the local DB fix lifecycle capability through the registry, and `doctor/fixes.rs` no longer directly owns local DB repair logic beyond fix-result mapping.
5. Existing doctor text/JSON output contracts remain stable; no user-visible behavior changes.
6. The `SetupLifecycle` and `PreviewLifecycle` traits are not implemented for local DB in this plan (setup already owns `bootstrap_local_db` and that ownership boundary is preserved).
7. The resulting architecture is documented in current-state `context/` files so the local DB lifecycle service is discoverable alongside the hooks pilot.

## Constraints and non-goals

- Planning decision: create a new plan instead of reopening the completed `cli-service-lifecycle-traits` plan.
- Planning decision: implement only `DiagnosticLifecycle` and `FixLifecycle` for local DB; `SetupLifecycle` and `PreviewLifecycle` are out of scope because setup already owns `bootstrap_local_db` and that boundary is preserved.
- Planning decision: follow the hooks lifecycle pattern exactly — create `local_db_lifecycle.rs`, implement the needed traits, register in `LifecycleRegistry`, wire doctor to resolve by service ID.
- In scope: `cli/src/services/local_db_lifecycle.rs` (new module), `cli/src/services/lifecycle_registry.rs` (registration), `cli/src/services/doctor/inspect.rs` (diagnosis rewiring), `cli/src/services/doctor/fixes.rs` (repair rewiring), `cli/src/services/doctor/types.rs` (problem kind additions if needed), `cli/src/services/doctor/mod.rs` (dependency wiring if needed), tests for the new lifecycle service and registry entries, and current-state context updates.
- Out of scope: `SetupLifecycle` or `PreviewLifecycle` for local DB, changes to `sce setup` local DB bootstrap, changes to `LocalDb::new()` or migration logic, new CLI flags or commands, JSON schema changes, or converting other services (OpenCode assets, config, auth, sync) to lifecycle traits.
- Out of scope: any changes to the hooks lifecycle service or its registration.
- Existing doctor output vocabulary (`[PASS]`, `[FAIL]`, `[MISS]`, problem categories, fix-result outcomes) must remain stable; lifecycle diagnostics map into existing doctor types at the orchestration boundary.

## Task stack

- [x] T01: `Implement local DB diagnostic lifecycle capability` (status:done)
  - Task ID: T01
  - Goal: Create `cli/src/services/local_db_lifecycle.rs` implementing `DiagnosticLifecycle` for the `local_db` service ID, owning local DB path resolution, parent-directory readiness, DB existence, and DB health/migration-readiness diagnosis with stable diagnostic kind constants that map into existing doctor problem types.
  - Boundaries (in/out of scope): In - the `LocalDbLifecycleService` struct, `LOCAL_DB_SERVICE_ID` constant, `DiagnosticLifecycle` implementation, stable diagnostic kind constants (e.g. `LOCAL_DB_PATH_UNRESOLVABLE`, `LOCAL_DB_PARENT_NOT_DIRECTORY`, `LOCAL_DB_PARENT_NOT_WRITABLE`, `LOCAL_DB_PARENT_MISSING`, `LOCAL_DB_HEALTH_CHECK_FAILED`), typed diagnostic records that map into existing `DoctorProblem` types, and unit tests for the diagnostic lifecycle. Out - `FixLifecycle` implementation, `SetupLifecycle` or `PreviewLifecycle` implementation, doctor rewiring, registry registration, changes to `LocalDb::new()` or migration logic, or changes to doctor output contracts.
  - Done when: `LocalDbLifecycleService` implements `LifecycleService` and `DiagnosticLifecycle`; diagnostic kind constants are defined; `diagnose()` produces typed `DiagnosticReport` entries for path resolution failure, parent-directory issues, and DB health failure; unit tests cover diagnostic scenarios without requiring real filesystem or database fixtures; `nix flake check` passes.
  - Verification notes (commands or checks): `nix flake check`; targeted Rust check through Nix during development.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/local_db_lifecycle.rs`, `cli/src/services/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'` completed; `nix flake check` passed (all CLI tests, clippy, fmt, pkl-parity, JS checks passed).
  - Notes: Added the local DB diagnostic lifecycle module with `LocalDbLifecycleService` implementing `LifecycleService` and `DiagnosticLifecycle`; defined five stable diagnostic kind constants (`LOCAL_DB_PATH_UNRESOLVABLE`, `LOCAL_DB_PARENT_MISSING`, `LOCAL_DB_PARENT_NOT_DIRECTORY`, `LOCAL_DB_PARENT_NOT_WRITABLE`, `LOCAL_DB_HEALTH_CHECK_FAILED`); added `resolve_local_db_parent_path()` helper; unit tests cover metadata, diagnostic report generation, kind constant stability, directory writability rejection, DB health rejection of directories, and parent path resolution. Fix lifecycle, registry registration, and doctor rewiring remain deferred to later tasks.

- [x] T02: `Implement local DB fix lifecycle capability` (status:done)
  - Task ID: T02
  - Goal: Add `FixLifecycle` implementation to `LocalDbLifecycleService`, owning local DB parent-directory bootstrap repair (creating the missing canonical SCE-owned DB parent directory when the resolved path matches the expected owned location).
  - Boundaries (in/out of scope): In - `FixLifecycle` implementation on `LocalDbLifecycleService`, fix logic that creates the missing canonical SCE-owned DB parent directory with deterministic refusal when the resolved path does not match the expected owned location, typed fix actions that map into existing `DoctorFixResultRecord` values, and unit tests for the fix lifecycle. Out - `SetupLifecycle` or `PreviewLifecycle` implementation, changes to `LocalDb::new()`, changes to doctor output contracts, or new fix classes beyond parent-directory bootstrap.
  - Done when: `LocalDbLifecycleService` implements `FixLifecycle`; fix logic creates the missing canonical DB parent directory when the path matches the expected owned location and refuses with a deterministic error otherwise; fix actions map into existing doctor fix-result vocabulary; unit tests cover fix scenarios without requiring real filesystem fixtures; `nix flake check` passes.
  - Verification notes (commands or checks): `nix flake check`; targeted Rust check through Nix during development.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/local_db_lifecycle.rs`
  - Evidence: `nix flake check` passed (all CLI tests, clippy, fmt, pkl-parity, JS checks passed).
  - Notes: Added `FixLifecycle` implementation on `LocalDbLifecycleService` with `fix()` method that resolves the DB path, checks parent directory state, and creates the missing canonical parent directory via `fs::create_dir_all()` when it matches the expected owned location. Extracted `fix_missing_parent_directory()` helper for testability. Deterministic refusal when the resolved parent path does not match the canonical SCE-owned location. Returns `LifecycleOutcome::Applied`/`Unchanged`/`Failed` mapping into existing doctor fix-result vocabulary. Unit tests cover: successful canonical parent creation, non-canonical parent refusal, unchanged when parent exists, failed for path without parent, and failed for creation error (e.g., /proc virtual filesystem).

- [x] T03: `Register local DB lifecycle in LifecycleRegistry and add registry tests` (status:done)
  - Task ID: T03
  - Goal: Register the local DB diagnostic and fix lifecycle facets in `LifecycleRegistry` for the `local_db` service ID, and add registry lookup tests for the new service ID and missing-capability handling.
  - Boundaries (in/out of scope): In - `LOCAL_DB_SERVICE_ID` registration in `LifecycleRegistry::diagnostic_lifecycle` and `LifecycleRegistry::fix_lifecycle`, registry lookup tests for `local_db` service ID, and missing-capability handling tests. Out - doctor rewiring, changes to hooks lifecycle registration, `SetupLifecycle` or `PreviewLifecycle` registration for local DB, or changes to doctor output contracts.
  - Done when: `LifecycleRegistry::diagnostic_lifecycle(LOCAL_DB_SERVICE_ID)` returns the local DB diagnostic lifecycle; `LifecycleRegistry::fix_lifecycle(LOCAL_DB_SERVICE_ID)` returns the local DB fix lifecycle; registry tests cover successful lookup and missing-capability handling for the `local_db` service ID; `nix flake check` passes.
  - Verification notes (commands or checks): `nix flake check`; targeted registry tests through Nix during development.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/lifecycle_registry.rs`
  - Evidence: `nix flake check` passed (all CLI tests, clippy, fmt, pkl-parity, JS checks passed).
  - Notes: Added `LOCAL_DB_LIFECYCLE_SERVICE` static, `LOCAL_DB_SERVICE_ID` match arms in `diagnostic_lifecycle()` and `fix_lifecycle()`, and six registry tests covering: diagnostic/fix lookup for `local_db`, `setup_lifecycle` returning `None` for `local_db` (setup is out of scope per plan constraints), and `None` for unknown service IDs across all three registry methods.

- [x] T04: `Move doctor local DB diagnosis through local DB lifecycle capability` (status:done)
  - Task ID: T04
  - Goal: Make doctor's local DB health inspection consume the local DB diagnostic lifecycle capability through the registry, so `doctor/inspect.rs` no longer directly owns local DB domain inspection logic beyond report assembly/mapping.
  - Boundaries (in/out of scope): In - migrating local DB path/health/parent-directory inspection logic into the local DB lifecycle service or mapping typed lifecycle diagnostics back into existing doctor report/problem types, preserving stable problem category/severity/fixability/remediation semantics. Out - doctor text/JSON output redesign, hook diagnosis changes, integration asset diagnosis changes, fix execution rewiring, or new problem categories beyond what local DB diagnosis currently surfaces.
  - Done when: local DB path resolution, parent-directory readiness, and DB health checks are owned by the local DB lifecycle diagnostic capability; `doctor/inspect.rs` no longer directly owns local DB domain inspection logic beyond report assembly/mapping; existing doctor output-shape tests still pass; `nix flake check` passes.
  - Verification notes (commands or checks): `nix flake check`; targeted doctor tests through Nix during development if needed.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/doctor/inspect.rs`, `cli/src/services/doctor/render.rs`, `cli/src/services/doctor/types.rs`
  - Evidence: `nix flake check` passed (all CLI tests, clippy, fmt, pkl-parity, JS checks passed).
  - Notes: Added `LocalDatabase` problem category and five local DB problem kinds (`LocalDbPathUnresolvable`, `LocalDbParentMissing`, `LocalDbParentNotDirectory`, `LocalDbParentNotWritable`, `LocalDbHealthCheckFailed`) to doctor types. Added `LocalDbHealth` struct with `LocalDbStatus` and `LocalDbParentStatus` enums to the doctor report. Wired `collect_local_db_health` in `inspect.rs` to consume the local DB diagnostic lifecycle through `LifecycleRegistry::diagnostic_lifecycle(LOCAL_DB_SERVICE_ID)` and map diagnostics into `DoctorProblem` types via `doctor_problem_from_local_db_diagnostic`. Added "Local database" section to text output and `local_db` field to JSON output. Doctor no longer directly owns local DB domain inspection logic beyond report assembly/mapping.

- [x] T05: `Move doctor local DB repair through local DB lifecycle capability` (status:done)
  - Task ID: T05
  - Goal: Make `sce doctor --fix` use the registered local DB fix lifecycle capability for parent-directory bootstrap repair instead of directly owning DB parent-directory creation logic.
  - Boundaries (in/out of scope): In - doctor fix-path rewiring for local DB parent-directory problems, mapping lifecycle fix outcomes to existing `DoctorFixResultRecord` values, preserving idempotent fixed/skipped/failed vocabulary, and tests for supported local DB repairs. Out - new fix classes beyond parent-directory bootstrap, hook repair changes, public dry-run flags, or changes to manual-only remediation.
  - Done when: doctor auto-fix obtains local DB parent-directory repair behavior through the lifecycle registry/capability; `doctor/fixes.rs` no longer directly owns local DB parent-directory creation logic beyond fix-result mapping; fix-mode text/JSON remains stable; `nix flake check` passes.
  - Verification notes (commands or checks): `nix flake check`; inspect doctor fix tests/output assertions for preserved result vocabulary.
  - Completed: 2026-04-28
  - Files changed: `cli/src/services/doctor/fixes.rs`
  - Evidence: `nix flake check` passed (all CLI tests, clippy, fmt, pkl-parity, JS checks passed).
  - Notes: Added `ProblemCategory::LocalDatabase` branch in `run_auto_fixes()` that calls `run_local_db_lifecycle_fix()` through `LifecycleRegistry::fix_lifecycle(LOCAL_DB_SERVICE_ID)`. Added `run_local_db_lifecycle_fix()`, `build_local_db_fix_results()`, and `local_db_fix_detail_from_lifecycle_outcome()` helper functions. Doctor no longer directly owns local DB parent-directory creation logic — all repair behavior routes through the local DB fix lifecycle capability. Fix-result vocabulary (Fixed/Skipped/Failed/Manual) is preserved through the existing `fix_result_from_lifecycle_outcome()` mapping.

- [x] T06: `Sync local DB lifecycle architecture context` (status:done)
  - Task ID: T06
  - Goal: Update durable context to describe the local DB lifecycle service, its diagnostic and fix capabilities, and the registry extension pattern alongside the hooks pilot.
  - Boundaries (in/out of scope): In - current-state updates to `context/cli/service-lifecycle.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/overview.md`; extension guidance for local DB lifecycle ownership boundaries. Out - historical progress narration, marking the plan as complete, or documenting unimplemented services as current runtime behavior.
  - Done when: context states that local DB is the second implemented lifecycle service with diagnostic and fix facets; doctor ownership boundaries match code truth; hooks lifecycle documentation remains accurate; future-service guidance is current-state oriented and does not overclaim unimplemented traits.
  - Verification notes (commands or checks): Review context against code truth; `nix run .#pkl-check-generated` if generated context/config surfaces are touched; otherwise note why generated-output parity is unaffected.
  - Completed: 2026-04-28
  - Files changed: (none needed - context already synchronized)
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` passed; verify-only pass confirmed context already accurately describes local DB lifecycle service as second implemented lifecycle capability with diagnostic and fix facets only, setup/preview deferred per plan constraints.

- [x] T07: `Run full validation and cleanup` (status:done)
  - Task ID: T07
  - Goal: Perform final repository validation, remove accidental temporary scaffolding, and verify code/context alignment for the local DB lifecycle implementation.
  - Boundaries (in/out of scope): In - full repo validation, cleanup of temporary shims/TODOs introduced during this plan, confirmation that no public CLI flag or output drift slipped in, and final context-sync verification. Out - adding more lifecycle service implementations or opportunistic architecture rewrites.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; no unintended temporary scaffolding remains; existing doctor/setup behavior is confirmed stable; context accurately reflects the implemented local DB lifecycle service.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted review that doctor text+JSON contracts were not intentionally changed; verify plan task statuses/evidence are updated by the executor.
  - Completed: 2026-04-28
  - Files changed: `context/plans/local-db-lifecycle.md` (task status updates)
  - Evidence: `nix run .#pkl-check-generated` passed; `nix flake check` passed; no temporary scaffolding introduced by this plan found; diff-trace.json files in context/tmp/ are pre-existing hook runtime artifacts, not from this plan; context already accurate from T06 verify-only pass.

## Open questions

- None at planning time. Resolved decisions: new plan, local DB as the second lifecycle service, diagnostic and fix facets only (setup and preview deferred), follow the hooks pattern exactly, preserve existing doctor output contracts.