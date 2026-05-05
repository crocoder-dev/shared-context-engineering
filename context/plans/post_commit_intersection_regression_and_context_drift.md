# Plan: Post-commit intersection regression coverage and context drift fixes

## Change summary

Fix the post-commit intersection follow-up issues by adding targeted regression coverage for the newly bounded recent-diff-trace query and the post-commit intersection orchestration, then repairing stale current-state documentation and the stale migration query comment.

This plan treats the code in `cli/src/services/hooks/mod.rs` and `cli/src/services/agent_trace_db/mod.rs` as the source of truth: `recent_diff_trace_patches` now accepts both `cutoff_time_ms` and `end_time_ms`, applies a lower and upper time bound, parses stored raw patch text with `parse_patch`, and the post-commit flow uses a `now_ms` window end for the DB query and persistence metadata.

## Success criteria

- `AgentTraceDb::recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` has direct regression coverage for lower-bound filtering, upper-bound filtering, and deterministic `time_ms ASC, id ASC` ordering.
- DB row handling coverage confirms valid raw patch text is parsed with `parse_patch`, malformed raw patch rows are skipped deterministically, and loaded/skipped accounting remains correct.
- `run_post_commit_intersection_flow` has direct unit coverage or an injectable seam that verifies the flow queries recent DB rows with the same bounded `[cutoff_ms, end_ms]` window used for stored intersection metadata.
- `run_post_commit_intersection_flow` coverage verifies the key orchestration contract: capture current commit patch, combine valid recent patches, intersect against the post-commit patch, persist one intersection row, and report loaded/skipped/file counts without relying only on broad compilation/flake checks.
- `context/sce/agent-trace-db.md` describes `recent_diff_trace_patches(cutoff_time_ms, end_time_ms)`, both `time_ms >= cutoff_time_ms` and `time_ms <= end_time_ms` filtering, and includes migration `003_add_diff_traces_time_ms_id_index.sql`.
- `context/cli/patch-service.md` no longer says `recent_diff_trace_patches` uses JSON load helpers; it reflects raw patch parsing via `parse_patch` for stored `diff_traces.patch` text.
- `cli/migrations/agent-trace/003_add_diff_traces_time_ms_id_index.sql` has a current query comment that includes the `time_ms <= ?2` bound.
- Existing `sce hooks diff-trace`, `commit-msg`, `pre-commit`, and `post-rewrite` behavior remains unchanged.
- `nix flake check` passes.

## Constraints and non-goals

- **In scope**: focused Rust tests/test seams around `AgentTraceDb` recent patch querying and post-commit intersection orchestration; focused current-state context documentation repairs; the stale SQL migration comment repair.
- **Out of scope**: changing runtime behavior beyond what is needed to introduce test seams safely; changing Agent Trace DB schema; adding new migrations; changing OpenCode plugin payloads; backfilling historical `diff_traces` rows; hosted/cloud sync; retry queue replay; broad refactors unrelated to testability.
- Prefer small, deterministic test seams over broad architectural changes. If a seam is introduced for testing, it should be private/internal unless production code already needs a public abstraction.
- Preserve stable CLI output and error text except where a test must codify existing behavior.
- Follow repository validation policy: prefer Nix-managed commands and `nix flake check` for full validation.

## Assumptions

- The implementation should create a new focused follow-up plan rather than reopening the already-completed `post_commit_patch_intersection_db` plan.
- The stale completed-plan validation text is not durable current-state documentation and does not need retroactive repair unless an implementation task discovers it is still used as active handoff state.

## Task stack

