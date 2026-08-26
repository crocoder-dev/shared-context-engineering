# Mutation-cursor protocol module (`mutation_trace`)

Pure Rust refinement of the verified `spec/mutation_cursor.qnt` protocol, living
at `cli/src/services/mutation_trace/`. It is not yet wired into any hook,
command, or database call site; that integration is out of scope for the
`mutation-cursor-protocol-kernel` plan and is left for a later plan.

## Current state

Domain types, `prepare`/`commit` transition logic, attribution/mutation-event
materialization, and snapshot-failure/database-failure taint actions exist so
far (`mutation-cursor-protocol-kernel` plan, tasks T01-T04). `types.rs`
defines the protocol's state (including the `ProtocolState` aggregate) and
pure accessors; `protocol.rs` implements `prepare` and `commit` (all four
boundary kinds — `Start`/`Advance`/`Close`/`Flush` — in one pass), refining
`prepareAvailable`/`prepare`/`commitAttempt`, `live_scopes_on`/
`attribution_for`, refining `liveScopesOn`/`attributionFor`, and `taint`/
`database_failure`, refining `taintHealthy`/`taint`/`recordDatabaseFailure`/
`databaseFailure`. Scope abandonment, recovery, and cross-action test
coverage land in later tasks of the same plan. Registered in
`cli/src/services/mod.rs` with `#[allow(dead_code)]`, matching the existing
precedent for modules not yet consumed by production call sites
(`bash_policy`, `repository_identity`, `agent_trace_export`).

`commit` materializes exactly one `MutationEvent` into `mutation_events` when
`changed` is true, with `active_scopes`/`attribution` computed by
`live_scopes_on`/`attribution_for` against the state as it existed *before*
the same call's own scope-lifecycle transition — a `Start` boundary's emitted
event never attributes the mutation to the scope it is about to activate, and
a `Close` boundary's emitted event still attributes to the scope it is about
to close.

## Module layout

- `mod.rs` — public module boundary and module-level doc comment.
- `types.rs` — state/domain types and pure accessors (`ProtocolState`,
  `WorktreeState`, `ScopeState`, `AttemptState`, `MutationEvent`, `Boundary`,
  `Attribution`, and the identity/status/failure-kind types they compose
  from).
- `protocol.rs` — pure transition logic: `prepare` (refining
  `prepareAvailable`/`prepare`) and `commit` (refining `commitAttempt`),
  returning a `CommitOutcome` that pairs the resulting `ProtocolState` with a
  `CommitEvaluation` (`accepted`/`observes`/`observed_change`/`changed`/
  `advances_revision`); `live_scopes_on` and `attribution_for` (refining
  `liveScopesOn`/`attributionFor`), each callable standalone or via `commit`'s
  internal `MutationEvent` materialization; `taint` (refining
  `taintHealthy`/`taint`) and `database_failure` (refining
  `recordDatabaseFailure`/`databaseFailure`), each a guarded no-op action
  independent of `prepare`/`commit`.
- `tests.rs` — `#[cfg(test)]` coverage for the current slice, sibling to
  `mod.rs`.

The module performs no Git, database, filesystem, environment, network,
async, or lock I/O: `types.rs` and `protocol.rs` only ever receive and
return plain domain values — `prepare` takes the currently observed tree as
an explicit `TreeId` parameter rather than reading Git itself; `commit`
operates on the tree already captured in the prepared `AttemptState`
(`before_tree`/`after_tree`) and takes no tree input of its own.

## Refinement decisions vs. the Quint model

`spec/mutation_cursor.md` states the model's enumerated identities
(`WorktreeId`/`ScopeId`/`TreeId`/`EventId`/`AttemptId`) are bounded
verification domains only, and that "production code must support larger and
unbounded identifier spaces." This module refines each as an opaque
`String`-wrapping newtype rather than a fixed enum.

Two consequences follow from that choice:

- The Quint functions `scopeWorktree`/`scopeActor` are pure `match` tables
  only because the model's `ScopeId` enum is pre-associated with a fixed
  worktree/actor. Since `ScopeState` already carries `worktree_id`/
  `actor_kind` fields, this module refines them as accessor methods on
  `ScopeState` instead of a lookup over `ScopeId`.
- `Boundary::Start`/`Advance`/`Close` carry only `scope`/`event`, exactly
  like the Quint constructors (`spec/mutation_cursor.qnt:31-35`) — no
  independent `worktree` field, so a boundary can never claim a worktree
  inconsistent with its own scope's true assignment, a state the Quint type
  cannot represent. `boundary_worktree(boundary, scopes: &BTreeMap<ScopeId,
  ScopeState>)` resolves a hook boundary's worktree by reading the `ScopeId`
  out of the boundary and looking up that exact key in `scopes`, mirroring
  how `commitAttempt`/`prepareAvailable` (`spec/mutation_cursor.qnt:418,458`)
  resolve it from `scopeWorktree(data.scope)` rather than from the boundary
  itself. The Rust refinement does not accept an arbitrary `ScopeState`
  alongside a boundary: the boundary's own `ScopeId` is the only key ever
  used to look one up, preserving the Quint relationship
  `scopeWorktree(boundary.scope)`. The result is `None` when that key is
  absent from `scopes`; `Flush` carries its worktree directly and does not
  consult `scopes` at all.
