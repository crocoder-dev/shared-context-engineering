# Plan: agent-trace-line-change-metadata

## Change summary

Extend generated Agent Trace JSON with exact line-change attribution counts under `metadata.sce.line_changes`, so downstream analytics can compute AI/mixed/unknown changed LOC per workspace, repository, and time period without deriving it from Agent Trace range spans (which can include unchanged diff context). This extends existing SCE vendor metadata; it does not add a database column, table, sync stream, or change the Agent Trace spec version.

Counts are derived from `PatchHunk.lines` on the canonical `post_commit_patch` — the same source already used for hunk classification (`ai` / `mixed` / `unknown`) — so a touched line is counted exactly once, in the same bucket its hunk is classified into, with no independent second classification pass for the common case. `PatchHunk.lines` already excludes unchanged unified-diff context lines, so `added`/`removed` counts are exact without needing a zero-context diff.

## Acceptance criteria

- [x] AC1: Every generated Agent Trace payload's `metadata.sce.line_changes` carries a stable `{ ai: {added, removed}, mixed: {added, removed}, unknown: {added, removed} }` shape, with all-zero counts when the trace has no touched lines.
  - Validate: `cli/src/services/agent_trace/tests.rs` unit test asserting the exact serialized field paths and a zero-touched-line case.
- [x] AC2: Counts equal the exact number of `TouchedLineKind::Added`/`Removed` entries in canonical `post_commit_patch` hunks, with additions and removals tracked separately, never derived from `end_line - start_line + 1`.
  - Validate: focused unit tests covering an AI-only hunk (`+3 -1`), a replacement-style hunk with both added and removed lines, and multi-hunk/multi-classification totals.
- [x] AC3: A hunk classified `mixed` contributes its *entire* canonical `post_commit_patch` touched-line count to `line_changes.mixed`, not just the touched lines that also appear in the AI intersection subset.
  - Validate: unit test where the intersection hunk's touched-line count is smaller than the post-commit hunk's, asserting the full post-commit count is recorded.
- [x] AC4: A hunk classified `unknown` contributes all of its touched lines to `line_changes.unknown`.
  - Validate: unit test with a post-commit hunk absent from the intersection patch.