- [x] T01: `Cover bounded recent diff-trace DB queries` (status:done)
  - Task ID: T01
  - Goal: Add targeted `AgentTraceDb` regression tests for the bounded recent patch query and row parsing behavior.
  - Boundaries (in/out of scope): In — tests for `recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` lower/upper time bounds, `time_ms ASC, id ASC` ordering, valid raw patch parsing via `parse_patch`, malformed-row skip accounting, and loaded/skipped counts. Out — hook orchestration tests, runtime behavior changes, schema changes, migration comment/doc updates.
  - Done when: Focused tests fail against a one-sided time-window implementation, pass with the current two-sided query, and confirm malformed raw patch text is skipped without failing the query.
  - Verification notes (commands or checks): Prefer a targeted Nix-wrapped Rust test for the Agent Trace DB module if practical, then `nix flake check` before handoff evidence; inspect that tests do not depend on wall-clock timing.
  - Completed: 2026-05-05
  - Files changed: `cli/src/services/agent_trace_db/mod.rs`
  - Evidence: Added DB-backed regression test `recent_diff_trace_patches_applies_bounded_window_ordering_and_parse_accounting`; direct targeted `cargo test` was blocked by the repository bash policy favoring `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` passed; `nix flake check` passed after exercising the new test through `checks.x86_64-linux.cli-tests`.
  - Notes: Test uses deterministic inserted `diff_traces` rows to cover inclusive lower/upper bounds, `time_ms ASC, id ASC` ordering, raw patch parsing, malformed-row skip accounting, and loaded/skipped counts without wall-clock timing.

- [x] T02: `Cover post-commit intersection window orchestration` (status:done)
  - Task ID: T02
  - Goal: Add direct regression coverage or a minimal injectable seam for `run_post_commit_intersection_flow` so the post-commit window and persistence orchestration are tested directly.
  - Boundaries (in/out of scope): In — private/internal test seam if needed for current time, git patch capture, recent patch query, and persistence; tests that assert the same `now_ms` value is used as the query end bound and persisted `recent_window_end_ms`; tests for combine/intersect/persist count behavior on valid and skipped recent rows. Out — DB query row-handling tests already covered by T01, changing hook CLI output semantics beyond codifying current behavior, changing installed hook templates, broad dependency-injection refactors.
  - Done when: Tests would catch the prior timestamp/window bug class by failing if the query end bound and persisted window end diverge or if future rows can be included in the post-commit comparison; orchestration still reports deterministic loaded/skipped/intersection file counts.
  - Verification notes (commands or checks): Run the narrowest Nix-wrapped Rust test that covers hooks post-commit orchestration if practical, then `nix flake check`; review that new seams remain tightly scoped and do not expose unnecessary public API.
  - Completed: 2026-05-05
  - Files changed: `cli/src/services/hooks/mod.rs`
  - Evidence: Added private/internal `run_post_commit_intersection_flow_with` seam and focused regression test `post_commit_intersection_flow_uses_same_window_end_for_query_and_persistence`; direct targeted `cargo test` was blocked by the repository bash policy favoring `nix flake check`; `nix develop -c sh -c 'cd cli && cargo fmt'` passed; `nix flake check` passed after exercising the new test through `checks.x86_64-linux.cli-tests`.
  - Notes: Test injects deterministic `now_ms`, captures the queried `(cutoff_ms, end_ms)` window, captures persisted intersection metadata, and asserts loaded/skipped counts plus one intersected file without changing public hook CLI output semantics.

- [x] T03: `Repair stale Agent Trace docs and SQL comment` (status:done)
  - Task ID: T03
  - Goal: Sync current-state context and the migration query comment with the implemented bounded recent-diff-trace query and raw patch parsing behavior.
  - Boundaries (in/out of scope): In — `context/sce/agent-trace-db.md` signature/window/migration list repairs, `context/cli/patch-service.md` runtime wiring correction from JSON load helpers to `parse_patch`, and `cli/migrations/agent-trace/003_add_diff_traces_time_ms_id_index.sql` comment update to include `time_ms <= ?2`. Out — unrelated context churn, retroactive edits to completed plans unless necessary for active handoff accuracy, behavior changes, generated config changes.
  - Done when: The listed docs and SQL comment match code truth: two-argument recent query, lower+upper time bounds, migration `003`, and raw patch parsing via `parse_patch`.
  - Verification notes (commands or checks): Review the touched docs against `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/hooks/mod.rs`, and the migration; run `nix run .#pkl-check-generated` if generated-output parity confirmation is desired or if any generated-owned surface is touched.
  - Completed: 2026-05-05
  - Files changed: `context/sce/agent-trace-db.md`, `context/cli/patch-service.md`, `cli/migrations/agent-trace/003_add_diff_traces_time_ms_id_index.sql`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/context-map.md`, `context/glossary.md`
  - Evidence: Reviewed touched docs/comment against `cli/src/services/agent_trace_db/mod.rs` and `cli/src/services/hooks/mod.rs`; context-sync pass repaired directly related root-context drift for the same hook/DB/patch-service contracts; `nix run .#pkl-check-generated` passed with generated outputs up to date after implementation and after context sync; `nix flake check` passed.
  - Notes: Documentation now reflects the inclusive `recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` window, migration `003_add_diff_traces_time_ms_id_index.sql`, and raw `diff_traces.patch` parsing via `parse_patch`; SQL migration comment now includes the upper `time_ms <= ?2` bound; root context now no longer describes post-commit as a no-op or patch operations as unwired.

