# Mutation-cursor runtime materialization (`mutation_trace`)

How the finite, pre-populated Quint `SCOPES`/`WORKTREES` universes map onto
the Rust protocol's unbounded identity space: the coordinator/store layer
materializes scope and worktree state, and
[`protocol.rs`](mutation-trace-protocol.md) only transitions identities that
already exist.

## Scope materialization

The Quint model's `SCOPES` universe is finite: `init` populates every
possible `ScopeId` with a `ScopeState` up front (`scopes' =
SCOPES.mapBy(scope => { status: NeverSeen, actorKind: scopeActor(scope),
worktreeId: scopeWorktree(scope) })`), so by the time any boundary is
evaluated, `scopeActor`/`scopeWorktree` already resolve for that scope — its
identity is a static fact of the model, not something a transition
establishes.

This module's `ScopeId` is an unbounded runtime string (see "Refinement
decisions vs. the Quint model" in [mutation-trace-protocol.md](mutation-trace-protocol.md)), so `ProtocolState.scopes` cannot be prepopulated with
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

## Worktree materialization

The same representation/refinement boundary applies to `WorktreeId`, one
level up from scope identity, and governs `taint`/`database_failure`/
`abandon`/`recover`:

- **Quint**: `WorktreeId` ranges over the finite `WORKTREES` universe, and
  `init` materializes a `WorktreeState` for every member up front — every
  `WorktreeId` already resolves before any action runs. This is why
  `recordDatabaseFailure` (`spec/mutation_cursor.qnt:808-828`) states no
  explicit worktree-existence guard: there is no state for it to guard
  against. That omission is a fact about the closed, pre-populated Quint
  domain, not evidence that an arbitrary unknown worktree is valid protocol
  input.
- **Rust production**: `WorktreeId` is an unbounded opaque runtime string, so
  `ProtocolState.worktrees` contains only worktrees a future coordinator/
  store layer has actually materialized. Every pure protocol action requires
  its target `WorktreeId` to already exist in `ProtocolState.worktrees`; an
  unknown `WorktreeId` is invalid/unresolved kernel input and causes a
  defensive no-op, exactly as an unregistered `ScopeId` does for `prepare`/
  `commit`. The pure kernel never creates a `WorktreeState`, infers one, or
  synthesizes a worktree from context.

A **missing** `WorktreeId` (absent from `ProtocolState.worktrees`) is not
equivalent to a **healthy** `WorktreeState` (`tainted: false`,
`failure_kind: Healthy`, ...): the former means the protocol has no
materialized state for that identity at all, while the latter means the
identity is known and currently healthy. `taint`, `database_failure`,
`abandon`, and `recover` all enforce this distinction with the same existence
guard — `abandon` resolves it through the referenced scope's own materialized
`worktree_id` rather than taking a `WorktreeId` directly — which keeps
`external_taint ⊆ ProtocolState.worktrees` an invariant of every state this
module can produce, since `database_failure` is the sole path that inserts
into `external_taint`. The concrete runtime refinement of `external_taint` is
the worktree-local `<git-dir>/sce/mutation-cursor-tainted` marker (see
[`mutation-trace-external-taint.md`](mutation-trace-external-taint.md)), armed
write-ahead before Agent Trace DB acquisition and overlaid onto
`database_failure` recovery only when a later invocation inherits it;
`WorktreeProjection::into_protocol_state()` itself always returns an empty
`external_taint`. A pre-protected marker inspect/persist failure means no
mutation boundary committed; a marker-*clear* failure means the boundary already
committed durably — the coordinator surfaces that as
`CoordinateError::MarkerClearAfterCommit`, carrying the committed
`CoordinateOutcome` so no evidence is lost, and leaves the marker armed so the
next invocation still promotes it to protocol `external_taint`.

Future responsibility split (mirrors "Scope materialization" above):
the coordinator/store layer resolves/materializes worktree identity/state and
loads a `ProtocolState`; `protocol.rs` only transitions already-known ones.

## Authoritative source

`spec/mutation_cursor.qnt` (verified Quint model) and `spec/mutation_cursor.md`
remain authoritative; [mutation-trace-protocol.md](mutation-trace-protocol.md)
describes the pure protocol module these rules sit beneath.
