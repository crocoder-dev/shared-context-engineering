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
`(content, LineProvenance, Option<ParentLineId>)` lines and advances it one
transition at a time.

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

### Parent-line identity

A removed line no longer exists in the committed file, so its identity cannot
come from new-file coordinates. `MutationLineage::from_parent` seeds a lineage
from the exact parent commit's tree and gives every baseline line a
`ParentLineId { path, line_number }`: its parent-side path and 1-based line
number. `from_baseline` (the added-line replay) gives no line a parent
identity.

- A carried line keeps its `ParentLineId` however far its position moves.
- Every line a transition adds has no parent identity, even when its text equals
  a parent line. Text equality never proves identity.
- When a transition removes a line that has a parent identity, the lineage
  records a removal of that identity, with the line's content and the
  provenance of the **removing** transition (`MutationAi` → AI with its scope,
  `MutationNonAi` → non-exclusive, `Unobserved` → `Unknown`). The provenance of
  the line's original author is never consulted. Removing a line that was added
  after the parent records nothing.
- **Restoration.** When a later transition adds content equal to an
  already-removed parent line of the same parent file (the current path, or the
  parent path it holds by an observed rename), that earlier removal claim
  becomes `Unknown` for good. A later removal of the restored line has no parent
  identity, so it cannot resurrect the older claim. Removals made by the same
  transition are not invalidated by its own additions.
- **Renames.** The lineage maps each tracked path to the parent path whose
  lines it holds. Only a rename the replayed diff itself reports moves that
  mapping; a delete-plus-add pair does not establish it.
- `reset_all` drops every parent identity and removal claim.