- `boundary_scope`/`boundary_event`/`boundary_event_key` return
  `Option<_>` (`None` for `Flush`) rather than mirroring the Quint model's
  arbitrary `Scope0`/`Event0` placeholder default.

`ActorKind` and `FailureKind` stay fixed Rust enums: unlike the identity
types, they represent real, closed sets (supported harnesses; snapshot
health), not bounded verification domains.

## Runtime scope materialization

The Quint model's `SCOPES` universe is finite: `init` populates every
possible `ScopeId` with a `ScopeState` up front (`scopes' =
SCOPES.mapBy(scope => { status: NeverSeen, actorKind: scopeActor(scope),
worktreeId: scopeWorktree(scope) })`), so by the time any boundary is
evaluated, `scopeActor`/`scopeWorktree` already resolve for that scope — its
identity is a static fact of the model, not something a transition
establishes.

This module's `ScopeId` is an unbounded runtime string (see "Refinement
decisions" above), so `ProtocolState.scopes` cannot be prepopulated with
every possible scope the way `init` does. Materializing a newly observed
scope's durable identity — `status: NeverSeen`, its `actor_kind`, and its
`worktree_id` — is therefore an **adapter/store responsibility, not a
protocol transition**:

- Quint: a finite universe means every `ScopeState` value already exists at
  `init`.
- Rust production: an unbounded identifier space means `ScopeState` is
  lazily materialized by the persistence/adapter layer *before* the scope's
  `ScopeId` is ever passed into `prepare`/`commit`.

Before invoking the pure protocol with a hook boundary (`Start`/`Advance`/
`Close`) that references a `ScopeId`, the surrounding coordinator/store
projection must ensure that scope already exists in `ProtocolState.scopes`.
`prepare`/`commit` do not infer identity from hook context, command type, or
any other heuristic: they never choose a default worktree, choose a default
actor, or synthesize a new `NeverSeen` scope. `boundary_worktree` returning
`None` for an unregistered scope, and `prepare`/`commit`'s resulting no-op,
are exactly this boundary — a missing `ScopeId` is unresolved protocol
input, not a scope the protocol may create.

### Identity immutability

Once a `ScopeId` is materialized, its `actor_kind` and `worktree_id` are
immutable identity facts for the lifetime of that scope. Only lifecycle
`status` transitions, exactly as the protocol already governs:

```text
NeverSeen -> Active -> Closed
NeverSeen -> Closed
Active -> Abandoned
```

If a future adapter observes an existing `ScopeId` with a conflicting
`actor_kind` or `worktree_id`, that is an identity/protocol error to reject
and report — never a record to silently overwrite. This is the concrete
adapter-side half of `ScopeActorIdentityIsStable` (`spec/mutation_cursor.qnt`);
the protocol-side half is that no transition in `protocol.rs` ever writes
`actor_kind`/`worktree_id` (only `status` fields change).

### Missing scope vs. `NeverSeen` scope

These are not equivalent:

- A **missing** `ScopeId` (absent from `ProtocolState.scopes`) means its
  identity has not been materialized — invalid/unresolved protocol input.
- An **existing** `ScopeState { status: NeverSeen, .. }` is a known,
  materialized scope identity that simply has not yet had an accepted
  `Start`.

The production entry path never calls `prepare`/`commit` with the first
case; the no-op behavior for a missing scope is a defensive kernel property,
not a path the coordinator is expected to exercise.

## Target end-state architecture

The plan's file split anticipates three later seams this module does not yet
implement, recorded here so a later plan does not have to rediscover the
layout:

```mermaid
flowchart LR
    coordinator["coordinator.rs\n(imperative shell:\nDB load, Git snapshot,\nCAS/retry, persist)"]
    protocol["protocol.rs\n(pure transitions —\nprepare/commit/attribution/\ntaint exist; abandon/\nrecovery land later)"]
    git_snapshot["git_snapshot.rs\n(isolated Git object store,\ntemporary index, tree capture/diff)"]
    store["store.rs\n(cursor/revision, scopes,\nprocessed events, mutation\nevidence, CAS transaction)"]

    coordinator --> protocol
    coordinator --> git_snapshot
    coordinator --> store
```

Each seam's responsibility, once built:

- **`coordinator.rs`** — receives hook/session identity, resolves the scope's
  actor/worktree identity, asks `store.rs` to load or materialize the scope,
  obtains a `ProtocolState`, and calls the pure protocol.
- **`store.rs`** — loads durable scope records; atomically creates a new
  scope record as `NeverSeen` when appropriate; never remaps `actor_kind`/
  `worktree_id` for an existing `ScopeId` (see "Runtime scope
  materialization" above).
- **`protocol.rs`** — assumes referenced scopes are already represented in
  `ProtocolState.scopes`; validates and transitions lifecycle state only.

`protocol.rs` stays free of any Git object, DB row, or CAS transaction
concept, and gains no such dependency as later tasks in this plan fill in its
remaining abandonment/recovery logic; `coordinator.rs`, `git_snapshot.rs`,
and `store.rs` are not created by this plan.

## Authoritative source

`spec/mutation_cursor.qnt` (verified Quint model) and `spec/mutation_cursor.md`
(model-boundary and implementation-refinement notes) remain the authoritative
description of protocol behavior; this module's doc comments cite concrete
spec line ranges per type/function. See `context/plans/mutation-cursor-protocol-kernel.md`
for current build-out status across tasks.
