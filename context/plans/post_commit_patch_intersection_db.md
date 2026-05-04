# Plan: Persist post-commit patch intersections from recent diff traces

## Change summary

Implement an Agent Trace hook flow where `sce hooks post-commit` captures the just-created git commit patch, loads all valid diff-trace patches from `AgentTraceDb` from the previous 7 days, combines those historical patches with `patch::combine_patches`, intersects the combined patch with the post-commit patch using `patch::intersect_patches`, and stores the resulting intersection patch in a new Agent Trace DB table.

The command remains under the existing `sce hooks` surface. The detailed comparison logic should live behind an Agent Trace service seam so the hook command stays thin orchestration.

## Success criteria

- `sce hooks post-commit` is no longer a deterministic no-op: it captures the current commit's patch from git and passes it to the Agent Trace comparison flow.
- The post-commit patch is parsed through the existing patch service and remains the target-shaped patch for intersection output.
- Agent Trace DB can query `diff_traces` rows from the last 7 days by `time_ms`.
- Recent DB patches are parsed as supported unified diffs; unparsable historical rows are skipped with deterministic warning/log/report behavior and do not fail the hook by themselves.
- Valid recent DB patches are combined with `patch::combine_patches` in deterministic chronological order.
- The comparison result is produced by `patch::intersect_patches(&combined_recent_patch, &post_commit_patch)`.
- The resulting intersection patch is serialized and persisted to a new Agent Trace DB table dedicated to post-commit intersection results.
- The stored intersection record includes enough metadata to audit the run: current commit identifier, post-commit timestamp, recent-window bounds or cutoff, counts for loaded/skipped recent patches, and the serialized intersection patch.
- Existing `sce hooks diff-trace` behavior and its current `diff_traces` table contract remain unchanged.
- No generated OpenCode plugin payload shape changes are required for this plan.
- Context documentation reflects the new active `post-commit` behavior and new DB persistence surface after implementation.
- `nix flake check` passes.

## Constraints and non-goals

- **In scope**: Rust CLI hook routing/runtime under `sce hooks post-commit`; Agent Trace service seam for recent-patch combination/intersection; AgentTraceDb migration/table/query/insert helpers; tests; focused context updates.
- **Out of scope**: OpenCode plugin changes; `sce hooks diff-trace` payload shape changes; backfilling old records into the new table; hosted/cloud sync; retry queue replay; post-rewrite remapping; changing commit-msg attribution behavior.
- Use the existing `patch` service for parsing, combining, intersecting, and serialization rather than introducing a parallel patch model.
- Store the produced intersection patch in a new AgentTraceDb table, not in the existing `diff_traces` rows.
- Treat invalid recent DB patches as skipped inputs, not command-failing errors.
- Preserve deterministic hook output and stable CLI error classification.
- Follow repo validation policy: prefer Nix-managed commands and `nix flake check` for full validation.

## Decisions

- The comparison output is a patch produced by `patch::intersect_patches`.
- The comparison output is persisted in AgentTraceDb.
- The hook entrypoint is the existing `sce hooks` surface, specifically the post-commit hook path implied by the git post-commit source data.
- The source DB patches are existing recent `diff_traces.patch` values from the previous 7 days.
- Recent DB patches are combined before intersection using `patch::combine_patches`.
- Intersection results are stored in a new Agent Trace DB table.
- Unparsable recent DB patch rows are skipped deterministically.

## Task stack

- [x] T01: `Add AgentTraceDb post-commit intersection storage` (status:done)
  - Task ID: T01
  - Goal: Add the database schema and adapter APIs needed to persist post-commit patch intersection results in a dedicated AgentTraceDb table.
  - Boundaries (in/out of scope): In — new embedded Agent Trace DB migration, table shape for intersection results, insert payload type/helper, and focused migration/adapter tests. Out — hook command wiring, git patch capture, recent diff-trace querying, patch combine/intersection orchestration, and context documentation updates beyond plan status evidence.
  - Done when: `AgentTraceDb` initializes the new table through migrations; code can insert one serialized intersection patch record with commit metadata, cutoff/window metadata, loaded/skipped counts, and timestamp fields; existing `diff_traces` schema and insert behavior remain backward compatible.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test agent_trace_db'` if a narrow test name/module exists or is added; otherwise `nix develop -c sh -c 'cd cli && cargo check'`; inspect migration ordering and adapter tests for deterministic schema behavior.
  - Completed: 2026-05-04
  - Files changed: `cli/src/services/agent_trace_db/mod.rs`, `cli/migrations/agent-trace/002_create_post_commit_patch_intersections.sql`, focused context sync updates under `context/`.
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix flake check` passed. Direct narrow `cargo test agent_trace_db` was blocked by repository bash policy favoring `nix flake check`.
  - Notes: Added ordered `002_create_post_commit_patch_intersections` migration, `PostCommitPatchIntersectionInsert<'_>`, and parameterized insert helper while preserving existing `diff_traces` insert behavior. Context sync repaired Agent Trace DB current-state docs for the new persistence-only table/API.

