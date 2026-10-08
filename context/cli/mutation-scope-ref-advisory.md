# Mutation-scope ref-reconciliation advisory (hook trigger)

The shared mutation-scope ingress (`cli/src/services/hooks/mutation_scope.rs`,
see [`mutation-scope-hook-ingress.md`](mutation-scope-hook-ingress.md)) emits a
lightweight, advisory-only signal that snapshot-ref reconciliation is
recommended. It never reconciles, lists refs, opens the Agent Trace DB, or spawns
Git. Reconciliation itself is operator-driven; this signal is only a reminder.

## Trigger policy

`drive_mutation_scope` takes an injected advisory seam (production:
`runtime::advise_after_completed_boundary`). The seam runs exactly once, and only
when all of these hold:

- the boundary is `Close` or `Flush`;
- the `coordinate_boundary(...)` result is `Ok` or
  `CoordinateError::MarkerClearAfterCommit` (a durably completed boundary).

It runs after the coordinator future has returned, so the coordinator's
`WorktreeLock` is already released. It never runs for `Start`, `Advance`,
`Abandon`, or a failed coordinate. The hook result is computed before the seam
runs and is never altered by it: every advisory outcome is only logged as
`sce.hooks.mutation_scope.ref_reconciliation_advisory` with an `outcome` field
(`warn` for `advised`, `advised_durability_uncertain`, `state_write_failed`,
`state_unavailable`; `debug` for `anchored`, `no_action`, `busy`). The advisory
never creates, clears, or modifies the external-taint marker.

## Advisory behavior

`ref_advisory::advise_if_due` (`cli/src/services/mutation_trace/runtime/`) takes
no Git, DB, store, or snapshot parameters. It derives the Git dir from the
filesystem (no Git subprocess), makes one zero-wait `try_lock` of
`WorktreeLock`, and reads/writes the small per-worktree state file
`<git-dir>/sce/ref-maintenance.json` (capped, versioned, past-event timestamps
only, atomic staging-and-swap writes). It anchors on first use and then advises
at most once per `RECONCILIATION_ADVISORY_AFTER` window (24 h), persisting
`last_advised` before reporting `Advised`. A busy lock or unreadable state is
reported, never an error.

## Limits

- Time-based, not evidence-based: orphan detection needs the Git/DB scan this
  path deliberately avoids.
- A reminder is not guaranteed exactly-once; a crash between the state write and
  the log line can lose one.
- Orphan pins are reclaimed only by an explicit reconciliation pass. Automatic
  reclamation is not provided.
