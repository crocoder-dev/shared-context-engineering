# Mutation-trace Agent Trace attribution

Mutation history is the causal source of committed-line attribution. Every
committed touched line is projected through mutation lineage; direct
`diff_traces` evidence only attributes lines that lineage cannot resolve, and
only when it meets the trust requirements below.

Attribution is **causal**, not textual. The retained mutation history is treated
as one ordered sequence of tree transitions and provenance is propagated forward
through it. Historical mutation events are never searched independently for text
matching the committed patch. Once a later transition removes a line an event
introduced, that event's provenance is dead and no older event can resurrect it.

## Ordered lineage

`cli/src/services/mutation_trace/lineage.rs` is a pure module (no Git, SQLite,
filesystem, or mutation state). It tracks, per repo path, a vector of
`(content, LineProvenance)` lines and advances it one transition at a time.

File identity is **exact logical repository path** equality only. A file's
logical path is `new_path` when non-empty and otherwise `old_path`; the lineage
keys its tracked vectors by that path and a reconstructed transition only
affects a tracked file when its own `old_path`/`new_path` is byte-equal to a
tracked path. There is no normalized-suffix or basename equivalence here:
`packages/foo/src/lib.rs` never pairs with `src/lib.rs`. Suffix matching stays
confined to the direct-evidence matchers in
[patch-service.md](patch-service.md).

`LineProvenance` is `Unknown`, `MutationAi { scope_id }`, or `MutationNonAi`.
A `TransitionOrigin` is:

- `MutationAi(scope)` — a recorded event whose `failure_kind` is
  `FailureKind::Healthy` and whose attribution is `Attribution::AiExclusive(scope)`;
  its added lines become `MutationAi`. Health is read from `failure_kind` alone;
  the store's decode validator guarantees the redundant `tainted` bit agrees.
- `MutationNonAi` — any other recorded event (contended, unscoped, or
  unhealthy); its added lines become `MutationNonAi`.
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
  between occurrences;
- a deleted or renamed-away tracked path stays tracked as an absent (empty)
  file, so a later transition that recreates it is applied rather than silently
  skipped. An `Added` transition for a path still tracked with content is a
  `LineageError`.

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

- **Every committed line is projected.** Direct evidence no longer removes
  lines before replay, so a weak or stale textual match can never hide a line
  from causal lineage. A committed patch with no touched lines performs zero
  SQLite and zero Git work.
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
  Mutation-AI coverage retains the contributing `ScopeId` through this lookup,
  then resolves its durable scope provenance once per distinct scope. A
  resolved session is copied to each contributing `TouchedLine`; a mutation-AI
  hunk receives a model only when every contributing line has the same known
  model. Missing provenance, a NULL model, or disagreement leaves the hunk
  model unset without changing its AI classification.
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

`resolve_post_commit_mutation_attribution(repository_root, &db,
committed_patch) -> MutationAttributionResult` is the read-only post-commit
entrypoint. It returns the mutation-AI, resolved non-AI, and unresolved line
sets. It
resolves the invoking worktree's identity from Git topology
([`git_snapshot::resolve_git_dir` + `resolve_worktree_id`](mutation-trace-runtime-coordinator.md),
which derive `WorktreeId` from `git rev-parse --absolute-git-dir` /
`--git-common-dir` rather than any persisted identity file), reads
`HEAD^{tree}` as `commit_tree`, and captures the commit attribution cut: under
the same worktree lock that serializes mutation-event transitions
([`worktree_lock`](mutation-trace-runtime-coordinator.md)), it reads
`MutationTraceStore::latest_mutation_event_revision` for the worktree. An event
produced after the commit has a higher revision and cannot participate. The
critical section is a single indexed read.

An unresolvable git dir, an unresolvable worktree identity, an unavailable
snapshot service, an unreadable `HEAD` tree, a lock timeout, or no mutation
history at all each yield an empty result (every line unresolved), so
post-commit falls back to trusted direct-only Agent Trace behavior. The entrypoint creates no identity and writes
no mutation-cursor state.

## Evidence precedence

Each committed touched line is resolved independently in
`build_agent_trace_from_evidence` (`cli/src/services/agent_trace.rs`), with
direct candidate matching in `cli/src/services/agent_trace/evidence.rs`:

| Mutation lineage | Direct evidence | Result |
| --- | --- | --- |
| `MutationAi` (session S) | unique, session S | S; model from scope, else from the direct row |
| `MutationAi` (session S) | anything else | S; the direct candidate is discarded |
| `MutationNonAi` | anything | not AI; direct evidence cannot override |
| unresolved | unique trusted match | the direct session and model |
| unresolved | conflicting sessions, historical-only, or none | unknown |

