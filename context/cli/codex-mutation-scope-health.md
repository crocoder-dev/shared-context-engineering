# Codex mutation-scope health classification

`cli/src/services/hooks/codex_mutation_scope/health.rs` maps the adapter's
persisted `<git-dir>/sce/codex-mutation-scope-state.json` (see
[codex-mutation-scope-integration.md](codex-mutation-scope-integration.md#recovery-and-durable-state))
onto the shared, doctor-facing `healthy | recovering | blocked | invalid`
vocabulary. The classifier is pure and read-only: it never writes state, and
`doctor` wiring is a separate task.

Every mapping below was proven by driving the adapter's real dispatch and
recovery functions (`admit_tracked_attempt`, `sweep_stale_lane_predecessors`,
`cleanup_attempts_matching`, `normalize_recovery_after_boundary_lock_acquired`),
never inferred from `RecoveryState` variant names.

## The mapping

| Persisted shape | Status | Why |
| --- | --- | --- |
| No state file | `healthy` | No adapter attempt has ever run; absence is not a problem. |
| `Clear`, any attempts | `healthy` | Live `PendingStart`/`Active` attempts are ordinary in-flight lifecycle state, not a recovery condition; new tracked admission still proceeds outside the exact same `(session_id, turn_id)` lane. |
| `Pending { generation }`, attempts empty | `recovering` | The next tracked `PreToolUse`, from any session, claims the flush (`AdmitDecision::FlushClaimed`) and clears recovery on success. |
| `Pending { generation }`, attempts non-empty | `recovering` | See [Why non-empty `Pending` is recovering, not blocked](#why-non-empty-pending-is-recovering-not-blocked). |
| `Flushing { generation }`, attempts empty | `recovering` | The only reachable path into `Flushing` always leaves attempts empty. An orphaned `Flushing` (its owning process gone) is reclaimed to `Pending` by `normalize_recovery_after_boundary_lock_acquired` on the next process to acquire the boundary lock, then retried as a fresh flush. |
| `Flushing { generation }`, attempts non-empty | `invalid` | Structurally impossible through production behavior: the only transition into `Flushing` requires attempts to already be empty, and `admit_tracked_attempt` refuses every new attempt while `Flushing`. A hand-seeded or corrupted file matching this shape is reported `invalid`, not `blocked`. |
| Read/parse error (missing file aside) | `invalid` | Malformed JSON, unsupported version, or a read failure; the error is surfaced in the health record's detail. |

## Why non-empty `Pending` is recovering, not blocked

This is the shape of the incident this feature exists to surface (see the
`doctor-mutation-scope-health` plan's change summary), and Codex can reach it
the same way Claude can: `abandon_attempt` calls `state::arm_recovery` *before*
invoking the ingress `abandon` seam, then removes the attempt only after the
seam call succeeds. If the seam call fails — during `PostToolUse` close,
`Stop`/`Interrupt`/`SubagentStop`/`SessionEnd` cleanup, or same-lane
predecessor sweeping — the error propagates and `remove_attempt` never runs,
leaving `Pending { generation }` with the stale attempt still present.

The production `PreToolUse` order is:

```text
with_boundary_lock
  -> normalize_recovery_after_boundary_lock_acquired
  -> sweep_stale_lane_predecessors
  -> admit_or_recover
      -> admit_tracked_attempt
```

The global `RecoveryBlocked` decision lives inside `admit_tracked_attempt`,
which `admit_or_recover` calls only *after* `sweep_stale_lane_predecessors`
has already run for this call. So a normal future tracked `PreToolUse` sharing
the stuck attempt's exact `(session_id, turn_id)` lane retries the stale
predecessor's abandonment (via the ordinary same-lane sweep) *before* it ever
reaches the recovery barrier. If that retried abandon succeeds, the stale
attempt is removed, `attempts` becomes empty, and the same call's own
`admit_tracked_attempt` invocation observes `Pending` with no attempts,
claims the flush (`AdmitDecision::FlushClaimed`), and — once the flush seam
call succeeds — clears recovery and admits itself. That is an existing,
ordinary, automatic recovery path, proven by driving it end to end; it is not
inferred from the enum variant name.

This does not mean the state is harmless in the meantime. Until that same-lane
successor arrives (or the stuck attempt's own turn produces another matching
lifecycle event), every *unrelated* tracked `PreToolUse` — a different session,
or the same session with a different `turn_id` — still reaches
`admit_tracked_attempt` with `attempts` non-empty and is denied with
`RecoveryBlocked`. The classifier's job is to distinguish "this persisted
state has a proven normal self-healing route" (`recovering`) from "no future
ordinary event can advance it" (`blocked`); it is not to claim every future
call will succeed. Non-empty `Pending` satisfies the former, not the latter,
so it is `recovering`.

Proven in `cli/src/services/hooks/codex_mutation_scope/mod.rs`:

- `pending_non_empty_recovery_denies_unrelated_admission_without_losing_the_recovery_path`:
  a failed abandon during `SessionEnd` cleanup leaves the state in this shape,
  `classify_health` reports `recovering`, and two successive real `PreToolUse`
  calls from an unrelated session are both denied without the ingress seam
  ever being invoked — `recovering` does not mean unrelated admission
  succeeds, only that a proven self-healing route exists.
- `same_lane_successor_retries_abandon_and_reaches_healthy_after_a_failed_lifecycle_abandon_ac4`:
  the real same-lane self-healing route that invalidates the old `blocked`
  classification. A tracked attempt `A` is stranded by a failed abandon; a
  later same-lane `PreToolUse` `B` retries the abandon and also fails closed
  (`recovering` persists); a further same-lane `PreToolUse` `C`, through the
  ordinary same-lane sweep this time succeeding, retries and clears `A`'s
  abandon, claims and completes the flush, and is itself admitted and started
  — driving the real `abandon(A) -> flush -> start(C)` seam-call ordering and
  ending with `classify_health` reporting `healthy`.
