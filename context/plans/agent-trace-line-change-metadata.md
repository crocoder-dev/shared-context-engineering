# Plan: agent-trace-line-change-metadata

## Change summary

Extend generated Agent Trace JSON with exact line-change attribution counts under `metadata.sce.line_changes`, so downstream analytics can compute AI/mixed/unknown changed LOC per workspace, repository, and time period without deriving it from Agent Trace range spans (which can include unchanged diff context). This extends existing SCE vendor metadata; it does not add a database column, table, sync stream, or change the Agent Trace spec version.

Counts are derived from `PatchHunk.lines` on the canonical `post_commit_patch` — the same source already used for hunk classification (`ai` / `mixed` / `unknown`) — so a touched line is counted exactly once, in the same bucket its hunk is classified into, with no independent second classification pass for the common case. `PatchHunk.lines` already excludes unchanged unified-diff context lines, so `added`/`removed` counts are exact without needing a zero-context diff.

## Acceptance criteria

- [ ] AC1: Every generated Agent Trace payload's `metadata.sce.line_changes` carries a stable `{ ai: {added, removed}, mixed: {added, removed}, unknown: {added, removed} }` shape, with all-zero counts when the trace has no touched lines.
  - Validate: `cli/src/services/agent_trace/tests.rs` unit test asserting the exact serialized field paths and a zero-touched-line case.
- [ ] AC2: Counts equal the exact number of `TouchedLineKind::Added`/`Removed` entries in canonical `post_commit_patch` hunks, with additions and removals tracked separately, never derived from `end_line - start_line + 1`.
  - Validate: focused unit tests covering an AI-only hunk (`+3 -1`), a replacement-style hunk with both added and removed lines, and multi-hunk/multi-classification totals.
- [ ] AC3: A hunk classified `mixed` contributes its *entire* canonical `post_commit_patch` touched-line count to `line_changes.mixed`, not just the touched lines that also appear in the AI intersection subset.
  - Validate: unit test where the intersection hunk's touched-line count is smaller than the post-commit hunk's, asserting the full post-commit count is recorded.
- [ ] AC4: A hunk classified `unknown` contributes all of its touched lines to `line_changes.unknown`.
  - Validate: unit test with a post-commit hunk absent from the intersection patch.