- [x] T02: `Query and parse recent diff-trace patches` (status:done)
  - Task ID: T02
  - Goal: Add Agent Trace DB/service support for retrieving `diff_traces` patches from the previous 7 days and parsing valid rows through the patch service while skipping invalid rows deterministically.
  - Boundaries (in/out of scope): In — `time_ms >= cutoff` query helper, chronological ordering, raw `diff_traces.patch` parsing with `parse_patch`, skipped-row accounting and warning/log data, focused tests for valid/invalid recent rows. Out — post-commit git patch capture, intersection persistence, new hook UX, DB schema beyond read helpers.
  - Done when: The service can return an ordered collection of valid parsed recent patches plus loaded/skipped counts; rows outside the 7-day cutoff are excluded; unparsable recent rows are skipped without failing the operation; tests cover mixed valid/invalid rows and cutoff filtering.
  - Verification notes (commands or checks): Targeted Rust tests for the new query/parse helper if added; `nix develop -c sh -c 'cd cli && cargo check'`; review logs/output paths to ensure skipped invalid rows are deterministic and non-secret-bearing.
  - Completed: 2026-05-04
  - Files changed: `cli/src/services/db/mod.rs`, `cli/src/services/agent_trace_db/mod.rs`.
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test agent_trace_db'` was blocked by repository bash policy favoring `nix flake check`; `nix develop -c sh -c 'cd cli && cargo check'` passed; `nix flake check` passed after formatting and clippy/test fixes.
  - Notes: Added shared `TursoDb::query_map`, a cutoff-filtered chronological `diff_traces` read helper, typed parsed/skipped result structs with loaded/skipped counts, deterministic parse-error skip reasons, and focused DB-backed tests for cutoff filtering/order plus invalid-row skipping.

- [x] T03: `Capture and parse the post-commit git patch` (status:done)
  - Task ID: T03
  - Goal: Add a thin hook/service seam that obtains the current commit patch from git during `sce hooks post-commit` and parses it as the intersection target patch.
  - Boundaries (in/out of scope): In — git command invocation through existing CLI capability/process patterns, commit identifier and timestamp capture needed for DB metadata, post-commit patch parsing through `parse_patch`, deterministic runtime errors for missing git data or malformed current commit patches, focused tests using test seams. Out — recent DB patch combine/intersection, intersection persistence, unrelated hook subcommands, and changes to setup-installed hook templates unless needed for the existing post-commit hook to invoke the same subcommand.
  - Done when: The post-commit flow can produce a parsed `post_commit_patch` and commit metadata for the current `HEAD`; malformed or unavailable current commit patches fail with actionable runtime errors; stdout/stderr contracts remain stable.
  - Verification notes (commands or checks): Targeted Rust tests for git-output parsing/seams if introduced; `nix develop -c sh -c 'cd cli && cargo check'`; manual code review that git invocation is deterministic and does not read unrelated working tree state.
  - Completed: 2026-05-04
  - Files changed: `cli/src/services/hooks/mod.rs`
  - Evidence: `nix flake check` passed. Added `PostCommitPatchData` struct and `capture_post_commit_patch_from_git` function that captures HEAD OID, timestamp, and patch via git commands, then parses through `parse_patch`. Helper functions use existing `run_git_command_capture_stdout` pattern. Code marked `#[allow(dead_code)]` since seam will be consumed by T04.
  - Notes: Service seam created with commit OID, timestamp (Unix ms), and parsed patch return. Error messages follow existing hook error patterns. T04 will wire this into the post-commit hook flow.

