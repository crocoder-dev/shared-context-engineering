# Plan: Agent Trace intersection persistence

## Change summary

Persist the `intersect_patches(...)` result into the existing Agent Trace database after commits. The current `sce hooks diff-trace` path already stores raw session diffs in `diff_traces.patch`; this change adds a post-commit flow that selects the latest diff-trace session, combines all patches for that session, intersects the combined constructed patch with the canonical `HEAD` post-commit patch, serializes the resulting `ParsedPatch` as JSON, and stores it in a new AgentTraceDb table.

This plan intentionally stores only the intermediate intersection `ParsedPatch` JSON, not the full `AgentTrace` payload from `build_agent_trace(...)`.

## Success criteria

- `sce hooks diff-trace` continues to store raw session diff payloads in `diff_traces` without changing the OpenCode plugin payload shape.
- `sce hooks post-commit` is no longer a pure no-op when usable diff-trace rows exist; it attempts intersection persistence for the latest recorded session.
- Latest-session selection is deterministic: choose the latest `session_id` from `diff_traces` ordered by highest `time_ms`, then highest `id` as tie-breaker.
- All `diff_traces.patch` rows for the selected session are loaded in deterministic order, parsed with `parse_patch(...)`, combined with `combine_patches(...)`, and intersected with the parsed `HEAD` post-commit patch via `intersect_patches(...)`.
- The intersection result is serialized as `ParsedPatch` JSON and inserted into a new AgentTraceDb table, tentatively named `patch_intersections`.
- Stored intersection rows retain enough metadata to audit provenance: `commit_sha`, `session_id`, ordered source `diff_trace` IDs, `intersection_json`, and `created_at`.
- Empty/missing session data, invalid stored patch data, invalid post-commit patch data, missing git `HEAD`, and DB failures surface deterministic runtime diagnostics without corrupting existing `diff_traces` rows.
- No full `AgentTrace` JSON is generated or persisted as part of this feature.
- No plugin, generated config, cloud sync, retry queue, backfill, or historical artifact import behavior is added.
- Context documentation reflects the new intersection persistence behavior after implementation.
- Final validation passes with the repository-preferred checks.

## Constraints and non-goals

- **In scope**: AgentTraceDb migration and adapter helpers; read helpers for latest session diff traces; post-commit patch capture from `git`; parsing/combining/intersecting patches; insertion of `ParsedPatch` intersection JSON; focused Rust tests; context sync for current-state docs.
- **Out of scope**: full `AgentTrace` JSON persistence; OpenCode plugin changes; changing `diff-trace` STDIN payload shape; generated config regeneration unless context sync unexpectedly requires it; cloud sync/export; retry/backfill/import flows; multi-session disambiguation beyond latest-session heuristic; changing `pre-commit`, `commit-msg`, or `post-rewrite` behavior.
- Existing raw diff persistence in `diff_traces` remains backward compatible.
- The latest-session heuristic is accepted as MVP behavior and must be documented as a known limitation for parallel session workflows.
- Implementation tasks remain atomic: each task should be landable as one coherent commit.

## Decisions

- Store only the `intersect_patches(...)` result as serialized `ParsedPatch` JSON.
- Use the existing Agent Trace DB, not the neutral LocalDb.
- Add a new AgentTraceDb table instead of replacing or overloading `diff_traces.patch`.
- In post-commit, choose the latest session by `(time_ms DESC, id DESC)` and combine all rows for that `session_id`.
- Do not require plugin/session environment changes for this MVP.

## Task stack