- [ ] AC5: The deleted-`.patch` embedded-expansion path never double-counts and `line_changes` reflects the literal canonical commit content (the deleted file's own removed lines), not the reconstructed content described inside the deleted patch artifact.
  - Validate: unit/golden test built on the existing `mixed_change_reconstruction` fixture (which already deletes a `.patch`-extension file), asserting the embedded reconstructed hunks are excluded from `line_changes` and the literal deleted-file hunk is counted exactly once.
- [ ] AC6: Agent Trace JSON produced before this change (containing `metadata.sce.version` but no `line_changes`) still deserializes successfully, with `line_changes` defaulting to all-zero counts.
  - Validate: unit test deserializing a literal legacy payload.
- [ ] AC7: The enriched payload still validates against the embedded Agent Trace schema, and the top-level Agent Trace `version` (`AGENT_TRACE_VERSION`) is unchanged.
  - Validate: `validate_agent_trace_value(...)` called on a built payload in tests; code review confirms `AGENT_TRACE_VERSION` is untouched.
- [ ] AC8: Golden fixtures carry the new `metadata.sce.line_changes` shape with correct computed values, and the test harness compares full `metadata` (or at minimum full `line_changes`) against fixture truth instead of only checking `version` is non-empty.
  - Validate: updated `cli/src/services/agent_trace/fixtures/**/golden.json`; strengthened assertion in `assert_builds_expected_agent_trace`.
- [ ] AC9: Current-state context documents the new contract: source (`PatchHunk.lines` on canonical `post_commit_patch`), hunk-level classification, full-hunk `mixed` counting, `unknown` meaning "unattributed" rather than "human", and that ratios are a downstream concern.
  - Validate: `context/sce/agent-trace-minimal-generator.md` (and reviewed sibling docs) checked against code truth.

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`
- `git diff --check`

### Context sync

- `context/sce/agent-trace-minimal-generator.md`
- `context/sce/agent-trace-db.md`
- `context/sce/agent-trace-hooks-command-routing.md`
- `context/context-map.md`
- `context/overview.md`
- `context/glossary.md`

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/agent_trace.rs` (new `LineChangeCounts`/`LineChangeAttribution` types, `AgentTraceSceMetadata.line_changes`, counting helper, `build_agent_trace` wiring including the deleted-`.patch` branch), `cli/src/services/agent_trace/tests.rs` (harness strengthening plus new focused tests), `cli/src/services/agent_trace/fixtures/**/golden.json` (add `line_changes` to all seven goldens), and the current-state context files listed under Context sync.
- **Out of scope:** `config/schema/agent-trace.schema.json` (its `metadata` field is already an open, unconstrained object — no schema change is required), any Agent Trace DB migration or new table/column, any new sync stream, DTO, or control-plane contract change (`agent_traces.trace_json` already transports raw/unmodified per `AgentTraceExportReader`), hook command routing changes, and ratio/percentage computation in SCE.
- **Constraints:** preserve `AGENT_TRACE_VERSION` unchanged; preserve backward deserialization compatibility via `#[serde(default)]` on `line_changes` (and its parent) so pre-existing `metadata.sce.version`-only payloads still deserialize; use `u64` counters incremented per touched line rather than unchecked `usize` casts; for every file where a `post_commit_patch` hunk already produces a `Conversation`, the recorded `line_changes` classification must be read from that same already-computed `Conversation.contributor.kind` rather than re-calling `classify_hunk` — no second independent classification pass for the common path.
- **Non-goal:** decomposing a `mixed` hunk into separate AI/human line counts; computing `ai_ratio`/`mixed_ratio`/`rest_ratio` in SCE; renaming or reinterpreting `unknown` as "human".

## Assumptions

- The deleted-`.patch` embedded-expansion edge case is resolved by classifying the deleted file's own literal `post_commit_patch` hunks against the top-level `intersection_patch` (the same file-lookup convention `build_trace_file` already uses internally) and recording those counts toward `line_changes`, while the embedded reconstructed hunks used to synthesize `Conversation` entries for that branch are never counted toward `line_changes`. This is a deliberate, documented exception to the "reuse the same decision" constraint above: the literal deleted-file hunks currently receive no `Conversation` at all in this branch, so there is no existing decision to reuse, and classifying them is a new (not duplicated) decision. This follows the change request's own "likely safe option" guidance and keeps `line_changes` describing actual canonical committed changes rather than the embedded patch's logical content.
- The existing `mixed_change_reconstruction` golden fixture already deletes a `.patch`-extension file (`cli/src/services/patch/fixtures/poem_edit_reconstruction/incremental_01.patch`) whose embedded content currently produces no `Conversation` entries in the golden output. This fixture is reused to cover the deleted-`.patch` edge case in AC5 instead of adding a new fixture suite.
- `LineChangeCounts`/`LineChangeAttribution` names and shape follow the change request's suggested types verbatim; no stronger existing repository naming convention was found for this concept.

## Task stack

- [x] T01: `Add line-change attribution counting to the Agent Trace generator` (status:done)
  - Task ID: T01
  - Scope: In — `cli/src/services/agent_trace.rs` (`LineChangeCounts`, `LineChangeAttribution`, `AgentTraceSceMetadata.line_changes` with `#[serde(default)]`, `record_hunk_line_changes` helper, threading a `LineChangeAttribution` accumulator through `build_agent_trace`'s normal per-file/per-hunk path by reading the already-computed `Conversation.contributor.kind`, and the deleted-`.patch` branch's separate literal-hunk classification per the Assumptions section); `cli/src/services/agent_trace/tests.rs` (all ten focused test scenarios from the change request: AI-only hunk, unknown-only hunk, mixed hunk counting the full canonical hunk, additions/removals tracked separately, multiple hunks with different classifications, multiple files aggregated exactly once, legacy-JSON backward-compatible deserialization, exact serialization-contract assertion, schema validation of the enriched payload, and the deleted-`.patch` double-counting case); `cli/src/services/agent_trace/fixtures/**/golden.json` (add computed `metadata.sce.line_changes` to all seven goldens) and strengthening `assert_builds_expected_agent_trace` to compare full `metadata`/`line_changes` against fixture truth. Out — schema file changes, DB/migration changes, sync/export changes, hook command routing, documentation.
  - Dependencies: none
  - Done when: AC1–AC8 all hold; `AGENT_TRACE_VERSION` is unchanged; no touched line is ever counted twice (including across the deleted-`.patch` branch); `cargo`/flake test coverage for `cli/src/services/agent_trace` passes.
  - Verify: repo-preferred `nix flake check` (targeted `cargo test agent_trace` may be attempted first but is expected to be blocked by the `use-nix-flake-check-over-cargo-test` bash policy per prior precedent in `context/plans/agent-trace-sce-metadata.md`); `nix run .#pkl-check-generated`.
  - Completed: 2026-08-20
  - Files changed: `cli/src/services/agent_trace.rs`; `cli/src/services/agent_trace/tests.rs`; `cli/src/services/agent_trace/fixtures/average_age_reconstruction/golden.json`; `cli/src/services/agent_trace/fixtures/file_rename_reconstruction/golden.json`; `cli/src/services/agent_trace/fixtures/hello_world_reconstruction/golden.json`; `cli/src/services/agent_trace/fixtures/mixed_change_reconstruction/golden.json`; `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/golden.json`; `cli/src/services/agent_trace/fixtures/poem_write_reconstruction/golden.json`; `cli/src/services/agent_trace/fixtures/text_file_lifecycle_reconstruction/golden.json`
  - Result: Added `LineChangeCounts`/`LineChangeAttribution` types and `AgentTraceSceMetadata.line_changes` (`#[serde(default)]`, all-zero default). Added `record_hunk_line_changes`, threaded a `LineChangeAttribution` accumulator through `build_trace_file`/`build_agent_trace`'s normal per-file/per-hunk path, reusing the already-computed `contributor_kind` (no second classification pass). Added a separate literal-hunk classification pass for the deleted-`.patch` branch, classifying `post_commit_file`'s own hunks against `intersection_patch` by `old_path` (not `new_path`, which is always empty for deleted files and would otherwise collide across multiple deleted files in the same patch, as the `mixed_change_reconstruction` fixture — which deletes both `.version` and the `.patch`-extension file in the same commit — exposed); embedded/reconstructed hunks from the deleted-`.patch` expansion are never counted. Computed and wrote exact `line_changes` values into all seven `golden.json` fixtures by hand from each fixture's `post_commit.patch` content and golden's existing per-hunk classifications, then verified them against `build_agent_trace`'s actual output. Strengthened `assert_builds_expected_agent_trace` to assert `actual_json["metadata"]["sce"]["line_changes"] == golden[...]["line_changes"]` (AC8).
  - Deviation: The plan called for ten new focused `#[test]` functions in `tests.rs` (AI-only hunk, unknown-only hunk, mixed-hunk-full-count, additions/removals-separate, multi-hunk/multi-classification, multi-file aggregation, deleted-`.patch` double-counting, legacy deserialization, exact serialized field paths, schema validation of an enriched payload) to serve as the named `Validate` method for AC1 (exact field paths + zero case), AC2 (AI-only/replacement/multi-hunk), AC3 (mixed-hunk-full-count vs. AI-intersection subset), AC4 (unknown-hunk-absent-from-intersection), and AC6 (legacy deserialization). All ten were written and passed (confirmed via `nix build .#checks.x86_64-linux.cli-tests`) before the user explicitly instructed their removal mid-task; after a clarifying confirmation (removal scope), they were deleted, keeping only the strengthened golden-fixture assertion (AC8) and the `golden.json` updates. As a result, AC1 (zero-case only, via `file_rename_reconstruction`'s all-zero golden), AC2, AC3, AC4, and AC6 no longer have the dedicated unit-test coverage the plan specified as their validation method — the underlying behavior was verified correct via those tests before deletion and remains implemented, but is no longer protected by a committed regression test for those specific scenarios (AC5 remains covered via `mixed_change_reconstruction_matches_golden_agent_trace`, and AC7 via the existing schema-validation assertions already in `assert_builds_expected_agent_trace`/`poem_edit_reconstruction_maps_each_hunk_to_one_range`). If regression protection for these scenarios matters later, dedicated tests should be reintroduced.
  - Verify: `nix flake check` (x86_64-linux) — all checks passed, including `cli-tests` (376 passed, 0 failed), `cli-clippy`, `cli-fmt`; `nix run .#pkl-check-generated` — no drift (107 files); `git diff --check` — no whitespace issues.
  - Context impact: Extends an existing, previously-documented SCE vendor metadata contract (`metadata.sce`) with a new field; no schema, DB, or sync-stream change. Context sync required per plan (T02) for `context/sce/agent-trace-minimal-generator.md` and sibling docs.
  - Context synchronization: synced

- [ ] T02: `Sync Agent Trace context documentation for line-change attribution metadata` (status:todo)
  - Task ID: T02
  - Scope: In — `context/sce/agent-trace-minimal-generator.md` (primary contract update: new `metadata.sce.line_changes` shape, source is canonical `post_commit_patch` `PatchHunk.lines`, additions/removals excluding unchanged context, hunk-level classification, full-hunk `mixed` counting, `unknown` meaning unattributed rather than proven-human, `changed = added + removed` as a downstream calculation, ratios as a downstream concern); reviewing and updating only if materially affected: `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/context-map.md`, `context/overview.md`, `context/glossary.md`. Out — historical/removed-feature Agent Trace docs, the plan file itself, unrelated documentation churn.
  - Dependencies: T01
  - Done when: `context/sce/agent-trace-minimal-generator.md` accurately states the `line_changes` contract per AC9; reviewed sibling docs are either updated or confirmed unaffected; no root-context edit is made unless code truth requires it.
  - Verify: manual review of updated context against `cli/src/services/agent_trace.rs` code truth; `git diff --check`.
  - Context synchronization: pending

## Open questions

None. The change request is unusually detailed and explicitly resolves the one design tension it flags (deleted-`.patch` embedded expansion) with a stated fallback ("a likely safe option is..."), which this plan adopts and records under Assumptions.
