# Mutation-trace store (`mutation_trace::store`)

Durable persistence for the verified mutation-cursor protocol
([`protocol.rs`](mutation-trace-protocol.md)), built by the
`mutation-cursor-store-persistence` plan. `store.rs` is the protocol's first
real database call site: it stores worktree/scope/processed-event/mutation-event
state in the repository-scoped Agent Trace DB (`RepositoryAgentTraceDb`) via
migration `004_mutation_trace_protocol.sql`.

## Boundary shape

```mermaid
flowchart LR
    protocol["protocol.rs\n(pure prepare/commit/taint/\nabandon/recover)"]
    diff["DurableTransition::between\n(pure structural diff of\nbefore/after ProtocolState)"]
    store["MutationTraceStore\n(SQL translation)"]
    db["RepositoryAgentTraceDb\n(TursoDb<M>)"]

    protocol -->|before, after| diff --> store --> db
```

The boundary is one-directional and structural. `protocol.rs` never depends
on SQL or `RepositoryAgentTraceDb`. `DurableTransition::between` diffs two
`ProtocolState` values field-by-field — it never branches on `Boundary`,
`BoundaryKind`, `Attribution`, or taint state, so it cannot make a persistence
decision based on protocol meaning. `store.rs` never interprets protocol
semantics either: it only translates an already-validated `DurableTransition`
into SQL statements and reconstructs domain values out of query rows.

## What's persisted, and what isn't

Five tables (`mutation_trace_worktrees`, `mutation_trace_scopes`,
`mutation_trace_processed_events`, `mutation_trace_events`,
`mutation_trace_event_active_scopes`) hold `WorktreeState`, `ScopeState`,
`EventKey` replay identity, and historical `MutationEvent`s.

Two `ProtocolState` fields are deliberately never persisted:

- `AttemptState` — explicitly transient in the domain model; no
  `mutation_trace_attempts` table exists.
- `external_taint` — a `database_failure()` cannot use the database it just
  failed against as the authoritative record that the write was uncertain.
  `DurableTransition::between` returns `Ok(None)` for a `database_failure`-only
  transition, so `store.commit` is never even called for it.

`revision` (on both worktree and event rows) is stored as an 8-byte
big-endian `BLOB`, via `encode_revision`/`decode_revision`, never a SQLite
`INTEGER` — every revision column carries
`CHECK (typeof(revision) = 'blob' AND length(revision) = 8)`, so this survives
`u64::MAX` exactly and rejects a same-length `TEXT` value. Every other
enum-shaped column (`ActorKind`, `FailureKind`, `ScopeStatus`, and the
`AttributionKind`/`BoundaryKind` discriminants derived from `Attribution` and
`Boundary`) has an explicit `encode_*`/`decode_*` function pair — none derives
from `Debug` or a serde representation.

## Read path

`MutationTraceStore::load_worktree(worktree, scope, event_key)` is the hot
path: it loads one worktree row, only that worktree's currently `Active`
scopes, plus one optional *effective referenced scope* (from `scope` or
`event_key.scope_id` — the two must agree when both are given), included
regardless of status. It never queries `mutation_trace_events`. A missing
effective scope, a `scope`/`event_key.scope_id` disagreement, or an effective
scope belonging to a different worktree all return `Err` rather than silently
loading, omitting, or reassigning it. The result, `WorktreeProjection`,
widens into a full `ProtocolState` via `into_protocol_state` (with `attempts`,
`mutation_events`, and `external_taint` always empty) so unmodified
`protocol.rs` functions can operate on it.

`MutationTraceStore::load_mutation_event(worktree, revision)` is the separate
cold path: it reconstructs one historical `MutationEvent`, including full
`Attribution`/`Boundary` decoding, by `(worktree_id, revision)`. It is never
called from `load_worktree` or from any hook-boundary path, so a
projection load never pays for the full historical event set.

`MutationTraceStore::load_scope(scope_id) -> Result<Option<ScopeState>>` is the
public single-scope read seam: one `mutation_trace_scopes` row by primary key,
returning the durable `status` / `actor_kind` / `worktree_id`, or `None` when no
row exists. It is a cold path and deliberately the narrowest scope read there
is — it consults neither `mutation_trace_events`,
`mutation_trace_processed_events`, nor the scope's `mutation_trace_worktrees`
row, and it must not widen into a projection; `load_worktree` remains the
projection seam for any caller that needs worktree state alongside a scope.

Unlike `load_worktree`, it **never adjudicates worktree identity**: a scope whose
stored `worktree_id` differs from the caller's own worktree is returned as-is
rather than rejected. The same row is a legitimate read from its owning worktree
and a cross-worktree reference from any other, so the comparison — and the
decision of what a mismatch means — belongs to the caller. `load_worktree`'s
stricter contract is unchanged: an effective referenced scope on another
worktree is still an `Err` there.

## Durable tree-root reads (ref reconciliation)

