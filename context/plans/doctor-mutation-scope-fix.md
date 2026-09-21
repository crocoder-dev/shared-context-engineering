# Plan: doctor-mutation-scope-fix

## Change summary

The completed `doctor-mutation-scope-health` plan gave `sce doctor` read-only
visibility into each adapter's persisted mutation-scope runtime state,
reporting `healthy | recovering | blocked | invalid` under the human-facing
"Agent tracing" row (internal contract: `mutation_scope_health`). It
deliberately added no repair path: a `blocked`/`invalid` state is
`manual_only`, and `sce doctor --fix` cannot touch it. The original Claude
incident that motivated that plan — a repository-wide `PreToolUse` lockout
from `recovery_pending = true` with a leftover `attempts` entry after a
failed abandon — is now *visible* in `sce doctor`, but still requires manual
state-file surgery to clear, and doctor gives the operator nowhere useful to
look.

This plan extends `sce doctor --fix` so that a `Blocked` Agent tracing state
can be repaired automatically, but only when the owning adapter can prove
the repair is safe from positive evidence — never a timestamp, file age, or
generic "clear the state" fallback. Health (`healthy/recovering/blocked/invalid`)
and repairability (`auto_fixable`/`manual_only` once `Blocked`) are modeled
as separate facts, exactly as the existing four-way status vocabulary
already separates "is this blocked" from "can doctor fix it."

Code inspection (not the two prior plans' documentation) establishes the
actual scope:

- **Codex** (`codex_mutation_scope::health::classify_health`) can never
  reach `Blocked` today — every reachable non-`Healthy` shape is
  `Recovering` (same-lane sweep) or `Invalid` (unreachable-flush, malformed).
  No adapter change is needed or proposed for Codex.
- **Pi** (`pi_mutation_scope::health::classify_health`) can never reach
  `Blocked` either — its D10 dead-owner sweep (`reconcile_stale_owners`)
  already retires a dead-owner `PendingStart`/`Executed` attempt on the next
  tracked `Start` from any session, so the only non-`Healthy` shapes are
  `Recovering`/`Invalid`. No adapter change is needed for Pi's own recovery
  behavior. Pi's `process_owner.rs` (PID + `/proc` start-time liveness,
  never TTL-based) is the one proven positive-evidence primitive this
  repository already has, and this plan extracts it into shared adapter
  infrastructure so OpenCode can reuse it rather than re-inventing it.
- **OpenCode** (`opencode_mutation_scope::health::classify_health`) reaches
  `Blocked` whenever any attempt is `PendingStart` — no boundary in the
  current adapter ever sweeps a stale `PendingStart` on behalf of an
  unrelated call (unlike Pi's D10 sweep or Codex's same-lane retry), and
  `AdapterAttempt` records no owner evidence today, so nothing can currently
  distinguish a crashed owner from a live one. This plan adds
  backward-compatible owner evidence and a repair path that only fires when
  the owner is positively, re-provably dead.
- **Claude** (`claude_mutation_scope::health::classify_health`) reaches
  `Blocked` whenever `recovery_pending == true` with non-empty `attempts` —
  proven by `claude_mutation_scope::health::tests::stale_non_empty_attempts_after_a_failed_abandon_stays_blocked_across_repeated_pre_tool_use_ac3`,
  which drives a real failed `abandon_attempt` seam call. `abandon_attempt`
  already runs only when Claude's own runtime has decided an attempt should
  be abandoned (a failed close, `PermissionDenied`, or a broad cleanup
  signal); the failure mode is that the seam's `abandon` call itself did not
  confirm, not that the decision was unsound. Claude's own persisted state
  has no phase-level distinction between "abandonment already decided,
  awaiting a retry" and "may still be running" — this plan adds one
  (`PendingAbandon`) so a doctor repair can safely retry exactly the
  already-established terminal cleanup, without inventing a new decision.

Both real repair paths reuse each adapter's own existing recovery protocol
operations (OpenCode's `flush`/`abandon`/`flush` sequence; Claude's
`abandon_attempt` seam call), but their synchronization boundaries are
adapter-specific. OpenCode's existing `AdapterBoundaryLock` serializes the
complete adapter lifecycle boundary across concurrent OpenCode processes;
its `AdapterStateLock` protects individual durable state transactions and is
never held across a mutation-scope seam call. The intended OpenCode repair
shape is: acquire the boundary lock, normalize orphaned recovery state,
perform state transactions that re-read and re-prove the dead owner,
persist `PendingStart -> PendingAbandon`, and establish/claim the recovery
generation,
release the state lock, run the normal `flush`/`abandon`/`flush` seam protocol,
record progress/completion in further individual state transactions, then
release the boundary lock.

Claude has no corresponding boundary lock in this plan. Its durable
`PendingAbandon` state is the safety evidence: a state transaction re-reads
and proves that the attempt is already `PendingAbandon`, releases the state
lock, retries the `abandon` seam, and uses another state transaction to
record completion. In both adapters, a stale lock-free diagnosis never
authorizes mutation; the repair function performs a fresh proof inside the
adapter's existing lifecycle-serialization boundary where one exists, while
every durable transition is protected by the state lock. Neither adapter
holds its state lock while calling a mutation-scope seam operation, and doctor
never performs `remove_attempt()`/direct JSON rewriting itself.

## Acceptance criteria

- [ ] AC1: For a `Blocked` Agent tracing row an adapter proves `auto_fixable`,
      plain `sce doctor` (no `--fix`) states the literal remediation
      `Run 'sce doctor --fix' to recover ...` in both human text and the
      JSON `problems[]` remediation text, and the `DoctorProblem` carries
      `fixability: auto_fixable` / `next_action: doctor_fix`.
  - Validate: a doctor test seeds an `AutoFixable`-repairable Blocked
    OpenCode or Claude state and asserts the rendered text and
    `--format json` payload both contain the literal string
    `sce doctor --fix` and `"fixability":"auto_fixable"` /
    `"next_action":"doctor_fix"`.
- [ ] AC2: For a `Blocked`/`Invalid` row that remains `manual_only`, both
      human text and JSON name the exact persisted adapter state-file path
      (e.g. `<git-dir>/sce/claude-mutation-scope-state.json`) and never
      suggest deleting it.
  - Validate: a doctor test seeds a `ManualOnly` Blocked/Invalid state and
    asserts the rendered text and JSON both contain the real
    `state::state_path(...)` value and that no rendered remediation string
    contains `delete`.
- [ ] AC3: `sce doctor --fix` never abandons or repairs an attempt without
      positive evidence freshly re-read and re-proven inside the adapter's
      existing lifecycle-serialization boundary where one exists, and every
      durable state transition is performed under the adapter state lock.
      OpenCode's boundary is explicitly its `AdapterBoundaryLock`; Claude's
      primary repairable proof is the durable `PendingAbandon` state and its
      state transition, not a state lock held across the seam. Adapter state
      locks are never held across mutation-scope seam calls. No repair path
      infers staleness from a timestamp, file modification time, or elapsed
      duration.
  - Validate: `grep -rn "SystemTime\|Instant::now\|\.elapsed()\|modified()" cli/src/services/hooks/claude_mutation_scope cli/src/services/hooks/opencode_mutation_scope cli/src/services/hooks/mutation_scope_owner.rs` finds no staleness use outside unrelated lock-timeout constants; the concurrent-race regressions in T03/T04 pass.
- [ ] AC4: A legacy persisted state file written before this change (no
      owner evidence, no `PendingAbandon` phase) remains readable and stays
      `manual_only` when `Blocked` — upgrading SCE never makes an
      old, ambiguous attempt auto-fixable on its own.
  - Validate: T03/T04 legacy-fixture regression tests pass.
- [ ] AC5: An interrupted repair (a crash or process kill between any two
      durable writes the repair introduces) leaves state that a later
      `sce doctor --fix` completes safely, without ever requiring state
      deletion and without resurrecting or duplicating a terminal/removed
      attempt.
  - Validate: T03/T04 crash-mid-repair regression tests pass.
- [ ] AC6: `sce doctor --fix` never reports a mutation-scope fix result
      `fixed` while the freshly recomputed final `classify_health` result
      for that target remains `Blocked`/`Invalid`; a final result of
      `Recovering` is accepted as a successful repair (`Blocked` ->
      `Recovering` counts as removing the durable wedge).
  - Validate: T05 postcondition regression tests, including one whose
      repair only reaches `Recovering` (a normal residual recovery step
      remains) and one where the repair leaves the target still `Blocked`
      (must not report `fixed`).
- [ ] AC7: The `mutation_scope_health` JSON array's shape
      (`target`/`status`/`reason`/`detail`) and the `healthy`/`recovering`/
      `blocked`/`invalid` status strings are unchanged; no `MutationScope*`
      Rust type or the `mutation_scope_health` field name is renamed.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`; inspect the JSON payload in that test output for the unchanged array shape and status strings.
- [ ] AC8: The stated safety invariants (doctor cannot abandon a
      potentially live attempt; an unprovable/unknown owner is never
      treated as proof of death; doctor cannot clear recovery while
      unresolved lifecycle evidence exists; concurrent hook-process and
      doctor execution cannot abandon a live attempt; a terminal/removed
      attempt is never resurrected; an interrupted repair stays fail-closed
      and retryable; a reported successful repair cannot leave
      `health == Blocked`) are stated formally and connected to the Rust
      implementation.
  - Validate: `nix run .#quint -- typecheck spec/doctor_recovery.qnt && nix run .#quint -- test spec/doctor_recovery.qnt`; T07's connection tests pass.

### Full validation

- `nix flake check`

### Context sync

- `context/sce/mutation-scope-health-status.md` (must describe the new
  repairability facet — `auto_fixable` becoming reachable for `Blocked`,
  the adapter-owned `assess_repairability`/`repair_blocked` boundary, and
  that a `Blocked` problem record may now dynamically change fixability)
- `context/sce/agent-trace-hook-doctor.md` (the `--fix` execution contract
  must describe the new adapter-owned mutation-scope repair step and where
  it sits in the initial-diagnosis -> existing-repairs -> final-diagnosis
  flow)
- `context/sce/doctor-human-text-contract.md` (the new `Remediation:` line
  under the "Agent tracing" row)
- `context/cli/claude-mutation-scope-integration.md` (the new
  `PendingAbandon` phase and its safety semantics)
- `context/cli/opencode-mutation-scope-adapter-lifecycle.md` and/or
  `context/cli/opencode-mutation-scope-integration.md` (the new
  backward-compatible owner-evidence field and the dead-owner repair path)
- `context/cli/pi-mutation-scope-integration.md` (the process-owner
  primitive is now shared, not Pi-only, if its description changes as a
  result)
