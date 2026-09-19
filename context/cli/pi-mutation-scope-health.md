# Pi mutation-scope health classification

`cli/src/services/hooks/pi_mutation_scope/health.rs` maps the adapter's
persisted `<git-dir>/sce/pi-mutation-scope-state.json` (see
[pi-mutation-scope-integration.md](pi-mutation-scope-integration.md#durable-state))
onto the shared, doctor-facing `healthy | recovering | blocked | invalid`
vocabulary. The classifier is pure and read-only: it never writes state, and
`doctor` wiring is a separate task.

Every mapping below was proven by driving the adapter's real dispatch and
recovery functions (`admit_or_recover`, `reconcile_stale_owners`,
`resolve_recovery`, `normalize_recovery_after_boundary_lock_acquired`), never
inferred from `RecoveryState`/`AttemptPhase` variant names or from the field
name `is_definitely_dead`.

## The mapping

| Persisted shape | Status | Why |
| --- | --- | --- |
| No state file | `healthy` | No adapter attempt has ever run; absence is not a problem. |
| `Clear`, only live- or uncertain-owner `PendingStart`/`Executed` attempts, no `PendingAbandon` | `healthy` | See [Why a sibling's `PendingStart`/`Executed` never makes this `blocked`](#why-a-siblings-pendingstartexecuted-never-makes-this-blocked). |
| `Clear`, at least one dead-owner `PendingStart`/`Executed` attempt, no `PendingAbandon` | `recovering` | See [Why a dead-owner attempt is `recovering`, not `blocked` or `healthy`](#why-a-dead-owner-attempt-is-recovering-not-blocked-or-healthy). |
| `Clear`, any `PendingAbandon` attempt | `invalid` | Structurally impossible through production behavior: a `PendingAbandon` attempt is only ever created by `begin_terminal_cleanup`, which arms `RecoveryState::Flushing` in the same write, and is only ever removed by `resolve_recovery`'s loop, which runs before `complete_recovery_flush` can transition recovery back to `Clear`. A hand-seeded or corrupted file matching this shape is reported `invalid`. |
| `Pending { generation }`, any attempt composition | `recovering` | A recovery-capable fresh tracked `tool_call` — one whose `(session_id, tool_call_id)` key is not already represented by a nonterminal attempt — claims the flush (`AdmitDecision::FlushClaimed`) and retries every currently outstanding `PendingAbandon` attempt through `resolve_recovery`, regardless of which session performs it and regardless of any co-existing `PendingStart`/`Executed` attempt. A duplicate `tool_call` for an already-tracked `PendingStart`/`Executed` key is instead reused idempotently by `admit_tracked_attempt` *before* `RecoveryState` is ever inspected, so not every individual `tool_call` necessarily advances recovery — but an ordinary future fresh `tool_call` has a proven automatic path, which is why this is `recovering` rather than `blocked`. |
| `Flushing { generation }`, any attempt composition | `recovering` | An orphaned `Flushing` (its owning process crashed between `begin_terminal_cleanup` and `resolve_recovery`) is reclaimed to `Pending` by `normalize_recovery_after_boundary_lock_acquired`, which runs at the very start of every tracked adapter boundary — `tool_call`, `tool_execution_end`, and `ToolExecutionAbandon` alike — before anything else. From `Pending`, a subsequent recovery-capable fresh `tool_call` claims and retries it as above; a duplicate `tool_call` for an already-tracked nonterminal key is not guaranteed to be the one that does so. |
| Read/parse error (missing file aside) | `invalid` | Malformed JSON, unsupported version, or a read failure; the error is surfaced in the health record's detail. |

Unlike Codex and OpenCode, Pi's investigation found **no persisted shape
reachable through ordinary production dispatch that classifies `blocked`**.
The reasons are specific to Pi's design and are explained below.

## Why a sibling's `PendingStart`/`Executed` never makes this `blocked`

OpenCode's `admit_tracked_attempt` denies a brand-new admission
(`UncertainAttemptBlocked`) whenever *any* tracked attempt anywhere in state
is `PendingStart`, because OpenCode's `PendingStart` is a narrow
crash-recovery window — see
[opencode-mutation-scope-health.md](opencode-mutation-scope-health.md). Pi's
`PendingStart` is architecturally different
([pi-mutation-scope-integration.md](pi-mutation-scope-integration.md#why-pendingstart-never-blocks-a-sibling-admission)):
it is the adapter's normal, possibly long-lived resting state for a tool
call's entire execution window, so Pi's `admit_tracked_attempt` never
inspects a sibling attempt's `PendingStart`/`Executed` phase when deciding
whether to admit a different key. Pi's only same-state admission gate is a
`PendingAbandon` attempt (`UncertainAttemptBlocked`), and that combination is
itself proven unreachable under `RecoveryState::Clear` (see the mapping
table above), so in practice `UncertainAttemptBlocked` never fires against
production-reachable state.

A live-owner or uncertain-owner attempt (a live pid whose exact
process-instance identity cannot be positively established, per
`is_definitely_dead` in `process_owner.rs`) is therefore ordinary in-flight
state: it never blocks any other admission, and this adapter's D10
stale-owner sweep leaves it completely untouched (proven by
`clear_recovery_is_healthy_with_an_uncertain_owner_pending_start_attempt_never_swept_by_an_unrelated_start`
in `health.rs`'s own test module, and by
`an_owner_that_cannot_be_positively_proven_dead_is_never_abandoned_by_an_unrelated_start`
in `mod.rs`). It is `healthy`, not merely "not yet observed to be a problem."

## Why a dead-owner attempt is `recovering`, not `blocked` or `healthy`

`reconcile_stale_owners` — the D10 sweep — runs unconditionally at the start
of `admit_or_recover`, before that call's own key is even considered, for
*every* tracked `tool_call` from *any* session. It repeatedly collects every
`PendingStart`/`Executed` attempt whose recorded `ProcessOwner` is positively
proven dead (any session, any prior process — not only one matching the
incoming key) and retires each batch together through the existing D8
flush/abandon/flush recovery sequence before the triggering call is admitted.

This is unfinished recovery work — the attempt's owning process is gone and
the scope was never legitimately closed — so it is not `healthy`. But because
the sweep is proven to run automatically on the very next tracked admission
from any session, with no manual state-file surgery required, it has the
proven self-healing path the `recovering` status exists to describe, so it is
not `blocked` either.

Proven in `health.rs`'s own test module by driving real dispatch end to end:

- `clear_recovery_with_a_dead_owner_pending_start_attempt_is_recovering_and_an_unrelated_session_start_sweeps_it_ac4`:
  session A starts, its recorded owner is forced dead, `classify_health`
  reports `recovering`, then an unrelated session B's `tool_call` drives the
  real `flush -> abandon -> flush -> start` sequence and `classify_health`
  reports `healthy` once B is admitted.
- `clear_recovery_with_a_dead_owner_executed_attempt_is_recovering_and_is_swept_without_a_synthetic_close`:
  the same proof for a dead-owner `Executed` attempt, additionally asserting
  no synthetic `close` operation is ever sent for it (D9: the current Git
  tree no longer represents the original terminal-observation time).
- `pending_recovery_from_an_interrupted_dead_owner_sweep_is_recovering_and_denies_the_triggering_start_until_resumed`:
  a transient seam failure mid-sweep leaves recovery durably `Pending` and
  denies the very call that triggered it, but `classify_health` still reports
  `recovering`, and the next tracked `tool_call` resumes and completes the
  interrupted sweep.

## Why `Pending`/`Flushing` are always `recovering`, unrelated attempts included

Unlike Codex, where only a same-`(session_id, turn_id)`-lane successor can
retry a stuck predecessor's abandonment before reaching the global recovery
gate, Pi's `admit_tracked_attempt` lets *any* caller whose key is not already
represented by a nonterminal attempt claim a `Pending` generation's flush,
and `resolve_recovery` retries *every* currently outstanding `PendingAbandon`
attempt in one pass regardless of which session or call originally doomed
it. Combined with the D10 sweep above (which can itself arm or re-arm the
same recovery generation for a dead-owner attempt), every reachable
`Pending`/`Flushing` shape has a proven any-caller resolution path — it is
just not guaranteed to be the very next `tool_call` received, since a
duplicate `tool_call` for an already-tracked `PendingStart`/`Executed` key is
reused idempotently before the recovery-state gate is ever reached (proven
by
`pending_recovery_reuses_a_duplicate_start_for_an_existing_nonterminal_key_without_advancing_recovery_then_a_fresh_start_recovers`
in `health.rs`'s own test module). An unrelated live/uncertain-owner
`PendingStart`/`Executed` attempt sitting alongside such a recovery does not
change this: `resolve_recovery` never inspects it, and (per the section
above) it was never a source of denial for other calls in the first place.

Proven in `health.rs`'s own test module:
`pending_recovery_from_a_failed_terminal_abandon_is_recovering_and_self_heals_on_the_next_start`
(a terminal-abandon seam failure leaves `Pending`, repeated unrelated
`tool_call`s are denied without changing the classification, then a working
seam self-heals) and
`orphaned_flushing_is_recovering_and_reclaimed_by_the_next_boundary` (a
simulated crash between `begin_terminal_cleanup` and `resolve_recovery`
leaves a literal orphaned `Flushing`, reclaimed and resolved by the next
tracked `tool_call`).