- [x] T04: `Final validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Run full validation, remove temporary scaffolding, and record evidence that the regression coverage and drift fixes satisfy this plan.
  - Boundaries (in/out of scope): In — full repo validation, generated-output parity check if applicable, cleanup of temporary test fixtures/scaffolding not meant to remain, plan evidence update, and final context accuracy verification. Out — new behavior changes beyond fixing validation failures in this plan's scope.
  - Done when: `nix flake check` passes; `nix run .#pkl-check-generated` passes if applicable or as final parity confirmation; no temporary artifacts remain; plan evidence maps each success criterion to tests, context/docs, or validation output.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; any targeted checks needed to verify fixes for failures found during final validation.
  - Completed: 2026-05-05
  - Files changed: `context/plans/post_commit_intersection_regression_and_context_drift.md`
  - Evidence: `nix run .#pkl-check-generated` passed with generated outputs up to date; `nix flake check` passed with all checks clean; `git status --short` was clean before the plan evidence update; reviewed `context/tmp/` and plan-related file matches and found no task-specific tracked temporary scaffolding requiring cleanup.
  - Success-criteria map: T01 covers bounded `AgentTraceDb::recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` filtering, ordering, raw patch parsing, malformed-row skip accounting, and loaded/skipped counts; T02 covers post-commit intersection window orchestration, combine/intersect/persist behavior, and loaded/skipped/file counts; T03 repairs current-state docs and the migration comment for the bounded query/raw patch parsing contract; T04 confirms final generated-output parity and full repo validation.
  - Notes: No behavior changes were needed during final validation; context-sync should treat this as a verify-only final plan evidence update unless it detects remaining current-state drift.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date.
- `nix flake check` -> exit 0; all checks passed (`cli-tests`, `cli-clippy`, `cli-fmt`, integration install checks, `pkl-parity`, npm JS checks, and config-lib JS checks evaluated for `x86_64-linux`).
- `git status --short` -> exit 0; working tree was clean before recording T04 evidence.

### Cleanup and context verification

- Reviewed `context/tmp/`; existing ignored hook runtime artifacts remain, but no task-specific tracked temporary scaffolding was introduced or required cleanup.
- Verified durable feature documentation is present and discoverable through `context/context-map.md` links to `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, and `context/cli/patch-service.md`.
- Context sync classification: verify-only for T04 because the task records final evidence and does not introduce new behavior, architecture, policy, or terminology. Root context files were read and left unchanged.

### Success-criteria verification

- [x] Bounded `AgentTraceDb::recent_diff_trace_patches(cutoff_time_ms, end_time_ms)` coverage: T01 evidence records direct regression coverage for lower/upper bounds, deterministic ordering, raw patch parsing, malformed-row skip accounting, and loaded/skipped counts.
- [x] Post-commit orchestration coverage: T02 evidence records direct coverage for shared query/persistence window end, combine/intersect/persist behavior, and loaded/skipped/file counts.
- [x] Context/docs and SQL comment drift repaired: T03 evidence records updates to `context/sce/agent-trace-db.md`, `context/cli/patch-service.md`, the migration comment, and related root context drift.
- [x] Existing hook behavior preserved: `nix flake check` passed after the regression tests and documentation repairs.
- [x] Full final validation passed: T04 evidence records clean generated-output parity and full flake validation.

### Failed checks and follow-ups

- None.

### Residual risks

- None identified.

## Open questions

None.
