# Mutation Cursor model boundary

`mutation_cursor.qnt` is a bounded protocol model, not a production implementation or an exhaustive model of Git.

## Bounded verification domain

The enum values for worktrees, scopes, trees, hook events, and attempts are finite verification identities. They are not runtime limits. The model verifies arbitrary interleavings within this domain; production code must support larger and unbounded identifier spaces.

`ScopeId` is the durable identity of an AI scope/session in this model. `ActorKind` identifies the harness. A separate `SessionId` is unnecessary unless one session can own multiple independent scopes.

## Event identity

Hook replay identity is scoped by `ScopeId` and `EventId` through `EventKey`. The real implementation must provide an equivalent uniqueness guarantee. If hook IDs are not unique per scope, the database key must include the actual delivery namespace, such as worktree, harness, session, and hook ID.

## Recovery policy

Recovery is conservative. It establishes a new cursor baseline, clears the failure state, and closes every active scope on that worktree. A fresh scope is required before exclusive AI attribution can resume. This prevents mutations made before recovery from being inherited by an old scope.

## Failure abstraction

`SnapshotFailure` and `DatabaseFailure` both taint the worktree and invalidate speculative attempts. They intentionally share the same attribution consequence: evidence can become unscoped, but never stronger. Concrete filesystem, Git, SQLite, and retry mechanics remain outside this model.

## Implementation refinement

The Rust/SQL implementation should map these model elements explicitly:

| Model | Implementation responsibility |
| --- | --- |
| `worktrees.cursorTree` | durable per-worktree cursor row |
| `worktrees.revision` | transaction CAS revision |
| `processedEvents` | durable replay/idempotency key table or column |
| `scopes` | durable scope lifecycle records |
| `attempts` | transient speculative observation state |
| `cursorHistory` | verification ledger; production may use mutation/evidence rows |
| `mutationEvents` | durable mutation evidence and attribution |

The transaction that accepts an attempt is the linearization point: it must validate revision, cursor, and replay identity before writing evidence, advancing the cursor, and changing lifecycle state atomically.
