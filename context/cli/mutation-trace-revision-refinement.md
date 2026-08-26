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

- **`commit`** — `ResolvedAttempt::evaluate`'s `accepted` flag folds in
  `next_revision(worktree_state.revision).is_some()`, unconditionally, regardless of
  whether this particular boundary would actually advance the revision (a `Flush`
  observing no change would not). An attempt that would overflow is rejected exactly
  like a stale one: no cursor movement, no scope-lifecycle transition, no
  processed-`EventKey` insertion, no `MutationEvent`. This decision is made before
  `ResolvedAttempt::apply` ever touches state, so `commit` never discovers an overflow
  partway through applying a transition. `apply` computes the checked
  `advanced_revision` once and reuses it for both the worktree update and any emitted
  `MutationEvent`'s revision field.
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

`tests.rs` has one test per revision-advancing action, each starting from
`revision: u64::MAX` and proving the guard holds rather than wrapping:

- `commit_does_not_wrap_revision_at_u64_max` (proves rejection, not a wrap)
- `taint_does_not_wrap_revision_at_u64_max`
- `abandon_does_not_wrap_revision_at_u64_max`
- `recover_does_not_wrap_revision_at_u64_max`

## Adapter responsibility

In practice a worktree revision reaching `u64::MAX` is not expected to happen; this
guard exists so that if it ever did, the pure kernel fails safely (a guarded no-op or
rejection) rather than silently corrupting cursor/CAS state. No coordinator/store
behavior is implied by this file beyond passing through whatever the pure kernel
returns — a future adapter that observes a stuck (non-advancing) worktree at
`u64::MAX` would need its own operational response, which is out of scope for the
protocol kernel itself.
