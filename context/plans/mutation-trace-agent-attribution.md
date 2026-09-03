# Plan: mutation-trace-agent-attribution

## Change summary

Extend the existing post-commit Agent Trace pipeline so observed mutation-cursor history can attribute committed touched lines that direct `diff_traces` evidence does not cover. Direct evidence remains independently sufficient and is resolved first. Mutation history is a bounded, read-only secondary source: inspect at most the newest 128 events for the current worktree, newest first, and let the first safely matching event resolve each remaining line. Only a healthy, untainted `AiExclusive(scope)` match contributes AI coverage; a safely matching contended, unscoped, unhealthy, or tainted event blocks older evidence for that line without contributing AI coverage.

This extends the implementation on PR #258 / `mutation-scope-runtime-integration`. It adds no harness adapter, mutation-protocol change, database migration, checkpoint state, timestamp window, or Agent Trace schema change. Raw evidence remains separate: mutation evidence stays in `mutation_trace_*`, direct evidence stays in `diff_traces`, `post_commit_patch_intersections` keeps its current direct-only meaning, and only the final combined Agent Trace is persisted in `agent_traces.trace_json`.

## Attribution contract

For every touched line in the canonical post-commit patch:

1. Resolve direct evidence with the existing direct `intersect_patches` behavior. A direct match is AI, retains its current provenance, is removed from the unresolved set, and is never consulted against mutation history.
2. For unresolved lines only, inspect mutation events for the current `WorktreeId` in descending revision order. Revision, not `created_at`, defines order. Event 128 may contribute; event 129 and older must not be inspected.
3. Within one reconstructed mutation patch, determine safe line matches in two passes. A file's logical path is `new_path` when non-empty and otherwise `old_path`; exact logical-path pairing wins, while normalized suffix equivalence is allowed only when it identifies exactly one mutation file and one unresolved target file.
   - Exact line pass: within a safely paired file, pair equal `kind`, `line_number`, and `content` one-to-one.
   - Historical fallback pass: among lines left unmatched by the exact pass, pair equal `kind` and `content` only when exactly one remaining mutation candidate and exactly one remaining unresolved target line share that key in the paired file.
   - Ambiguous repeated-content or file matches are not matches and leave the target unresolved. False negatives are acceptable. The existing permissive direct-evidence fallback is unchanged.
4. The first safely matching event resolves a line. If the event is untainted, has `FailureKind::Healthy`, and has `Attribution::AiExclusive(_)`, add the line to mutation-derived AI coverage. For `AiContended`, `IneligibleUnscoped`, unhealthy, or tainted events, record the line as resolved/non-AI. In either case remove it from the unresolved set so older events cannot reclaim it.
5. A mutation page query/decode failure or an inspected event's tree-diff/patch-parse failure is a barrier. Preserve direct evidence and results from newer successfully reconstructed events, inspect no older event, and leave all remaining lines unresolved/unknown.
6. Fetch descending mutation rows with `requested_limit = min(MUTATION_ATTRIBUTION_PAGE_SIZE, MAX_MUTATION_ATTRIBUTION_EVENTS - inspected_events)`. Every request is therefore capped by both the 32-row page size and the remaining 128-event budget; the reader never requests rows beyond that budget, even if the constants are no longer exact multiples (for example, page size 32 and horizon 130 yields requests of 32, 32, 32, 32, then 2). `inspected_events` increments when the consumer begins reconstructing an event, including an event whose reconstruction fails. Rows returned by SQLite and events reconstructed with Git are distinct counts.
7. Stop reconstructing immediately when the unresolved set becomes empty. If this happens at event 4, events 5–32 may already be loaded in the current page but are not reconstructed or diffed, and no next DB page is requested.
8. Final hunk classification uses the union of direct and mutation-derived AI coverage: all touched lines covered is `ai`, a non-empty proper subset is `mixed`, and no AI-covered line is `unknown`. Resolved/non-AI and still-unresolved lines are both non-AI coverage for this classification.
9. Mutation-only AI coverage carries no model, session, tool, or tool-version provenance. `ScopeId`, `ActorKind`, and mutation attribution fields are never translated into direct provenance.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: Existing direct `diff_trace` attribution and matching are unchanged, direct matches are resolved before mutation history, and mutation state cannot revoke direct AI evidence.
  - Validate: existing Agent Trace/direct patch behavior tests remain green; table-driven resolver tests prove directly covered lines are absent from mutation matching; the `direct_only` golden remains schema-valid and semantically unchanged.