- [x] T01: `Add patch intersection table and DB insert helper` (status:done)
  - Task ID: T01
  - Goal: Add AgentTraceDb schema and adapter support for persisting patch intersection JSON rows.
  - Boundaries (in/out of scope): In — new idempotent migration under `cli/migrations/agent-trace/`, migration registration in `cli/src/services/agent_trace_db/mod.rs`, typed insert payload, parameterized insert helper, and narrow adapter tests or compile coverage. Out — hook runtime changes, diff-trace retrieval logic, patch parsing/intersection logic, and context docs beyond plan status updates.
  - Done when: `AgentTraceDb::new()` runs both existing and new migrations; the new `patch_intersections` table can store `commit_sha`, ordered source diff trace IDs, `intersection_json`, and `created_at`; insertion uses parameterized SQL; existing `diff_traces` insert behavior is unchanged.
  - Verification notes (commands or checks): Prefer `nix develop -c sh -c 'cd cli && cargo check'` for narrow compile verification; final repo validation remains `nix flake check`.
  - Completed: 2026-05-04
  - Files changed: `cli/migrations/agent-trace/002_create_patch_intersections.sql`; `cli/src/services/agent_trace_db/mod.rs`; focused context sync in `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/cli-command-surface.md`, `context/sce/agent-trace-db.md`, and `context/sce/shared-turso-db.md`.
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'` passed; `nix develop -c sh -c 'cd cli && cargo check'` passed; direct targeted `cargo test agent_trace_db` was blocked by the repo bash policy in favor of `nix flake check`; `nix flake check` passed; `nix run .#pkl-check-generated` passed after context sync.
  - Notes: Added the idempotent `patch_intersections` table migration, deterministic migration registration after `diff_traces`, `PatchIntersectionInsert`, and `insert_patch_intersection(...)` with bound SQL parameters. Existing `insert_diff_trace(...)` behavior is unchanged; post-commit runtime wiring remains follow-up scope.

- [x] T02: `Add latest-session diff trace read helpers` (status:done)
  - Task ID: T02
  - Goal: Add deterministic AgentTraceDb helpers to identify the latest diff-trace session and load all raw diff patches for that session.
  - Boundaries (in/out of scope): In — typed read DTOs for diff trace rows, latest session query ordered by `time_ms DESC, id DESC`, session row query ordered by `time_ms ASC, id ASC`, and deterministic empty-result handling. Out — post-commit hook wiring, patch parsing/intersection, new plugin/session binding behavior, and DB schema changes beyond what T01 creates.
  - Done when: callers can ask AgentTraceDb for the latest available `session_id` and retrieve all `id` + `patch` rows for that session in stable order; no rows returns a clear no-data outcome rather than a panic; existing insert APIs remain compatible.
  - Verification notes (commands or checks): Narrow Rust checks/tests for query helper behavior where practical; `nix develop -c sh -c 'cd cli && cargo check'`.
  - Completed: 2026-05-04
  - Files changed: `cli/src/services/agent_trace_db/mod.rs`; `cli/src/services/db/mod.rs`; focused context sync in `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/agent-trace-db.md`, and `context/sce/shared-turso-db.md`.
  - Evidence: direct targeted `cargo test agent_trace_db` was blocked by the repo bash policy in favor of `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` passed; `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix flake check` passed before context sync and again after context sync; `nix run .#pkl-check-generated` passed after context sync.
  - Notes: Added `latest_diff_trace_session_id()` with `(time_ms DESC, id DESC)` latest-session selection, `diff_trace_patches_for_session(...)` with `(time_ms ASC, id ASC)` row ordering, `DiffTracePatchRow`, and a shared synchronous `TursoDb::query_map(...)` helper needed to decode query rows. Empty latest-session state returns `None`; missing session rows return an empty vector. Context sync repaired localized AgentTraceDb/shared Turso references; no post-commit hook wiring was added.

- [x] T03: `Build intersection from stored session diffs and HEAD patch` (status:done)
  - Task ID: T03
  - Goal: Introduce a focused service/helper that turns stored session diff patches plus a post-commit patch string into serialized intersection `ParsedPatch` JSON.
  - Boundaries (in/out of scope): In — parse stored raw diff patches with `parse_patch(...)`, combine them with `combine_patches(...)`, parse the `HEAD` post-commit patch, run `intersect_patches(...)`, serialize the result with `serde_json`, and return provenance metadata needed for DB insertion. Out — DB table creation, DB read queries, hook command routing, full `build_agent_trace(...)` output, and plugin changes.
  - Done when: valid stored diffs + valid post-commit patch produce deterministic `ParsedPatch` JSON; invalid stored or post-commit patch data returns actionable errors; empty source diff lists are handled explicitly; unit tests cover at least one successful intersection and one invalid-patch error path.
  - Verification notes (commands or checks): Targeted Rust tests for the new helper if added; `nix develop -c sh -c 'cd cli && cargo check'`.
  - Completed: 2026-05-04
  - Files changed: `cli/src/services/agent_trace.rs`; `cli/src/services/agent_trace/tests.rs`; focused context sync in `context/sce/agent-trace-minimal-generator.md`, `context/cli/patch-service.md`, `context/context-map.md`, and `context/glossary.md`.
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt'` passed after adding temporary dead-code allowances for the not-yet-wired helper; direct targeted `cargo test patch_intersection_builder` was blocked by the repo bash policy in favor of `nix flake check`; `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix flake check` passed; `nix run .#pkl-check-generated` passed after context sync.
  - Notes: Added pure `build_patch_intersection_json(...)` support that parses ordered source diff-trace patches, combines them, parses the post-commit patch, serializes the `intersect_patches(...)` result as `ParsedPatch` JSON, and returns ordered source diff-trace IDs for later DB insertion. Empty source lists, invalid source patches, invalid post-commit patches, and serialization failures have deterministic error variants. Context sync classified the change as localized Agent Trace library behavior, updated domain docs/glossary/map entries, and left hook runtime docs at the current no-op post-commit state. No DB, hook routing, generated config, plugin, or full AgentTrace persistence behavior was added.

