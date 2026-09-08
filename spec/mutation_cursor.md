# Mutation Cursor model boundary

`mutation_cursor.qnt` is a bounded protocol model, not a production implementation or an exhaustive model of Git.

## Bounded verification domain

The enum values for worktrees, scopes, trees, hook events, and attempts are finite verification identities. They are not runtime limits. The model explores arbitrary interleavings within this finite domain up to the configured verification depth; production code must support larger and unbounded identifier spaces. CI's symbolic verification uses representative subsets of attempts, events, and trees (`Attempt0..2`, `Event0..2`, and `Tree0..2`) to reduce symmetric search; general simulation and deterministic tests retain access to the complete canonical domains.

CI selects `verifyStep`, which includes only enabled protocol actions, excludes no-op mutations, and omits explicit stuttering. The canonical `step` remains available for unrestricted simulation. Because the CI identity subsets are smaller, the bounded CI claim applies to those representative domains rather than every canonical identity.

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

`worktrees.cursorTree`, `worktrees.revision`, scope state, `processedEvents`, and `mutationEvents` represent state durably stored in the Agent Trace database. `worktrees.needsRebaseline` is a durable protocol marker for an ambiguous cursor interval; it is distinct from both snapshot failure and external database taint. Verification-only histories and attempt bookkeeping are intentionally excluded from database-failure checkpoints.

A snapshot failure occurs while the database is healthy. `taint(worktree)` therefore records `SnapshotFailure` in the durable worktree state and increments the worktree revision. This invalidates speculative attempts that were already prepared before the failure. It does not quarantine later attempts: once a subsequent snapshot is prepared against the tainted state and the normal freshness checks pass, it may advance the cursor. Because failure states weaken attribution, any evidence emitted while tainted is `IneligibleUnscoped` until recovery.

Database unavailability is different. `databaseFailure(worktree)` changes only:

```text
externalTaint: Set[WorktreeId]
```

`externalTaint` is the abstract external durability boundary: conceptually, the filesystem `TAINTED` marker that can survive an unavailable database. It is not a database row and does not model marker paths or filesystem syscalls. Its concrete refinement in the CLI is the worktree-local marker file `<git-dir>/sce/mutation-cursor-tainted`, armed write-ahead at the start of the protected runtime section — before Agent Trace DB acquisition — and cleared only after a proven durable completion; it becomes protocol `externalTaint` (overlaid onto `databaseFailure` recovery) only when a later invocation inherits it. If that trailing clear fails *after* the boundary has committed durably, the CLI keeps the marker armed and returns the committed outcome inside a distinct error rather than reporting a boundary failure, so the next invocation still recovers conservatively. See `context/cli/mutation-trace-external-taint.md`.

The SCE-owned Git snapshot refs that protect durable cursor/evidence trees
(`refs/sce/mutation-cursor/<worktree-id>/<tree-sha>`) are **never modeled**.
They are reclaimed by an imperative per-worktree maintenance pass that deletes
only a ref whose tree is a durable root of no worktree in the repository; that
pass deletes **only SCE's own refs, never Git objects directly**, and Git
performs object garbage collection itself on its normal schedule. See
`context/cli/mutation-trace-ref-reconciliation.md`.

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

For external taint or snapshot failure, recovery retains the stronger existing behavior of abandoning active scopes. The recovery path is:

```text
observe taint or externalTaint
    ↓
snapshot current worktree
    ↓
establish current tree as the new cursor baseline
    ↓
produce no evidence for the recovery baseline
    ↓
abandon every active scope on the worktree
    ↓
commit recovery to DB
    ↓
clear the taint/failure state and externalTaint
```

Taint or external-taint recovery abandons active scopes because no trustworthy normal close boundary was observed. Healthy `needsRebaseline` recovery instead preserves surviving active scopes: only the ambiguous skipped interval is discarded, and those scopes may resume attribution after the new baseline. No filesystem details, SQLite/Turso internals, retries, or OS crash timing are modeled.