- `context/context-map.md`, only if navigation actually needs it
- A new shared doc for the extracted process-owner module and/or the
  doctor-repair invariants, only if implementation reveals content
  substantial enough that folding it into the docs above would be unclear
  (mirroring the exception the prior plan took for
  `mutation-scope-health-status.md` itself) — do not create one
  speculatively.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/claude_mutation_scope/{state,mod,health}.rs`;
  `cli/src/services/hooks/opencode_mutation_scope/{state,mod,health}.rs`;
  `cli/src/services/hooks/mutation_scope_health.rs` (new `Repairability`
  enum); a new shared process-owner module extracted from
  `cli/src/services/hooks/pi_mutation_scope/process_owner.rs`;
  `cli/src/services/hooks/pi_mutation_scope/{mod,health}.rs` (import-only
  change to consume the extracted module); `cli/src/services/doctor/{mod,inspect,render,fixes,types}.rs`;
  a new focused Quint model under `spec/`; the context docs listed above.
- **Out of scope:** Codex and Pi adapter *behavior* (their recovery
  protocols, state machines, and reachable health shapes are unchanged;
  Pi's only change is the pure extraction with identical behavior);
  `MutationScopeHealthStatus`'s four-way vocabulary (unchanged, per the
  existing `doctor-mutation-scope-health` plan); `Invalid`-state auto-repair
  or migration; generic doctor integration-asset repair; DB schema repair;
  fixing the existing Claude/OpenCode/Codex asset `ManualOnly`
  inconsistencies unrelated to mutation-scope; redesigning `ServiceLifecycle`
  providers; general `doctor --fix` cleanup unrelated to mutation-scope
  recovery; extending `spec/mutation_cursor.qnt` itself (the new formal
  model is a separate, focused file — the doctor-repair concern is not part
  of the verified core mutation-cursor protocol that file owns).
- **Constraints:** never delete an adapter state file, clear
  `recovery_pending`/`RecoveryState` generically, reset attempts, or
  fabricate an abandon outside an adapter's own real protocol operations;
  never use a TTL/file-age/elapsed-duration heuristic as proof of staleness;
  every repair mutates state only through each adapter's existing recovery
  protocol seam (`flush`/`abandon` payloads through the mutation-scope
  ingress), never by doctor calling `remove_attempt()` or rewriting JSON
  directly. OpenCode repair must acquire its `AdapterBoundaryLock` before
  lifecycle recovery, and its `AdapterStateLock` may protect only an
  individual read/re-proof/durable-transition transaction; it must be
  released before every seam call and reacquired for later durable progress.
  Claude must use its existing state lock for each fresh read/proof and
  durable transition, but must release it before every mutation-scope seam
  call; this plan does not add a Claude boundary lock. Neither adapter may
  trust an earlier lock-free classification as authorization, and the
  repair function itself must perform the fresh proof. A new persisted field
  must deserialize from a state file written before this change
  (backward-compatible `#[serde(default)]`, matching the existing precedent
  in `opencode_mutation_scope::state::AdapterState`); the JSON
  `mutation_scope_health` array's field set and status strings, and every
  existing `MutationScope*` Rust type name, are preserved exactly.
- **Non-goal:** this plan does not make `Invalid` states auto-fixable; it
  does not unify the four adapters onto one shared repair trait (per the
  request's guidance, prefer adapter-owned functions dispatched by doctor's
  existing `IntegrationTargetId` match, matching how
  `inspect_mutation_scope_health` already dispatches, unless a real
  multi-adapter shared abstraction turns out to be unavoidable); it does
  not broaden doctor's mutation-scope repair into a general "reset
  integration state" feature.

## Assumptions

- Codex and Pi need no adapter-behavior change because their current
  classifiers (`codex_mutation_scope::health::classify_health`,
  `pi_mutation_scope::health::classify_health`) never produce `Blocked`,
  confirmed by direct inspection of both functions and their existing
  regression tests. If a future adapter change ever makes either reach
  `Blocked`, this plan's investigation should be re-run for that adapter
  rather than assumed to still hold.
- Claude's dead-owner liveness path (for a `PendingStart`/`Active` attempt
  with no established abandon intent) is included only if T04's own
  investigation finds a currently-reachable ambiguous shape that needs it;
  Claude's primary, currently-proven `Blocked` shape (a failed abandon
  seam call) does not need owner evidence at all, since the abandon
  decision was already made by Claude's own runtime before the seam call
  failed — only the seam confirmation is missing, which
  `PendingAbandon`-retry addresses directly.
- The new adapter-owned repairability distinction is modeled as
  `assess_repairability`/`repair_blocked` functions on each adapter's
  existing module rather than a new Rust trait, per the request's explicit
  instruction not to introduce a trait merely for architectural appearance;
  doctor dispatches to them the same way `inspect_mutation_scope_health`
  already dispatches `classify_health` by `IntegrationTargetId`.

## Task stack