`load_tree_roots(worktree)` and `load_all_tree_roots()` are two further
cold-path, read-only queries — siblings of `load_mutation_event`, never
reached from `load_worktree` or a hook-boundary path — that expose the set of
Git tree SHAs the mutation-cursor protocol still durably depends on, for the
per-worktree ref-reconciliation pass (see
[`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md)).

- `load_tree_roots(worktree) -> BTreeSet<TreeId>` returns one worktree's roots:
  its `mutation_trace_worktrees.cursor_tree` plus the `before_tree` and
  `after_tree` of every `mutation_trace_events` row for that worktree,
  deduplicated. A worktree with no durable row yields the empty set, not an
  error. Nothing from another worktree, from `mutation_trace_scopes` /
  `mutation_trace_processed_events` / `mutation_trace_event_active_scopes`, or
  from transient `AttemptState` / `external_taint` is ever included.
- `load_all_tree_roots() -> BTreeSet<TreeId>` returns the union of those same
  three `TreeId` columns across **every** worktree, deduplicated; an empty
  repository yields the empty set. This is the reconciler's repository-wide
  retention set: linked worktrees share one Git object database, so a ref
  owned by one worktree may be the last SCE ref protecting a tree only another
  worktree durably requires.

Each query is backed by a **single SQL statement** — a `UNION` of the
`cursor_tree` / `before_tree` / `after_tree` columns
(`SELECT_TREE_ROOTS_BY_WORKTREE_SQL` / `SELECT_ALL_TREE_ROOTS_SQL`) — run
through one `query_map` call, never independent per-table `SELECT`s unioned in
Rust. So the whole root set is read from one coherent database snapshot: a
concurrent mutation-cursor commit that atomically moves `cursor_tree` from `T`
to `X` and inserts `MutationEvent { before_tree = T, after_tree = X }` in the
same transaction cannot expose a torn set that omits `T` — the statement
observes either the pre-commit snapshot (`cursor_tree` still `T`) or the
post-commit snapshot (`before_tree` is `T`). The one-statement property is the
concurrency boundary here, and it is enforced by a regression test, not left
to code review: a `#[cfg(test)]` read-statement counter in `services::db`
(`count_read_statements`) asserts that one `load_tree_roots` / one
`load_all_tree_roots` call issues exactly one `TursoDb` read — splitting
either into a cursor `SELECT` plus an events `SELECT` fails it. A separate
state-transition test only checks that `T` stays a root across an atomic
cursor advance and is explicitly not treated as proof of snapshot isolation.
These are pure reads: no schema change, no migration, no write path.

## Write path

`MutationTraceStore::commit(transition: &DurableTransition) -> Result<CasResult>`
translates the transition into one worktree CAS `UPDATE`, zero or more scope
status `UPDATE`s, an optional processed-event `INSERT`, and an optional
mutation-event `INSERT` plus its active-scope `INSERT`s — all run through
`TursoDb::execute_transactional_cas_batch` inside exactly one
`BEGIN IMMEDIATE` transaction. The worktree `UPDATE`'s `WHERE worktree_id = ?
AND revision = ?` clause is the CAS guard: `execute_transactional_cas_batch`
treats its affected-row count as the outcome (`0` rows → no-op commit,
`CasResult::Conflict`; `1` row → every other statement runs,
`CasResult::Applied`). Every non-guard statement also carries
`expect_rows_affected(1)`, which fails the transaction deterministically on an
unexpected affected-row count. A `(scope_id, event_id)` replay is a distinct
failure path: the processed-event `INSERT`'s `PRIMARY KEY` constraint rejects
it as a SQL error before any row-count check applies. Both paths roll back
the transaction and propagate out of `commit()` as `Err`, never as
`CasResult::Conflict`, and neither is retried unless the underlying error is
`Busy`/`BusySnapshot`.

`execute_transactional_cas_batch` keeps three outcomes distinct: a stale
revision is a `Conflict` that is never retried; a transient DB failure
(`Busy`/`BusySnapshot`) retries the whole transaction from a fresh
`BEGIN IMMEDIATE`; and any other deterministic SQL/constraint failure
propagates out of `commit()` as `Err`, never as `CasResult::Conflict`.

`DurableTransition`'s six fields are private — `between()` is the only way to
construct one outside `store.rs`, so `commit()` can trust the structural
invariants `between()` already proved (single worktree, revision advances by
exactly one, at most one new processed/mutation event) without re-validating
them.

## Initialization

`initialize_worktree`/`register_scope` are idempotent idle-inserts (`INSERT
... ON CONFLICT DO NOTHING`) outside the CAS commit path:
`initialize_worktree` never overwrites an existing cursor, and
`register_scope` requires the referenced worktree to already have a durable
row and never auto-creates it. An existing scope is returned unchanged only
when its stored `worktree_id` and `actor_kind` match the request; a mismatch
returns `Err`.

## Non-goals

- No Git or filesystem I/O — `store.rs` itself calls neither Git nor the
  filesystem. `runtime/git_snapshot.rs` and `runtime/coordinator.rs` (through
  its public `coordinate()` entrypoint) now wire this store together with the
  Git snapshot service (see
  [`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md));
  `store.rs` itself remains unmodified and still performs no Git or
  filesystem I/O of its own.
- No attribution or boundary-kind decisions — `DurableTransition::between`
  and `store.rs` are both structurally blind to protocol meaning.
- No retry-after-`Conflict` loop — `CasResult::Conflict` is returned to the
  caller; retrying with a freshly reloaded revision is the calling adapter's
  responsibility, not this module's. `runtime::coordinator`'s bounded
  CAS-retry loop is now that adapter.
- No row deletion — `store.rs` never deletes a terminal (`Closed`/`Abandoned`)
  scope row or a historical `mutation_trace_events` row; scope garbage
  collection is out of scope. `load_tree_roots` / `load_all_tree_roots` are
  read-only durable-tree queries for ref reconciliation and change nothing
  about this.
