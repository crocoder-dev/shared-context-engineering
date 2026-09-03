# Mutation-trace Agent Trace attribution

Mutation history is a conservative secondary source for committed touched lines
that direct `diff_traces` evidence does not cover. Direct evidence is always
resolved first and is never revoked or replaced by mutation evidence.

Attribution is **causal**, not textual. The retained mutation history is treated
as one ordered sequence of tree transitions and provenance is propagated forward
through it. Historical mutation events are never searched independently for text
matching the committed patch. Once a later transition removes a line an event
introduced, that event's provenance is dead and no older event can resurrect it.

## Ordered lineage

`cli/src/services/mutation_trace/lineage.rs` is a pure module (no Git, SQLite,
filesystem, or mutation state). It tracks, per repo path, a vector of
`(content, LineProvenance)` lines and advances it one transition at a time.

`LineProvenance` is `Unknown`, `MutationAi { scope_id }`, or `MutationNonAi`.
A `TransitionOrigin` is:

- `MutationAi(scope)` — a recorded event that is untainted, `FailureKind::Healthy`,
  and `Attribution::AiExclusive(scope)`; its added lines become `MutationAi`.
- `MutationNonAi` — any other recorded event (contended, unscoped, unhealthy, or
  tainted); its added lines become `MutationNonAi`.
- `Unobserved` — a transition with no recorded event: the conservative baseline
  reload after a history gap, and the final latest-observed-tree to
  committed-tree tail. Its added lines are `Unknown`.

`MutationLineage::apply(patch, origin)` transforms the tracked line vectors
structurally from the hunk positions (`parse_patch` drops context lines, so
carried context is reconstructed from `old_count`/`new_count` and the removed/
added line numbers):

- context / carried line — provenance carried forward unchanged even as its line
  number moves;
- removed line — deleted permanently, verified against the tracked line's
  content;
- added line — a new entry whose provenance comes only from `origin`;
- replacement (`-old` / `+new`) — remove the old entry, create a new one from
  `origin`; textual similarity never transfers provenance;
- duplicate identical lines stay at distinct positions; provenance never jumps
  between occurrences.

Any structurally inconsistent transition (content mismatch, inconsistent hunk
lengths, out-of-range hunk) returns `LineageError`; the caller fails closed for
the affected file.

## Bounded history consumer

`resolve_bounded_mutation_attribution` in
`cli/src/services/mutation_trace/runtime/mutation_attribution.rs` composes the
store's descending [`load_mutation_event_page`](mutation-trace-store.md) reader
and read-only Git tree access over two injectable traits —
`MutationEventPageSource` (implemented for `MutationTraceStore`) and
`TreeReadSource` (implemented for `GitSnapshotService`, adding `file_at_tree`
alongside `diff_trees`).

- **Direct evidence resolves first.** `attribution::exclude_direct_coverage`
  removes directly covered committed lines by `(logical path, kind, line_number,
  content)` before any mutation-history work. If no lines remain, the consumer
  performs zero SQLite and zero Git work.
- **Load window.** The invoking worktree's events are paged newest first, bounded
  by both `MAX_MUTATION_ATTRIBUTION_EVENTS = 128` and the commit attribution cut
  (`revision <= ceiling`, applied as an exclusive `ceiling + 1` first cursor).
  Every request asks for `min(MUTATION_ATTRIBUTION_PAGE_SIZE, 128 − loaded)`
  rows; at most four pages under the 32/128 constants. Event 128 may contribute;
  event 129 is never loaded. Traversal is current-worktree-only and
  timestamp-independent; no `created_at` participates.
- **Replay oldest to newest.** The window is reversed. The baseline is the
  oldest retained event's `before_tree`, every line `Unknown`, read with
  `file_at_tree`. Each event's transition is `diff_trees(before, after)` parsed
  and applied with its `MutationAi`/`MutationNonAi` origin.
- **Transition continuity.** Before each event, if its `before_tree` does not
  equal the previous event's `after_tree`, the tracked files are reloaded to an
  all-`Unknown` baseline from that `before_tree` and older provenance does not
  cross the gap. Newer events still establish provenance.
- **Unobserved tail.** After the last replayed event, if its `after_tree`
  differs from `commit_tree`, `diff(after, commit_tree)` is applied as an
  `Unobserved` transition: new and replaced tail lines are `Unknown`, surviving
  lines keep their provenance.
- **Projection.** Only after the lineage reaches `commit_tree`, each committed
  added line is looked up at its exact committed-tree position:
  `MutationAi -> mutation AI coverage`, `MutationNonAi -> resolved non-AI`,
  `Unknown` / missing / content mismatch -> unresolved.
