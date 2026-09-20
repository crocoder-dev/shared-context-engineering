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

- [ ] T01: `Formalize the safe doctor-repair protocol and its invariants` (status:todo)
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
  - Context synchronization: pending

- [ ] T02: `Extract shared positive process-owner evidence from Pi` (status:todo)
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
  - Context synchronization: pending

- [ ] T03: `OpenCode: persisted owner evidence and safe PendingStart repair` (status:todo)
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
  - Context synchronization: pending

- [ ] T04: `Claude: persisted terminal-cleanup evidence and safe PendingAbandon repair` (status:todo)
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
  - Context synchronization: pending

- [ ] T05: `Shared repairability contract and doctor --fix orchestration` (status:todo)
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
  - Context synchronization: pending

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