## Scope lifecycle

A scope has one of four statuses:

- `NeverSeen` — no accepted start has been observed;
- `Active` — eligible to contribute to attribution;
- `Closed` — ended at a trustworthy normal close boundary;
- `Abandoned` — ended without a trustworthy final observation boundary.

`Closed` and `Abandoned` are terminal. `Abandon(scope)` changes only an active scope to `Abandoned`; it never reactivates a terminal scope. It increments the worktree revision, leaves the cursor unchanged, and sets `needsRebaseline`. Until recovery establishes a new baseline, normal observations emit no mutation evidence. An abandoned scope must not receive exclusive attribution for the unobserved gap preceding abandonment.

Starting a new scope never infers that an existing scope is stale from `ActorKind`. Existing active scopes remain active regardless of harness type, and the new scope becomes active independently:

```text
existing active scopes → remain Active
new scope → Active
```

`ScopeId` is the session/scope identity. If a real session is stale, production must establish that through an explicit session/process/generation guarantee and invoke abandonment or recovery; harness type alone is not sufficient. Until then, subsequent work is `AiContended` while two or more scopes are active.

Attribution is computed for the transition observed *at a boundary*, and is:

- any unconfirmed live Codex scope on the worktree → `IneligibleUnscoped`;
- otherwise zero active AI scopes → `IneligibleUnscoped`;
- otherwise one active AI scope → `AiExclusive(scope)`;
- otherwise two or more active AI scopes → `AiContended`.

Failure and external-taint states can only weaken attribution to `IneligibleUnscoped`; they never strengthen it.

## Unconfirmed Codex scopes

A Codex mutation scope's `Start` is a write-ahead admission boundary. It records that SCE established the scope before the harness's aggregate pre-tool decision was known — not that the tool ultimately executed. An arbitrary third-party sibling pre-tool hook can deny the execution after SCE's own `Start` succeeded, and the harness exposes no aggregate-denial signal, so the resulting scope state is indistinguishable from a genuinely running one.

A live Codex scope is therefore **unconfirmed** at every boundary except its own `Close`. Its `Close` is driven by the post-tool signal, which a denied tool never reaches, so that boundary is positive evidence the tool actually executed. One boundary closes at most one scope, so any *other* live Codex scope stays unconfirmed even there.

While a worktree has any unconfirmed live Codex scope, the whole transition is `IneligibleUnscoped` — the uncertain scope is not merely dropped from the live set and the remaining scopes attributed, because that would still be a positive attribution claim made under incomplete knowledge. `MutationEvent.activeScopes` still records the complete actual live set; only attribution eligibility changes.

This deliberately produces a false negative (real contention reported as ineligible) rather than a false positive (a zombie scope reported as contending or exclusive).

## Verification properties and scenarios

The model includes safety properties covering:

- standalone abandonment requiring a conservative rebaseline;
- protocol history proving mutation evidence crosses only trustworthy cursor states;
- database failure not mutating durable protocol state;
- external taint not strengthening attribution;
- recovery baseline before clearing external taint;
- recovery abandoning active scopes;
- closed and abandoned terminality;
- same-actor and different-actor contention;
- `AiExclusive` requiring exactly one active scope;
- `AiContended` requiring multiple active scopes;
- no positive attribution while an unconfirmed live Codex scope exists;
- a boundary that does not confirm a live Codex scope never contending with it;
- a second live Codex scope suppressing attribution even at a confirming `Close`;
- CAS/replay safety and cursor/evidence consistency.

Deterministic runs cover database-unavailable state preservation, external-taint recovery, abandoned-scope non-reactivation, same-actor and different-actor contention, an unconfirmed Codex scope blocking cross-harness contention, a `Flush` never confirming a Codex scope, a Codex `Close` confirming both exclusive and contended attribution, a second live Codex scope suppressing a confirming `Close`, and a terminal Codex scope not suppressing later attribution.

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
