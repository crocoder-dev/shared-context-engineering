# Mutation-scope runtime: the harness-adapter contract

The crate-visible surface of `cli/src/services/mutation_trace/runtime/`, and the
lifecycle contract every harness adapter (Codex, Claude Code, OpenCode, Pi) must
uphold when it drives that surface.

Built by the `mutation-scope-runtime-integration` plan
(`context/plans/mutation-scope-runtime-integration.md`). **No harness is wired to
it yet.** This file is the contract a future adapter is written against, not a
description of shipped adapter behavior.

The mechanics behind each entrypoint live in their own domain files:
[`mutation-trace-runtime-coordinator.md`](mutation-trace-runtime-coordinator.md)
(`coordinate()`),
[`mutation-trace-scope-abandonment.md`](mutation-trace-scope-abandonment.md)
(`abandon_scope()`),
[`mutation-trace-protected-worktree.md`](mutation-trace-protected-worktree.md)
(the shared safety prefix), and
[`mutation-trace-protocol.md`](mutation-trace-protocol.md) (the pure protocol).
This file is the layer above them: what an adapter is required to do, and why.

## The exported seam

`runtime/mod.rs` re-exports exactly nine names at `pub(crate)`, reachable as
`crate::services::mutation_trace::runtime::*`:

| Name | From | Role |
| --- | --- | --- |
| `coordinate` | `coordinator` | the observed-boundary entrypoint |
| `RuntimeBoundary` | `coordinator` | `Start` / `Advance` / `Close` / `Flush` |
| `CoordinateOutcome` | `coordinator` | its success value |
| `CoordinateError` | `coordinator` | its error surface |
| `ExternalTaintOperation` | `coordinator` | `Inspect` / `Persist`, carried by `CoordinateError::ExternalTaintMarker` |
| `abandon_scope` | `scope_runtime` | the unobserved-boundary entrypoint |
| `AbandonScopeOutcome` | `scope_runtime` | its success value |
| `AbandonRecoveryReason` | `scope_runtime` | the reason inside `RecoveryRequired` |
| `AbandonScopeError` | `scope_runtime` | its error surface |

`ExternalTaintOperation` crosses the boundary because it is part of
`CoordinateError`'s own public shape — a crate-visible error a caller cannot
match on is not a usable seam. It lives in `protected_worktree.rs` and reaches
the seam through `coordinator.rs`'s `pub use super::protected_worktree::
ExternalTaintOperation`, so the type becomes crate-visible **without**
`protected_worktree` becoming a public module.

Every `mod` declaration in `runtime/mod.rs` stays private. Nothing else is
reachable from `git_snapshot`, `external_taint`, `worktree_lock`,
`ref_reconciliation`, or `protected_worktree` — in particular `ProtectedWorktree`,
`ProtectedWorktreeError`, and `WORKTREE_LOCK_TIMEOUT` remain internal to
`runtime`, as does `reconcile_worktree`. An adapter drives the runtime only
through the two entrypoints; it never assembles the safety prefix itself.

Both re-export statements carry `#[allow(unused_imports)]`, matching the
repository's existing precedent for a seam whose consumers do not exist yet
(`services/style.rs`, `services/hooks/codex/apply_patch/mod.rs`). The
module-level `#[allow(dead_code)] pub mod mutation_trace;` in `services/mod.rs`
covers unused *items*, not unused re-exports. No placeholder consumer was added
to satisfy `clippy --all-targets -- -D warnings`.

## What a mutation scope is

**A scope is one independently mutation-capable execution.** Not one session,
not one process, not one harness.

The practical consequence: a main agent and a subagent that can each edit the
worktree concurrently are two scopes and must carry **distinct `ScopeId`s**. If
an adapter gives them one shared `ScopeId`, their intervals collapse into a
single exclusivity claim and the protocol can never report `AiContended` for two
executions that genuinely raced.

