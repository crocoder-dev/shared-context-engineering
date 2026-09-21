# OpenCode mutation-scope health classification

`cli/src/services/hooks/opencode_mutation_scope/health.rs` maps the adapter's
persisted `<git-dir>/sce/opencode-mutation-scope-state.json` (see
[opencode-mutation-scope-integration.md](opencode-mutation-scope-integration.md#adapter-lifecycle-and-recovery))
onto the shared, doctor-facing `healthy | recovering | blocked | invalid`
vocabulary. The classifier is pure and read-only: it never writes state, and
`doctor` wiring is a separate task.

Every mapping below was proven by driving the adapter's real dispatch and
recovery functions (`admit_tracked_attempt`, `establish_tracked_start`,
`resolve_recovery`, `normalize_recovery_after_boundary_lock_acquired`,
`begin_terminal_cleanup`), never inferred from `RecoveryState`/`AttemptPhase`
variant names.

## The mapping

`has_pending_start` and `has_pending_abandon` below mean "at least one
persisted attempt is in that phase," independent of how many other attempts
exist in other phases (including each other). The classifier's evaluation
order matters: a `PendingAbandon` under `Clear` is checked first (it is
structurally impossible, so it wins as `invalid` even if a `PendingStart` is
also present), then `PendingStart` is checked *regardless of `RecoveryState`*
(so it takes precedence over `Pending`/`Flushing` recovery, not just over
`Clear`), and only once both attempt-phase checks are exhausted does the
`RecoveryState` alone decide `healthy` vs. `recovering`.

| Persisted shape | Status | Why |
| --- | --- | --- |
| No state file | `healthy` | No adapter attempt has ever run; absence is not a problem. |
| `Clear`, no `PendingStart`, no `PendingAbandon` (attempts empty or only `Active`) | `healthy` | Live `Active` attempts are ordinary in-flight lifecycle state, not a recovery condition. |
| An attempt is `PendingStart`, for *any* `RecoveryState` (`Clear`, `Pending`, or `Flushing`, with or without a concurrently outstanding `PendingAbandon`) | `blocked` | See [Why a stale `PendingStart` is blocked, not recovering — even alongside a Pending/Flushing recovery generation](#why-a-stale-pendingstart-is-blocked-not-recovering--even-alongside-a-pendingflushing-recovery-generation). |
| `Pending { generation }`, no `PendingStart` (attempts empty or only `PendingAbandon`) | `recovering` | See [Why `Pending` is recovering when no `PendingStart` is outstanding](#why-pending-is-recovering-when-no-pendingstart-is-outstanding). |
| `Flushing { generation }`, no `PendingStart` (attempts empty or only `PendingAbandon`) | `recovering` | Every `PreToolUse`-equivalent boundary (`ToolExecuteBefore`, `ShellEnv`, `ToolExecuteAfter`, `ToolError`) calls `normalize_recovery_after_boundary_lock_acquired` under the same boundary lock before doing anything else, which unconditionally reclaims an observed `Flushing` back to `Pending`. A live in-progress flush is never observable at rest (the boundary lock is held for the whole flush); only an orphaned one (crashed mid-flush) is ever seen by doctor. The next tracked boundary reclaims it to `Pending`; a subsequent recovery-capable tracked admission then claims that pending generation and retries `resolve_recovery`. |
| `Clear`, an attempt is `PendingAbandon` (with or without a `PendingStart` also present) | `invalid` | Structurally impossible through production behavior: `PendingAbandon` is set only inside `begin_terminal_cleanup`, which atomically arms `Flushing` in the same write; recovery only returns to `Clear` via `complete_recovery_flush`, which runs only after every `PendingAbandon` attempt in `resolve_recovery`'s loop has already been abandoned and removed. A hand-seeded or corrupted file matching this shape is reported `invalid`. |
| Read/parse error (missing file aside) | `invalid` | Malformed JSON, unsupported version, or a read failure; the error is surfaced in the health record's detail. |

### The state matrix in full

The classifier's two boolean attempt-phase facts (`has_pending_start`,
`has_pending_abandon`) times the three `RecoveryState` variants give twelve
combinations. `Active` attempts never affect the answer, so they are omitted
below. Reachability was checked by driving the real dispatcher, not inferred.
All twelve cells are additionally locked down as a completeness proof by
`health_classification_matrix_covers_all_twelve_recovery_and_attempt_phase_combinations`
in `cli/src/services/hooks/opencode_mutation_scope/health.rs`, which asserts
the classifier's output for every row against hand-built state (including an
always-present `Active` attempt, proving it never changes the result); the
real-dispatch regressions below separately prove *why* the semantically
meaningful equivalence classes have those classifications by driving
production code, not just the classifier's output:

| `RecoveryState` | `PendingAbandon`? | `PendingStart`? | Reachable in production? | Status | What advances it |
| --- | --- | --- | --- | --- | --- |
| `Clear` | no | no | yes (idle / only `Active`) | `healthy` | n/a |
| `Clear` | no | yes | yes | `blocked` | only that exact call's own `ToolExecuteAfter`/`ToolError`, or a duplicate `Start` redelivery — nothing else |
| `Clear` | yes | no | **no** (structurally impossible) | `invalid` | n/a — hand-seeded/corrupted only |
| `Clear` | yes | yes | **no** (structurally impossible, same reason) | `invalid` | n/a — hand-seeded/corrupted only |
| `Pending` | no | no | yes (abandon succeeded, rebaseline flush then failed) | `recovering` | next tracked admission retries the rebaseline flush |
| `Pending` | yes | no | yes (ambiguity flush or abandon itself failed) | `recovering` | next tracked admission retries `resolve_recovery` for every `PendingAbandon` attempt |
| `Pending` | no | yes | yes (as above, with an unrelated stale `PendingStart` also outstanding) | `blocked` | recovery itself still advances and can reach `Clear`, but the `PendingStart` survives it — see the critical regression below |
| `Pending` | yes | yes | yes (this task's critical regression) | `blocked` | same as above: `resolve_recovery` clears the `PendingAbandon`, `PendingStart` is untouched |
| `Flushing` | no | no | yes (orphaned crash mid-flush, no doomed attempts left) | `recovering` | next tracked boundary reclaims `Flushing` → `Pending`; a subsequent recovery-capable tracked admission claims that generation and runs `resolve_recovery` |
| `Flushing` | yes | no | yes (orphaned crash mid-flush) | `recovering` | same reclaim-then-claim-and-retry path |
| `Flushing` | no | yes | yes (orphaned crash mid-flush, unrelated stale `PendingStart`) | `blocked` | recovery is reclaimed and can still clear, `PendingStart` does not |
| `Flushing` | yes | yes | yes (this task's Case B) | `blocked` | same as the `Pending`/mixed row: recovery clears, `PendingStart` survives |

The four `blocked` rows are one equivalence class under a single invariant:
`resolve_recovery` (`cli/src/services/hooks/opencode_mutation_scope/mod.rs`)
only ever iterates attempts already in `PendingAbandon`; it never inspects or
retires a `PendingStart`. So whenever any `PendingStart` is outstanding,
*no* value of `RecoveryState` — `Clear`, `Pending`, or `Flushing`, and
regardless of whether a `PendingAbandon` is also outstanding — has a future
ordinary event that retires it. The classifier therefore checks
`has_pending_start` as one condition spanning all of `RecoveryState`, not as
a per-recovery-state special case; see
[`classify_health`](../../cli/src/services/hooks/opencode_mutation_scope/health.rs).

## Why `Pending` is recovering when no `PendingStart` is outstanding

Unlike Codex's same-lane-scoped sweep, OpenCode's recovery retry is not scoped
to any particular session or call. `admit_tracked_attempt`'s `Pending { g }`
arm unconditionally transitions to `Flushing { g }` and returns
`AdmitDecision::FlushClaimed`, regardless of which call is being admitted or
whether any `PendingAbandon` attempts are outstanding:

```text
establish_tracked_start
  -> with_boundary_lock
      -> normalize_recovery_after_boundary_lock_acquired
      -> admit_or_recover
          -> admit_tracked_attempt   (Pending{g} -> Flushing{g}, FlushClaimed)
          -> resolve_recovery        (flush, abandon every PendingAbandon, flush, complete)
```

`resolve_recovery` re-reads the persisted attempts and retries `abandon` for
**every** attempt still in `PendingAbandon`, not just one tied to the admitting
call. So the very next tracked admission — from any session, any call — always
attempts full recovery resolution. If every seam call in that retry succeeds,
recovery clears to `Clear`, the stale attempts are removed, and the admitting
call itself proceeds. If any step fails, `relinquish_recovery_flush` returns
the state to `Pending` and the call is denied, but the same automatic retry
fires again on the next tracked admission. This holds whether `attempts` is
empty (a prior rebaseline-flush failure after a successful abandon, e.g. an
interrupted `regression_c`-shaped sequence) or non-empty with only
`PendingAbandon` attempts (an abandon itself failed, e.g.
`regression_a`/`regression_b`-shaped sequences): either way, a normal future
tracked admission has a proven, existing path to advance recovery itself
all the way to `Clear`.

That guarantee is about `RecoveryState` alone, though, and it is **not**
sufficient by itself to guarantee a return to normal admission: `PendingStart`
is a second, independent durable-wedge condition that `resolve_recovery`
never inspects (see
[Why a stale `PendingStart` is blocked, not recovering](#why-a-stale-pendingstart-is-blocked-not-recovering--even-alongside-a-pendingflushing-recovery-generation)
below). So this section's claim is scoped precisely: `Pending`/`Flushing`
recovery is `recovering` only when no `PendingStart` attempt is also
outstanding. When one is, the classifier reports `blocked` even while
recovery is actively advancing (or has already reached `Clear`) for an
unrelated `PendingAbandon`, because reaching `Clear` on the recovery
generation does not by itself restore normal admission.

Proven in `cli/src/services/hooks/opencode_mutation_scope/health.rs`:

- `pending_recovery_with_pending_abandon_attempts_is_recovering_and_an_unrelated_admission_clears_it`:
  a failed `abandon` during `ToolError` cleanup leaves `Pending` with a
  `PendingAbandon` attempt and no `PendingStart`; `classify_health` reports
  `recovering`; two further admissions under the same still-failing seam are
  denied without changing the classification; then an unrelated call, once
  the seam succeeds, clears the stale attempt and is itself admitted.
- `pending_recovery_with_empty_attempts_is_recovering_and_the_next_admission_clears_it`:
  a successful `abandon` followed by a failed rebaseline `flush` leaves
  `Pending` with `attempts` already empty; the next unrelated call's admission
  still resolves it.
- `orphaned_flushing_with_pending_abandon_attempts_is_recovering_and_reclaimed_by_the_next_boundary`:
  a `Flushing` state seeded to simulate a crash mid-flush, with no
  `PendingStart` present, is reclaimed to `Pending` and retried by the next
  tracked admission.

## Why a stale `PendingStart` is blocked, not recovering — even alongside a Pending/Flushing recovery generation

This is the OpenCode-specific shape of the incident this feature exists to
surface (see the `doctor-mutation-scope-health` plan's change summary).
`AttemptPhase::PendingStart` sits outside the `RecoveryState` machine
entirely: `admit_tracked_attempt`'s `RecoveryState::Clear` arm denies every
*new* admission with `AdmitDecision::UncertainAttemptBlocked` whenever any
attempt is still `PendingStart`, and — critically — `resolve_recovery`'s
retry loop (the mechanism that drives `Pending`/`Flushing` back to `Clear`)
only ever iterates attempts already in `AttemptPhase::PendingAbandon`; it
never inspects or retires a `PendingStart` attempt, regardless of which
recovery generation is in flight or whether that generation is for a
completely unrelated call. Nothing in the adapter clears a `PendingStart`
attempt on behalf of an unrelated call. The only two paths that retire a
`PendingStart` attempt are keyed to that exact `(session_id, call_id)`:

- a duplicate `ToolExecuteBefore` redelivery for the same key (idempotent
  replay, not a recovery mechanism), or
- that same key's own `ToolExecuteAfter`/`ToolError`, which drives
  `abandon_and_consume` for that specific attempt.

If the process that owns that call has died before either of those arrives, no
future *ordinary* lifecycle event from any other call can ever clear it —
`server_disposed_cannot_sweep_another_processes_attempt` proves `ServerDisposed`
is deliberately inert for this, and (unlike Pi's D10 sweep) nothing in the
`ToolExecuteBefore`/`ToolExecuteAfter`/`ToolError`/`ShellEnv` dispatch path ever
inspects owner liveness on behalf of an unrelated call. This is the same
"valid persisted state + fail-closed admission + no reachable self-healing
transition" shape as the Claude incident, just triggered by `PendingStart`
instead of a non-empty `attempts` list under `recovery_pending`. OpenCode does
now record the same positive owner evidence as Pi (via the shared
`mutation_scope_owner` module) and can prove a `PendingStart` attempt's owner
positively dead — see
[Owner evidence and the doctor-repair path](opencode-mutation-scope-adapter-lifecycle.md#owner-evidence-and-the-doctor-repair-path)
— but that liveness check is not wired into this ordinary hook lifecycle at
all; it backs a separate `doctor`-invoked repair path only.

**This holds even when recovery is simultaneously `Pending` or `Flushing` for
an unrelated `PendingAbandon` attempt.** Recovery reaching `Clear` retires
only the `PendingAbandon` attempts it processed; it says nothing about any
`PendingStart` attempt that was never in its retry loop. A state can
therefore make visible progress on one axis (recovery generation advancing,
even completing) while remaining durably wedged on the other (an unrelated
`PendingStart` that never gets swept). The classifier's ordering reflects
this: `has_pending_start` is checked as a single condition spanning every
`RecoveryState` value, not nested inside the `Clear` arm.

Proven in `cli/src/services/hooks/opencode_mutation_scope/health.rs`:

- `pending_start_with_clear_recovery_is_blocked_and_denies_repeated_unrelated_admissions_ac4`:
  a `Start` whose seam call fails leaves the attempt `PendingStart` with
  recovery `Clear`; `classify_health` reports `blocked`; two further
  admissions from unrelated calls, under a since-healthy seam, are both
  denied without self-clearing; and `classify_health` still reports `blocked`
  afterward.
- `pending_recovery_with_a_pending_start_attempt_is_blocked_even_though_an_unrelated_pending_abandon_can_still_clear_ac4`
  (the critical regression this task exists to fix): call A starts and is
  then abandoned via `ToolError`; its terminal cleanup arms recovery
  `Flushing`, but the ambiguity flush fails and `relinquish_recovery_flush`
  leaves it `Pending`, with A `PendingAbandon`. Independently, call B's own
  `Start` seam failed earlier, leaving B `PendingStart` under what was then
  `Clear` recovery. The persisted state — `Pending` recovery, A
  `PendingAbandon`, B `PendingStart` — is asserted `blocked`, not
  `recovering`. An unrelated call C then triggers real recovery resolution:
  `resolve_recovery` abandons A and drives recovery to `Clear`, but C is
  still denied `UncertainAttemptBlocked` because B is untouched; the
  persisted state is asserted to be `Clear` + B still `PendingStart`, and
  `classify_health` still reports `blocked`. A further unrelated call D is
  denied again, proving the adapter does not self-clear.
- `orphaned_flushing_with_a_pending_start_attempt_is_blocked_not_recovering_ac4`
  (the `Flushing` analog / Case B): the same shape, but the crash is
  captured as an orphaned `Flushing` (via a direct `begin_terminal_cleanup`
  call simulating a crash before `resolve_recovery` ever ran — the only way
  to observe a literal `Flushing` at rest, since every in-process caller
  calls `resolve_recovery` immediately afterward under the same boundary
  lock). `classify_health` reports `blocked` while `Flushing`; a subsequent
  unrelated call reclaims `Flushing` → `Pending`, resolves A's recovery to
  `Clear`, and is itself denied because B's `PendingStart` survives;
  `classify_health` still reports `blocked` afterward.
