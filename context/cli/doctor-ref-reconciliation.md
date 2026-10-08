# Doctor snapshot-ref reconciliation

`sce doctor --fix` is the only production caller of snapshot-ref reconciliation.
Plain `sce doctor` only reports whether a pass is recommended. Automatic
reclamation is not delivered; it is a documented follow-up (see
[`mutation-scope-ref-advisory.md`](mutation-scope-ref-advisory.md) and
[`mutation-trace-ref-reconciliation.md`](mutation-trace-ref-reconciliation.md)).

## Runtime facade

`cli/src/services/mutation_trace/runtime/ref_doctor.rs` is the only crate-visible
surface doctor uses; it keeps the maintenance types private to `runtime`:

- `run_explicit_reconciliation(repository_root)` wraps `reconcile_explicit`
  (one `WorktreeLock` acquisition, verified-existing Agent Trace DB, state
  recorded under the same lock) and returns a plain `ReconciliationFix`.
- `inspect_reconciliation_recommendation(repository_root)` derives the Git dir
  from the filesystem, reads `<git-dir>/sce/ref-maintenance.json`, and applies the
  pure `evaluate_recommendation`. It takes no lock and writes no state or ref. It
  returns `None` unless reconciliation is recommended or the state is unusable.

Doctor receives both as injected seams (`execute_doctor_with_lifecycle_providers`).

## `--fix` reporting

After the existing lifecycle, config and mutation-scope repairs, `--fix` always
runs one pass, regardless of the recommendation, failure streak, or timestamp
validity, and appends one fix-result row (category `mutation_scope_health`):

| Pass result | Outcome | Detail |
| --- | --- | --- |
| `Completed` | `fixed` | deleted / retained / locally required counts |
| `CompletedStatePersistFailed` | `fixed` | counts plus a diagnostics-not-recorded warning |
| `Failed` | `failed` | typed kind (for example `agent_trace_db_missing`, `agent_trace_db_incompatible_schema`, `agent_trace_db_missing_metadata`, `agent_trace_db_repository_mismatch`, `malformed_ref`, `missing_required_pins`, `durable_roots_unavailable`, `delete_transaction_failed`) and message, plus any state warning |
| `Skipped(Busy)` | `skipped` | another boundary or pass holds the worktree lock; retry |

A busy pass leaves state unchanged and is not a failure. While a pass runs, a
mutation boundary may wait up to the 10 s coordinator lock timeout.

## Plain `sce doctor`

When reconciliation is recommended (stale since the last success, an outstanding
failed attempt, invalid timestamps, or an unreadable state file) the text report
adds a `Snapshot ref reconciliation` section and the JSON report carries a
`ref_reconciliation` object (`null` otherwise). The recommendation does not depend
on whether a hook reminder was ever seen.
