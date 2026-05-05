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

- [ ] T02: `Cover post-commit intersection window orchestration` (status:todo)
  - Task ID: T02
  - Goal: Add direct regression coverage or a minimal injectable seam for `run_post_commit_intersection_flow` so the post-commit window and persistence orchestration are tested directly.
  - Boundaries (in/out of scope): In — private/internal test seam if needed for current time, git patch capture, recent patch query, and persistence; tests that assert the same `now_ms` value is used as the query end bound and persisted `recent_window_end_ms`; tests for combine/intersect/persist count behavior on valid and skipped recent rows. Out — DB query row-handling tests already covered by T01, changing hook CLI output semantics beyond codifying current behavior, changing installed hook templates, broad dependency-injection refactors.
  - Done when: Tests would catch the prior timestamp/window bug class by failing if the query end bound and persisted window end diverge or if future rows can be included in the post-commit comparison; orchestration still reports deterministic loaded/skipped/intersection file counts.
  - Verification notes (commands or checks): Run the narrowest Nix-wrapped Rust test that covers hooks post-commit orchestration if practical, then `nix flake check`; review that new seams remain tightly scoped and do not expose unnecessary public API.

- [ ] T03: `Repair stale Agent Trace docs and SQL comment` (status:todo)
  - Task ID: T03
  - Goal: Sync current-state context and the migration query comment with the implemented bounded recent-diff-trace query and raw patch parsing behavior.
  - Boundaries (in/out of scope): In — `context/sce/agent-trace-db.md` signature/window/migration list repairs, `context/cli/patch-service.md` runtime wiring correction from JSON load helpers to `parse_patch`, and `cli/migrations/agent-trace/003_add_diff_traces_time_ms_id_index.sql` comment update to include `time_ms <= ?2`. Out — unrelated context churn, retroactive edits to completed plans unless necessary for active handoff accuracy, behavior changes, generated config changes.
  - Done when: The listed docs and SQL comment match code truth: two-argument recent query, lower+upper time bounds, migration `003`, and raw patch parsing via `parse_patch`.
  - Verification notes (commands or checks): Review the touched docs against `cli/src/services/agent_trace_db/mod.rs`, `cli/src/services/hooks/mod.rs`, and the migration; run `nix run .#pkl-check-generated` if generated-output parity confirmation is desired or if any generated-owned surface is touched.

- [ ] T04: `Final validation and cleanup` (status:todo)
  - Task ID: T04
  - Goal: Run full validation, remove temporary scaffolding, and record evidence that the regression coverage and drift fixes satisfy this plan.
  - Boundaries (in/out of scope): In — full repo validation, generated-output parity check if applicable, cleanup of temporary test fixtures/scaffolding not meant to remain, plan evidence update, and final context accuracy verification. Out — new behavior changes beyond fixing validation failures in this plan's scope.
  - Done when: `nix flake check` passes; `nix run .#pkl-check-generated` passes if applicable or as final parity confirmation; no temporary artifacts remain; plan evidence maps each success criterion to tests, context/docs, or validation output.
  - Verification notes (commands or checks): `nix flake check`; `nix run .#pkl-check-generated`; any targeted checks needed to verify fixes for failures found during final validation.

## Open questions

None.
