# Mutation-scope ref-reconciliation advisory (hook trigger)

The shared mutation-scope ingress (`cli/src/services/hooks/mutation_scope.rs`,
see [`mutation-scope-hook-ingress.md`](mutation-scope-hook-ingress.md)) emits a
lightweight, advisory-only signal that snapshot-ref reconciliation is
recommended. It never reconciles, lists refs, opens the Agent Trace DB, or spawns
Git. Reconciliation itself is operator-driven; this signal is only a reminder.

## Trigger policy

`drive_mutation_scope` takes an injected advisory seam (production:
`runtime::advise_after_completed_boundary`) and a generic diagnostic writer
`E: Write` (production: `std::io::stderr()`, supplied by
`run_mutation_scope_from_payload`). The seam runs exactly once, and only when all
of these hold:

- the boundary is `Close` or `Flush`;
- the `coordinate_boundary(...)` result is `Ok` or
  `CoordinateError::MarkerClearAfterCommit` (a durably completed boundary).

It runs after the coordinator future has returned, so the coordinator's
`WorktreeLock` is already released. It never runs for `Start`, `Advance`,
`Abandon`, or a failed coordinate. The hook result is computed before the seam
runs and is never altered by it, including when the diagnostic write fails. The
advisory never creates, clears, or modifies the external-taint marker.

## Outcomes, logging and diagnostics

Every outcome is logged as `sce.hooks.mutation_scope.ref_reconciliation_advisory`
with an `outcome` field (plus `persistence_warning` when present). Actionable
outcomes are additionally written, one `SCE: ...` line each, to the diagnostic
writer. Stdout stays empty and never carries advisory text.

| Outcome | Log | Diagnostic (stderr) |
|---|---|---|
| `Anchored` | debug | none |
| `NoAction` | debug | none |
| `Busy` | debug | none |
| `Advised` | warn | `SCE: Reconciliation is recommended. Run sce doctor --fix.` |
| `AdvisedDurabilityUncertain` | warn, with `persistence_warning` | the line above, then `SCE: Advisory-state durability could not be confirmed: <warning>.` |
| `StateWriteFailed` | warn, with `persistence_warning` | `SCE: Reconciliation advisory state could not be written: <warning>. Run sce doctor to check reconciliation status.` (does not claim advice was persisted) |
| `StateUnavailable` | warn | `SCE: Reconciliation advisory state is unavailable. Run sce doctor to check reconciliation status.` |

The diagnostic does not depend on the logger (it is emitted with `logger = None`),
and `Logger::warn` and the observability defaults are unchanged: observability
routes only ERROR records to stderr, and only when file logging is off, so WARN
alone would not reach the operator. A write or flush error on the diagnostic
writer is ignored. Only this path writes the advisory diagnostic to stderr.

## Advisory behavior and cadence

`ref_advisory::advise_if_due` (`cli/src/services/mutation_trace/runtime/`) takes
no Git, DB, store, or snapshot parameters. It derives the Git dir from the
filesystem (no Git subprocess), makes one zero-wait `try_lock` of
`WorktreeLock`, and reads/writes the small per-worktree state file
`<git-dir>/sce/ref-maintenance.json` (capped, versioned, past-event timestamps
only, atomic staging-and-swap writes). It persists `last_advised` before
reporting `Advised`. A busy lock or unreadable state is reported, never an error.

- **Time-based reminders:** the first use anchors the state. A stale state
  becomes eligible after `RECONCILIATION_ADVISORY_AFTER` (24 h); further
  stale-state reminders are suppressed for 24 h after `last_advised`.
- **Failure-triggered reminders:** an outstanding failed explicit reconciliation
  is recommended immediately. A newly recorded failure (recorded after
  `last_advised`) may be advised even inside the previous 24 h window; once
  recorded, that reminder is suppressed. Repeated reminders for the same
  unresolved failure follow the normal 24 h cadence. A successful explicit
  reconciliation clears the failure-based recommendation.
- Anchoring and clock-skew normalization are the only other state writes, so
  state is not written "at most once per window" unconditionally.
- `sce doctor` reports the recommendation from state regardless of whether a hook
  reminder was ever delivered.

## Delivery guarantee and limits

- Best-effort, not exactly-once, and not guaranteed to be seen by a human.
  `last_advised` is persisted before the diagnostic, so a crash or an unseen
  stderr can still consume one reminder; the next reminder follows the cadence.
- Claude Code and Codex run the hook as a command and can capture its stderr, but
  rendering of exit-0 hook stderr is up to the harness. The OpenCode plugin and
  the Pi extension discard the child's stderr, so there the structured log file
  and `sce doctor` are the supported surfaces. Changing the plugins is out of
  scope here.
- Time-based, not evidence-based: orphan detection needs the Git/DB scan this
  path deliberately avoids.
- Orphan pins are reclaimed only by an explicit reconciliation pass. Automatic
  reclamation is not provided.