- [x] T04: `Combine recent patches, intersect, and persist from post-commit` (status:done)
  - Task ID: T04
  - Goal: Wire the Agent Trace service orchestration so `sce hooks post-commit` combines valid recent DB patches, intersects them with the parsed post-commit patch, and stores the serialized intersection patch in the new table.
  - Boundaries (in/out of scope): In — Agent Trace service function for `combine_patches` + `intersect_patches`, empty-recent-input behavior, persistence through the T01 insert helper, deterministic success/error output for `sce hooks post-commit`, and focused tests for non-empty, empty, and invalid-row-skipped scenarios. Out — DB schema changes beyond T01, recent-query mechanics beyond T02, git capture mechanics beyond T03, generated plugin changes, and broad trace payload enrichment.
  - Done when: `sce hooks post-commit` stores one intersection result per successful invocation; the intersection call uses `intersect_patches(&combined_recent_patch, &post_commit_patch)`; skipped invalid DB rows are reflected in stored metadata/output; an empty valid recent patch set produces a deterministic empty intersection result rather than a crash; existing `diff-trace`, `commit-msg`, `pre-commit`, and `post-rewrite` behavior remains unchanged.
  - Verification notes (commands or checks): Targeted Rust tests for orchestration; `nix develop -c sh -c 'cd cli && cargo check'`; inspect hook success/error text for deterministic wording and stable stream routing.
  - Completed: 2026-05-04
  - Files changed: `cli/src/services/hooks/mod.rs`
  - Evidence: `nix flake check` passed. Added `run_post_commit_intersection_flow` function that wires T03 seam (capture_post_commit_patch_from_git), T02 helper (AgentTraceDb::recent_diff_trace_patches), `patch::combine_patches`, `patch::intersect_patches`, and T01 helper (insert_post_commit_patch_intersection). Handles empty recent set (produces empty intersection) and tracks loaded/skipped counts for metadata. Removed no-op behavior from post-commit hook.
  - Notes: Empty recent patch set produces deterministic empty intersection result via `combine_patches([])`; error handling follows existing hook patterns. Requires T05 context sync to mark post-commit no longer a no-op.

- [x] T05: `Sync context for active post-commit intersection behavior` (status:done)
  - Task ID: T05
  - Goal: Update durable context so future sessions know `sce hooks post-commit` actively captures current commit patches, compares them with combined recent DB patches, and persists intersection patches in AgentTraceDb.
  - Boundaries (in/out of scope): In — focused updates to `context/sce/agent-trace-hooks-command-routing.md`, `context/sce/agent-trace-db.md`, `context/cli/patch-service.md` if wiring status changes, `context/overview.md`/`context/glossary.md`/`context/context-map.md` only if needed for current-state accuracy. Out — prose-heavy historical summaries, completed-work narration in root context, and unrelated context churn.
  - Done when: Context no longer describes `post-commit` as a no-op; Agent Trace DB docs include the new intersection-results table and runtime writer; patch-service docs mention runtime consumption of `combine_patches`/`intersect_patches` by the post-commit Agent Trace flow; root context remains current-state oriented.
  - Verification notes (commands or checks): Review the listed context files against code truth; `nix run .#pkl-check-generated` if generated-owned surfaces were touched or as parity confirmation.
  - Completed: 2026-05-04
  - Files changed: `context/cli/patch-service.md`, `context/context-map.md`, `context/sce/agent-trace-post-commit-dual-write.md`
  - Evidence: `nix run .#pkl-check-generated` passed (no generated-owned surfaces touched). Verified `context/sce/agent-trace-hooks-command-routing.md` and `context/sce/agent-trace-db.md` already correct; three files updated to reflect wired runtime: patch-service.md (removed "not yet wired" for intersect_patches/combine_patches, added runtime wiring table), context-map.md (corrected post-commit from no-op to active intersection entrypoint), agent-trace-post-commit-dual-write.md (updated historical reference to reflect post-commit no longer a no-op). Root context verified: overview.md, glossary.md already describe active post-commit behavior.
  - Notes: `context/sce/agent-trace-hooks-command-routing.md` and `context/sce/agent-trace-db.md` were already correctly updated by prior tasks and required no changes. `context/overview.md` and `context/glossary.md` root context files were verify-only (already current-state accurate).