- **When is direct evidence sufficient?** Only when lineage left the line
  unresolved and a direct candidate is (1) *fresh* — recorded after the
  parent commit's committer-time second (`direct_evidence_cutoff_ms`), still
  inside the 7-day window; (2) an *exact* touched-line match on file identity,
  `kind`, `line_number`, and `content`; and (3) *unambiguous* — every fresh
  exact candidate for that line comes from one session. Each candidate row is
  kept separate (never merged into one patch), so evidence is consumed by its exact
  occurrence and a duplicate line elsewhere in the file is not covered.
- **When does lineage override direct evidence?** Whenever lineage resolves
  the line: `MutationAi` attributes to the owning scope's session, and
  `MutationNonAi` (contended, unscoped, or unhealthy transitions) keeps it
  non-AI.
- **What happens when sources disagree?** The causal lineage result wins over
  a textual match. Disagreeing direct candidates for one line fail closed to
  unknown; SCE never picks the newest or first row. Overridden candidates do
  not reach `conversation.related`, `contributor.model_id`, or the `tool`
  object (emitted only when a direct candidate supports a final line
  attribution).
- **What happens when history is incomplete?** Gaps, the 128-event window, the
  revision ceiling, and the unobserved tail keep lines `Unknown`, as before.
  An unknown line then needs fresh, exact, single-session direct evidence;
  historical `(kind, content)` similarity alone never attributes it.
- **How are already-committed changes not credited again?** Direct rows
  recorded before the parent commit are outside the query window, positional
  exactness rejects drifted boilerplate, and lineage attributes only the
  transition that most recently produced the committed line.
- **What remains unsupported?** Removed lines are never resolved by lineage
  (only exact fresh direct evidence can cover them). Direct-only edits whose
  line numbers drift through later uncaptured edits now resolve to unknown or
  `mixed` instead of `ai`. The freshness bound uses the committer time of the
  first parent, so an uncommitted edit made before the parent commit (partial
  staging) loses direct attribution. Mutation scopes without a recorded model
  still produce hunks without `model_id`.

The former `intersect_patches` / `combine_patches` helpers and their historical
`(kind, content)` fallback were removed from `patch.rs`.

## Post-commit composition

- **Evidence stays separated.** `post_commit_patch_intersections` stores only
  the trusted direct-evidence projection (fresh, exact, single-session) and its
  `recent_window_cutoff_ms` records the freshness-bounded cutoff. Mutation
  evidence never enters `diff_traces`.
- **No fabricated direct provenance.** The mutation-AI patch is target-shaped
  and carries only the resolved observational session/model metadata described
  above; it carries no tool or tool-version metadata. `ScopeId`, `ActorKind`,
  and `AiExclusive(scope)` are never translated into direct provenance. See
  [../sce/agent-trace-minimal-generator.md](../sce/agent-trace-minimal-generator.md).
- **Final persistence.** The single resolved Agent Trace is validated against
  the embedded schema and stored in `agent_traces.trace_json`; no schema or
  migration is added. Hook wiring detail lives in
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

- `lineage/tests.rs` — added AI line carries provenance; removed line loses it
  permanently; identical remove/re-add takes the new transition's provenance;
  context provenance survives line-number movement; `Unobserved` introduces
  `Unknown`; content mismatch is a `LineageError`; duplicate lines do not let
  provenance jump; a deleted-then-recreated file stays tracked; a renamed-away
  path stays tracked as absent; adding a file still tracked with content fails
  closed.
- `runtime/mutation_attribution/tests.rs` — real Git trees: an observed AI event
  beats an old session's historical text match; AI edits to a deleted and
  recreated file keep provenance (the `control-plane` `494e5693` incident
  shape); an unobserved tail, a history gap, an event outside the 128-event
  window, and an event beyond the revision ceiling each keep the line unknown
  and keep the old session out of the trace.
- `hooks/commit_hooks/attribution_tests.rs` — dated Git history: stale
  already-committed sessions do not credit a later commit; lineage overrides a
  fresh exact match from another session; non-exclusive lineage blocks direct
  evidence; historical content at other positions does not attribute; fresh
  exact direct-only evidence still attributes with session, model, and tool;
  conflicting direct sessions fail closed; one source line covers one duplicate
  occurrence; the staged co-author check ignores stale and historical-only
  rows; real Turso persistence from `diff_traces` through
  `post_commit_patch_intersections`, `MutationTraceStore`, and
  `agent_traces.trace_json`.