- [x] T04: `Wire post-commit hook to persist latest-session intersection` (status:done)
  - Task ID: T04
  - Goal: Update `sce hooks post-commit` to capture `HEAD` commit metadata, use latest-session diff traces, build the intersection JSON, and insert it into AgentTraceDb.
  - Boundaries (in/out of scope): In — `post-commit` runtime path in `cli/src/services/hooks/mod.rs`, git `HEAD` SHA + patch capture reuse or extraction, latest-session DB reads, intersection helper call, DB insert call, deterministic success/no-data/error messages, and focused tests/test seams. Out — changing `diff-trace` payload shape, OpenCode plugin behavior, full AgentTrace generation, retry/backfill behavior, and non-post-commit hook behavior.
  - Done when: `sce hooks post-commit` persists one `patch_intersections` row when a latest session and valid `HEAD` patch exist; source diff trace IDs are stored in the same order used for `combine_patches(...)`; no available diff traces returns a deterministic no-op/no-data message; failures are surfaced as runtime errors with actionable context; existing hook trace artifact behavior remains compatible if retained.
  - Verification notes (commands or checks): Targeted hook tests if introduced; manual code inspection for ordering and failure semantics; `nix develop -c sh -c 'cd cli && cargo check'`.
  - Completed: 2026-05-04
  - Files changed: `cli/src/services/hooks/mod.rs`; plan evidence in `context/plans/agent_trace_intersection_persistence.md`; context sync in `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, `context/context-map.md`, `context/cli/cli-command-surface.md`, `context/cli/patch-service.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/agent-trace-minimal-generator.md`, `context/sce/agent-trace-post-commit-dual-write.md`, `context/sce/agent-trace-retry-queue-observability.md`, and `context/sce/agent-trace-implementation-contract.md`.
  - Evidence: direct targeted `cargo test post_commit_intersection` was blocked by the repo bash policy in favor of `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` passed; `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix flake check` passed; `nix run .#pkl-check-generated` passed after context sync.
  - Notes: `post-commit` now skips deterministically when no diff-trace rows exist, preserves the existing disabled no-op, selects the latest session via AgentTraceDb helpers, loads ordered source rows, captures `HEAD` SHA and patch only after source rows are available, builds `ParsedPatch` intersection JSON, persists `patch_intersections` with JSON-serialized ordered source IDs, and retains best-effort hook trace artifact persistence. `diff-trace`, plugin payloads, full AgentTrace JSON generation, retry/backfill, and non-`post-commit` hook behavior were not changed.

- [x] T05: `Sync context for intersection persistence behavior` (status:done)
  - Task ID: T05
  - Goal: Update current-state context docs so future sessions understand raw diff persistence, post-commit intersection persistence, and the latest-session MVP heuristic.
  - Boundaries (in/out of scope): In — focused updates to `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/cli/patch-service.md` if runtime wiring status changes, `context/context-map.md`, and `context/glossary.md` entries as needed. Out — broad narrative history, unrelated architecture rewrites, completed-work summaries in durable context, and generated config changes.
  - Done when: Context states that `diff_traces` stores raw session diffs while `patch_intersections` stores post-commit `ParsedPatch` intersection JSON from the latest-session heuristic; docs do not claim full AgentTrace JSON is persisted by this feature.
  - Verification notes (commands or checks): Read affected context files for current-state accuracy; run `nix run .#pkl-check-generated` if generated parity could be affected, otherwise verify no generated-owned paths were touched.
  - Completed: 2026-05-04
  - Files changed: None (verify-only — all target context files already accurate after T04 sync)
  - Evidence: `nix flake check` passed; `nix run .#pkl-check-generated` passed (generated outputs up to date); all five target context files (`agent-trace-db.md`, `agent-trace-hooks-command-routing.md`, `patch-service.md`, `context-map.md`, `glossary.md`) already document both `diff_traces` and `patch_intersections` tables, the latest-session heuristic, and explicitly state no full `AgentTrace` JSON is persisted.
  - Notes: Verify-only task. Context was already aligned with implemented code truth from T01-T04. No edits were required.