- [ ] AC2: For a line without direct coverage, the newest safely matching healthy, untainted `AiExclusive` event within the 128-event horizon is sufficient AI evidence, including for a delayed commit while the event remains in-horizon.
  - Validate: pure resolver tests cover exclusive and delayed-event cases; the `exclusive_without_direct` golden and real Git/DB Bash-style mutation regression produce `ai`.
- [ ] AC3: The newest safely matching mutation wins: contended, unscoped, unhealthy, or tainted matches resolve a line as non-mutation-AI and prevent every older event from claiming it.
  - Validate: table-driven resolver tests cover every non-positive state, and the `newer_nonexclusive_blocks` golden plus real overwrite regression remain non-AI.
- [ ] AC4: Mutation matching prefers exact file/kind/line/content identity and permits kind/content fallback only for a unique remaining candidate in both the mutation patch and unresolved target; ambiguous repeated-content or path matches never contribute AI evidence.
  - Validate: focused pure tests cover exact matching, unique fallback, duplicate mutation candidates, duplicate unresolved targets, and ambiguous logical-file pairing; a repeated-identical-line regression remains unresolved/unknown.
- [ ] AC5: Traversal is current-worktree-only, revision-descending, and timestamp-independent; every request is capped by both the 32-row page size and the remaining 128-event budget, so total loaded/inspected events never exceed 128, event 128 may contribute, and event 129 is never loaded or inspected. The current constants imply at most four pages but do not require the horizon to be divisible by page size.
  - Validate: store tests order revisions `1`, `255`, `256`, and `u64::MAX`; counting consumer tests assert every requested row count is `min(page size, remaining budget)`, no more than four pages under 32/128, at most 128 loaded/inspected events and tree diffs, event 128 eligibility, and event 129 exclusion; query inspection shows exact `worktree_id`, exclusive revision cursor, `ORDER BY revision DESC`, capped limit, and no `created_at` predicate.
- [ ] AC6: Early termination distinguishes already-loaded rows from reconstruction work: after all lines resolve at event 4, events 5–32 in that page are not reconstructed/diffed and no second page is requested.
  - Validate: a counting reader/reconstructor test asserts one page loaded, four events inspected/diffed, zero reconstruction for rows 5–32, and zero next-page request.
- [ ] AC7: Any page query/decode or inspected-event diff/parse failure is a barrier that retains direct/newer results, performs no older reconstruction or page request, and leaves remaining lines unknown.
  - Validate: injected failure tests assert the resolved/AI/non-AI/unresolved sets and separate SQLite-row/page versus Git-reconstruction counters at the barrier.
- [ ] AC8: Mutation lookup uses only the invoking linked worktree's existing identity; absent identity or absent mutation history cleanly falls back to direct-only behavior, and attribution lookup itself never creates identity or mutation state.
  - Validate: linked-worktree tests exclude foreign rows; focused read-only identity tests leave an absent `<git-dir>/sce/checkout-id` absent and preserve direct-only output.
- [ ] AC9: Combined Agent Trace construction uses separate direct and mutation AI patches; mutation-only attribution fabricates no model/session/tool metadata, while direct provenance remains available when direct evidence covers part or all of a hunk.
  - Validate: complete `mutation_only_no_provenance`, `direct_plus_mutation`, and `partial_combined` goldens validate expected and actual JSON against the embedded schema and assert provenance fields exactly.
- [ ] AC10: Raw and final persistence boundaries remain unchanged: mutation evidence is never inserted into `diff_traces`, `post_commit_patch_intersections` remains the direct-only intersection, no schema/migration/checkpoint is added, and the schema-valid combined result is stored in `agent_traces.trace_json`.
  - Validate: real DB tests inspect all three tables after post-commit; `git diff --name-only origin/mutation-scope-runtime-integration...HEAD -- cli/migrations` is empty.