- [x] AC5: The deleted-`.patch` embedded-expansion path never double-counts and `line_changes` reflects the literal canonical commit content (the deleted file's own removed lines), not the reconstructed content described inside the deleted patch artifact.
  - Validate: unit/golden test built on the existing `mixed_change_reconstruction` fixture (which already deletes a `.patch`-extension file), asserting the embedded reconstructed hunks are excluded from `line_changes` and the literal deleted-file hunk is counted exactly once.
- [x] AC6: Agent Trace JSON produced before this change (containing `metadata.sce.version` but no `line_changes`) still deserializes successfully, with `line_changes` defaulting to all-zero counts.
  - Validate: unit test deserializing a literal legacy payload. **[Deviation: the dedicated regression test was written, passed, then deliberately deleted mid-T01 at the user's explicit instruction (see T01 Deviation note); the user confirmed on 2026-08-20 that this AC's dedicated-test requirement is intentionally waived, not an oversight. Verified instead by code inspection: `AgentTraceSceMetadata.line_changes` carries `#[serde(default)]` (`cli/src/services/agent_trace.rs:128`), and `LineChangeAttribution`'s `ai`/`mixed`/`unknown` fields each carry `#[serde(default)]` with `LineChangeCounts`/`LineChangeAttribution` both deriving `Default` (`agent_trace.rs:133-150`) — a JSON object missing `line_changes` deserializes it as `LineChangeAttribution::default()` (all-zero) by serde's standard `#[serde(default)]` semantics. No regression test protects this; see Residual risks in the Validation Report.]**
- [x] AC7: The enriched payload still validates against the embedded Agent Trace schema, and the top-level Agent Trace `version` (`AGENT_TRACE_VERSION`) is unchanged.
  - Validate: `validate_agent_trace_value(...)` called on a built payload in tests; code review confirms `AGENT_TRACE_VERSION` is untouched.
- [x] AC8: Golden fixtures carry the new `metadata.sce.line_changes` shape with correct computed values, and the test harness compares full `metadata` (or at minimum full `line_changes`) against fixture truth instead of only checking `version` is non-empty.
  - Validate: updated `cli/src/services/agent_trace/fixtures/**/golden.json`; strengthened assertion in `assert_builds_expected_agent_trace`.
- [x] AC9: Current-state context documents the new contract: source (`PatchHunk.lines` on canonical `post_commit_patch`), hunk-level classification, full-hunk `mixed` counting, `unknown` meaning "unattributed" rather than "human", and that ratios are a downstream concern.
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

- [x] T02: `Sync Agent Trace context documentation for line-change attribution metadata` (status:done)
  - Task ID: T02
  - Scope: In — `context/sce/agent-trace-minimal-generator.md` (primary contract update: new `metadata.sce.line_changes` shape, source is canonical `post_commit_patch` `PatchHunk.lines`, additions/removals excluding unchanged context, hunk-level classification, full-hunk `mixed` counting, `unknown` meaning unattributed rather than proven-human, `changed = added + removed` as a downstream calculation, ratios as a downstream concern); reviewing and updating only if materially affected: `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/context-map.md`, `context/overview.md`, `context/glossary.md`. Out — historical/removed-feature Agent Trace docs, the plan file itself, unrelated documentation churn.
  - Dependencies: T01
  - Done when: `context/sce/agent-trace-minimal-generator.md` accurately states the `line_changes` contract per AC9; reviewed sibling docs are either updated or confirmed unaffected; no root-context edit is made unless code truth requires it.
  - Verify: manual review of updated context against `cli/src/services/agent_trace.rs` code truth; `git diff --check`.
  - Completed: 2026-08-20
  - Files changed: `context/overview.md`
  - Result: `context/sce/agent-trace-minimal-generator.md`'s contract section, domain-types table, payload-shape narrative, and JSON example were verified line-by-line against `cli/src/services/agent_trace.rs` (`AgentTraceSceMetadata`, `LineChangeCounts`, `LineChangeAttribution`, `record_hunk_line_changes`, `build_agent_trace`'s per-file/per-hunk accumulation and the deleted-`.patch` literal-hunk `old_path` classification branch) — already accurate, no edit needed. `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/context-map.md`, and `context/glossary.md` were reviewed and found already updated for `line_changes` (bundled into T01's own context-synchronization pass, commit `a0a7ed4c`) — confirmed accurate, no further edit needed. `context/overview.md` was found materially affected: its post-commit hook description enumerates the same persisted `metadata.sce` payload fields at the same granularity as `agent-trace-db.md` (`metadata.sce.version`, range `content_hash`) but omitted `line_changes`; added "always-emitted `metadata.sce.line_changes` touched-line attribution counts" to that sentence, matching existing phrasing style.
  - Verify: `git diff --check` — no whitespace issues. Manual review against `cli/src/services/agent_trace.rs` code truth performed as described above.
  - Context impact: Documentation-only change completing the context sync for T01's `line_changes` addition; no code, schema, or contract change. Five-root-file pass required per workflow.
  - Context synchronization: synced

## Open questions

None. The change request is unusually detailed and explicitly resolves the one design tension it flags (deleted-`.patch` embedded expansion) with a stated fallback ("a likely safe option is..."), which this plan adopts and records under Assumptions.

## Validation Report

**Status:** validated  
**Date:** 2026-08-20

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (107 files, no drift; inventory sha256 8500d6e4d8cbbe7ae540c52254a0b35b6e48834956823eeaf05e8af347d68bdb)
- `nix flake check` -> exit 0 (all checks passed, including `checks.x86_64-linux.cli-tests`, `cli-clippy`, `cli-fmt`)
- `git diff --check` -> exit 0 (no whitespace issues)

### Success-criteria verification

- [x] AC1: stable `{ai,mixed,unknown}` shape with zero-touched-line case -> `assert_builds_expected_agent_trace` (`cli/src/services/agent_trace/tests.rs:113-116`) asserts `actual_json["metadata"]["sce"]["line_changes"] == golden[...]["line_changes"]` exactly for all 7 fixtures; `file_rename_reconstruction/golden.json` is the all-zero case; every built payload is schema-validated (`validate_agent_trace_value`, tests.rs:67,90).
- [x] AC2: exact per-line `TouchedLineKind::Added`/`Removed` counts, added/removed tracked separately -> same golden comparison across 7 fixtures with independently hand-computed values spanning asymmetric ratios (e.g. `average_age_reconstruction` ai `{91,9}`, `mixed_change_reconstruction` unknown `{1,15}`); `record_hunk_line_changes` (`agent_trace.rs:156-173`) increments a `u64` counter per matched `TouchedLineKind`, never `end_line - start_line + 1`.
- [x] AC3: `mixed` hunk records its full post-commit touched-line count, not the smaller AI-intersection subset -> `poem_edit_reconstruction`'s first hunk has 3 touched lines in `post_commit.patch` but only 1 overlaps `incremental_01.patch`'s AI intersection; golden `mixed: {added:3, removed:3}` records the full hunk; verified by `poem_edit_reconstruction_matches_golden_agent_trace`.
- [x] AC4: `unknown` hunk records all touched lines -> same fixture's second hunk (`old_start=10`, "loops"→"lowops") has no corresponding hunk in either incremental patch; golden `unknown: {added:1, removed:1}` records it, verified by the same test.
- [x] AC5: deleted-`.patch` branch never double-counts, records literal canonical content only -> `mixed_change_reconstruction_matches_golden_agent_trace` covers the fixture deleting a `.patch`-extension file; `agent_trace.rs:596-627` routes the embedded reconstructed hunks through a separate `discarded_line_changes` accumulator that is never merged into `line_changes`, while the literal deleted-file hunks are classified by `old_path` against the top-level `intersection_patch` and recorded once.
- [x] AC6: legacy payload (`metadata.sce.version` only, no `line_changes`) still deserializes with all-zero default -> the dedicated unit test named by this AC's `Validate:` line was intentionally not reintroduced, per the user's explicit 2026-08-20 confirmation that its removal in T01 was a deliberate instruction, not an oversight. Verified instead by code inspection, authorized by the plan owner: `AgentTraceSceMetadata.line_changes` (`agent_trace.rs:128`) and `LineChangeAttribution`'s `ai`/`mixed`/`unknown` fields (`agent_trace.rs:144,146,148`) all carry `#[serde(default)]`, with `LineChangeCounts`/`LineChangeAttribution` both deriving `Default` (`agent_trace.rs:133-150`); by serde's standard semantics a JSON object missing `line_changes` deserializes it as `LineChangeAttribution::default()` (all-zero).
- [x] AC7: enriched payload validates against schema; `AGENT_TRACE_VERSION` unchanged -> `validate_agent_trace_value` called on every built payload (tests.rs:67,90) plus dedicated schema tests; `AGENT_TRACE_VERSION = "0.1.0"` (`agent_trace.rs:31`) is outside this change's diff scope.
- [x] AC8: golden fixtures carry `line_changes`; harness compares full `line_changes` against fixture truth -> all 7 `golden.json` fixtures updated; `assert_builds_expected_agent_trace` asserts exact equality (tests.rs:113-116).
- [x] AC9: context documents source, hunk-level classification, full-hunk `mixed` counting, `unknown` as unattributed, backward-compat default -> `context/sce/agent-trace-minimal-generator.md:32,34,50,101-102` states the contract; reviewed against code truth in T02; `context/overview.md` updated to mention `line_changes`.

### Failed checks and follow-ups

- None.

### Residual risks

- AC2–AC4 are currently verified only through golden-fixture integration tests (`poem_edit_reconstruction`, `average_age_reconstruction`, `mixed_change_reconstruction`, etc.) rather than the plan's originally-specified isolated unit tests; the T01 Deviation note flags this as a known, intentional coverage reduction. The fixture coverage happens to exercise the exact scenarios these ACs describe, but a future fixture edit could silently narrow that coverage without a dedicated test to catch it.
- AC6 has no regression test: backward-compatible deserialization of legacy `line_changes`-absent payloads is protected only by the `#[serde(default)]` attribute remaining in place, with no test to catch its accidental removal. Per the user's 2026-08-20 confirmation, this is an accepted, intentional gap, not a defect requiring repair.
