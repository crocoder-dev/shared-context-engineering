# Doctor-repair safety model (`spec/doctor_recovery.qnt`)

Standalone, verified Quint model of the safety pattern behind `sce doctor
--fix`'s repair of a `Blocked` Agent-tracing state (introduced by the
`doctor-mutation-scope-fix` plan's T01; consumed by T03's OpenCode repair,
T04's Claude repair, and T05's doctor orchestration). It does not extend
`spec/mutation_cursor.qnt` — doctor-repair bookkeeping (attempt phases,
adapter-wide recovery/fix state, an abstract owner-liveness oracle, and two
racing actors) is a separate, focused concern from the verified core
mutation-cursor protocol that file owns.

## What it models

Two actors race over one adapter's durable state: the adapter's own hook
process (which allocates, advances, and can itself decide to abandon an
attempt) and `sce doctor --fix` (which may additionally attempt a repair, but
only under positive owner-liveness evidence). A `StateTxnHolder`
(`StateTxnFree`/`DoctorStateTxn`) models the short durable-transaction lock
each real adapter releases before every mutation-scope seam call.

| Abstract concept | Real Claude/OpenCode mapping |
| --- | --- |
| `AttemptPhase = NotAllocated \| PendingStart \| Active \| PendingAbandon \| Removed` | OpenCode's `AdapterAttempt` phase field; Claude's attempt phase, including the `PendingAbandon` phase this plan's T04 added |
| `RecoveryState = Clear \| Pending \| Flushing` (one adapter-wide value, not per-attempt) | OpenCode's `AdapterState.recovery`; Claude's whole-file `recovery_pending` flag/barrier |
| `Health = Healthy \| Recovering \| Blocked \| Invalid` via `adapterHealth(phase, recovery)` | each adapter's own `classify_health`, computed from every attempt's phase plus the one shared recovery fact — never from a single attempt in isolation |
| `FixState = NotAttempted \| RepairAttempted \| RepairCompleted \| ReportedFixed \| ReportedManual` (adapter-wide) | doctor's one fix-result lifecycle per adapter target (`assess_repairability` -> `repair_blocked` -> re-diagnose -> one `DoctorFixResultRecord`) |
| `OwnerReading = SeenAlive \| SeenDead \| SeenUnknown` | `mutation_scope_owner::is_definitely_dead`'s `Alive`/`Dead`/`Unknown` result (PID + `/proc` start-time liveness, never time-based) |
| `stateTxn: StateTxnHolder` | OpenCode's `AdapterStateLock` transaction (and Claude's equivalent short state-file lock) around one re-read/re-prove/durable-transition step — **not** OpenCode's coarser `AdapterBoundaryLock`, which this model has no dedicated variable for (see below) |

`RecoveryState`/`FixState` are single adapter-wide variables, not
per-attempt: both real adapters report one health value and one fix result
per target, and `begin_terminal_cleanup`/`resolve_recovery`-shaped code
processes every `PendingAbandon` attempt in one shared pipeline, not one at a
time. `completeAbandon` only clears `recovery` to `Clear` once no attempt
remains `PendingAbandon`, so a second obligation can join an in-flight
recovery pipeline without disturbing it.

**Scope boundary:** `stateTxn` verifies only the short per-transaction lock.
OpenCode's `AdapterBoundaryLock` — which serializes a repair's whole
lifecycle, seam calls included — is a separate, coarser concern outside this
model's variables; T03 is responsible for acquiring it around the whole
`repair_blocked` call, wrapping (not replacing) the `stateTxn`-shaped
transactions this model verifies. Claude has no equivalent boundary lock in
this plan: its repair safety rests on the durable `PendingAbandon` evidence
itself plus its own state-lock transactions.

## The eight `Safety` invariants

Each is proven, at minimum, by the Rust regression test(s) named — driving
each adapter's real dispatch/recovery functions, not reading enum names.