- [ ] AC11: Real Git/DB post-commit flows prove mutation-only AI attribution, direct-plus-mutation completion, newer non-exclusive blocking, linked-worktree isolation, and the 128/129 boundary without changing checkpoint or auto-sync ordering.
  - Validate: focused post-commit integration tests read and schema-validate persisted `agent_traces.trace_json` and inspect direct-only/raw evidence tables; T03's separate counting tests prove traversal work that is not observable from persisted output.

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix run .#pkl-check-generated`
- `nix flake check`
- Inspect the PR diff against `origin/mutation-scope-runtime-integration` and prove: event 128 is inspected and eligible; event 129 is neither loaded nor inspected; resolving at event 4 leaves already-loaded rows 5–32 unreconstructed and requests no next page; mutation attribution has no `created_at` query; and `cli/migrations/` is unchanged.

### Context sync

- Add `context/cli/mutation-trace-agent-attribution.md` with the direct-first, safe-match, current-worktree-only, newest-match-wins, failure-barrier, and 128-event bounded-history invariant.
- Update `context/cli/mutation-trace-store.md` for the descending cursor-paged cold reader and its 8-byte revision ordering contract.
- Update `context/cli/mutation-trace-snapshot-service.md` because the attribution-history consumer becomes a read-only `GitSnapshotService::diff_trees` caller; keep capture/pin ownership with the coordinator and update only the caller inventory needed for this change.
- Update `context/cli/patch-service.md` to distinguish unchanged direct matching from the stricter unique mutation-evidence matcher.
- Update `context/sce/agent-trace-minimal-generator.md` for separated direct/mutation evidence, combined coverage, and direct-only provenance.
- Update `context/sce/agent-trace-hooks-command-routing.md` for bounded post-commit mutation lookup, unchanged direct intersection persistence, and final combined persistence.
- Update `context/context-map.md` and `context/overview.md` to index and summarize the new behavior.
- Inspect `context/architecture.md`, `context/patterns.md`, and `context/glossary.md` during the mandatory root context pass; update only where the shipped architecture or canonical terminology materially changes.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** a pure safe mutation-attribution resolver over reconstructed `ParsedPatch` values; a worktree-scoped descending mutation-event reader; a bounded read-only history consumer using the existing tree-diff command; a separated direct/mutation Agent Trace evidence builder; post-commit composition; compact semantic goldens; focused traversal/counting tests; real Git/DB regressions; and the durable context listed above.
- **Out of scope:** Codex, Claude Code, OpenCode, or Pi harness adapters; changes to `spec/mutation_cursor.qnt`, mutation protocol transitions, mutation event production, or scope lifecycle; Agent Trace schema changes; database migrations; mutation consumption/checkpoint state; timestamp-based mutation attribution; configurable page/horizon limits; mutation-history retention/deletion; commit-msg mutation attribution; or a general Git history attribution engine.
- **Constraints:** keep `MUTATION_ATTRIBUTION_PAGE_SIZE = 32` and `MAX_MUTATION_ATTRIBUTION_EVENTS = 128`; keep existing direct `intersect_patches` matching unchanged; use a separate strict mutation matcher with exact-first and unique-fallback semantics; count an event as inspected when reconstruction begins; stop all older traversal at a query/decode/diff/parse barrier; use `resolve_git_dir` plus `read_checkout_id` rather than creating identity solely for attribution; reuse `git diff --binary --full-index --no-ext-diff --no-textconv <before> <after>` and `parse_patch`; perform no mutation-cursor write; keep `post_commit_patch_intersections` direct-only; persist final combined attribution only to `agent_traces.trace_json`; and derive provenance exclusively from direct evidence.
- **Non-goal:** converting mutation events into synthetic `diff_traces` or treating `AiExclusive(scope)`, `ScopeId`, or `ActorKind` as proof of a model, session, tool, or tool version.