- **Conservative failure.** A page-query failure truncates history (the window
  is simply smaller and the baseline older). A tree-diff / patch-parse /
  structural-apply failure reloads the affected files to an all-`Unknown`
  baseline from a real tree state and replay continues. A tail failure leaves
  tail lines `Unknown`. Bounded history that cannot prove an older line's
  provenance is a false negative, never a false positive. The barrier kind is
  reported on the result; the function never returns `Err`.
- **Work counters.** `loaded_pages` / `loaded_rows` (database) are separate from
  `inspected_events` / `reconstructed_events` (Git); `gap_resets` counts
  conservative reloads during replay.

The consumer performs no mutation-cursor write and creates no worktree or scope
identity.

## Commit attribution cut

`resolve_post_commit_mutation_ai_patch(repository_root, &db, direct_coverage,
committed_patch) -> ParsedPatch` is the read-only post-commit entrypoint. It
resolves the invoking worktree's *existing* checkout identity
([`checkout::resolve_git_dir`](checkout-identity.md) + `read_checkout_id`, never
`get_or_create_*`), reads `HEAD^{tree}` as `commit_tree`, and captures the
commit attribution cut: under the same worktree lock that serializes
mutation-event transitions
([`worktree_lock`](mutation-trace-runtime-coordinator.md)), it reads
`MutationTraceStore::latest_mutation_event_revision` for the worktree. An event
produced after the commit has a higher revision and cannot participate. The
critical section is a single indexed read.

An unresolvable git dir, an absent/unreadable checkout identity, an unavailable
snapshot service, an unreadable `HEAD` tree, a lock timeout, or no mutation
history at all each yield an empty patch, so post-commit falls back to
direct-only Agent Trace behavior. The entrypoint creates no identity and writes
no mutation-cursor state.

## Post-commit composition

- **Direct evidence stays authoritative.** The post-commit flow computes the
  existing direct `intersect_patches` intersection and passes it as
  `direct_coverage`; only committed lines it does not cover reach mutation
  history. `post_commit_patch_intersections` keeps its direct-only meaning and
  mutation evidence never enters `diff_traces`.
- **No fabricated provenance.** The mutation-AI patch is target-shaped and
  carries no model, session, tool, or tool-version metadata. `ScopeId`,
  `ActorKind`, and `AiExclusive(scope)` are never translated into direct
  provenance. Hunk model/session and the top-level `tool` object still derive
  from direct evidence only; mutation-only coverage merely widens `ai` / `mixed`
  classification. See
  [../sce/agent-trace-minimal-generator.md](../sce/agent-trace-minimal-generator.md).
- **Final persistence.** The single combined Agent Trace (direct + mutation AI
  coverage) is validated against the embedded schema and stored in
  `agent_traces.trace_json`; no schema, migration, or checkpoint is added. Hook
  wiring detail lives in
  [../sce/agent-trace-hooks-command-routing.md](../sce/agent-trace-hooks-command-routing.md).

## Known limitation

A tree-snapshot system cannot observe an intermediate remove/re-add that leaves
no tree-state difference. If the latest observed tree already contains an AI
line and a human removes and re-adds byte-identical text before the commit so
that the committed tree matches the latest observed tree, the lineage still
reports the surviving AI provenance. The fix this path delivers is that observed
history must be *causal*: it does not reconstruct mutations that produced no
observable tree difference.

## Regressions

- `lineage.rs` — added AI line carries provenance; removed line loses it
  permanently; identical remove/re-add takes the new transition's provenance;
  context provenance survives line-number movement; `Unobserved` introduces
  `Unknown`; content mismatch is a `LineageError`; duplicate lines do not let
  provenance jump; deleted file drops out.
- `runtime/mutation_attribution/tests.rs` — surviving AI line attributed; AI
  survives an unrelated later mutation; a stale AI mutation cannot resurrect
  through an unobserved tail; a non-AI replacement owns the new line; a history
  gap is not crossed; bounded-history baseline starts `Unknown`; an unobserved
  tail adds `Unknown` but keeps surviving AI; an event past the commit cut has
  no influence; event 128 contributes and 129 is never loaded; a page-query
  failure is a conservative barrier; a reconstruction failure reloads and
  continues; real `GitSnapshotService` + store seams.
- `runtime/tests.rs` — a still-relevant event behind 128 newer events is never
  loaded or reconstructed.
- `hooks/mod.rs` (`mutation_attribution_e2e`) — real Git/DB: mutation-only `ai`
  without provenance; direct+mutation completion; a newer non-exclusive event
  keeps a line non-AI; adversarial linked-worktree isolation; the three
  persistence layers stay separated (`diff_traces` and
  `post_commit_patch_intersections` direct-only, `agent_traces.trace_json`
  combined).
