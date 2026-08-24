# Mutation Cursor model boundary

`mutation_cursor.qnt` is a bounded protocol model, not a production implementation or an exhaustive model of Git.

## Bounded verification domain

The enum values for worktrees, scopes, trees, hook events, and attempts are finite verification identities. They are not runtime limits. The model verifies arbitrary interleavings within this domain; production code must support larger and unbounded identifier spaces.

`ScopeId` is the durable identity of an AI scope/session in this model. `ActorKind` identifies the harness. A separate `SessionId` is unnecessary unless one session can own multiple independent scopes.

## Protocol architecture

The model preserves the protocol boundary:

```text
read durable worktree state at revision R
        ↓
take speculative Git snapshot
        ↓
derive transition
        ↓
DB transaction / CAS
        ↓
fresh → commit
stale → reject/retry
```

`worktreeTrees` is the abstract current worktree tree. Git commands and snapshot mechanics are not modeled.

## Event identity

Hook replay identity is scoped by `ScopeId` and `EventId` through `EventKey`. The real implementation must provide an equivalent uniqueness guarantee. If hook IDs are not unique per scope, the database key must include the actual delivery namespace, such as worktree, harness, session, and hook ID.

## Failure and durability boundary

`worktrees.cursorTree`, `worktrees.revision`, scope state, `processedEvents`, and `mutationEvents` represent state durably stored in the Agent Trace database. `worktrees.needsRebaseline` is a durable protocol marker for an ambiguous cursor interval; it is distinct from both snapshot failure and external database taint.

A snapshot failure occurs while the database is healthy. `taint(worktree)` therefore records `SnapshotFailure` in the durable worktree state, invalidating subsequent speculative attempts until recovery.

Database unavailability is different. `databaseFailure(worktree)` changes only:

```text
externalTaint: Set[WorktreeId]
```

`externalTaint` is the abstract external durability boundary: conceptually, the filesystem `TAINTED` marker that can survive an unavailable database. It is not a database row and does not model marker paths or filesystem syscalls.

Thus the model does **not** perform this contradictory transition:

```text
DB write fails
    ↓
update DB-backed revision or tainted flag
```

Instead:

```text
DB operation fails
    ↓
durable DB protocol state remains unchanged
    ↓
externalTaint contains the worktree
```

While externally tainted, normal attempts cannot commit evidence. Recovery represents the next successful SCE invocation. The same recovery action also handles a healthy worktree marked `needsRebaseline`; in that case it preserves surviving active scopes because only the skipped interval is ambiguous:

```text
observe current worktree
    ↓
establish current tree as the new cursor baseline
    ↓
produce no evidence for the skipped interval
    ↓
clear needsRebaseline
```

For external taint or snapshot failure, recovery retains the stronger existing behavior of abandoning active scopes. The external-taint recovery path is:


```text
observe externalTaint
    ↓
snapshot current worktree
    ↓
establish current tree as the new cursor baseline
    ↓
produce no evidence for the skipped interval
    ↓
abandon every active scope on the worktree
    ↓
commit recovery to DB
    ↓
clear externalTaint
```

Taint or external-taint recovery abandons active scopes because no trustworthy normal close boundary was observed. Healthy `needsRebaseline` recovery instead preserves surviving active scopes: only the ambiguous skipped interval is discarded, and those scopes may resume attribution after the new baseline. No filesystem details, SQLite/Turso internals, retries, or OS crash timing are modeled.

## Scope lifecycle

A scope has one of four statuses:

- `NeverSeen` — no accepted start has been observed;
- `Active` — eligible to contribute to attribution;
- `Closed` — ended at a trustworthy normal close boundary;
- `Abandoned` — ended without a trustworthy final observation boundary.

`Closed` and `Abandoned` are terminal. `Abandon(scope)` changes only an active scope to `Abandoned`; it never reactivates a terminal scope. It increments the worktree revision, leaves the cursor unchanged, and sets `needsRebaseline`. Until recovery establishes a new baseline, normal observations emit no mutation evidence. An abandoned scope must not receive exclusive attribution for the unobserved gap preceding abandonment.

If a new scope starts on the same worktree with the same actor while that actor already has an active scope, the model performs stale-scope rollover atomically:

```text
old same-actor active scopes → Abandoned
observe/rebaseline current worktree conservatively
new scope → Active
```

The old cursor-to-current-tree gap produces no exclusive evidence for the old scope. A different actor does not trigger rollover: existing scopes remain active, and subsequent work is `AiContended` while two or more scopes are active.

Attribution remains:

- zero active AI scopes → `IneligibleUnscoped`;
- one active AI scope → `AiExclusive(scope)`;
- two or more active AI scopes → `AiContended`.

Failure and external-taint states can only weaken attribution to `IneligibleUnscoped`; they never strengthen it.

## Verification properties and scenarios

The model includes safety properties covering:

- standalone abandonment requiring a conservative rebaseline;
- protocol history proving mutation evidence crosses only trustworthy cursor states;
- database failure not mutating durable protocol state;
- external taint not strengthening attribution;
- recovery baseline before clearing external taint;
- recovery and rollover abandoning active scopes;
- closed and abandoned terminality;
- no exclusive evidence for an abandoned unobserved gap;
- same-actor rollover and different-actor contention;
- `AiExclusive` requiring exactly one active scope;
- `AiContended` requiring multiple active scopes;
- CAS/replay safety and cursor/evidence consistency.

Deterministic runs cover database-unavailable state preservation, external-taint recovery, abandoned-scope non-reactivation, same-actor rollover, and different-actor contention.

## Implementation refinement

The Rust/SQL implementation should map these model elements explicitly:

| Model | Implementation responsibility |
| --- | --- |
| `worktrees.cursorTree` | durable per-worktree cursor row |
| `worktrees.revision` | transaction CAS revision |
| `worktrees.tainted` / `failureKind` | durable snapshot-failure state when the DB is healthy |
| `externalTaint` | external durability signal, such as the filesystem taint marker |
| `processedEvents` | durable replay/idempotency key table or column |
| `scopes` | durable scope lifecycle records, including abandonment |
| `attempts` | transient speculative observation state |
| `cursorHistory` | verification ledger; production may use mutation/evidence rows |
| `mutationEvents` | durable mutation evidence and attribution |

The transaction that accepts an attempt is the linearization point: it must validate revision, cursor, and replay identity before writing evidence, advancing the cursor, and changing lifecycle state atomically. Database-unavailable handling is outside that transaction because the transaction cannot update the durable protocol state.