## Assumptions

- PR #258 / `origin/mutation-scope-runtime-integration` is the implementation and validation baseline, not `main`.
- `MutationEvent.tainted == false && MutationEvent.failure_kind == FailureKind::Healthy` defines healthy event state; either contrary value makes a safely matching event non-positive and blocking.
- Direct evidence continues using the current `intersect_patches` exact-first then permissive historical fallback. The strict uniqueness rule applies only to mutation-derived matching.
- A page reader may materialize up to 32 decoded SQLite rows before the consumer resolves at an event within that page; early termination bounds subsequent Git reconstruction and prevents the next page rather than retroactively unloading those rows.

## Task stack

- [x] T01: `Freeze pure attribution semantics` (status:done)
  - Task ID: T01
  - Scope: In — add the pure resolver and strict mutation-line matcher. Inputs are target-shaped direct coverage, unresolved committed lines, and newest-first already-reconstructed `MutationPatchEvidence`; outputs are mutation-derived AI coverage, resolved/non-AI lines, and still-unresolved lines. Resolve exact matches before unique historical fallback and process safe matches newest-first. Out — Agent Trace JSON/goldens, DB pagination, Git reconstruction, worktree identity, hook composition, and persistence.
  - Dependencies: none
  - Done when: table-driven tests prove direct lines are never mutation-consulted; healthy untainted exclusive matches add AI coverage; every other safe match resolves non-AI and blocks older events; exact matching and unique fallback work one-to-one; duplicate mutation candidates, duplicate unresolved targets, and ambiguous logical-file matches remain unresolved; output sets are deterministic.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_attribution`.
  - Completed: 2026-09-03
  - Files changed: `cli/src/services/mutation_trace/attribution.rs`, `cli/src/services/mutation_trace/mod.rs`
  - Result: Added the pure `MutationPatchEvidence`/`MutationAttributionResult` domain types, newest-first resolver, and strict mutation matcher. Direct coverage is excluded before mutation matching; exact identity wins over unique kind/content fallback; exact and normalized-suffix file pairing is conservative; healthy untainted `AiExclusive` matches produce mutation AI coverage, while all other safe matches resolve as non-AI and prevent older evidence from reclaiming lines. Added focused table-driven regressions for direct precedence, newest blocking, fallback ordering, ambiguity, and unhealthy/tainted states.
  - Verify (actual): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_attribution` — 6 attribution tests passed, 0 failed. Additional `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` and `nix flake check` runs passed.
  - Context impact: Domain. Adds a pure mutation-attribution module and public resolver/matcher types for later pagination, history-consumer, and Agent Trace composition tasks; no persistence, protocol, schema, or repository-wide context change in this task.
  - Context synchronization: synced

- [ ] T02: `Add descending mutation-event pagination` (status:todo)
  - Task ID: T02
  - Scope: In — add a cold-path `MutationTraceStore` page reader filtered by exact `worktree_id`, ordered by fixed-width big-endian `revision DESC`, using an exclusive revision cursor and a requested limit capped at `MUTATION_ATTRIBUTION_PAGE_SIZE = 32`; return only revision, before/after trees, taint/failure, attribution kind, and the attribution scope needed to decode `AiExclusive`. Out — active scopes, processed events, boundary projection, Git reconstruction, the caller-owned 128-event cap, and post-commit wiring.
  - Dependencies: T01
  - Done when: one call returns at most 32 rows; pagination has no duplicates or omissions; another worktree cannot contribute; revisions `u64::MAX`, `256`, `255`, and `1` sort correctly; the exclusive cursor continues from the last returned revision; no timestamp field or predicate participates.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::store::mutation_attribution`.
  - Context synchronization: pending

- [ ] T03: `Build bounded mutation-history consumer` (status:todo)
  - Task ID: T03
  - Scope: In — compose the T02 page reader, existing read-only tree-to-tree Git diff command, patch parsing, and T01 resolver; track separate loaded-row/page, inspected-event, and Git-reconstruction counts; request pages only while unresolved lines remain and fewer than 128 events have been inspected, with every `requested_limit` equal to `min(MUTATION_ATTRIBUTION_PAGE_SIZE, MAX_MUTATION_ATTRIBUTION_EVENTS - inspected_events)`. Out — Agent Trace rendering/provenance and production post-commit composition.
  - Dependencies: T01, T02
  - Done when: every DB request is capped by page size and remaining event budget; event 128 can be loaded, inspected, and resolve a line; event 129 is neither loaded nor inspected; total loaded/inspected events never exceed 128; the current 32/128 constants produce no more than four pages without assuming divisibility; resolving at event 4 leaves loaded rows 5–32 unreconstructed and requests no second page; irrelevant events count toward the horizon; query/decode/diff/parse failure stops all older traversal while preserving direct/newer results.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace::runtime::mutation_attribution`.
  - Context synchronization: pending

