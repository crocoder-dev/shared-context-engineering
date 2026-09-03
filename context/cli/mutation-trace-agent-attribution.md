# Mutation-trace Agent Trace attribution

The pure attribution seam in `cli/src/services/mutation_trace/attribution.rs`
adds mutation history as a conservative secondary source for committed touched
lines. It does not read Git, SQLite, the filesystem, or mutation state. A caller
supplies target-shaped direct coverage, the committed lines still unresolved,
and already-reconstructed mutation evidence in newest-first order.

## Resolution contract

- Directly covered lines are removed before mutation matching. Mutation history
  cannot revoke or replace direct evidence.
- Each mutation event is considered in caller-supplied newest-first order. The
  first safe event matching a line resolves it; a later event cannot reclaim
  that line.
- An event contributes mutation-derived AI coverage only when it is untainted,
  has `FailureKind::Healthy`, and carries `Attribution::AiExclusive(_)`.
  Contended, unscoped, unhealthy, and tainted matches resolve as non-AI.
- Mutation AI, resolved non-AI, and still-unresolved results retain the
  committed target patch's file/hunk shape and deterministic target ordering.
  Mutation-derived results do not acquire direct model, session, tool, or
  tool-version provenance.

## Safe matching

A `MutationPatchEvidence` patch is matched to unresolved target lines using a
separate strict matcher; the existing permissive direct `intersect_patches`
operation is unchanged.

1. A file's logical path is `new_path` when it is non-empty, otherwise
   `old_path`.
2. Exact logical-path pairing is attempted first. Normalized suffix pairing is
   allowed only when exactly one mutation file and one unresolved target file
   can be paired.
3. Within a safely paired file, equal `(kind, line_number, content)` keys are
   matched first, one-to-one.
4. Remaining lines may use the historical `(kind, content)` fallback only when
   that key occurs exactly once in each side.
5. Repeated candidates and ambiguous file or line matches are left unresolved;
   attribution prefers false negatives to guessed ownership.

The matcher exposes location-based matches so repeated lines cannot be
silently collapsed in the result. The seam is intentionally independent of
pagination, event reconstruction, worktree identity, and post-commit Agent
Trace persistence; those boundaries remain owned by their respective runtime
and storage services.

## Bounded history consumer

`resolve_bounded_mutation_attribution` in
`cli/src/services/mutation_trace/runtime/mutation_attribution.rs` is the
runtime consumer that feeds the pure seam above. It composes the store's
descending [`load_mutation_event_page`](mutation-trace-store.md) reader, a
tree-to-tree Git diff, `patch.rs::parse_patch`, and the resolver, over two
injectable traits — `MutationEventPageSource` (implemented for
`MutationTraceStore`) and `TreeDiffSource` (implemented for
`GitSnapshotService`, reusing its existing `diff_trees`).

- **Direct evidence resolves first.** The consumer's first step is
  `attribution::exclude_direct_coverage(committed_target, direct_coverage)`,
  the same `(logical path, kind, line_number, content)` exclusion the pure
  resolver applies internally. Direct-covered committed lines are removed
  before the first mutation-history page request. If no unresolved lines
  remain, the bounded consumer performs no SQLite or Git work — zero page
  requests, zero loaded rows, zero inspected events, zero tree diffs — and
  returns an empty result with `barrier: None`. A partially direct-covered
  target sends only the remaining lines into traversal. Mutation history can
  never revoke or re-resolve a directly covered line.
- **Aggregation uses logical target-file identity.** Per-event result parts
  are unioned with a mutation-attribution-local helper keyed on
  `new_path` when non-empty, otherwise `old_path` — not raw `new_path`. Two
  distinct deleted files (both with an empty `new_path`) stay separate in the
  combined `mutation_ai_patch` / `resolved_non_ai_patch`. The helper preserves
  target-shaped file/hunk metadata and deterministic ordering, deduplicates
  only identical selected target lines, and introduces no provenance. Global
  `combine_patches` is unchanged.
- **Current-worktree-only, revision-descending, timestamp-independent.**
  Traversal pages by exclusive revision cursor for exactly the invoking
  worktree; no `created_at` value participates.
- **Bounded horizon.** The consumer — not the store — owns
  `MAX_MUTATION_ATTRIBUTION_EVENTS = 128`. Every page request asks for
  `min(MUTATION_ATTRIBUTION_PAGE_SIZE, MAX_MUTATION_ATTRIBUTION_EVENTS −
  inspected_events)` rows, so it never loads past the budget even if the two
  constants stop being exact multiples. Event 128 may be loaded, inspected,
  and resolve a line; event 129 is never loaded or inspected. Under the
  current 32/128 constants this is at most four pages.
- **Early termination.** The moment the unresolved set empties, traversal
  stops: rows already materialized in the current page are left
  unreconstructed and no further page is requested. A short page also ends
  traversal rather than issuing a guaranteed-empty follow-up query.
- **Separate work counters.** `loaded_pages` / `loaded_rows` (database) are
  tracked apart from `inspected_events` / `reconstructed_events` (Git).
  `inspected_events` increments when reconstruction begins, including an event
  whose tree diff or patch parse then fails; irrelevant events that match
  nothing still count toward the horizon.
- **Failure barrier.** A page query/decode failure, or an inspected event's
  tree-diff/patch-parse failure, is a hard barrier: it keeps direct evidence
  and every line already resolved by a newer successfully reconstructed
  event, inspects no older event, requests no further page, and leaves every
  remaining line unresolved/unknown. The barrier kind is reported on the
  result; the function never returns `Err`.

The consumer performs no mutation-cursor write and creates no worktree or
scope identity. Post-commit composition and Agent Trace provenance remain
owned by their later hook and generator seams.