A `ScopeId` is durably bound to one worktree for life. `abandon_scope()` rejects
a target whose durable `worktree_id` differs from the `WorktreeId` the invocation
derived from its own checkout (`AbandonScopeError::WorktreeIdentityMismatch`),
and writes nothing.

## `Start` / `Advance` / `Close`

Each is a `RuntimeBoundary` passed to `coordinate()`, which captures a Git
snapshot, drives the protocol, and advances the worktree cursor to the observed
tree. The interval between two consecutive observed boundaries is what the
protocol can attribute.

- **`Start { scope, event, actor_kind }`** — the scope's first boundary. The
  protocol observes it only from `ScopeStatus::NeverSeen`; an accepted, observing
  `Start` transitions the scope to `Active`. The event it emits attributes to the
  scopes live *before* the activation, so a `Start` never attributes the
  preceding interval to the scope it is starting.
- **`Advance { scope, event, actor_kind }`** — every subsequent mutation
  boundary. Accepted only while the scope is live.
- **`Close { scope, event, actor_kind }`** — the terminal observed boundary,
  accepted from `NeverSeen` or live, transitioning the scope to `Closed`. Its
  emitted event still attributes to the scope it is closing.
- **`Flush`** — a worktree-level observation carrying no scope and no fields;
  the worktree is the one this invocation derived from its own checkout.

All three scope-carrying variants supply `actor_kind`, and `coordinate()`
registers the scope's durable `(worktree_id, actor_kind)` identity on every one
of them, not only on `Start` — a mismatch against an existing row is
`CoordinateError::ScopeIdentityConflict`.

Two obligations follow, and both are easy to get wrong:

**A failed tool still requires `Advance`.** The boundary marks *an observation of
the worktree*, not a successful edit. A tool that errored may still have written
files (a partial write, a half-applied patch, a script that failed after its side
effect). Skipping the `Advance` does not discard those mutations — it folds them
into the next observed interval, where they are attributed to whatever was live
then. Emit the boundary on failure exactly as on success.

**A `ScopeId` is never reused after a terminal status.** Once a scope is `Closed`
or `Abandoned`, a later `Start` on that same `ScopeId` does not observe — the
`NeverSeen` guard rejects it — so the scope is not reactivated and the boundary
silently fails to establish what the adapter thinks it established. A new
execution always gets a fresh `ScopeId`.

## `abandon_scope()` requires positive staleness evidence

```rust
abandon_scope(repository_root, &scope, open_db)
    -> Result<AbandonScopeOutcome, AbandonScopeError>
```

Abandonment is for a scope the adapter can **prove** is stale: the execution's
final worktree boundary was never observed and never will be. The canonical
evidence is a dead process — a recorded PID that no longer exists, a supervisor
that reports the execution terminated, an explicit harness signal.

**Staleness must never be inferred from `ActorKind`.** `ActorKind` names the
harness that owns a scope (`ClaudeCode`, `Codex`, `OpenCode`, `Pi`). It says
nothing about whether that scope's execution is still running. An adapter that
abandons every scope carrying some other harness's `ActorKind` — or every scope
carrying its own, on startup — destroys live executions' evidence and, through
D1 below, can force recovery that invalidates unrelated live scopes. Absence of
evidence that a scope is alive is not evidence that it is dead.

Abandonment is deliberately **not** a `RuntimeBoundary` variant. It takes no Git
snapshot, moves no cursor, and emits no `MutationEvent`; it only transitions
already-durable state (status → `Abandoned`, `revision` + 1,
`needs_rebaseline = true`). Because it adds no new observation, it refines no new
Quint action and requires **no change to `spec/mutation_cursor.qnt`**.

## The abandon → successor-`Start` sequence

The reason to abandon is almost always to start a successor safely:

```text
abandon_scope(A)  →  coordinate(Start(B))
```

The abandonment sets `needs_rebaseline` on the worktree, so the successor
`Start(B)` re-baselines the cursor to the tree it observes and emits **no**
mutation evidence for the ambiguous A→B interval. That interval is discarded
rather than misattributed — which is the entire point.

What each outcome implies for that successor:

| Outcome | Durable effect | Then what |
| --- | --- | --- |
| `Abandoned { revision }` | A is `Abandoned`, revision +1, `needs_rebaseline` set | proceed to `Start(B)`; the gap is discarded |
| `AlreadyTerminal { status, revision }` | none — A was already `Closed`/`Abandoned` | proceed to `Start(B)`; a successful no-op |
| `RecoveryRequired { reason }` | none; the external-taint marker stays armed | see D1 — the next `coordinate()` performs the stronger inherited-taint recovery |
| `Err(_)` | none, or none beyond a settled outcome carried in the error | **do not treat this as a safely started successor** |

That last row is the one that matters. A failed abandonment means the stale scope
may still be `Active`, so starting B anyway leaves the zombie live alongside it:
the interval before `Start(B)` can still be claimed `AiExclusive(A)`, and every
overlapping interval afterwards becomes `AiContended` against a scope whose
execution is dead. An adapter must surface the failure, not paper over it by
starting the successor.

`AbandonScopeError::MarkerClearAfterCompletion { source, completed }` is the one
error that carries an already-settled outcome: the durable transition **did**
succeed and only the trailing marker clear failed. Read `completed` rather than
retrying the abandonment.

## D1: a missing or `NeverSeen` target forces conservative strong recovery

`abandon_scope()` on a `ScopeId` with no durable row (`MissingScope`), or one
whose row is still `NeverSeen` (`NeverSeenScope`), returns `RecoveryRequired` and
**leaves the external-taint marker armed**. The next `coordinate()` on that
worktree therefore performs *inherited-taint* recovery — and `protocol::recover`
abandons **every** live scope on a worktree recovering from external taint, not
only the scope the adapter named.

```text
execution lifecycle not durably observed
        ↓
filesystem interval may contain unknown mutations
        ↓
cannot safely preserve exclusive attribution assumptions
        ↓
force conservative recovery
```

A missing row means the scope's `Start` never committed while the execution may
well have run and edited files; a `NeverSeen` row means the identity exists but
no accepted `Start` was ever observed for it. Neither proves the execution
mutated nothing, and the runtime cannot bound what happened inside that interval.

**The tradeoff, stated plainly:** this is a false-negative cost. Legitimately
live scopes on the same worktree can be abandoned by a recovery they did nothing
to cause, and the evidence for their in-flight intervals is discarded. That cost
is accepted because the alternative is a false positive — attributing an interval
exclusively to a scope while an unobserved execution may have been mutating the
same worktree. **Attribution safety outranks preserving potentially valid
evidence.**

This is why an adapter must not call `abandon_scope()` speculatively on
`ScopeId`s it cannot vouch for: the cost of a wrong guess is paid by other,
healthy scopes.

It is not in tension with the ordinary `Abandoned` outcome, whose
`needs_rebaseline`-only recovery preserves live scopes by design. D1 covers only
the recovery-required outcomes, where the stronger recovery is the whole point.

## The `AiExclusive` attribution boundary

`Attribution::AiExclusive(scope)` means **exactly one mutation scope was live
over that interval**. It is a statement about scope exclusivity, and nothing
more.

It is **not** standalone proof that no human edited the worktree. A developer
typing in their editor while an agent's scope is live produces mutations inside
that interval, and the protocol will still label the interval
`AiExclusive(scope)` — the runtime observes trees, not authorship. `AiContended`
likewise means two or more scopes overlapped, not that two humans disagreed.

Consumers building human-vs-AI authorship claims need evidence beyond this
signal; the protocol deliberately does not supply it. The complementary states
are `AiContended` (more than one live scope) and `IneligibleUnscoped` (no live
scope, or the worktree is unhealthy, externally tainted, or needs rebaseline).

## Status

The seam is exported and the contract is recorded. No harness hook, plugin,
extension, or command calls either entrypoint yet; each harness's concrete
`ScopeId` / `EventId` format and its stale-process detection are still open, as
is any repository-scoped cleanup of unowned checkout identities.