- [x] T01: `Formalize the safe doctor-repair protocol and its invariants` (status:done)
  - Task ID: T01
  - Scope: In — a new, focused Quint model (e.g. `spec/doctor_recovery.qnt`,
    exact name chosen at implementation time) modeling the shared abstract
    pattern this plan introduces: attempt phases (a start-pending phase, an
    executing/active phase, and a phase meaning "abandonment already
    decided"), an abstract three-valued owner-liveness oracle (`Alive` /
    `Dead` / `Unknown`, deliberately never time-based), a per-adapter lock,
    and two actors — the adapter's own hook process and doctor — racing
    over the same durable state. The model must state and check, as
    invariants or temporal properties: doctor cannot abandon a potentially
    live attempt; `Unknown` owner state is never treated as proof of death;
    doctor cannot clear recovery while unresolved lifecycle evidence
    exists; a doctor repair action cannot produce a state outside the
    adapter's own reachable transition set; concurrent hook-process and
    doctor execution cannot cause a live attempt to be abandoned; a
    terminal/removed attempt is never resurrected; an interrupted repair
    remains fail-closed and retryable (every state it can stop in is
    itself a legitimate, already-reachable state); a reported successful
    repair cannot coincide with final `health == Blocked`. Out — any Rust
    behavior change; any change to `spec/mutation_cursor.qnt` itself (this
    is a separate, focused model, not an extension of the verified core
    mutation-cursor protocol, since doctor-repair concerns are adapter
    bookkeeping the core protocol does not model).
  - Dependencies: none
  - Done when: the new `.qnt` file typechecks; it has its own `quint test`
    example-based sanity tests; each invariant above is expressed as a
    Quint `invariant` (or temporal property) and a documented
    `quint run --invariant=... [--max-samples=...]` / `quint verify`
    invocation (recorded in the file's own header comment) finds no
    violation within a stated, documented bound; the file's header states
    explicitly which real Claude/OpenCode concepts each abstract phase and
    the owner oracle correspond to, so T03/T04 can be checked against it.
  - Verify: `nix run .#quint -- typecheck spec/doctor_recovery.qnt`; `nix run .#quint -- test spec/doctor_recovery.qnt`; the invariant-check command recorded in the file's own header.
  - Completed: 2026-09-21 (corrected three times on 2026-09-21 — the first
    pass's model was directionally useful but several named invariants
    were weaker than claimed; the second pass fixed that but still
    modeled `recovery`/`health`/`fixState` per attempt where the real
    adapters persist one recovery/health/fix-report fact for the whole
    adapter, and left `recoveryProgress`/`completeAbandon` runnable while
    `DoctorHolds`; the third pass fixed both of those but introduced two
    new mismatches of its own: it added a `recovery == Clear` precondition
    to `hookDecideAbandon`/`doctorAttemptRepairWith` that wrongly encoded
    "at most one attempt can be `PendingAbandon`" as a shared protocol
    invariant, and it described the model's `lock: LockHolder` variable as
    corresponding primarily to OpenCode's `AdapterBoundaryLock` even though
    the model required that variable free during the seam-adjacent
    `recoveryProgress`/`completeAbandon` actions — the real
    `AdapterBoundaryLock` remains held across seam calls; only the shorter
    `AdapterStateLock` is released before them. This record describes the
    fourth pass, which removes the single-`PendingAbandon` restriction and
    renames the lock abstraction so it stops claiming to be
    `AdapterBoundaryLock`)
  - Files changed: `spec/doctor_recovery.qnt` (new, then corrected in
    place three times — no T01b/T01c/T01d, same task)
  - Result: `spec/doctor_recovery.qnt` is a standalone Quint model (does
    not extend `spec/mutation_cursor.qnt`) of the safe doctor-repair
    pattern. `phase: AttemptId -> AttemptPhase` stays per-attempt, but
    `recovery: RecoveryState` and `fixState: FixState` are now single
    adapter-wide variables (not `AttemptId -> ...` maps), matching the real
    OpenCode `AdapterState { recovery: RecoveryState, attempts:
    Vec<AdapterAttempt> }` shape and Claude's whole-file
    `recovery_pending` flag — both adapters report one health/one fix
    result for the adapter, never one per attempt:
    - `AttemptPhase = NotAllocated | PendingStart | Active | PendingAbandon
      | Removed` (unchanged from both earlier passes).
    - `RecoveryState = Clear | Pending | Flushing`, still a single `var
      recovery` for the whole adapter (unchanged shape from the third
      pass), distinguishing "no terminal recovery obligations remain
      anywhere in the adapter" (`Clear`), "one or more terminal recovery
      obligations exist and need processing" (`Pending`), and "the
      adapter-wide recovery sequence is currently processing one or more
      obligations" (`Flushing`). It is not ownership of one particular
      attempt: multiple `PendingAbandon` attempts may be covered by the
      same shared `recovery` value at once. The third pass had added a
      `recovery == Clear` precondition to `hookDecideAbandon`/
      `doctorAttemptRepairWith`'s eligibility specifically to force at
      most one attempt to be `PendingAbandon` at a time; this fourth pass
      removes that precondition, because the real OpenCode
      `begin_terminal_cleanup(scope_ids: &[String])` sets every matching
      attempt to `PendingAbandon` in one call and `resolve_recovery()`
      filters and processes every currently-`PendingAbandon` attempt, not
      at most one, and Claude's broad cleanup signals
      (`Stop`/`SessionEnd`/`UserPromptSubmit`/`SubagentStop`, per T04) may
      likewise decide to abandon several attempts at once.
      `abandonTransition` now takes the current `recovery` value as a
      parameter and computes `recovery: if (rec == Clear) Pending else
      rec` — establishing a new `PendingAbandon` obligation moves `Clear
      -> Pending` but leaves an already-`Pending`/`Flushing` recovery
      unchanged, so a second obligation can join an in-flight recovery
      pipeline without disturbing it. `recoveryProgress` still advances
      `Pending -> Flushing` for the shared pipeline. `completeAbandon` no
      longer clears `recovery` unconditionally on removing its attempt —
      it now computes `otherObligationsRemain = exists a. (phase with
      this attempt set to Removed).get(a) == PendingAbandon` and sets
      `recovery' = if (otherObligationsRemain) Pending else Clear`, so
      global recovery only clears once every terminal obligation is
      resolved; a remaining obligation falls the pipeline back to
      `Pending` (an explicit "retry remaining obligations" step, needing a
      fresh `recoveryProgress` before the next `completeAbandon`) rather
      than staying `Flushing` for an attempt whose own flush already
      finished. Both `recoveryProgress` and `completeAbandon` still
      require the short state-transaction abstraction free (see the
      lock-naming correction below) — the second pass left this
      seam/recovery pair runnable while that variable was held, the third
      pass fixed that, and this pass preserves it unchanged; every test
      already released it before driving
      `recoveryProgress`/`completeAbandon`.
    - `Health = Healthy | Recovering | Blocked | Invalid` now comes from a
      new `pure def adapterHealth(phase: AttemptId -> AttemptPhase,
      recovery: RecoveryState): Health` — one health value for the whole
      adapter, derived from `hasPendingStart = exists a. phase.get(a) ==
      PendingStart` and `hasPendingAbandon = exists a. phase.get(a) ==
      PendingAbandon` in the exact required priority order: `recovery ==
      Clear and hasPendingAbandon -> Invalid`, else `hasPendingStart ->
      Blocked`, else `recovery == Clear -> Healthy`, else `Recovering`
      (covering both `Pending` and `Flushing`). The old per-attempt `pure
      def health(phase, recovery)` is gone; every caller now calls
      `adapterHealth(phase, recovery)` with no attempt argument.
      `testInvalidTakesPriorityOverBlockedWhenBothConditionsHold` (new)
      proves the ordering directly: one attempt hand-set to `PendingAbandon`
      with `recovery == Clear` and a second, different attempt genuinely
      `PendingStart` (both conditions live at once) still classifies
      `Invalid`, not `Blocked`.
    - `FixState = NotAttempted | RepairAttempted | RepairCompleted |
      ReportedFixed | ReportedManual`, now a single `var fixState` for the
      whole adapter (previously `AttemptId -> FixState`), matching that
      `sce doctor` reports one fix result per adapter, never one per
      attempt. `doctorAttemptRepairWith` sets it to `RepairAttempted`
      (unconditionally, whether or not the repair turns out eligible);
      `completeAbandon` promotes `RepairAttempted -> RepairCompleted`; a
      `doctorReportResult` action (now nullary — no attempt argument, since
      there is one fix result, not one per attempt) reads the fresh
      `adapterHealth(phase, recovery)` at report time and decides
      `ReportedFixed` only when it is `Healthy`/`Recovering`, else
      `ReportedManual`. Making `fixState` adapter-wide surfaced a case
      neither earlier pass had to consider: since `adapterHealth` now
      depends on *every* attempt's phase, an unrelated attempt allocating
      fresh (`hookAllocate`, `NotAllocated -> PendingStart`) after a
      `ReportedFixed` report can make `adapterHealth` swing to `Blocked`
      purely because of that unrelated attempt, staling the old report.
      `hookAllocate` is the only action that can newly introduce
      `hasPendingStart` (every other action only removes a `PendingStart`
      contributor or moves through `PendingAbandon`/`Recovering`, which
      never regresses toward `Blocked`/`Invalid`), so `hookAllocate` now
      resets `fixState' = NotAttempted` instead of passing it through
      unchanged — the prior report is intentionally treated as stale once
      fresh, doctor-relevant lifecycle activity begins. This reset is a
      no-op in every existing test (each calls `hookAllocate` exactly once,
      at the very start, while `fixState` is still its `init` value of
      `NotAttempted`), and it was required for `--invariant=Safety` to find
      no violation at the stated bounds — omitting it reproduces a real
      counterexample.
    - A shared `pure def abandonTransition(phase, everWasPendingAbandon,
      rec, attempt)` (this pass restores a `recovery` parameter the third
      pass had dropped — the successor `recovery` value can no longer
      always be the literal `Pending`, since an already-`Pending`/
      `Flushing` shared recovery must be left unchanged when a second
      obligation joins it) returns the `PendingStart/Active ->
      PendingAbandon` + `recovery: if (rec == Clear) Pending else rec`
      result record. Both `hookDecideAbandon` and `doctorAttemptRepairWith`
      (after its own `stateTxn == DoctorStateTxn and phase == PendingStart
      and reading == SeenDead` eligibility check — the third pass's
      `recovery == Clear` conjunct is removed here) call this same
      function, preserving the second pass's "doctor cannot reach a shape
      the ordinary hook path could not also reach" property.
    - `OwnerReading = SeenAlive | SeenDead | SeenUnknown` and
      `soundReadings(alive)` are unchanged. `doctorAttemptRepairWith`'s
      `phase.get(attempt) != NotAllocated` precondition (added in the
      second pass) is unchanged.
    - The same eight named invariants as the third pass (one renamed, per
      below), combined into `val Safety`. Their statements are otherwise
      unchanged text from the third pass, but two of them are now
      meaningfully different in what they prove, because "at most one
      attempt is ever mid-`PendingAbandon`" is no longer true and was
      never actually required for either to hold:
      `RecoveryNeverClearedWithUnresolvedAbandon` (`recovery == Clear
      implies (forall a. phase.get(a) != PendingAbandon)`) now genuinely
      constrains `completeAbandon`'s new `otherObligationsRemain` logic —
      with the third pass's single-`PendingAbandon` restriction in place
      this invariant held almost trivially (there was never a second
      `PendingAbandon` attempt to protect against); with that restriction
      removed, this is the invariant that actually forces
      `completeAbandon` to check the whole resulting attempt set before
      clearing `recovery`, and `testMultiplePendingAbandonShareOneRecoveryPipeline`
      (new) exercises exactly that: completing the first of two
      concurrently-`PendingAbandon` attempts must leave `recovery !=
      Clear`. `DoctorRepairProducesOnlyOrdinaryLifecycleShapes`
      (`phase.get(a) == Removed or (phase.get(a) == PendingAbandon and
      (recovery == Pending or recovery == Flushing))`, unchanged text from
      the second pass's fix) already held for any number of concurrent
      `PendingAbandon` attempts — it says nothing about how many other
      attempts share the same `recovery` value, so removing the
      single-`PendingAbandon` restriction changes nothing about this
      invariant's proof. `InterruptedRecoveryStaysInOrdinaryRetryableState`
      likewise already generalized to multiple attempts without
      modification: each attempt in `everWasPendingAbandon` and not yet in
      `everRemoved` independently must be `PendingAbandon` with `recovery
      == Pending or Flushing`, which holds per-attempt regardless of how
      many other attempts satisfy the same clause simultaneously.
      `ReportedFixedRequiresHealthyFinalState` is renamed
      `ReportedFixedExcludesBlockedOrInvalid` (same body:
      `fixState == ReportedFixed implies (adapterHealth(phase, recovery)
      != Blocked and adapterHealth(phase, recovery) != Invalid)`) — the
      old name overstated the requirement as "healthy," when `Recovering`
      is deliberately still accepted as a successful report (`Blocked` ->
      `Recovering` counts as removing the durable wedge, per this plan's
      AC6); the new name says exactly what the invariant checks.
    - `pendingAbandonFromDoctor` is unchanged — still a per-attempt
      diagnostic/test bookkeeping set, not adapter-wide, since it tracks
      which specific attempts doctor touched (a real, per-attempt fact),
      not the adapter's health or fix-report state.
    - Thirteen `run` tests (up from eleven): the third pass's eleven
      tests, updated only where the `doctorAcquireLock`/`doctorReleaseLock`
      action names changed to `doctorAcquireStateTxn`/
      `doctorReleaseStateTxn` (see the lock-naming correction below; no
      test's assertions changed), plus two new tests.
      `testMultiplePendingAbandonShareOneRecoveryPipeline` reaches
      `Attempt0 = PendingAbandon, Attempt1 = PendingAbandon, recovery ==
      Pending` through ordinary `hookAllocate`/`hookDecideAbandon` calls on
      both attempts (not a hand-constructed state), asserts
      `adapterHealth(...) == Recovering` and `Safety` there, then completes
      only `Attempt0` (`recoveryProgress` + `completeAbandon`) and asserts
      `Attempt0 == Removed`, `Attempt1 == PendingAbandon`, `recovery !=
      Clear`, and `adapterHealth(...) == Recovering` still — proving
      `completeAbandon`'s per-attempt-set recovery-clearing logic. It then
      drives `Attempt1` through its own
      `recoveryProgress`/`completeAbandon` and only then asserts `recovery
      == Clear` and `adapterHealth(...) == Healthy`.
      `testRepairingOneDeadBlockerLeavesOtherBlockerBlocked` reaches
      `Attempt0 = PendingStart` with a proven-dead owner and `Attempt1 =
      PendingStart` with an unknown owner, both contributing to an initial
      `Blocked` classification; it repairs only `Attempt0`
      (`doctorAcquireStateTxn`/`doctorAttemptRepairWith(Attempt0,
      SeenDead)`/`doctorReleaseStateTxn`) and asserts `adapterHealth(...)
      == Blocked` still (because `Attempt1` remains `PendingStart`) and
      `doctorReportResult` produces `ReportedManual`, not `ReportedFixed`;
      it then completes `Attempt0`'s abandonment and asserts
      `adapterHealth(...) == Blocked` and `fixState != ReportedFixed`
      persist even after that attempt reaches `Removed`, since `Attempt1`
      is still an unrepaired `PendingStart` blocker — the core proof that
      repairing one repairable blocker does not mean the adapter itself is
      repaired; only the freshly recomputed adapter-wide classifier may
      authorize `ReportedFixed`. Both new tests are ordinary-transition
      regressions (no hand-constructed `all { ... }` state), matching every
      other test's convention except the two adversarial ones documented
      below.
      `testPendingAbandonWithClearRecoveryIsInvalid` and
      `testInvalidTakesPriorityOverBlockedWhenBothConditionsHold` are
      otherwise unchanged in structure (aside from the `lock' = lock` ->
      `stateTxn' = stateTxn` field rename in their hand-constructed `all {
      ... }` blocks) and still deliberately omit `.expect(Safety)` for the
      same reason as before (the constructed state is intentionally
      adversarial/unreachable).
  - Deviation: the file still carries no comments (including no header
    comment), per the repository's standing "no comments in code"
    instruction; the concept mapping and verification results below stand
    in for the file's own header comment, as in every earlier pass.
    - Concept mapping (abstract -> real), unchanged from the third pass
      except the lock abstraction, corrected below: `PendingStart` ->
      OpenCode's `AdapterAttempt` `PendingStart` (the phase its `Blocked`
      classification keys on) and, for Claude, a `PendingStart`/`Active`
      attempt with no established abandon intent; `Active` -> an attempt
      progressing under its owner, or a Claude attempt past `PendingStart`
      with no abandon decision yet; `PendingAbandon` -> Claude's new
      `PendingAbandon` phase (T04) and OpenCode's `PendingStart ->
      PendingAbandon` durable transition (T03) once the dead-owner
      condition is proven — and, as of this pass, multiple attempts may
      independently carry this phase at once, all covered by the one
      shared `recovery` value; `Removed` -> the attempt gone after a
      successful seam sequence; `recovery: RecoveryState` (adapter-wide) ->
      OpenCode's `AdapterState.recovery` field directly, and Claude's
      whole-file `recovery_pending` flag/barrier state — `Clear` is "no
      terminal recovery obligation outstanding anywhere in the adapter,"
      `Pending` is "one or more obligations recorded, pipeline not
      actively running," `Flushing` is "the shared pipeline is actively
      processing one or more obligations," matching
      `begin_terminal_cleanup`/`resolve_recovery`'s real multi-attempt
      batch shape, not a single-attempt lock; `adapterHealth(phase,
      recovery)` -> `classify_health`'s `healthy`/`recovering`/`blocked`/
      `invalid` result, computed the same way the real classifiers do —
      from every attempt's phase plus the one adapter-wide recovery fact,
      never from a single attempt in isolation; `fixState` (adapter-wide)
      -> the one doctor fix-result lifecycle
      `execute_doctor_with_lifecycle_providers` drives per adapter target
      (assess -> repair -> re-diagnose -> record one
      `DoctorFixResultRecord`), with `ReportedFixed` standing for the
      final report deciding `Fixed` only from the freshly recomputed
      adapter-wide `classify_health` (never from a repair function's
      `Ok(())` alone, and never per-attempt, and never from having
      repaired only some of several contributing blockers — see
      `testRepairingOneDeadBlockerLeavesOtherBlockerBlocked` above);
      `ownerAlive` (ground truth) -> the real, single owning process
      instance (PID + `/proc` start-time identity,
      `mutation_scope_owner`), monotonic once dead; `OwnerReading` ->
      `mutation_scope_owner::is_definitely_dead`'s `Alive`/`Dead`/`Unknown`
      result.
      Lock mapping, corrected this pass: the third pass's completion
      record described `lock: LockHolder` (`LockFree`/`DoctorHolds`) as
      corresponding primarily to OpenCode's `AdapterBoundaryLock`, while
      the model itself required that variable free during
      `recoveryProgress`/`completeAbandon` (the seam-adjacent actions) —
      but the real `AdapterBoundaryLock` remains held across OpenCode's
      seam calls; it serializes the whole repair lifecycle, seam sequence
      included. Only the shorter `AdapterStateLock` is released before
      every seam call. Those two claims cannot both describe real
      OpenCode, so the variable is renamed `stateTxn: StateTxnHolder`
      (`StateTxnFree`/`DoctorStateTxn`), and the two lock actions are
      renamed `doctorAcquireStateTxn`/`doctorReleaseStateTxn` to match.
      `stateTxn == StateTxnFree` during `recoveryProgress`/`completeAbandon`
      now correctly means only "the short durable state-transaction lock
      is released," saying nothing about a larger OpenCode boundary lock:
      `stateTxn`/`StateTxnHolder` -> OpenCode's `AdapterStateLock`
      transaction used to re-read/re-prove/persist a durable transition,
      and, for Claude, its ordinary short state-file lock transaction
      around the same re-read/re-prove/persist step. OpenCode's
      `AdapterBoundaryLock` itself is outside this model variable
      entirely — it is a separate, coarser serialization concern (the
      complete repair lifecycle boundary across concurrent OpenCode
      processes, including the seam calls) that this model does not need
      a dedicated variable for, since none of the eight `Safety` invariants
      depend on cross-process boundary serialization; T03 is responsible
      for implementing `AdapterBoundaryLock` acquisition around its whole
      `repair_blocked` call, wrapping (not replacing) the shorter
      `AdapterStateLock`-shaped `stateTxn` transactions this model
      verifies. For Claude there is no equivalent boundary lock in this
      plan; Claude's repair safety rests on the durable `PendingAbandon`
      evidence itself plus its own individual state-lock transactions, not
      on a boundary-shaped variable. With the renaming, `doctorDiagnose` ->
      `assess_repairability`'s unlocked, possibly-stale read;
      `doctorAcquireStateTxn`/`doctorAttemptRepair`/`doctorReleaseStateTxn`
      -> `repair_blocked`'s durable read/re-prove/transition (run inside
      OpenCode's separate `AdapterBoundaryLock`, and without one for
      Claude); `recoveryProgress` -> running the adapter's existing
      recovery-protocol seam operations (OpenCode's `flush`/`abandon`/
      `flush`; Claude's `abandon_attempt`'s seam `abandon`) — modeled with
      `stateTxn == StateTxnFree` required, matching the repository
      invariant that `AdapterStateLock` (and Claude's equivalent state
      lock) is never held across a seam call; `completeAbandon` -> the
      durable transaction recording that seam sequence's success, shared
      by both the ordinary hook-process retry path and doctor's repair
      path so doctor never invents a new terminal transition.
    - Verification command and result: `quint run spec/doctor_recovery.qnt
      --invariant=Safety --max-samples=10000 --max-steps=30` -> `[ok] No
      violation found (389ms at 25707 traces/second)`; a second, stronger
      bound (`--max-samples=20000 --max-steps=50`) -> `[ok] No violation
      found (1328ms at 15060 traces/second)`. Both bounds were re-run
      against this fourth pass specifically (not carried over from the
      third pass's record), since removing the single-`PendingAbandon`
      restriction changes which states are reachable.
  - Verify outcomes: `quint typecheck spec/doctor_recovery.qnt` -> exit 0
    (no output, clean typecheck); `quint test spec/doctor_recovery.qnt
    --match '^test.*'` -> `13 passing`, 0 failed; `quint run
    spec/doctor_recovery.qnt --invariant=Safety --max-samples=10000
    --max-steps=30` -> `[ok] No violation found`; `quint run
    spec/doctor_recovery.qnt --invariant=Safety --max-samples=20000
    --max-steps=50` -> `[ok] No violation found`; `nix flake check` ->
    `all checks passed!` (re-run for this correction, including the
    existing `mutation-trace-quint-connect` check against the `spec/`
    tree; no check is yet wired specifically to `doctor_recovery.qnt` —
    that connection is T07's job per this task's own scope).
  - Context impact: Establishes `spec/doctor_recovery.qnt` as the reference
    formal model T03/T04's real Rust implementations and T07's
    Quint-Connect-or-documented-mapping connection must be checked against.
    T03/T04 should read two corrected facts above before writing the
    OpenCode/Claude repair code: (1) one adapter-wide `recovery` pipeline
    may safely cover multiple concurrent `PendingAbandon` obligations, and
    it cannot clear to `Clear` until every one of them is resolved — doctor
    and the ordinary hook path may each independently mark different
    attempts `PendingAbandon`, and `resolve_recovery`-shaped completion
    logic must check the whole attempt set, not just the attempt it is
    currently finishing; (2) the model's `stateTxn`/`StateTxnHolder`
    variable corresponds to OpenCode's short `AdapterStateLock` transaction
    only, never to `AdapterBoundaryLock` — T03 must still acquire
    `AdapterBoundaryLock` around the whole `repair_blocked` call per this
    plan's own constraints section, and that acquisition is a fact outside
    this model's verified surface, not something `stateTxn == StateTxnFree`
    stands in for. Doctor's fix result and the classifier's health input
    remain both single adapter-level facts derived from every attempt's
    phase plus one shared recovery value, not computed or reported per
    attempt, and the seam sequence (`flush`/`abandon`/`flush` for
    OpenCode, `abandon_attempt`'s seam `abandon` for Claude) must run with
    the state-transaction lock released, matching `AdapterStateLock` never
    being held across a seam call. No context doc listed in this plan's
    "Context sync" section is implicated by this task alone (it adds no
    new Rust-facing contract); T07's own context-sync pass is where a new
    shared doc for the model, if warranted, would be considered per the
    plan's non-speculative instruction.
  - Context synchronization: synced

- [x] T02: `Extract shared positive process-owner evidence from Pi` (status:done)
  - Task ID: T02
  - Scope: In — move `ProcessOwner`, `current_process_owner`,
    `process_owner_for`, and `is_definitely_dead` out of
    `cli/src/services/hooks/pi_mutation_scope/process_owner.rs` into a new
    shared module reachable by other adapters (e.g.
    `cli/src/services/hooks/mutation_scope_owner.rs`, sibling to the
    existing shared `mutation_scope_health.rs`), with identical logic
    (`kill(pid, 0)` liveness plus Linux `/proc/{pid}/stat` start-time
    PID-reuse proofing, conservative `false` on non-Linux/non-unix, and the
    same "never assume dead" test suite including the static
    no-TTL/elapsed-time-token source scan); update
    `pi_mutation_scope::{mod,health}.rs` to import the shared module with
    zero behavior change. Out — any new consumer of the extracted module
    (OpenCode/Claude wiring happens in T03/T04); any change to Pi's own D10
    dead-owner sweep behavior, reachable health statuses, or persisted
    `owner` field shape.
  - Dependencies: T01 (the extracted module's `Alive`/`Dead`/`Unknown`
    liveness contract must match T01's abstract owner oracle)
  - Done when: `pi_mutation_scope` no longer defines its own
    `ProcessOwner`/`is_definitely_dead`; every existing Pi mutation-scope
    regression (dispatch, recovery, and health-classifier tests) passes
    with identical outcomes; the extracted module's own test suite,
    including the static "no TTL/elapsed-time primitive" source scan
    (now scoped to the new file), passes.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_scope_owner`
  - Completed: 2026-09-21
  - Files changed: `cli/src/services/hooks/mutation_scope_owner.rs` (new —
    moved `ProcessOwner`/`current_process_owner`/`process_owner_for`/
    `is_definitely_dead` and their full test suite verbatim from
    `pi_mutation_scope/process_owner.rs`); `cli/src/services/hooks/mod.rs`
    (registers `pub mod mutation_scope_owner;`);
    `cli/src/services/hooks/pi_mutation_scope/process_owner.rs` (deleted);
    `cli/src/services/hooks/pi_mutation_scope/mod.rs` (drops the local
    `pub(crate) mod process_owner;` declaration);
    `cli/src/services/hooks/pi_mutation_scope/{state,health,lifecycle,lifecycle_tests}.rs`
    (import paths repointed to `crate::services::hooks::mutation_scope_owner::...`,
    no logic changes; `cargo fmt` reordered the new `use` blocks).
  - Result: `ProcessOwner`, `current_process_owner`, `process_owner_for`, and
    `is_definitely_dead` now live in the shared, adapter-neutral
    `cli/src/services/hooks/mutation_scope_owner.rs`, sibling to the existing
    `mutation_scope_health.rs`, with byte-identical logic (`kill(pid, 0)`
    liveness, Linux `/proc/{pid}/stat` start-time PID-reuse proofing,
    conservative `false` on non-Linux/non-unix) and its own complete test
    suite including the static no-TTL/elapsed-time-token source scan
    (re-scoped to `include_str!("mutation_scope_owner.rs")`).
    `pi_mutation_scope` no longer defines any of these symbols; `state.rs`,
    `health.rs`, and `lifecycle.rs`/`lifecycle_tests.rs` now import them from
    the shared module. No behavior, persisted `owner` field shape, or
    reachable Pi health status changed. Two active context docs
    (`context/cli/pi-mutation-scope-health.md`,
    `context/cli/pi-mutation-scope-integration.md`) cited the old
    `pi_mutation_scope/process_owner.rs` path and were corrected to name the
    new shared module.
  - Deviation: while running the plan's own `Verify` commands, the initial
    `SCE_CLI_PACKAGE_FALLBACK=1 cargo test` run showed 7 pre-existing,
    change-unrelated failures (`no such table: mutation_trace_scope_provenance`)
    caused by a stale `cli/package-fallback`/incremental-build cache;
    confirmed pre-existing by stashing this task's changes and reproducing
    the identical failure on the unmodified baseline. Running
    `bash scripts/prepare-cli-generated-assets.sh` and clearing the stale
    `cli/target/debug/build/shared-context-engineering-*` directories
    resolved it; both plan `Verify` commands then passed cleanly, including
    via the documented `nix develop -c ./scripts/run-cli-cargo.sh` wrapper.
    Not a T02 regression or scope item.
  - Verify outcomes: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope` -> `94 passed; 0 failed`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_scope_owner` -> `8 passed; 0 failed`; `cargo fmt --manifest-path cli/Cargo.toml -- --check` -> clean; `cargo clippy --manifest-path cli/Cargo.toml` -> no warnings on touched modules.
  - Context impact: A shared, adapter-neutral process-owner-liveness module
    now exists at `cli/src/services/hooks/mutation_scope_owner.rs` for T03
    (OpenCode) to consume; no user-visible behavior, public interface,
    persisted data shape, or architecture-boundary change. `domain`-scoped:
    the two Pi-specific context docs that named the old file path were
    corrected; no root context file (`overview.md`/`architecture.md`/
    `glossary.md`/`patterns.md`/`context-map.md`) referenced the old path,
    so none needed edits.
  - Context synchronization: synced

- [x] T03: `OpenCode: persisted owner evidence and safe PendingStart repair` (status:done)
  - Task ID: T03
  - Scope: In — `cli/src/services/hooks/opencode_mutation_scope/{boundary_lock,events,lifecycle,mod,os_lock,payload,state,health,tests}.rs`.
    Preserve the existing lock hierarchy: `AdapterBoundaryLock`
    (`opencode-mutation-scope-boundary.lock`) serializes complete OpenCode
    lifecycle boundaries, while `AdapterStateLock`
    (`opencode-mutation-scope-state.lock`) protects only individual durable
    state operations and is never held across a seam call. Add an optional,
    backward-compatible `owner: Option<ProcessOwner>`
    (`#[serde(default)]`, mirroring the existing `next_recovery_generation`/
    `recovery` precedent in `AdapterState`) to `AdapterAttempt`, stamped
    with the shared `current_process_owner()` when a `PendingStart` attempt
    is allocated; a state file with no `owner` for an attempt (any file
    written before this change) must deserialize to `owner: None`, never
    inferred. Add `assess_repairability(git_dir) -> Repairability`, called
    only when `classify_health` reports `Blocked`: `AutoFixable` only when
    every `PendingStart` attempt currently contributing to the `Blocked`
    classification has `owner: Some(owner)` with `is_definitely_dead(owner)`
    true; `ManualOnly` otherwise (no recorded owner, a live owner, or
    unprovable liveness). Add `repair_blocked(git_dir, repository_root, seam) -> Result<RepairOutcome>`:
    acquire the OpenCode `AdapterBoundaryLock`, normalize orphaned recovery
    state, then use an `AdapterStateLock` transaction to re-read state fresh
    and re-prove the exact dead-owner condition `assess_repairability` found
    (never trusting the caller's earlier lock-free read). Release the state
    lock before driving the attempt through the adapter's real recovery
    protocol: the existing `PendingStart` -> `PendingAbandon` persisted
    transition, then the same `flush`/`abandon`/`flush` seam sequence
    `ToolError` cleanup already uses. Each progress/completion write is its
    own state transaction after the seam call; the state lock is never held
    across `flush`, `abandon`, or any other seam operation. Remove the
    attempt only when that protocol's own success proves it valid. A losing
    re-proof (owner now alive/unknown, or the attempt already resolved) is a
    safe no-op, not an error. Out — Claude, Codex, Pi; any change to
    OpenCode's
    existing `ToolError`/generation-tracked recovery machinery for
    non-`PendingStart` shapes; any change to `classify_health`'s existing
    four status boundaries (`Blocked` stays `Blocked` either way —
    repairability is an additional fact, not a fifth status).
  - Dependencies: T01, T02
  - Done when: a state file predating this change (no `owner` field) still
    parses, and its `PendingStart` `Blocked` shape classifies `ManualOnly`;
    a `PendingStart` attempt with a positively dead owner classifies
    `AutoFixable` and `repair_blocked` clears it end to end (final
    `classify_health` reports `Healthy` or `Recovering`, never `Blocked`);
    a `PendingStart` attempt with a live or unprovable owner classifies
    `ManualOnly` and `repair_blocked` is a safe no-op; a test simulates a
    concurrent live hook process rewriting state between
    `assess_repairability`'s read and `repair_blocked`'s lock acquisition,
    proving the fresh state-transaction re-proof refuses to abandon the
    now-different state; a test interrupts `repair_blocked` after the
    `PendingAbandon` persist but before the seam call resolves, and proves
    the next `repair_blocked` (or the ordinary recovery path) safely
    continues from `PendingAbandon` without state deletion or attempt
    duplication.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml opencode_mutation_scope`
  - Completed: 2026-09-21
  - Files changed: `cli/src/services/hooks/opencode_mutation_scope/state.rs`
    (imports `current_process_owner`/`is_definitely_dead`/`ProcessOwner` from
    the shared `mutation_scope_owner` module; adds `owner:
    Option<ProcessOwner>` to `AdapterAttempt` with `#[serde(default)]`;
    stamps it in `allocate_pending_start` via `current_process_owner()`;
    factors `begin_terminal_cleanup`'s tail into a shared
    `transition_to_pending_abandon_and_arm_flush` helper; adds
    `reprove_dead_owner_pending_start_and_begin_repair`, a single
    lock-protected read-reprove-transition, and the `#[cfg(test)]`
    `set_attempt_owner_for_tests` helper);
    `cli/src/services/hooks/opencode_mutation_scope/health.rs` (adds the
    local `Repairability { AutoFixable, ManualOnly }` enum and
    `assess_repairability`; adds ten new regression tests covering legacy
    no-owner deserialization, live/dead/mixed-owner assessment, end-to-end
    repair, the live-owner no-op, the concurrent-race re-proof refusal, the
    all-or-nothing rejection when a sibling `PendingStart` attempt's owner
    goes stale between the unlocked assessment and the locked re-proof, and
    the interrupted-repair resume via the ordinary recovery path; fixes the
    pre-existing `matrix_attempt` literal to set `owner: None`);
    `cli/src/services/hooks/opencode_mutation_scope/lifecycle.rs` (adds
    `RepairOutcome { Repaired, NoOp }` and `repair_blocked`, which runs
    inside the existing `with_boundary_lock` helper);
    `cli/src/services/hooks/opencode_mutation_scope/mod.rs` (re-exports
    `assess_repairability`, `Repairability`, `repair_blocked`,
    `RepairOutcome`, each `#[allow(unused_imports)]` pending T05's doctor
    wiring, matching the existing `pi_mutation_scope/mod.rs` precedent for
    not-yet-consumed exports).
  - Result: `AdapterAttempt` now carries optional, backward-compatible
    owner evidence stamped at `PendingStart` allocation. `assess_repairability`
    classifies a `Blocked` adapter `AutoFixable` only when every currently
    `PendingStart` attempt has a recorded owner positively proven dead by the
    shared `is_definitely_dead`; a legacy file with no `owner` field, a live
    owner, or an unprovable owner all stay `ManualOnly`. `repair_blocked`
    acquires the `AdapterBoundaryLock`, normalizes orphaned recovery, then
    performs one lock-protected read-reprove-transition
    (`reprove_dead_owner_pending_start_and_begin_repair`) that re-evaluates
    liveness fresh against the current state rather than trusting any
    earlier read. This re-proof is all-or-nothing over the adapter's
    complete current `PendingStart` set, not a per-attempt filter: it
    collects every attempt currently `PendingStart` and requires every one
    of them to have a recorded owner positively proven dead. If any current
    blocker is live, unknown, or ownerless, `repair_blocked` returns `NoOp`,
    no attempt is transitioned, recovery is untouched, and no seam call
    occurs. Only when every current blocker is positively dead do all
    current `PendingStart` attempts transition together to `PendingAbandon`,
    arming the existing recovery pipeline; `repair_blocked` then releases
    the state lock and drives the unmodified `flush`/`abandon`/`flush` seam
    sequence via the existing `resolve_recovery`, so the state lock is never
    held across a seam call and no new seam behavior was introduced. A
    losing re-proof (owner no longer `PendingStart` by the time the lock is
    acquired, or a sibling attempt's owner is no longer provably dead) makes
    `repair_blocked` a safe no-op with no seam call and no write — proven by
    a new regression test,
    `repair_blocked_is_all_or_nothing_when_auto_fixable_assessment_becomes_stale`,
    that seeds two independently dead-owner `PendingStart` attempts
    (doctor's initial unlocked `assess_repairability` read reports
    `AutoFixable`), then rewrites only the second attempt's persisted owner
    to a live owner before calling `repair_blocked`, and asserts the fresh
    re-proof rejects the whole batch: `RepairOutcome::NoOp`, both attempts
    still `PendingStart`, recovery still `Clear`, and zero seam calls — so
    the first attempt's still-dead owner is never enough on its own once any
    other current blocker fails the all-dead proof. A seam failure
    mid-repair leaves the existing `PendingAbandon`/`Pending` recovery
    state, which the adapter's pre-existing ordinary recovery path (any
    later tracked admission) resumes and completes without duplicating or
    resurrecting the attempt — proven by a new regression test that
    interrupts `repair_blocked` on a failing `abandon` seam call and then
    drives an unrelated `ToolExecuteBefore` to completion.
  - Deviation: `repair_blocked`'s signature is `repair_blocked(git_dir,
    repository_root, logger, seam) -> Result<RepairOutcome>`, adding a
    `logger: Option<&dyn Logger>` parameter beyond the plan's literal
    `repair_blocked(git_dir, repository_root, seam)`, matching every other
    seam-driving function in this module (`resolve_recovery`,
    `abandon_and_consume`, `establish_tracked_start`) which already thread a
    logger through for fail-closed observability; `repair_blocked` calls
    `resolve_recovery` directly and needed the same parameter. `Repairability`
    and `RepairOutcome` are defined locally in this module (in `health.rs`
    and `lifecycle.rs` respectively) rather than in the shared
    `mutation_scope_health.rs`, since T05 (dependent on this task) is the
    task explicitly scoped to add "the shared `Repairability` enum" there;
    this task's own in-scope file list does not include
    `mutation_scope_health.rs`.
  - Verify outcomes: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml opencode_mutation_scope` -> `113 passed; 0 failed` (103 pre-existing + 10 new, including the
    `repair_blocked_is_all_or_nothing_when_auto_fixable_assessment_becomes_stale`
    follow-up regression); `cargo fmt --manifest-path cli/Cargo.toml -- --check` -> clean; `SCE_CLI_PACKAGE_FALLBACK=1 cargo clippy --manifest-path cli/Cargo.toml --all-targets` -> no warnings; `grep -rn "SystemTime\|Instant::now\|\.elapsed()\|modified()" cli/src/services/hooks/opencode_mutation_scope` -> only the pre-existing, unrelated `os_lock.rs` lock-timeout deadline (unchanged by this task), matching AC3's "no staleness use outside unrelated lock-timeout constants."
  - Context impact: OpenCode's persisted state file gains a new optional
    `owner` field (backward-compatible, `#[serde(default)]`) and three new
    `pub(crate)` symbols (`Repairability`, `assess_repairability`,
    `repair_blocked`/`RepairOutcome`) that are not yet consumed by doctor —
    T05 wires them into `execute_doctor_with_lifecycle_providers`. No
    user-visible behavior changed yet (doctor's `--fix` still cannot repair
    OpenCode until T05 dispatches to these functions), no existing
    `MutationScope*` type or JSON shape changed, and `classify_health`'s
    four status boundaries are unchanged. `domain`-scoped: the context docs
    this plan's "Context sync" section names for OpenCode
    (`context/cli/opencode-mutation-scope-adapter-lifecycle.md` and/or
    `context/cli/opencode-mutation-scope-integration.md`) describe the new
    owner-evidence field and dead-owner repair path once T05/T06 make the
    repair path reachable through `sce doctor --fix`; recording that
    dependency here for T05's own context-sync pass rather than updating
    those docs prematurely for a repair path doctor cannot yet invoke. No
    root context file (`overview.md`/`architecture.md`/`glossary.md`/
    `patterns.md`/`context-map.md`) is implicated by an adapter-internal,
    not-yet-wired addition.
  - Context synchronization: synced

- [x] T04: `Claude: persisted terminal-cleanup evidence and safe PendingAbandon repair` (status:done)
  - Task ID: T04
  - Scope: In — `cli/src/services/hooks/claude_mutation_scope/{state,mod,health}.rs`.
    Add a `PendingAbandon` `AttemptPhase` variant alongside the existing
    `PendingStart`/`Active`; change `abandon_attempt` to persist the
    attempt's phase as `PendingAbandon` before calling the seam's `abandon`
    operation (replacing today's whole-file `mark_recovery_pending()` with
    per-attempt durable evidence, or keeping both if the barrier still
    needs the file-level flag — decide from the actual barrier logic), so a
    failed seam call leaves explicit per-attempt retryable evidence instead
    of today's ambiguous state. Audit `cleanup_attempts_matching`'s current
    early-return-on-first-failure loop (`abandon_attempt(...)?` inside the
    loop stops the whole batch on the first failure, leaving any
    later-matched attempt untouched in its pre-cleanup phase rather than
    uniformly `PendingAbandon`) and correct it, if the investigation
    confirms this is reachable, so every attempt a broad cleanup signal
    (`Stop`/`SessionEnd`/`UserPromptSubmit`/`SubagentStop`) decides to
    abandon is durably marked `PendingAbandon` before any seam call is
    attempted. Add `assess_repairability`/`repair_blocked` mirroring T03's
    shape: `AutoFixable` only when every attempt contributing to the
    `Blocked` classification is `PendingAbandon` (no `PendingStart`/`Active`
    attempt present — those have no established abandon intent and stay
    `ManualOnly`). Claude's `repair_blocked` re-reads state and re-proves
    every present attempt is `PendingAbandon` in an individual state-lock
    transaction, releases that lock before each already-established seam
    `abandon` call, and uses later state transactions to remove only
    successfully abandoned attempts and clear the recovery barrier only once
    no attempts remain. It must never hold the state lock across mutation-
    scope ingress or seam calls, and this cleanup pass must not add a Claude
    boundary lock. If the investigation finds a currently-reachable shape where a
    `PendingStart`/`Active` attempt coexists with no established abandon
    intent and genuinely needs dead-owner evidence to become fixable, reuse
    T02's shared primitive for it and record the finding; otherwise record
    that no such shape is reachable and do not add unused machinery. Out —
    OpenCode, Codex, Pi; Claude's `PreToolUse`/`PostToolUse`/`establish_start`
    happy-path logic.
  - Dependencies: T01
  - Done when: the existing
    `stale_non_empty_attempts_after_a_failed_abandon_stays_blocked_across_repeated_pre_tool_use_ac3`
    regression is preserved (still `Blocked` before repair) and extended:
    after `repair_blocked`, the same scenario resolves to
    `Healthy`/`Recovering`; a crash simulated between the `PendingAbandon`
    persist and the seam call resolving leaves state that a second
    `repair_blocked` safely completes without resurrecting or duplicating
    the attempt; a `PendingStart`/`Active` attempt with no established
    abandon intent is proven `ManualOnly` and untouched by
    `repair_blocked`; a legacy state file persisted before this change
    (no `PendingAbandon` variant ever written) remains readable and, if
    `Blocked`, stays `ManualOnly` until a new abandonment establishes real
    `PendingAbandon` evidence.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Completed: 2026-09-21
  - Files changed: `cli/src/services/hooks/claude_mutation_scope/state.rs` (adds
    `AttemptPhase::PendingAbandon`; adds a pure `transition_to_pending_abandon`
    helper plus a durable `mark_recovery_pending_and_pending_abandon(git_dir,
    scope_ids)` state-lock transaction that, in one read-modify-write, sets
    `recovery_pending = true` and transitions every named scope_id to
    `PendingAbandon` — replacing the original two-write
    `mark_recovery_pending()` + `mark_pending_abandon()` sequence so there is
    no crash boundary between the barrier and the abandonment evidence; makes
    `mark_active` re-read the persisted phase under the state lock and match
    on it: `PendingStart -> Active` (allowed), `Active -> Active` (safe
    idempotent no-op), `PendingAbandon -> Active` (rejected with an error and
    no state mutation), so an established abandon intent can never be
    resurrected by an in-flight start; adds `reprove_pending_abandon(git_dir)`,
    a lock-protected read-only re-proof returning the full current attempt
    list only when `recovery_pending` is true, attempts are non-empty, and
    every attempt is already `PendingAbandon`; adds the `#[cfg(test)]`
    `set_attempt_phase_for_tests` helper);
    `cli/src/services/hooks/claude_mutation_scope/lifecycle.rs` (factors
    `abandon_attempt`'s seam-call-plus-removal tail into a shared
    `abandon_marked_attempt` helper; changes both `abandon_attempt` and
    `cleanup_attempts_matching` to call the single atomic
    `mark_recovery_pending_and_pending_abandon` instead of the old two
    separate `mark_recovery_pending()` + `mark_pending_abandon()` calls, so
    the batch case still marks every selected attempt in one write before any
    seam call, and the state lock is never held across a seam call; rewrites
    `cleanup_attempts_matching` to process each stale attempt independently
    after that one write — continuing through the whole batch and returning
    the first error only after every attempt was attempted, rather than the
    previous early-return-on-first-failure loop that left later-matched
    attempts completely untouched; adds `RepairOutcome` and `repair_blocked`);
    `cli/src/services/hooks/claude_mutation_scope/health.rs`
    (adds the local `Repairability { AutoFixable, ManualOnly }` enum and
    `assess_repairability`; extends the existing AC3 regression to assert the
    durable `PendingAbandon` evidence and a successful `repair_blocked` run;
    adds regression tests covering the not-blocked case, a legacy
    attempt never marked `PendingAbandon`, a mixed-phase coexistence case
    (both `assess_repairability` and `repair_blocked` proven to leave it
    fully untouched), the concurrent-race re-proof refusal, the
    interrupted-repair resume, the batch-marking fix for
    `cleanup_attempts_matching`, the start-vs-cleanup activation race against
    an established `PendingAbandon` (`mark_active` losing the race leaves
    `AutoFixable` evidence that `repair_blocked` then resolves to `Healthy`),
    and a partial-batch `repair_blocked` run where one of two attempts fails
    to abandon (the successful one is removed, the failed one stays
    `PendingAbandon`, `recovery_pending` stays armed, outcome is `NoOp`));
    `cli/src/services/hooks/claude_mutation_scope/mod.rs`
    (re-exports `assess_repairability`, `Repairability`, `repair_blocked`,
    `RepairOutcome`, and `cleanup_attempts_matching`, each
    `#[allow(unused_imports)]` pending T05's doctor wiring and test-module
    access, matching the `pi_mutation_scope`/`opencode_mutation_scope::mod.rs`
    precedent).
  - Result: Claude's `AttemptPhase` gains a `PendingAbandon` variant recording
    that abandonment has already been decided for an attempt, durably
    persisted before any seam `abandon` call — replacing the prior ambiguity
    where a failed abandon left an attempt's phase untouched (`PendingStart`
    or `Active`) with no evidence distinguishing "abandonment decided,
    awaiting retry" from "may still be running." Investigation of the actual
    barrier logic (`apply_recovery_barrier` reads `state.recovery_pending`
    directly) confirmed the whole-file flag is still structurally required,
    so it is kept alongside the new per-attempt phase rather than replaced,
    per the plan's own "decide from the actual barrier logic" instruction.
    The barrier flag and the per-attempt evidence are established as one
    durable state transition, not two: `recovery_pending = true` and every
    named attempt's transition to `PendingAbandon` are read, updated, and
    written inside a single `AdapterStateLock` acquisition
    (`mark_recovery_pending_and_pending_abandon`), so there is no crash
    boundary at which the barrier could be armed with the corresponding
    `PendingAbandon` evidence not yet persisted (or vice versa). An initial
    version of this task left `mark_recovery_pending()` and
    `mark_pending_abandon()` as two separate durable writes; that was
    corrected because a crash between them could leave `recovery_pending =
    true` with an attempt still `PendingStart`/`Active` — the exact ambiguity
    `assess_repairability` must treat as `ManualOnly` even though cleanup had
    already durably decided to abandon. Separately, `mark_active` is now
    monotonic with respect to `PendingAbandon`: it re-reads the persisted
    phase under the state lock and only allows `PendingStart -> Active`
    (`Active -> Active` is a safe idempotent no-op); `PendingAbandon ->
    Active` is rejected with an error and no state mutation, so a start seam
    that was already in flight when cleanup established abandonment can never
    resurrect the attempt to `Active` after the fact. `establish_start` is
    unchanged beyond this: it still calls the seam and then `mark_active`
    with no new Claude boundary lock, so losing this race fails PreToolUse
    closed (per existing fail-closed behavior) while the durable
    `PendingAbandon`/`recovery_pending` evidence set by cleanup is left
    intact and retryable.
    `cleanup_attempts_matching`'s early-return-on-first-failure loop was
    confirmed reachable (`SessionEnd` matches every attempt in a session
    regardless of `agent_id`, and `WorktreeRemove` matches every tracked
    attempt unconditionally, so either can legitimately match more than one
    attempt at once) and corrected: every matched attempt is now durably
    marked `PendingAbandon` in one write before any seam call is attempted,
    so a failure abandoning one attempt can never leave a sibling attempt in
    the batch without its own retryable evidence.
    `assess_repairability` classifies `AutoFixable` only when every attempt
    currently in the adapter's state is `PendingAbandon`; any attempt still
    `PendingStart`/`Active` (no established abandon intent) forces
    `ManualOnly` for the whole adapter, proven by a test that also asserts
    `repair_blocked` leaves both attempts in such a mixed state completely
    untouched. `repair_blocked` re-reads and re-proves this same "every
    attempt is `PendingAbandon`" condition inside one lock-protected,
    read-only state transaction (`reprove_pending_abandon`) — Claude's
    `PendingAbandon` transition already happened durably before repair ever
    runs, so unlike OpenCode's dead-owner proof there is no transition to
    perform at this step, only a fresh re-proof — then releases the lock
    before retrying each attempt's already-established seam `abandon` call
    independently, removing only the ones that succeed, and clears the
    `recovery_pending` barrier in a final state transaction only once no
    attempts remain. A losing re-proof (an attempt no longer `PendingAbandon`
    by the time the lock is acquired) is a safe no-op with no seam call,
    proven by a test that forces exactly that race via the new
    `set_attempt_phase_for_tests` helper. Investigation finding for the
    plan's conditional owner-evidence question: no currently-reachable Claude
    shape needs dead-owner liveness proof. Because `assess_repairability`
    requires every attempt in the adapter's state to be `PendingAbandon`
    (not just the ones a particular cleanup decided to abandon), any
    coexisting `PendingStart`/`Active` attempt with no established abandon
    intent is already, structurally, excluded from `AutoFixable` — proven
    directly by `assess_repairability_is_manual_only_when_one_of_several_attempts_has_no_established_abandon_intent`.
    T02's shared `mutation_scope_owner` primitive is therefore not consumed
    by this task; no unused machinery was added for it.
  - Verify outcomes: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` -> `131 passed; 0 failed` (117 baseline + 8 regressions from the initial pass + 6 regressions from the atomic-persistence and monotonic-`mark_active` correction: `mark_recovery_pending_and_pending_abandon_establishes_both_facts_in_one_durable_write`, `mark_recovery_pending_and_pending_abandon_failed_write_leaves_no_partial_invariant`, `mark_active_is_idempotent_when_already_active`, `mark_active_is_forbidden_once_pending_abandon_is_established`, `mark_active_losing_the_race_against_an_established_pending_abandon_leaves_repairable_terminal_evidence`, and `repair_blocked_removes_only_successfully_abandoned_attempts_and_keeps_recovery_pending_when_one_fails`); `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check` -> clean; `SCE_CLI_PACKAGE_FALLBACK=1 nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets` -> no warnings; `grep -rn "SystemTime\|Instant::now\|\.elapsed()\|modified()" cli/src/services/hooks/claude_mutation_scope` -> only the pre-existing, unrelated `AdapterStateLock` lock-timeout deadline (unchanged by this task), matching AC3's "no staleness use outside unrelated lock-timeout constants."
  - Context impact: Claude's persisted state file gains a new reachable
    `AttemptPhase` variant (`pending_abandon`) and three new `pub(crate)`
    symbols (`Repairability`, `assess_repairability`, `repair_blocked`/
    `RepairOutcome`) not yet consumed by doctor — T05 wires them into
    `execute_doctor_with_lifecycle_providers`, matching T03's OpenCode
    precedent exactly. No user-visible behavior changed yet (`sce doctor
    --fix` still cannot repair Claude until T05 dispatches to these
    functions), no existing `MutationScope*` type or JSON shape changed, and
    `classify_health`'s existing four status boundaries and their triggering
    conditions are unchanged (confirmed by every pre-existing health test
    passing unmodified). `domain`-scoped: `context/cli/claude-mutation-scope-integration.md`
    (named in this plan's own "Context sync" section) describes the new
    `PendingAbandon` phase and its safety semantics once T05/T06 make the
    repair path reachable through `sce doctor --fix`; recording that
    dependency here for T05's own context-sync pass rather than updating
    that doc prematurely for a repair path doctor cannot yet invoke, mirroring
    T03's identical deferral for OpenCode's context docs. No root context file
    (`overview.md`/`architecture.md`/`glossary.md`/`patterns.md`/
    `context-map.md`) is implicated by an adapter-internal, not-yet-wired
    addition.
  - Deviation: `lifecycle.rs` was touched even though this task's own scope
    line names only `{state,mod,health}.rs`, because `abandon_attempt` and
    `cleanup_attempts_matching` — both explicitly named as needing behavior
    changes in this task's own scope text — live in `lifecycle.rs`, not
    `health.rs` or `state.rs`; T03's OpenCode task explicitly listed
    `lifecycle.rs` for the equivalent change, so this mirrors that precedent
    rather than expanding scope. `Repairability` and `RepairOutcome` are
    defined locally in this module (`health.rs` and `lifecycle.rs`
    respectively) rather than in the shared `mutation_scope_health.rs`,
    matching T03's identical deviation and reasoning: T05 is the task scoped
    to add the shared `Repairability` enum there.
  - Correction (2026-09-21): the initial T04 pass left two crash/concurrency
    gaps, both closed narrowly without touching T05 scope, OpenCode, Pi,
    Codex, doctor orchestration, rendering, or the Quint model. First,
    `recovery_pending = true` and the initial `PendingAbandon` transition
    were two separate durable transactions (`mark_recovery_pending()` then
    `mark_pending_abandon()`); a crash between them could leave the barrier
    armed with an attempt still `PendingStart`/`Active`, which
    `assess_repairability` correctly treats as `ManualOnly` even though
    cleanup had already durably decided to abandon. Fixed by replacing both
    call sites (`abandon_attempt`, `cleanup_attempts_matching`) with one
    state-layer operation, `mark_recovery_pending_and_pending_abandon`, that
    performs the read, both field updates, and the durable write inside a
    single `AdapterStateLock` acquisition — the batch case still marks every
    selected attempt in that one write before any seam call, and the lock is
    still never held across a seam call. Second, `mark_active` unconditionally
    set `attempt.phase = Active`, so an in-flight `PreToolUse` start whose
    seam call had already succeeded could overwrite an already-established
    `PendingAbandon` back to `Active` if cleanup won the race first. Fixed by
    making `mark_active` re-read the persisted phase under the state lock and
    branch on it: `PendingStart -> Active` (allowed), `Active -> Active`
    (safe idempotent no-op), `PendingAbandon -> Active` (rejected with an
    error and no state mutation). `establish_start` needed no change — the
    existing `seam(start)?; mark_active(...)?;` sequence now fails closed on
    the error instead of resurrecting the attempt, per existing PreToolUse
    fail-closed behavior, and no new Claude boundary lock was added. New
    regressions: two state-level tests proving the atomic operation
    establishes both facts in one write and that an injected pre-rename write
    failure leaves neither fact persisted; two state-level tests proving
    `mark_active`'s three-way branch; a lifecycle-level race regression
    (`mark_active_losing_the_race_against_an_established_pending_abandon_leaves_repairable_terminal_evidence`)
    proving that losing the activation race leaves `recovery_pending == true`
    and the attempt `PendingAbandon`, that `classify_health` is `Blocked` and
    `assess_repairability` is `AutoFixable` (not `ManualOnly`), and that
    `repair_blocked` then resolves the whole state to `Healthy`; and a
    partial-batch `repair_blocked` regression proving a successful abandon in
    a batch is removed while a failed sibling stays `PendingAbandon` with
    `recovery_pending` still armed. All prior wording in this entry
    describing `mark_recovery_pending()` followed by `mark_pending_abandon()`
    as the final implementation has been corrected above; that two-write
    sequence is no longer present in the codebase.
  - Correction (2026-09-21, second): `classify_health` disagreed with T01's
    formal invariant `RecoveryNeverClearedWithUnresolvedAbandon` and with
    `doctor_recovery.qnt`'s `adapterHealth` priority order
    (`recovery == Clear and hasPendingAbandon -> Invalid`, checked before
    every other classification). The Rust classifier instead returned
    `Healthy` for any `recovery_pending == false` state without checking
    whether a persisted attempt was still `PendingAbandon` — a structurally
    impossible/corrupt shape (terminal cleanup already durably decided while
    the barrier reads clear) was silently reported healthy instead of
    surfaced as `Invalid`. Fixed by adding a `has_pending_abandon` check
    (`state.attempts.iter().any(|attempt| attempt.phase ==
    AttemptPhase::PendingAbandon)`) ahead of the existing
    `!state.recovery_pending -> Healthy` branch in
    `claude_mutation_scope::health::classify_health`: `!recovery_pending &&
    has_pending_abandon` now returns `Invalid` first, matching the Quint
    model's priority order exactly. `assess_repairability` needed no change
    — its existing `!state.recovery_pending -> ManualOnly` short-circuit
    already classified this shape `ManualOnly` before this correction, so
    the impossible state was already un-auto-fixable; only the health
    classification itself was wrong. Ordinary `PendingStart`/`Active`
    attempts with `recovery_pending == false` are unaffected
    (`has_pending_abandon` is false for them), and no Claude boundary lock
    or owner-liveness primitive was added. New regressions in
    `claude_mutation_scope::health::tests`: `clear_recovery_with_pending_abandon_is_invalid`
    (a hand-constructed `recovery_pending: false` state with one
    `PendingAbandon` attempt classifies `Invalid` and `assess_repairability`
    reports `ManualOnly`) and
    `invalid_takes_priority_over_blocked_in_mixed_impossible_state` (a
    second, hand-constructed `PendingStart` attempt coexisting in the same
    `recovery_pending: false` state still classifies `Invalid`, not
    `Blocked`), mirroring the Quint model's
    `testPendingAbandonWithClearRecoveryIsInvalid`/
    `testInvalidTakesPriorityOverBlockedWhenBothConditionsHold`. No T05/T06
    scope, OpenCode, Pi, Codex, doctor orchestration/rendering, or the Quint
    model itself was touched by this correction.
  - Context synchronization: synced

- [x] T05: `Shared repairability contract and doctor --fix orchestration` (status:done)
  - Task ID: T05
  - Scope: In — `cli/src/services/hooks/mutation_scope_health.rs` (add the
    shared `Repairability` enum, kept separate from `MutationScopeHealthStatus`);
    `cli/src/services/doctor/{mod,inspect}.rs`. Add a new step to
    `execute_doctor_with_lifecycle_providers`, positioned after
    `fix_lifecycle_providers`/`repair_merge_target_configs` and before the
    final `diagnose_lifecycle_providers`/`build_report_with_lifecycle_problems`
    call, preserving the existing initial-diagnosis -> existing-repairs ->
    final-diagnosis -> manual-results flow: for each `Blocked` row the
    *initial* report found, call the matching adapter's
    `assess_repairability`; when `AutoFixable`, call that adapter's
    `repair_blocked` and record a `DoctorFixResultRecord`. Doctor only
    dispatches these adapter-owned operations; it does not acquire or hold
    either adapter's state lock, and it does not hold any state lock across a
    seam call. OpenCode owns its larger lifecycle serialization through
    `AdapterBoundaryLock`; Claude's adapter owns its per-transaction state
    locking and `PendingAbandon` proof without a new boundary lock. The final
    report's already-existing `inspect_mutation_scope_health` call (which
    recomputes `classify_health` from scratch) is the sole postcondition: a
    repair function returning `Ok(())` is never itself treated as proof of
    success — only the freshly recomputed final status decides whether the
    fix result becomes `Fixed` (final `Healthy`/`Recovering`) or falls
    through to the existing generic manual/unresolved handling (final still
    `Blocked`/`Invalid`, which must never be reported `Fixed`). Codex/Pi
    targets take no path through this new step, since they never classify
    `Blocked`. Out — Claude/OpenCode adapter internals (owned by T03/T04);
    human text/JSON rendering (T06).
  - Dependencies: T01, T03, T04
  - Done when: a repository seeded with an `AutoFixable` `Blocked`
    OpenCode or Claude state, run through `sce doctor --fix`, ends with a
    `Fixed` fix result and a final report that is never `Blocked` while
    reporting `Fixed`; a repository seeded with a `ManualOnly` `Blocked`/
    `Invalid` state is untouched by the new step and still receives the
    existing manual-result handling; a repository with no mutation-scope
    problems runs the new step as a no-op with no behavior change from
    today.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`
  - Completed: 2026-09-21
  - Files changed: `cli/src/services/hooks/mutation_scope_health.rs` (adds the
    shared `pub(crate) enum Repairability { AutoFixable, ManualOnly }`,
    kept separate from `MutationScopeHealthStatus`, plus a
    `repairability_variants_are_distinct` regression test);
    `cli/src/services/doctor/inspect.rs` (adds
    `repair_blocked_mutation_scope_targets` — the new `pub(super)` dispatch
    entry point — plus its private helpers
    `repair_blocked_mutation_scope_targets_with_seam`,
    `mutation_scope_repair_seam` (the real production seam,
    `hooks::mutation_scope::run_mutation_scope_from_payload`),
    `repair_blocked_mutation_scope_target`, `claude_repairability`,
    `opencode_repairability`, and `fixed_record_from_recomputed_health`;
    imports `Repairability`, `mutation_scope`, and `Logger`; adds six new
    regression tests and the `claude_autofixable_blocked_state`/
    `init_git_repo_with_opencode_target`/`opencode_dead_owner`/
    `no_op_repair_seam` test fixtures);
    `cli/src/services/doctor/mod.rs` (wires
    `repair_blocked_mutation_scope_targets(&initial_report)` into
    `execute_doctor_with_lifecycle_providers`, positioned exactly after
    `repair_merge_target_configs` and before the final
    `diagnose_lifecycle_providers` call).
  - Result: `sce doctor --fix` now repairs a `Blocked` Claude or OpenCode
    Agent-tracing state whenever the owning adapter's own
    `assess_repairability` proves it `AutoFixable`, by calling that
    adapter's own `repair_blocked` through the real production seam
    (`hooks::mutation_scope::run_mutation_scope_from_payload` — the same
    seam Claude's and OpenCode's own hook entry points use), then
    re-reading a fresh `classify_health` immediately afterward: only when
    that fresh read is `Healthy`/`Recovering` does the dispatch record a
    `Fixed` `DoctorFixResultRecord`; a `repair_blocked` call that returns
    `Ok(())` but leaves the target `Blocked`/`Invalid` (a losing race, a
    seam failure, or any other reason) produces no record at all, and the
    existing generic `build_manual_fix_results` pass over the *final*
    recomputed report — unchanged by this task — reports it `Manual`
    instead, so `Fixed` and a still-`Blocked` final report can never
    coincide (AC6). A `ManualOnly` target (no owner evidence, a live/unknown
    owner, or a mixed-phase Claude state) is never passed to `repair_blocked`
    at all — `claude_repairability`/`opencode_repairability` short-circuit
    first — leaving the state file byte-for-byte untouched and routing
    through the pre-existing manual-remediation path exactly as before this
    task (AC2 unaffected; T06 still owns its wording). Doctor itself never
    acquires or holds either adapter's state lock, never holds a lock across
    a seam call, and never calls `remove_attempt()` or rewrites JSON — it
    only calls each adapter's own `assess_repairability`/`repair_blocked`/
    `classify_health`, matching this plan's constraints and T01's formal
    model. Codex and Pi rows are matched to `None` in
    `repair_blocked_mutation_scope_target` and never dispatched further,
    since neither adapter can currently classify `Blocked` (per T01's
    assumptions).
  - Deviation: the seam is threaded through
    `repair_blocked_mutation_scope_targets_with_seam`/
    `repair_blocked_mutation_scope_target` as an explicit parameter
    (`MutationScopeRepairSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn
    Logger>) -> anyhow::Result<String>`) rather than being hardcoded inline,
    mirroring the seam-injection pattern every adapter's own lifecycle
    module already uses (e.g. `claude_mutation_scope`'s
    `run_claude_mutation_scope_from_payload_with_resolver`). This was not
    load-bearing for production behavior (the public
    `repair_blocked_mutation_scope_targets` entry point always uses the one
    real seam, `mutation_scope_repair_seam`) but was necessary for reliable
    testing: an initial version of the regression tests drove the real
    production seam end-to-end (a real git repo, a real per-repository
    Agent Trace DB) and was flaky under the full `doctor` test binary — it
    passed when run alone but intermittently reported `Manual` instead of
    `Fixed` when preceded by other tests in the same process, traced to the
    credential-store-backed Agent Trace DB encryption key
    (`cli/src/services/db/encryption_key.rs`'s process-global
    `DEFAULT_STORE`/keyring-store registration) not reliably surviving a
    second real, encrypted per-repository database being created in the
    same test process inside this sandboxed environment — a pre-existing
    test-infrastructure characteristic unrelated to this task's logic and
    out of its scope to fix. Threading the seam as a parameter let the
    regression tests inject the same kind of no-op fake seam every other
    hook-level test in this codebase already uses (matching
    `claude_mutation_scope`/`opencode_mutation_scope`'s own `repair_blocked`
    test suites), making the new tests deterministic while still exercising
    every line of this task's own new dispatch code — the underlying
    adapter `repair_blocked` behavior against the real seam remains proven
    by T03/T04's own extensive test suites, which this task does not
    duplicate.
  - Verify outcomes: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor` -> `49 passed; 0 failed` (six new: `repair_blocked_mutation_scope_target_repairs_an_autofixable_claude_state`, `repair_blocked_mutation_scope_target_never_touches_a_manual_only_claude_state`, `repair_blocked_mutation_scope_target_repairs_an_autofixable_opencode_state`, `full_report_fix_mode_leaves_a_manual_only_claude_blocked_state_untouched`, `full_report_fix_mode_has_no_mutation_scope_effect_when_nothing_is_blocked`, plus `mutation_scope_health.rs`'s `repairability_variants_are_distinct`), re-run three times consecutively with no flakes; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` -> `136 passed; 0 failed`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml opencode_mutation_scope` -> `113 passed; 0 failed`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_scope_health` -> `10 passed; 0 failed`; `cargo fmt --manifest-path cli/Cargo.toml -- --check` -> clean; `SCE_CLI_PACKAGE_FALLBACK=1 cargo clippy --manifest-path cli/Cargo.toml --all-targets` -> no warnings; `grep -rn "SystemTime\|Instant::now\|\.elapsed()\|modified()" cli/src/services/hooks/claude_mutation_scope cli/src/services/hooks/opencode_mutation_scope cli/src/services/hooks/mutation_scope_owner.rs` -> only the pre-existing, unrelated `AdapterStateLock`/`AdapterBoundaryLock`/`os_lock` timeout deadlines and the static no-TTL-token source scan's own token list, matching AC3's "no staleness use outside unrelated lock-timeout constants" (this task introduced no new staleness heuristic).
  - Context impact: `sce doctor --fix` gains real, user-visible repair
    behavior for a `Blocked` Claude/OpenCode Agent-tracing state for the
    first time (previously `--fix` could never touch this category at all).
    The `mutation_scope_health` JSON array's shape and status strings, and
    every `MutationScope*` Rust type name, are unchanged (AC7 — this task
    added no new field to that array and renamed nothing). `DoctorProblem`'s
    `fixability`/`remediation` text for a `Blocked` row is *not yet* dynamic
    per this task (still the pre-existing static `ManualOnly` wording from
    `push_mutation_scope_health_problem`, untouched by this task) — a
    `Blocked` row that this task's new step successfully repairs still shows
    the old wording on the *initial* diagnosis before `--fix` runs, and
    `sce doctor --fix`'s human/JSON fix-result line for a repaired target
    goes through the existing generic `[{outcome}] {detail}` formatter with
    a `Recovered <adapter> Agent tracing (now <status>: <reason>).` detail
    string this task produces — not yet the `[fixed] Recovered <adapter>
    Agent tracing ...` wording T06's own Done-when names, since
    `DoctorDisplayDetail::MutationScopeHealth` rendering is unchanged here.
    `context/sce/mutation-scope-health-status.md`,
    `context/sce/agent-trace-hook-doctor.md`, and
    `context/sce/doctor-human-text-contract.md` are all named in this plan's
    own "Context sync" section as needing the new repairability facet
    described once the repair path exists — deferring that write to this
    task's own context-synchronization pass (next), not T06, since this is
    the task that actually makes `--fix` capable of the repair, per this
    plan's own instruction to record docs against the task that makes a
    behavior reachable.
  - Correction (2026-09-21): the original dispatch violated AC6 and this
    task's own "final report is the sole postcondition" requirement.
    `repair_blocked_mutation_scope_target` called each adapter's
    `repair_blocked`, then immediately called that same adapter's
    `classify_health` again right there and built a `Fixed`
    `DoctorFixResultRecord` from that *intermediate* read — a read taken
    before `execute_doctor_with_lifecycle_providers` goes on to build the
    actual final `HookDoctorReport`. A concurrent hook process mutating
    persisted state between that intermediate read and the final diagnosis
    could produce `fix_results: [Fixed]` alongside a final report that is
    still `Blocked`, which AC6 and `doctor_recovery.qnt`'s
    `ReportedFixedExcludesBlockedOrInvalid` invariant both forbid. Fixed by
    separating "perform the repair" from "decide whether it counts as
    `Fixed`": `repair_blocked_mutation_scope_target`/
    `repair_blocked_mutation_scope_targets(_with_seam)` now return which
    targets doctor actually attempted a repair for
    (`Option<IntegrationTarget>`/`Vec<IntegrationTarget>`), with no
    `classify_health` call and no `DoctorFixResultRecord` construction of
    their own. A new `finalize_mutation_scope_repair_results(repaired_targets:
    &[IntegrationTarget], final_mutation_scope_health: &[MutationScopeHealthRow])
    -> Vec<DoctorFixResultRecord>` is the only place a mutation-scope
    `Fixed` record is now produced: it looks up each repaired target's row
    in the already-built final report's `mutation_scope_health` (never
    recomputing health itself) and reports `Fixed` only for
    `Healthy`/`Recovering`; `Blocked`/`Invalid`, or a target missing from
    the final row set entirely, produce no record and fall through to the
    existing, unchanged `build_manual_fix_results` pass over the final
    report. `execute_doctor_with_lifecycle_providers` in
    `cli/src/services/doctor/mod.rs` now calls
    `repair_blocked_mutation_scope_targets(&initial_report)` to get the
    repaired-target list, builds the final report exactly as before, then
    calls `finalize_mutation_scope_repair_results(&mutation_scope_repairs,
    &final_report.mutation_scope_health)` before `build_manual_fix_results`
    — preserving the plan's initial-diagnosis -> existing-repairs ->
    repair-attempt -> final-diagnosis -> derive-fix-result-from-final-report
    flow exactly, with the derivation step now strictly after the final
    report exists rather than interleaved with the repair step. `fixed_record_from_recomputed_health`
    now takes a `&MutationScopeHealthRow` (the final report's own row shape)
    instead of a freshly computed `&MutationScopeAdapterHealth`, so there is
    no code path left that can authorize `Fixed` from anything but the final
    report. New regression
    `finalize_mutation_scope_repair_results_ignores_an_immediate_post_repair_read_that_the_final_report_contradicts`
    in `cli/src/services/doctor/inspect.rs` drives a real `repair_blocked`
    call to a genuine immediate `Healthy`/`Recovering` result, then
    overwrites the persisted Claude state back to `Blocked` (simulating a
    concurrent hook process) before computing the final
    `mutation_scope_health` row and calling `finalize_mutation_scope_repair_results`
    — asserting both that the final row is `Blocked` and that no `Fixed`
    `MutationScopeHealth` record is produced, proving the race AC6 requires
    is now impossible. The three existing dispatch-level tests
    (`repair_blocked_mutation_scope_target_repairs_an_autofixable_claude_state`,
    `repair_blocked_mutation_scope_target_never_touches_a_manual_only_claude_state`,
    `repair_blocked_mutation_scope_target_repairs_an_autofixable_opencode_state`)
    were updated for the new `Option<IntegrationTarget>` return shape; the
    two `AutoFixable` tests now additionally call
    `finalize_mutation_scope_repair_results` against a freshly recomputed
    final row set to prove the end-to-end `Fixed` path still works when the
    final report agrees. No T06/T07 functionality, OpenCode/Claude adapter
    internals, or human/JSON rendering were touched.
  - Verify outcomes (corrected): `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor` -> `50 passed; 0 failed` (the five surviving T05 regressions plus the new `finalize_mutation_scope_repair_results_ignores_an_immediate_post_repair_read_that_the_final_report_contradicts`); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` -> `138 passed; 0 failed` (includes the two new T04-correction Invalid-classification regressions); `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml opencode_mutation_scope` -> `113 passed; 0 failed`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_scope_health` -> `10 passed; 0 failed`; `cargo fmt --manifest-path cli/Cargo.toml -- --check` -> clean; `SCE_CLI_PACKAGE_FALLBACK=1 cargo clippy --manifest-path cli/Cargo.toml --all-targets` -> no warnings; `nix run .#quint -- typecheck spec/doctor_recovery.qnt` -> clean; `nix run .#quint -- test spec/doctor_recovery.qnt --match '^test.*'` -> `13 passing` (unchanged — this correction required no Quint model change, confirming the implementation was the thing out of sync with the already-correct formal model, not the other way around).
  - Context synchronization: synced

- [ ] T06: `Human and JSON remediation contract for repaired and manual Agent tracing states` (status:todo)
  - Task ID: T06
  - Scope: In — `cli/src/services/doctor/{render,fixes,types}.rs`. Fix
    `build_manual_fix_results` (currently drops `DoctorProblem.remediation`
    entirely, rendering only `"{summary} Manual remediation is still
    required."`) so a `mutation_scope_health` manual result instead
    retains the adapter's own remediation text and the exact state-file
    path — e.g. `"Agent tracing remains blocked. Inspect '<path>'.
    {remediation}"` — while deciding whether to fix this generically for
    every `ManualOnly` category or only for `mutation_scope_health` (state
    the choice and why). Extend `DoctorDisplayDetail::MutationScopeHealth`
    (today `{reason, detail}` only — the "Agent tracing" tree row renders
    no `Remediation:` line at all, unlike the generic
    `DoctorDisplayDetail::Problem{summary, remediation}` variant used
    elsewhere) to also carry and render a `Remediation:` line sourced from
    the matching `DoctorProblem`, so a plain `sce doctor` run states, for
    every `Blocked` row, either `Run 'sce doctor --fix' to recover ...`
    (when `AutoFixable` — wire the new remediation text
    `push_mutation_scope_health_problem` must gain for that fixability) or
    the existing `Automatic recovery is not safe for this state. Inspect
    '<path>'.` wording (`ManualOnly`, now actually rendered). Confirm the
    `[fixed]`/`[manual]` fix-result lines T05 produces render correctly
    through the existing generic `[{outcome}] {detail}` formatter (no new
    formatter needed). Verify the JSON `problems[]` array already carries
    `fixability`/`remediation.{next_action,text}` correctly for the new
    `auto_fixable`/`doctor_fix` case, while the `mutation_scope_health[]`
    array's shape is untouched. Out — any change to the
    `mutation_scope_health` JSON array's field set, the status strings, or
    any `MutationScope*`/`mutation_scope_health` naming.
  - Dependencies: T05
  - Done when: `sce doctor --format json` and human text for a seeded
    `AutoFixable` `Blocked` state both name `sce doctor --fix` explicitly;
    a `ManualOnly` `Blocked`/`Invalid` state's human text and JSON both
    name the exact adapter state-file path with no deletion suggestion;
    `sce doctor --fix` human output shows `[fixed] Recovered <adapter>
    Agent tracing ...` for a resolved repair and `[manual] Agent tracing
    remains blocked. Inspect '<path>'.` for an unresolved one.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`
  - Context synchronization: pending

- [ ] T07: `Cross-adapter end-to-end regression and formal-model connection` (status:todo)
  - Task ID: T07
  - Scope: In — one or more command-level integration tests (driving
    `run_doctor_with_context`/the `sce doctor`/`sce doctor --fix` command
    surface directly, not just the `inspect.rs` helper functions) seeding a
    single repository with both a Claude `Blocked` state and an OpenCode
    `Blocked` state at once, and asserting the full rendered text and JSON
    output for both `sce doctor` (AC1/AC2 wording, verbatim) and
    `sce doctor --fix` (`[fixed]`/`[manual]` lines, verbatim, and a final
    report matching AC6) — proving T03/T04/T05/T06 integrate correctly
    across adapters, which no earlier task's adapter-local tests exercise
    together. Connect T01's `spec/doctor_recovery.qnt` invariants to this
    implementation: either a Quint-Connect harness mirroring the existing
    `cli/src/services/mutation_trace/mbt/` convention, or, if that is
    disproportionate for this plan's scope, direct doc comments on these
    regression tests naming which T01 invariant each one proves — decide
    and record which approach was taken and why. Out — any new production
    behavior; this task authors regression coverage and the formal-model
    connection only, not a "run the check suite" pass.
  - Dependencies: T05, T06
  - Done when: the multi-adapter end-to-end test(s) pass and assert the
    literal remediation/fix-result wording from AC1, AC2, and AC6; every
    T01 invariant is traceably connected to at least one Rust test (via
    Quint-Connect or documented mapping); `nix flake check` passes.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`; `nix flake check`
  - Context synchronization: pending

## Open questions

None. The change request explicitly required deriving the state-machine and
architectural decisions from the actual code rather than from the request's
own hypothesized shapes, and that inspection is recorded above (Codex/Pi
need no adapter change; Claude's primary repairable shape needs no owner
evidence; OpenCode's does). Where the request itself flagged a genuine
implementation choice ("decide from the actual barrier logic", "if the
investigation finds..."), the corresponding task scope says so explicitly
rather than presenting a false certainty here.