- [ ] T04: `Build Agent Trace from separated evidence` (status:todo)
  - Task ID: T04
  - Scope: In — add internal `AgentTraceEvidence { direct_patch, mutation_ai_patch }`; keep `build_agent_trace(...)` as the direct-only compatibility path; classify from combined AI coverage while deriving hunk model/session and top-level tool provenance only from direct evidence; add compact complete JSON goldens for `direct_only`, `exclusive_without_direct`, `direct_plus_mutation`, `partial_combined`, `newer_nonexclusive_blocks`, and `mutation_only_no_provenance`. Out — DB reads, Git reconstruction, horizon/page mechanics, worktree identity, and hook persistence.
  - Dependencies: T01
  - Done when: expected and actual JSON for every compact fixture validate against the embedded schema before normalized comparison; direct-only output is unchanged; combined full coverage is `ai`, partial coverage is `mixed`, zero coverage is `unknown`; mutation-only output omits invented provenance; direct provenance survives combined classification.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml agent_trace::`.
  - Context synchronization: pending

- [ ] T05: `Wire mutation attribution into post-commit` (status:todo)
  - Task ID: T05
  - Scope: In — preserve the existing direct diff-trace combination/intersection and `post_commit_patch_intersections` write; resolve existing checkout identity read-only; run T03 only for unresolved committed lines in that worktree; pass direct and mutation AI patches separately to T04; validate and persist the combined result in `agent_traces.trace_json`; retain current checkpoint and auto-sync ordering. Out — identity creation solely for attribution, mutation-cursor writes, changes to `diff_traces`, migrations/schema, commit-msg behavior, and harness adapters.
  - Dependencies: T03, T04
  - Done when: missing identity/history falls back to direct-only behavior; foreign-worktree rows cannot contribute; the attribution-specific path uses no identity-creation or mutation-write API; no mutation evidence is copied into `diff_traces` or the direct intersection row; final persistence, passive checkpoint, and auto-sync retain their existing success/failure order.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::post_commit`.
  - Context synchronization: pending

- [ ] T06: `Add E2E regressions and durable context` (status:todo)
  - Task ID: T06
  - Scope: In — real Git/repository-DB regressions for Bash-style exclusive mutation without direct evidence, direct-plus-mutation completion, newer non-exclusive overwrite, linked-worktree isolation, a relevant event older than 128 newer events, and persisted final output; add/update the durable context files listed under Context sync. Out — harness adapters, protocol/schema work, and unrelated context repair.
  - Dependencies: T05
  - Done when: end-to-end post-commit tests read schema-valid combined JSON from `agent_traces.trace_json`; raw/direct-only tables preserve their meanings; mutation-only AI and direct-plus-mutation classify correctly; newer non-exclusive and foreign-worktree evidence cannot claim lines; the relevant 129th event is never reconstructed; durable context states the direct-first, strict-match, bounded, failure-barrier, provenance, and persistence contracts.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::runtime::tests::`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::`; inspect the context files listed under Context sync against the implemented code.
  - Context synchronization: pending

## Open questions

None. The request supplies the precedence, safe matching rule, failure policy, bounded traversal, persistence boundary, and task dependencies needed to implement the change without inventing attribution semantics.