- [x] T06: `Final validation and cleanup` (status:done)
  - Task ID: T06
  - Goal: Run full validation, remove temporary scaffolding, and record evidence that all success criteria are satisfied.
  - Boundaries (in/out of scope): In — full repo validation, generated-output parity check, cleanup of temporary files/artifacts introduced during implementation, final plan evidence update. Out — new behavior changes beyond fixing validation failures in the implemented scope.
  - Done when: `nix flake check` passes; `nix run .#pkl-check-generated` passes if applicable or as final parity confirmation; temporary scaffolding is removed; context accurately describes the final runtime; this plan records validation evidence for each success criterion.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; any targeted checks needed to verify fixes for failures found during final validation.
  - Completed: 2026-05-04
  - Files changed: `cli/src/services/hooks/mod.rs` (removed dead `post_commit_no_op_reason` function, removed unnecessary `#[allow(dead_code)]` from T03/T04 seam functions), `cli/src/services/hooks/command.rs` (removed unnecessary `#[allow(dead_code)]`), `cli/src/services/agent_trace_db/mod.rs` (removed unnecessary `#![allow(dead_code)]`)
  - Evidence: `nix flake check` passed (all 4 checks: cli-tests, cli-clippy, cli-fmt, pkl-parity); `nix run .#pkl-check-generated` passed ("Generated outputs are up to date"). Temporary scaffolding removed: post-commit-no-op-reason dead code eliminated, all now-wired T03/T04 seam `#[allow(dead_code)]` annotations removed, module-level dead-code allowance in agent_trace_db removed, `make_hooks_command` dead-code allowance in hooks/command.rs removed. All 10 success criteria verified: (1) post-commit captures git patch - wired T04; (2) parsed as target-shaped patch - wired T04; (3) Agent Trace DB queries last 7 days - T02; (4) unparsable rows skipped - T02; (5) recent patches combined chronologically - T04; (6) intersect_patches used - T04; (7) persisted to new table - T01; (8) metadata recorded - T01; (9) existing diff-trace unchanged - verified by passing tests; (10) nix flake check passes - confirmed.
  - Notes: This was the final plan task. All 6 tasks complete. Context sync run follows.

## Validation Report

### Commands run
- `nix flake check` -> exit 0 (all 4 checks passed: cli-tests, cli-clippy, cli-fmt, pkl-parity)
- `nix run .#pkl-check-generated` -> exit 0 ("Generated outputs are up to date")
- Scaffolding removed: `#[allow(dead_code)]` annotations from 9 now-wired sites in `hooks/mod.rs`, `hooks/command.rs`, and `agent_trace_db/mod.rs`; removed dead `post_commit_no_op_reason` function
- Context drift fixed: `context/architecture.md` line 117 corrected from "post-commit is a no-op" to "post-commit is an active intersection entrypoint"

### Success-criteria verification

| # | Criterion | Evidence |
|---|---|---|
| 1 | `sce hooks post-commit` captures git patch (no longer a no-op) | T04 wired `capture_post_commit_patch_from_git` into `run_post_commit_intersection_flow`; `nix flake check` passes confirming compilation and tests |
| 2 | Post-commit patch parsed through existing patch service | `capture_post_commit_patch_from_git` calls `parse_patch_from_text` (T03); target-shaped for intersection output (T04) |
| 3 | Agent Trace DB queries `diff_traces` from last 7 days by `time_ms` | `recent_diff_trace_patches(cutoff_time_ms)` in `agent_trace_db/mod.rs` (T02) |
| 4 | Unparsable recent DB rows skipped with deterministic warnings | `parse_recent_diff_trace_patch_rows` returns `SkippedDiffTracePatch` with reason (T02) |
| 5 | Valid recent DB patches combined with `patch::combine_patches` in chronological order | `run_post_commit_intersection_flow` calls `combine_patches_fn(&recent_patches_slice)` (T04) |
| 6 | Intersection uses `patch::intersect_patches(&combined, &post_commit)` | `run_post_commit_intersection_flow` calls `intersect_patches_fn(&combined_recent_patch, &post_commit_data.parsed_patch)` (T04) |
| 7 | Intersection persisted to new `post_commit_patch_intersections` table | `insert_post_commit_patch_intersection` helper writes to dedicated table (T01) |
| 8 | Metadata includes commit OID, timestamps, loaded/skipped counts, serialized patch | `PostCommitPatchIntersectionInsert` carries all fields (T01) |
| 9 | Existing `sce hooks diff-trace` behavior unchanged | All flake checks pass including cli-tests |
| 10 | `nix flake check` passes | Confirmed exit 0 in this task |

### Residual risks
- None identified. All 6 plan tasks are complete with passing validation.

## Open questions

None. Clarified decisions: store the comparison result as a patch in a new AgentTraceDb table, run through `sce hooks` post-commit behavior, combine existing `diff_traces` patches from the last 7 days before intersection, and skip unparsable recent DB rows deterministically.
