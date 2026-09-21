# Claude mutation-scope health classification

`claude_mutation_scope::health::classify_health` is a read-only diagnostic
classifier over the checkout-local adapter state described in
[Checkout-local adapter state](claude-mutation-scope-integration.md#checkout-local-adapter-state).
It returns the shared `healthy | recovering | blocked | invalid` health
vocabulary that `sce doctor` already consumes (see
[`doctor-human-text-contract.md`](../sce/doctor-human-text-contract.md)). It
reads the same `state::read_state` result the adapter itself uses and maps it
as:

| Persisted state | Status | Reason |
| --- | --- | --- |
| `recovery_pending == false` | `Healthy` | no persisted recovery barrier is armed |
| `recovery_pending == true && attempts.is_empty()` | `Recovering` | the recovery barrier's own flush path can clear this automatically |
| `recovery_pending == true && attempts` non-empty | `Blocked` | the recovery barrier's flush path never runs from this shape, and nothing else advances it |
| `read_state` fails (malformed JSON, unsupported version, read error) | `Invalid` | the state cannot be safely interpreted |

**Healthy** covers the absence of a state file (`read_state`'s default) as
well as an explicit `recovery_pending == false`. This is recovery health
specifically, not "no active tool calls" — the adapter may still have live
`attempts` in `pending_start`/`active`/`pending_abandon` phase; live attempts
alone, with `recovery_pending == false`, are still `Healthy`.

**Recovering** (`recovery_pending == true && attempts.is_empty()`) reflects
actual adapter behavior, not the `recovery_pending` name alone: the next
mutation-capable `PreToolUse` reaches
[the recovery barrier](claude-mutation-scope-integration.md#abandonment-cleanup-signals),
finds `attempts` empty, runs `{"operation":"flush"}` through the seam, and —
on success — calls `clear_recovery_pending` before proceeding. This is a
normal, self-healing admission path with no manual intervention.

**Blocked** (`recovery_pending == true && attempts` non-empty) is the exact
shape of the incident that motivated this classifier and the wider
`doctor-mutation-scope-health` plan. `apply_recovery_barrier()` sees
`recovery_pending == true` with non-empty `attempts` and returns `Deny`. From
the ordinary hook lifecycle alone, nothing retries the abandon that left
those attempts stale, removes them, flushes, or clears `recovery_pending` —
the barrier's only self-healing transition (flush) is gated on
`attempts.is_empty()`, which this shape never satisfies. So once the hook
invocation that produced this persisted state has returned, every subsequent
ordinary mutation-capable `PreToolUse` continues to deny without advancing
recovery through that path alone — a durable repository-wide lockout, not
merely "currently denied." The `doctor-mutation-scope-fix` plan adds a
separate, `doctor`-invoked repair path (`assess_repairability`/
`repair_blocked`) for exactly this shape once every outstanding attempt
already carries durable `pending_abandon` evidence; that repair path is not
part of the ordinary hook lifecycle this classifier observes, and is not yet
wired into `sce doctor --fix` as of this classifier's own read-only
behavior.

**Invalid** applies when `state::read_state` cannot safely read or parse the
state file — malformed JSON or an unsupported/invalid persisted version, per
the existing state reader. A fail-closed but structurally valid state (i.e.
`Blocked`) is never reported as `Invalid`.

**Read-only boundary.** This classifier is diagnostic only: it reads the
existing checkout-local adapter state and nothing else. It never modifies
`attempts`, never clears `recovery_pending`, never calls `flush` or
`abandon`, and never alters mutation attribution. Recovery/repair is a
separate concern this classifier does not perform.

**Observation semantics.** Classification is a snapshot of persisted state,
read the same way `state::read_state` reads it for the adapter's own use.
There is a narrow window in which a currently executing hook has already
persisted `recovery_pending = true` with non-empty `attempts` but is still
about to complete its abandon/remove sequence — the classifier does not
attempt to prove global process liveness across that window. `Blocked` means
that, if the operation that produced the observed durable state has stopped
progressing, future ordinary adapter lifecycle events have no self-healing
path from that state — not a claim that no process anywhere could possibly
still be mid-write.

## Related context

- [Claude mutation-scope integration](claude-mutation-scope-integration.md)
- [OpenCode mutation-scope health classification](opencode-mutation-scope-health.md)
- [Pi mutation-scope health classification](pi-mutation-scope-health.md)