- [x] T06: `Final validation and cleanup` (status:done)
  - Task ID: T06
  - Goal: Run final validation, remove temporary scaffolding, and record evidence that all feature success criteria are satisfied.
  - Boundaries (in/out of scope): In — full repo validation, generated-output parity check, formatting/lint/test evidence, cleanup of temporary files under `context/tmp/` if any were created, and plan evidence updates. Out — new feature behavior beyond fixes required to pass validation.
  - Done when: `nix flake check` passes; `nix run .#pkl-check-generated` passes or is documented as unnecessary only if no generated/context parity surface is touched; no temporary implementation scaffolding remains; this plan records validation evidence and residual risks.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; inspect `context/tmp/` for temporary scaffolding only if implementation created any.
  - Completed: 2026-05-04
  - Files changed: None (validation-only)
  - Evidence: `nix flake check` passed (all 15 derivations evaluated, `all checks passed!`); `nix run .#pkl-check-generated` passed (`Generated outputs are up to date.`); `context/tmp/` contains only legitimate hook runtime artifacts (diff-trace and post-commit JSON files from actual hook invocations), no implementation scaffolding.
  - Notes: Final plan task. All six tasks (T01-T06) are complete.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format)
- `nix run .#pkl-check-generated` -> exit 0 (Generated outputs are up to date.)
- No temporary scaffolding to remove; `context/tmp/` contains only legitimate hook runtime artifacts.

### Success-criteria verification
- [x] `sce hooks diff-trace` continues to store raw session diff payloads in `diff_traces` without changing the OpenCode plugin payload shape -> confirmed: `diff-trace` runtime unchanged; context docs accurate.
- [x] `sce hooks post-commit` is no longer a pure no-op when usable diff-trace rows exist -> confirmed: post-commit selects latest session, builds intersection, inserts into `patch_intersections`.
- [x] Latest-session selection is deterministic: `(time_ms DESC, id DESC)` -> confirmed: `latest_diff_trace_session_id()` implements this ordering.
- [x] All `diff_traces.patch` rows for selected session loaded in deterministic order, parsed, combined, intersected with HEAD patch -> confirmed: `diff_trace_patches_for_session()` uses `(time_ms ASC, id ASC)`, `build_patch_intersection_json()` handles parse/combine/intersect.
- [x] Intersection result serialized as `ParsedPatch` JSON and inserted into `patch_intersections` table -> confirmed: `insert_patch_intersection()` with parameterized SQL.
- [x] Stored intersection rows retain provenance metadata (`commit_sha`, `session_id`, source IDs, `intersection_json`, `created_at`) -> confirmed: migration schema and insert payload.
- [x] Empty/missing session data, invalid patches, missing HEAD, DB failures surface deterministic diagnostics without corrupting `diff_traces` -> confirmed: error variants in `PatchIntersectionBuildError`, hook runtime error handling.
- [x] No full `AgentTrace` JSON is generated or persisted -> confirmed: only `ParsedPatch` intersection JSON stored; context docs explicitly state this boundary.
- [x] No plugin, generated config, cloud sync, retry queue, backfill, or historical artifact import behavior added -> confirmed: scope limited to DB migration, read helpers, intersection builder, post-commit wiring.
- [x] Context documentation reflects new intersection persistence behavior -> confirmed: T05 verify-only pass confirmed all context files accurate.
- [x] Final validation passes with repository-preferred checks -> confirmed: `nix flake check` passed, `nix run .#pkl-check-generated` passed.

### Residual risks
- The latest-session heuristic (`time_ms DESC, id DESC`) is an MVP approximation; parallel session workflows may select the wrong session. Documented as a known limitation for future follow-up.
- No backfill of existing `context/tmp/*-diff-trace.json` artifacts into AgentTraceDb; only new hook invocations populate the database.

## Open questions

None for MVP planning. Future follow-up: replace the latest-session heuristic with explicit session-to-commit binding if parallel session workflows need stronger correctness.