`committed_removals(current_path, parent_path, lines)` resolves a canonical
file's removed lines. It returns `Unknown` for every line unless the lineage
establishes that `current_path` holds `parent_path`. Where the canonical diff
and the replay disagree about which parent lines were removed, the two chose
different alignments among equal content. In that case every committed removal
sharing the disagreeing content stays `Unknown`.

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
  event 129 is never loaded. Traversal is current-worktree-only. The generic
  consumer is timestamp-independent; the post-commit entrypoint pages through a
  commit-bounded source whose SQL also requires `created_at` before the
  [commit-time cutoff](#commit-time-cutoff), so the 128-event budget counts only
  temporally eligible events.
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

## Removed-line attribution

Added and removed lines answer different questions:

- **Added line:** which transition most recently introduced the surviving
  content? This is the gap-tolerant replay above.
- **Removed line:** which transition effectively removed this exact line of the
  parent commit? The remover is the transition that removed it, not the actor
  that originally wrote it. A human removing an AI-written line is not an AI
  removal. An AI removing a human-written parent line can be.

Removed lines use a second, stricter lineage. It shares the same event window
and each event's reconstructed `diff_trees` patch with the added-line replay,
but it never resets, so it cannot weaken added-line attribution. One commit can
resolve its additions and still leave its removals unknown.

A committed removed line receives mutation-derived attribution only when
**all** of these hold:

1. **Exact parent.** The commit has exactly one parent. Root and merge commits
   have no `parent_tree`, so every removal stays unresolved.
2. **Anchor.** Some loaded event (within the 128-event window and the revision
   cut) has `before_tree == parent_tree`. The replay starts at the **last** such
   event. When the worktree returns to the exact parent tree, identity
   restarts there, because the tree is byte-identical to the parent. No anchor
   means every removal stays unresolved; the window is never widened to find one.
3. **Uninterrupted chain.** From the anchor to the last loaded event, each
   `before_tree` equals the previous `after_tree`, and every transition
   reconstructs and applies cleanly. The first gap or failure abandons the
   whole removal replay. Removal claims are never carried across a reset.
4. **Per-path reconciliation.** For every tracked path, the replayed lines must
   equal the commit tree's file, and any unobserved tail from the last event's
   `after_tree` to `commit_tree` must leave that path byte-identical. A path that
   fails either check has its removal claims (and those of the parent path it
   holds) made `Unknown`. A tail touching only other paths, such as untracked
   or unstaged files, does not affect it.
5. **Canonical identity.** The canonical removed line's old-side identity
   (`old_path`, old line number, content) names a recorded removal: the
   committed file holds that parent path by observed history, the content
   matches, the alignment agrees for that content, and no later restoration
   invalidated it.

The verified result is AI (with the scope's session and model, resolved through
the existing scope-provenance lookup) for a healthy `AiExclusive` remover, and
`NonExclusive` for any other recorded remover. It is never human authorship.
Every other committed removal is unresolved. `BoundedMutationAttribution.
removal_replay_anchored` reports whether conditions 1–3 held.

**Partial staging.** Removal attribution trusts a path only when the committed
file equals the last observed worktree file. If the AI removed `B` and `C` from
the worktree but only the removal of `B` was staged, the committed file differs
from the observed tree, so `B` stays unresolved. This proves only what the
observed tree transitions support. It is not an audit of who ran `git add` or
assembled the index, and an unobserved change that nets to zero on a path cannot
be detected.

**Persistence.** No commit-boundary or per-line history is persisted across
commits. The parent anchor must be found in the bounded mutation-event window,
which is why a long session (more than 128 events since the parent tree) or
history before a gap leaves removals unknown.

## Commit attribution cut

`resolve_post_commit_mutation_attribution(repository_root, &db, commit_oid,
committed_patch, cutoff) -> MutationAttributionResult` is the read-only
post-commit entrypoint. A `None` cutoff (untrusted commit time) returns an
empty result immediately. It returns the mutation-AI, resolved non-AI, and unresolved line
sets. It
resolves the invoking worktree's identity from Git topology
([`git_snapshot::resolve_git_dir` + `resolve_worktree_id`](mutation-trace-runtime-coordinator.md),
which derive `WorktreeId` from `git rev-parse --absolute-git-dir` /
`--git-common-dir` rather than any persisted identity file). It resolves
`commit_tree` and the first-parent `parent_tree` from the commit OID captured at
hook time (`GitSnapshotService::committed_trees`: `rev-parse --verify
<oid>^{tree}`, `rev-list --parents --max-count=1 <oid>`). It never reads a
mutable `HEAD`, so a `HEAD` move after capture cannot change the anchoring.
`parent_tree` is `None` for root and merge commits. It then captures the commit
attribution cut: under
the same worktree lock that serializes mutation-event transitions
([`worktree_lock`](mutation-trace-runtime-coordinator.md)), it reads
`MutationTraceStore::latest_mutation_event_revision` for the worktree. An event
produced after the commit has a higher revision and cannot participate. The
critical section is a single indexed read.

An unresolvable git dir, an unresolvable worktree identity, an unavailable
snapshot service, an unreadable commit tree, a lock timeout, or no mutation
history at all each yield an empty result (every line unresolved), so
post-commit falls back to trusted direct-only Agent Trace behavior. The entrypoint creates no identity and writes
no mutation-cursor state.

The revision ceiling is only a stable-snapshot bound. It is read inside
post-commit processing, after Git has already created the commit, so events
recorded between commit creation and the hook (for example an agent deleting a
line again after the working tree was restored) still sit below it. Temporal
eligibility comes from the commit-time cutoff below.

## Commit-time cutoff

This is a conservative risk-reduction rule, not a causal commit boundary.

- **Source.** `capture_post_commit_patch_from_git` reads `HEAD` once for the
  commit OID, then reads the committer timestamp with `git show --format=%ct
  --no-patch <oid>` and the parent timestamp from `<oid>^`. Mutable `HEAD` is
  never read again. A root commit keeps `parent_commit_time_ms = None`.
- **Cutoff.** `CommitTimeCutoff` (`agent_trace_db/mod.rs`) is the start of the
  committer-timestamp Unix second. Evidence is eligible only when its
  database-generated `created_at` is strictly before that instant, so anything
  recorded during the commit's second or later is excluded. Git timestamps have
  one-second resolution, so the system cannot tell which side of the commit an
  event in that second falls on; it prefers a false negative.
- **Insertion time, not event time.** The rule uses `created_at` (database
  insertion time). `diff_traces.time_ms` is the reported event time and can be
  old for a late insert, so it never replaces the insertion-time check.
- **Both sources.** Mutation events and direct evidence use the same rule.
  Filtering only one would let the other reintroduce a late AI claim: a late
  direct row could otherwise recover an `Unknown` line that lineage correctly
  left unresolved.
- **Before the limit.** The predicate is part of every mutation page query:

  ```sql
  WHERE worktree_id = ?1
    [AND revision < ?cursor]
    AND created_at < strftime('%Y-%m-%dT%H:%M:%fZ', ?cutoff_seconds, 'unixepoch')
  ORDER BY revision DESC LIMIT ?limit
  ```

  Filtering after loading the newest 128 events would let a burst of late
  events fill the window and push out eligible older ones. Only the post-commit
  path uses `load_mutation_event_page_before_commit`; `load_mutation_event_page`
  and its other consumers are unchanged.
- **Trust check.** `trusted_commit_time_cutoff` accepts the timestamp only when
  `GIT_COMMITTER_DATE` is absent from the hook environment, the value is a
  whole non-negative Unix second with a four-digit UTC year, the commit second
  does not start after post-commit entry (no future tolerance; Git and the
  database read the same host clock), and it started at most
  `COMMIT_TIME_MAX_AGE_MILLIS` (5 minutes) before entry. The check cannot prove
  the timestamp is authentic. A slow, legitimate hook beyond 5 minutes loses
  attribution; that false negative is accepted.
- **Fail closed.** When the timestamp is untrusted, `attribution_cutoff` is
  `None`: no direct-evidence query runs, mutation attribution returns an empty
  result, and every committed touched line is `Unknown`. The Agent Trace is
  still built and persisted with its usual schema and the real commit
  timestamp, and the commit itself is never blocked. Nothing falls back to the
  current time or an unlimited query. If the commit OID or metadata cannot be
  read, the flow errors before any evidence is queried.
- **Assumptions.** The rule depends on the committer timestamp and the database
  clock roughly agreeing. It does not protect against backdated or
  future-dated commits that pass the trust check, clock rollback, significant
  skew, evidence delayed before insertion, or old evidence that already
  matched an unrelated change.

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
  unresolved and a direct candidate is (1) *fresh* — reported after the
  parent commit's committer-time second (`direct_evidence_cutoff_ms`), still
  inside the 7-day window, reported before and inserted into the database
  before the [commit-time cutoff](#commit-time-cutoff); (2) an *exact* touched-line match on repository
  file identity, `kind`, `line_number`, and `content` — file identity is the
  normalized repository-relative path (absolute paths only inside the
  repository root, Claude structured relative paths resolved only against
  their recorded absolute `cwd` and rejected without one, no suffix
  equivalence, `..` escapes rejected; see
  `agent_trace/repository_path.rs`); and (3) *unambiguous* — every fresh
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
  object. The `tool` object is derived only from the direct rows supporting
  finally accepted lines and is omitted when those rows name different tools
  or any lacks a tool name; a differing version is dropped.
- **Which evidence may influence outputs?** Only accepted evidence. The same
  acceptance (`agent_trace/evidence.rs`: repository file identity → exact line
  match → single-session resolution) feeds the staged co-author preflight, the
  persisted direct intersection, and the builder; the builder then reconciles
  with mutation lineage before deriving session, model, and tool metadata. The
  co-author preflight runs before mutation resolution and does not consult
  mutation events, but still rejects conflicting direct candidates.
- **What happens when history is incomplete?** Gaps, the 128-event window, the
  revision ceiling, and the unobserved tail keep lines `Unknown`, as before.
  Removed lines additionally need the exact parent anchor and an uninterrupted
  chain (see [Removed-line attribution](#removed-line-attribution)).
  An unknown line then needs fresh, exact, single-session direct evidence;
  historical `(kind, content)` similarity alone never attributes it.
- **How are already-committed changes not credited again?** Direct rows
  recorded before the parent commit are outside the query window, positional
  exactness rejects drifted boilerplate, and lineage attributes only the
  transition that most recently produced the committed line.
- **How are touched lines counted?** Each committed touched line receives
  exactly one final decision — `Ai`, `NonExclusive`, or `Unknown` — and
  `metadata.sce.line_changes` increments one `{ai, non_exclusive, unknown}`
  bucket per line from that decision. `non_exclusive` means lineage resolved a
  non-exclusive or unscoped transition (not human authorship); `unknown` means
  unresolved. Counters cover the canonical committed patch exactly and do not
  depend on hunk grouping. See
  [../sce/agent-trace-minimal-generator.md](../sce/agent-trace-minimal-generator.md).
- **How do decisions become exported ranges?** Only surviving added lines get
  ranges, at their post-commit positions. Contiguous lines with the same
  decision, accepted session, and model coalesce; ranges group into one
  conversation per accepted session (or per model for session-less AI, or per
  unresolved decision kind). A Git hunk never decides a contributor, and SCE
  does not emit `mixed` for partial AI coverage. Hashes cover the exact
  committed bytes of each range. Paths are the decoded Git identity (C-quoted
  names are decoded byte-exactly and must be UTF-8). Only regular and
  executable blobs at the recorded revision are line-addressable; symlinks,
  gitlinks, and directories emit no ranges but keep their counters. In the
  post-commit hook a missing path or failed Git read aborts the trace instead
  of persisting unverified ranges. See
  [../sce/agent-trace-minimal-generator.md](../sce/agent-trace-minimal-generator.md#ranges-and-conversations).
- **What remains unsupported?** Removed lines resolve through lineage only under
  the [removed-line conditions](#removed-line-attribution). Otherwise, only
  exact fresh direct evidence can cover them, and the precedence above is
  unchanged. They count as `unknown` even when the line's original author is
  known. Removed lines have no post-commit position, so their decisions appear
  only in counters, never as ranges. Direct-only edits whose line numbers drift through later uncaptured
  edits resolve to `unknown` instead of `ai`. The freshness bound uses the committer time of the
  first parent, so an uncommitted edit made before the parent commit (partial
  staging) loses direct attribution. Mutation scopes without a recorded model
  still produce ranges without `model_id`.

The former `intersect_patches` / `combine_patches` helpers and their historical
`(kind, content)` fallback were removed from `patch.rs`.

## Post-commit composition

- **Evidence stays separated.** `post_commit_patch_intersections` stores only
  the trusted direct-evidence projection (fresh, exact, single-session).
  `recent_window_cutoff_ms` records the inclusive freshness lower bound and
  `recent_window_end_ms` the exclusive upper bound: the start of the commit
  second (rows must also have `created_at` before it). When the commit time is
  untrusted, both hold the lower bound, an empty window, and no rows load.
  Mutation evidence never enters `diff_traces`.
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
  closed. Parent identity: AI, non-exclusive, and unobserved removals; identity
  stable under insertion above; distinct identities across one hunk and across
  hunks; duplicate text keeps two identities and a disagreeing alignment stays
  unknown; removing a post-parent line records nothing; restoration invalidates,
  and restore-then-redelete never resurrects the AI claim; matching text in an
  unrelated file is not a restoration; whole-file deletion; deleted-then-recreated
  files; observed renames versus delete-plus-add; reset and per-path distrust.
  A seeded randomized test (600 histories of insert/remove/replace/restore with
  repeated content, two AI scopes, non-exclusive, unobserved transitions, and
  gaps) checks removal identities, added-line provenance, and canonical
  projection against an independent reference model and LCS diff.
- `runtime/mutation_attribution/tests.rs` — real Git trees: an observed AI event
  beats an old session's historical text match; AI edits to a deleted and
  recreated file keep provenance (the `control-plane` `494e5693` incident
  shape); an unobserved tail, a history gap, an event outside the 128-event
  window, and an event beyond the revision ceiling each keep the line unknown
  and keep the old session out of the trace. Removals: proven AI (with scope
  session/model, no ranges) and non-exclusive removals; a missing parent
  anchor, a gap, a parent outside the window, an unobserved tail removal, and a
  partially staged removal stay unresolved; a tail distrusts only the paths it
  touches; restore-then-redelete keeps no stale AI claim, while an exact return
  to the parent tree re-anchors; duplicates map to their own parent line; a
  rename resolves under its old path; added lines resolve identically with or
  without a parent anchor; `committed_trees` follows the captured OID after
  `HEAD` moves and yields no parent for root and merge commits.
- `hooks/commit_hooks/attribution_tests.rs` — dated Git history: stale
  already-committed sessions do not credit a later commit; lineage overrides a
  fresh exact match from another session; non-exclusive lineage blocks direct
  evidence; historical content at other positions does not attribute; fresh
  exact direct-only evidence still attributes with session, model, and tool;
  conflicting direct sessions fail closed; one source line covers one duplicate
  occurrence; the staged co-author check ignores stale and historical-only
  rows; real Turso persistence from `diff_traces` through
  `post_commit_patch_intersections`, `MutationTraceStore`, and
  `agent_traces.trace_json`. A deletion-only commit, resolved through
  `resolve_post_commit_mutation_attribution` after `HEAD` has moved, persists
  `ai.removed = 3`, `non_exclusive.removed = 1`, `unknown.removed = 1`, and
  `"files": []`. A mixed add/modify/delete commit keeps the same added-line
  attribution and ranges as an unanchored replay.
- Commit-time cutoff. `runtime/mutation_attribution/tests.rs`: rows created at
  `:03.999` are eligible and `:04.000`, `:04.001`, `:04.999`, `:05.000` are not;
  140 late events do not consume the 128-event budget, pages span
  correctly, and the cap still holds. `hooks/commit_hooks/attribution_tests.rs`:
  the same boundary for `diff_traces` (including old `time_ms` with late
  `created_at`) while the staged preflight query is unchanged; a late AI
  deletion stays `unknown` (an unbounded control would credit it); a
  pre-commit AI deletion keeps its session and model; a delayed hook keeps the
  earlier session while a late restore-and-redelete is ignored; an in-second
  pre-commit event is the documented false negative; late direct evidence
  cannot claim lines; an overridden, future, or stale commit time disables
  both sources while still persisting a trace; a failed metadata read queries
  nothing; timestamps follow the captured OID after `HEAD` moves. Each flow
  test checks `ai + non_exclusive + unknown` against the committed added and
  removed lines.