| Invariant | Rust regression coverage |
| --- | --- |
| `DoctorNeverAbandonsALiveOwner` — doctor cannot abandon a potentially live attempt | `opencode_mutation_scope::health::tests::repair_blocked_is_a_safe_no_op_when_the_pending_start_owner_is_live`, `opencode_mutation_scope::health::tests::assess_repairability_is_manual_only_when_the_pending_start_owner_is_live` |
| `UnknownOwnerNeverProvesDeath` — an unprovable/unknown owner is never proof of death | `opencode_mutation_scope::health::tests::assess_repairability_is_manual_only_for_a_legacy_pending_start_attempt_with_no_recorded_owner`; `doctor::inspect::tests::full_report_multi_adapter_diagnose_names_doctor_fix_for_one_target_and_the_real_path_for_the_other` (an unowned OpenCode `PendingStart` stays `ManualOnly` in the same report where Claude's proven-dead-equivalent state is `AutoFixable`) |
| `RecoveryNeverClearedWithUnresolvedAbandon` — doctor cannot clear recovery while unresolved lifecycle evidence exists | `claude_mutation_scope::health::tests::repair_blocked_removes_only_successfully_abandoned_attempts_and_keeps_recovery_pending_when_one_fails` |
| `NoOrdinaryTransitionProducesInvalidHealth` — `Invalid` is reserved for structurally impossible states, never an ordinary transition's output | `claude_mutation_scope::health::tests::clear_recovery_with_pending_abandon_is_invalid`, `claude_mutation_scope::health::tests::invalid_takes_priority_over_blocked_in_mixed_impossible_state`, `opencode_mutation_scope::health::tests::clear_recovery_with_a_pending_abandon_attempt_is_a_structurally_impossible_state_classified_invalid` |
| `DoctorRepairProducesOnlyOrdinaryLifecycleShapes` — a doctor repair cannot produce a state outside the adapter's own reachable transition set | `opencode_mutation_scope::health::tests::repair_blocked_clears_a_dead_owner_pending_start_end_to_end`; `claude_mutation_scope::health::tests::repair_blocked_removes_only_successfully_abandoned_attempts_and_keeps_recovery_pending_when_one_fails` |
| `RemovedAttemptsAreNeverResurrected` — a terminal/removed attempt is never resurrected | `claude_mutation_scope::state::tests::removing_an_already_removed_attempt_is_a_safe_no_op`, `opencode_mutation_scope::state::tests::removing_an_already_removed_attempt_is_a_safe_no_op`, `opencode_mutation_scope::tests::regression_f_start_replay_for_a_pending_abandon_identity_never_reactivates` |
| `InterruptedRecoveryStaysInOrdinaryRetryableState` — an interrupted repair remains fail-closed and retryable | `claude_mutation_scope::health::tests::repair_blocked_interrupted_by_a_failing_seam_leaves_state_a_later_repair_completes_without_duplication`, `opencode_mutation_scope::health::tests::repair_blocked_interrupted_before_the_seam_resolves_leaves_state_the_ordinary_recovery_path_completes_without_duplication` |
| `ReportedFixedExcludesBlockedOrInvalid` — a reported `fixed` repair cannot coincide with a final `Blocked`/`Invalid` health | `doctor::inspect::tests::finalize_mutation_scope_repair_results_ignores_an_immediate_post_repair_read_that_the_final_report_contradicts`; `doctor::inspect::tests::full_report_multi_adapter_fix_mode_resolves_one_target_and_leaves_the_other_manual` (Claude's `Fixed` report is derived only from the final, freshly recomputed row, at the same moment the adjacent OpenCode row in that identical final report is still `Blocked`) |

## Why no Quint-Connect harness

Unlike [`mutation-trace-quint-connect.md`](mutation-trace-quint-connect.md),
which continuously replays generated Quint traces through a pure Rust
refinement (`mutation_trace::protocol`) of `spec/mutation_cursor.qnt`, no
equivalent trace-replay harness connects `spec/doctor_recovery.qnt` to Rust.
`doctor_recovery.qnt`'s actions (`hookAllocate`, `doctorAttemptRepairWith`,
`recoveryProgress`, `completeAbandon`, ...) correspond to real adapter I/O —
durable state files, OS locks, `/proc` owner liveness, the real
mutation-scope seam — with no equivalent pure core to trace against;
extracting one purely to enable a trace-replay harness would be new
production behavior outside any task's declared scope in this plan. The
table above is the documented-mapping alternative instead, connecting every
`Safety` invariant to at least one real regression test, per the
`doctor-mutation-scope-fix` plan's T07.

## No comments in the model file itself

`spec/doctor_recovery.qnt` carries no header comment, per this repository's
standing no-comments-in-code convention. This file's concept-mapping table
and invariant list are the durable substitute for that header, mirroring the
same choice `spec/mutation_cursor.qnt`'s own authors made.

## Verification

- `nix run .#quint -- typecheck spec/doctor_recovery.qnt`
- `nix run .#quint -- test spec/doctor_recovery.qnt --match '^test.*'`
- `nix run .#quint -- run spec/doctor_recovery.qnt --invariant=Safety --max-samples=20000 --max-steps=50`

No dedicated Nix check currently runs `doctor_recovery.qnt` the way
`checks.mutation-trace-quint-connect` runs the mutation-cursor MBT harness —
the commands above are run manually and their results recorded in the
`doctor-mutation-scope-fix` plan's T01/T07 task records. `spec/` is already
included in the Nix build's fileset (see
[mutation-trace-quint-connect.md](mutation-trace-quint-connect.md)'s "CI: two
Nix checks" section), so `nix flake check`'s existing checks build
successfully alongside this file without needing further Nix wiring.

## Authoritative source

`spec/doctor_recovery.qnt` remains authoritative for the model itself. See
[mutation-scope-health-status.md](../sce/mutation-scope-health-status.md),
[claude-mutation-scope-health.md](claude-mutation-scope-health.md), and
[opencode-mutation-scope-health.md](opencode-mutation-scope-health.md) for
the concrete `classify_health`/`assess_repairability`/`repair_blocked`
implementations this model verifies the safety pattern of. See
`context/plans/doctor-mutation-scope-fix.md` (T01, T07) for build-out status
and verification-run evidence.
