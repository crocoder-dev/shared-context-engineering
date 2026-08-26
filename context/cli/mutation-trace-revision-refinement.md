# Bounded revision refinement (`mutation_trace`)

Part of the [mutation-cursor protocol module](mutation-trace-protocol.md). This file
covers one specific refinement gap between the Quint model and its Rust kernel:
worktree revision counters.

## The gap

`spec/mutation_cursor.qnt` models a worktree's `revision` as `int`
(`spec/mutation_cursor.qnt:39`) — an unbounded integer with no upper limit. The Rust
refinement uses `WorktreeState::revision: u64`. Every action that advances a
worktree's revision by one — `commit`, `taint`, `abandon`, `recover` — must therefore
handle the one case Quint's model cannot even express: a worktree already at
`revision: u64::MAX`.

A raw `revision + 1` at that boundary wraps to `0` in release-mode Rust, which would
silently violate `MutationEventsHavePositiveRevision`,
`MutationEventUniquePerWorktreeRevision`, and every CAS-freshness assumption built on
revision monotonically increasing.

## The refinement

`protocol.rs` defines a private helper:

```rust
fn next_revision(revision: u64) -> Option<u64> {
    revision.checked_add(1)
}
```

Every revision-advancing site routes through it instead of a raw `+ 1`:

- **`commit`** — revision headroom is required only for transitions that actually
  advance the revision:

  ```text
  commit:
      non-Flush                -> headroom required
      Flush with observed change -> headroom required
      Flush with no observed change -> no headroom required, even at u64::MAX

  taint / abandon / recover:
      always advance when they execute -> headroom always required
  ```

  `ResolvedAttempt::evaluate` computes `would_advance_revision` (`!is_flush(boundary) ||
  tree_changed`) before deciding `accepted`, and only requires
  `next_revision(worktree_state.revision).is_some()` when `would_advance_revision` is
  true. A non-`Flush` boundary always advances revision when accepted, so it always
  needs headroom. A `Flush` advances revision only when it observes a real tree change
  (`advances_revision = accepted && (!is_flush(boundary) || observed_change)`), so a
  fresh no-change `Flush` may commit at `revision: u64::MAX` — matching Quint's
  `commitAttempt`, which accepts and commits that case without advancing revision. An
  attempt that *would* overflow is rejected exactly like a stale one: no cursor
  movement, no scope-lifecycle transition, no processed-`EventKey` insertion, no
  `MutationEvent`. This decision is made before `ResolvedAttempt::apply` ever touches
  state, so `commit` never discovers an overflow partway through applying a transition.
  `apply` computes the checked `advanced_revision` only when `evaluation.advances_revision`
  is true (`None` otherwise), reuses it for the worktree update, and reuses it again for
  any emitted `MutationEvent`'s revision field — `changed` can only be true when
  `advances_revision` is also true, so that reuse never has to synthesize a revision
  for a non-advancing commit.
- **`taint`**, **`abandon`**, **`recover`** — each treats `next_revision` returning
  `None` as an additional guarded no-op, alongside their existing existence/
  precondition guards (unknown worktree, already-tainted, non-live scope, and so on).

## Why guard instead of document a precondition

An earlier revision of the refinement matrix stated this only as an assumption
("callers must keep revision `< u64::MAX`") rather than enforcing it. That is unsound:
nothing in the type system or the pure kernel's own logic would have caught a caller
violating it, and a wrap is silent — no panic, no rejected attempt, just a `revision`
that jumps to `0` and re-enables CAS checks that should have stayed permanently stale.
Checked arithmetic makes the boundary executable instead of assumed.

## Test coverage

`tests.rs` has one no-wrap test per unconditionally-advancing action, each starting
from `revision: u64::MAX` and proving the guard holds rather than wrapping:

- `taint_does_not_wrap_revision_at_u64_max`
- `abandon_does_not_wrap_revision_at_u64_max`
- `recover_does_not_wrap_revision_at_u64_max`

`commit` gets two tests, together encoding that headroom is required for
advancement, not for acceptance in general:

- `commit_that_would_advance_is_rejected_at_u64_max` — a `Start` boundary (always
  advances revision when accepted) at `revision: u64::MAX` is rejected, not wrapped.
- `no_change_flush_commits_at_u64_max_without_advancing_revision` — a `Flush`
  boundary observing no tree change at `revision: u64::MAX` commits successfully,
  with `revision` staying at `u64::MAX` and no `MutationEvent` emitted.

## Adapter responsibility

In practice a worktree revision reaching `u64::MAX` is not expected to happen; this
guard exists so that if it ever did, the pure kernel fails safely (a guarded no-op or
rejection) rather than silently corrupting cursor/CAS state. No coordinator/store
behavior is implied by this file beyond passing through whatever the pure kernel
returns — a future adapter that observes a stuck (non-advancing) worktree at
`u64::MAX` would need its own operational response, which is out of scope for the
protocol kernel itself.
