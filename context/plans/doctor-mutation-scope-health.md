# Plan: doctor-mutation-scope-health

## Change summary

`sce doctor` currently only checks whether each harness's mutation-scope hook
*registration* is installed and structurally current (files, hook entries,
Codex trust/policy state). It has no visibility into the mutation-scope
*runtime* state each adapter persists at
`<git-dir>/sce/{adapter}-mutation-scope-state.json`.

This plan is motivated by a real failure observed in the Claude
mutation-scope adapter. During an abandonment, the adapter's shared cleanup
path ran `mark_recovery_pending()` then called the seam's `abandon`
operation; the seam call failed, so `remove_attempt()` never ran (the error
short-circuited the cleanup helper). The repository was left with persisted
state approximately `recovery_pending = true` with one or more stale
`attempts` left over from an old session. Every subsequent mutation-capable
`PreToolUse` then hit the adapter's recovery barrier, which — with
`recovery_pending == true` and `attempts` non-empty — returns `Deny`
immediately. Crucially, the barrier's *only* self-healing path
(`{"operation":"flush"}` through the seam, which clears `recovery_pending`)
only runs when `attempts.is_empty()`; it never retries the failed abandon and
never removes the stale attempts on its own. The result was a persistent,
repository-wide lockout of mutation-capable Claude tools that required manual
state-file repair to clear — with no operator-facing signal anywhere in `sce
doctor`, in either its JSON or human text output. The only way to discover it
was to read the private state file directly and correlate it with test names
in the adapter's own source.

This plan extends existing behavior rather than replacing it: it adds a new,
adapter-owned diagnostic classifier per harness (Claude, Codex, OpenCode, Pi)
that maps that harness's own persisted state into one shared, generic status —
`healthy | recovering | blocked | invalid` — and wires `sce doctor` to report
that status per adapter in both `--format json` and the human text output,
using the existing `[PASS]`/`[WARN]`/`[FAIL]`/`[MISS]` vocabulary. `doctor`
never inspects or interprets a `RecoveryState` enum or an `attempts` list
itself; it only consumes the four-way status each adapter module already
computed.

The feature detects two genuinely different conditions and must not conflate
them:

- **recovery in progress / automatically recoverable** — the persisted state
  reflects unfinished recovery work, but an existing normal lifecycle event
  can advance it without manual intervention (`Recovering`);
- **a durable recovery wedge** — the persisted state blocks new mutation work
  and, once the hook call that produced it has returned, no future ordinary
  adapter event can clear it on its own (`Blocked`).

Not every `recovery_pending` (or equivalently-shaped) state is stuck. Claude's
`recovery_pending == true && attempts.is_empty()` is `Recovering`: the next
mutation-capable `PreToolUse` flushes and clears it automatically. Only
`recovery_pending == true && attempts non-empty` — the exact shape of the real
incident above — is `Blocked`. Codex, OpenCode, and Pi use a different,
generation-based `RecoveryState` state machine with additional attempt phases
(including `PendingAbandon` for OpenCode and Pi, and `ProcessOwner`-based
liveness for Pi); which combinations are transiently self-healing versus
genuinely stuck is not yet established for those three adapters, so this plan
includes that investigation, proven with tests that exercise the adapters'
real state-machine functions, before each adapter's classifier is
implemented.

## Health status definitions

Every adapter classifier returns exactly one of these four statuses. They are
shared, generic, and defined once (`healthy | recovering | blocked |
invalid`); no adapter redefines them.

### `healthy`

The persisted adapter state is valid and no recovery condition prevents normal
tracked-tool admission. Normal future mutation-capable work can proceed
without first completing a recovery operation. Absence of a state file is
`healthy` (see AC5) — it means "this adapter has no persisted recovery
problem," not "the adapter has already been exercised."

### `recovering`

The persisted adapter state is valid and currently reflects unfinished
recovery work, **but** the adapter has an existing normal lifecycle/admission
path that can make progress from this state without manual state surgery. If
currently executing hook calls finish, and future ordinary lifecycle/admission
events occur, the adapter has a defined automatic path that can eventually
clear the condition.

Examples may include: recovery pending with no unresolved attempts, where the
next tracked admission claims or performs a flush; a positively dead owner
that an existing stale-owner recovery boundary can retire; another proven
transient generation/recovery state whose normal successor path advances it.

Do not classify something `recovering` merely because its enum variant is
named `Pending` or `Flushing`. Each `recovering` classification must be proven
by driving the adapter's real recovery path and observing it advance — see
[T03–T05](#task-stack) and AC4.

### `blocked`

The persisted adapter state is valid but new tracked mutation work is denied,
**and**, assuming any hook invocation that produced the state has already
returned, future ordinary lifecycle/admission events have no normal automatic
path that can clear the blocker. In other words: valid persisted state + a
fail-closed admission decision + no reachable self-healing path from future
normal events = `blocked`.

This is the status that must detect the real Claude incident. `blocked` does
not need to prove that no process anywhere on the machine could possibly still
be finishing a write at the exact instant doctor reads the file (see
[Read-only observation semantics](#read-only-observation-semantics) below); it
means that the persisted state, treated as the durable state from which the
next ordinary event must continue, has no normal self-healing transition.

Do not weaken this plan into a classifier that merely reports "currently
fail-closed." A transient recovery state that denies admission right now but
has a proven future self-healing path is `recovering`, not `blocked`. The
distinction the classifier exists to draw is exactly this one: does the
persisted state have a normal future self-recovery path, or not.

### `invalid`

The state file exists but cannot be safely interpreted: malformed JSON,
unsupported version, read error, or a structurally impossible persisted
combination proven unreachable by the adapter's own state machine (proven by
tests/code, not assumed). `invalid` is never used merely because admission is
currently blocked — that is `blocked`. `invalid` means the state cannot be
trusted or interpreted at all, not that it is a durable wedge.

### Read-only observation semantics

Doctor is a read-only snapshot. There is a narrow observation window in which
doctor reads a state shape that a currently executing hook process is about
to change — for example Claude may briefly persist `recovery_pending = true`
with non-empty `attempts` between `mark_recovery_pending()`, the `abandon`
seam call, and `remove_attempt()`, all inside one still-running hook process.

This plan does not attempt to close that window by weakening `blocked`.
Health classifies the durable state observed at inspection time according to
whether normal future adapter lifecycle events can recover from that
persisted state, assuming the operation that produced it has stopped
progressing. This feature is intended to expose durable wedges; it is not a
distributed-process liveness oracle, and it does not prove that no currently
executing process could write a newer state milliseconds later. The
classifier stays pure and read-only regardless.

## Acceptance criteria

- [ ] AC1: `sce doctor --format json` reports mutation-scope health
      (`healthy | recovering | blocked | invalid`) for every configured/
      detected integration target whose mutation-scope adapter applies (see
      AC5 for the absent-state-file case), with a stable machine-readable
      reason/detail attached whenever the status is not `healthy`. Doctor does
      not render mutation-scope health for an integration target it did not
      resolve as configured/detected.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor` (JSON-shape assertions), plus manual `sce doctor --format json` against a repository with hand-seeded adapter state files in each status.
- [ ] AC2: `sce doctor` human text output maps `healthy -> [PASS]`,
      `recovering -> [WARN]`, `blocked -> [FAIL]`, `invalid -> [FAIL]`.
      Healthy rows follow the existing compact healthy-row contract
      (`context/sce/doctor-human-text-contract.md`); `recovering`, `blocked`,
      and `invalid` rows expand with a short human reason.
  - Validate: rendering unit tests in `cli/src/services/doctor/render.rs`'s test module asserting the row shape for each of the four statuses.
- [ ] AC3: For the Claude adapter, tests prove:
  - `recovery_pending == false` -> `Healthy`.
  - `recovery_pending == true` with `attempts` empty -> `Recovering`, **and**
    the next normal recovery-capable boundary (the adapter's own
    `{"operation":"flush"}` recovery-barrier path) actually flushes and clears
    `recovery_pending`.
  - `recovery_pending == true` with `attempts` non-empty -> `Blocked`, **and**
    after the operation that created the state has returned, ordinary future
    mutation-capable `PreToolUse` calls deny without advancing it (repeated
    denial, not just one).
  - Include a regression reproducing the actual failure sequence: allocate/
    persist a live attempt, mark recovery pending, simulate an `abandon` seam
    failure so the stale attempt remains, then assert the next tracked
    `PreToolUse` is denied and a second, later tracked `PreToolUse` is *still*
    denied (it never becomes `recovering` or self-clears). Doctor/classifier
    must report `Blocked` for that persisted state. This PR diagnoses the
    incident; it does not fix the underlying Claude recovery-barrier bug.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` (or equivalent module path chosen in T02).
- [ ] AC4: For each of Codex, OpenCode, and Pi:
  1. a table-driven classification matrix covers every meaningful reachable
     `RecoveryState`/attempt-phase combination;
  2. real-dispatch behavioral tests prove the future admission/recovery
     semantics for every distinct semantic equivalence class used by that
     matrix — multiple matrix rows may rely on the same behavioral proof when
     they are equivalent under a demonstrated invariant;
  3. the classifier implements exactly those proven semantics; and
  4. classifications are never inferred merely from enum/variant names.

  The matrix test is the exhaustiveness layer (every meaningful persisted
  state combination is classified); the real-dispatch regressions are the
  behavioral-justification layer (what ordinary future admission/lifecycle
  behavior actually does for each distinct equivalence class). Neither
  substitutes for the other, and AC4 is not satisfied by classifier-only
  testing.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex_mutation_scope`, `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml opencode_mutation_scope`, `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope` (or equivalent module paths chosen in T03–T05).
- [ ] AC5: A harness whose mutation-scope state file does not exist in this
      repository (never run, or never set up) reports `healthy`. A state file
      that exists but fails to parse (malformed JSON, unsupported version, read
      failure) reports `invalid` with the read error surfaced in the detail/
      reason. A structurally impossible parseable state may be reported as
      `invalid` only when tests/code prove the adapter cannot legitimately
      persist it. This holds for every adapter.
  - Validate: per-adapter unit tests for the absent-file and malformed-file cases (same test modules as AC3/AC4).
- [ ] AC6: Doctor consistency — for every adapter, `recovering` produces a
      `Warning`-severity `DoctorProblem` and overall readiness may remain
      `ready`; `blocked` and `invalid` each produce an `Error`-severity
      `DoctorProblem` and overall readiness is `not_ready`. JSON health
      status, `DoctorProblem` severity, top-level readiness, the human-text
      row, and the summary warning/blocking-problem counts never contradict
      each other.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`, including readiness/summary-count assertions for a `recovering`-only repository and a `blocked`/`invalid` repository.

### Full validation

- `nix flake check`

### Context sync

- `context/sce/agent-trace-hook-doctor.md` (doctor's canonical health-and-repair contract must describe this new runtime-liveness facet alongside its existing structural-registration checks, including the new `Warning`/`Error` problem-severity mapping for `recovering`/`blocked`/`invalid`)
- `context/sce/doctor-human-text-contract.md` (must document the new status rows and their `[PASS]`/`[WARN]`/`[FAIL]` vocabulary mapping)
- `context/cli/claude-mutation-scope-integration.md`, `context/cli/codex-mutation-scope-integration.md`, `context/cli/opencode-mutation-scope-integration.md`, `context/cli/pi-mutation-scope-integration.md` (each adapter's proven recovering-vs-blocked mapping and the reasoning behind it belongs in that adapter's own authoritative doc)
- `context/context-map.md`, only if navigation actually needs updating
- Do not create a new shared mutation-scope-health context document unless implementation reveals shared semantics that cannot cleanly live in the existing docs listed above; the [Health status definitions](#health-status-definitions) section of this plan is the shared contract's origin and should migrate into `context/sce/agent-trace-hook-doctor.md` during T06 context sync rather than spawning a new file.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/claude_mutation_scope/`,
  `cli/src/services/hooks/codex_mutation_scope/`,
  `cli/src/services/hooks/opencode_mutation_scope/`,
  `cli/src/services/hooks/pi_mutation_scope/` (new read-only health-classifier
  functions and their tests only); a new small shared status-type module used
  by all four adapters and by doctor; `cli/src/services/doctor/{types,inspect,render,mod}.rs`
  to consume and render the four-way status and wire it into the existing
  `DoctorProblem`/readiness model; the context docs listed above.
- **Out of scope:** any change to the underlying recovery/barrier protocol
  logic, `RecoveryState` transitions, attempt terminal semantics, attribution
  semantics, or fail-closed behavior — this plan only reads and classifies
  existing persisted state, never changes when or how a barrier arms, clears,
  or retries. It does not fix the Claude recovery-barrier bug described in the
  change summary.
- **Constraints:** classifier functions are pure and read-only — they must
  never write to an adapter's state file (doctor is diagnostic-only, matching
  the existing `ServiceLifecycle::diagnose` vs `fix` split); doctor reports an
  adapter's mutation-scope health only for a target it already detects or has
  configured, consistent with its existing configured/detected/empty target
  resolution; the shared status type and its `healthy | recovering | blocked |
  invalid` vocabulary is defined once and reused by all four adapters, not
  redefined per adapter; non-`healthy` statuses participate in the existing
  `DoctorProblem`/readiness model rather than existing as a parallel
  display-only system (see AC6).
- **Non-goal:** this plan does not add a `doctor --fix` remediation path for a
  `blocked` or `invalid` mutation-scope state. Given the barrier's deliberate
  fail-closed design (D12/D19: a lost abandonment must not be silently
  forgotten), automatic repair is a separate decision this plan does not make;
  the goal here is making the stuck state visible, not resolving it
  automatically. Specifically, `sce doctor --fix` must not delete state files,
  delete attempts, clear `recovery_pending`/`RecoveryState`, increment or
  reset generations, fabricate an abandon, or reset process-owner evidence.
  For `blocked`/`invalid` records, T06 must supply deterministic manual
  remediation guidance and must not casually recommend deleting the state
  file; if no safe generic recovery command currently exists for a given
  adapter/status, the remediation text must say so rather than inventing one.
  Designing that safe canonical recovery operation is a separate change.

## Task stack

- [x] T01: `Define the shared mutation-scope health status contract` (status:done)
  - Task ID: T01
  - Scope: In — a new small module (e.g. `cli/src/services/mutation_trace/scope_health.rs` or a peer location chosen at implementation time) defining the `MutationScopeHealthStatus` enum (`Healthy`, `Recovering`, `Blocked`, `Invalid`) and the shared per-adapter health record (adapter identity, status, human-readable reason, and any machine detail doctor needs to render or serialize it). Out — any adapter-specific classification logic, and any doctor wiring.
  - Dependencies: none
  - Done when: the shared `MutationScopeHealthStatus` enum exists; the per-adapter `MutationScopeAdapterHealth` record exists; the type derives whatever traits its current consumers need (`Debug`/`Clone`/`PartialEq`/`serde::Serialize` as required by JSON rendering); the record carries adapter identity, status, a human-readable reason, and optional machine detail; focused unit tests cover the shared type/record behavior; and no adapter-specific classifier logic or doctor integration is introduced. The detailed semantics of `Healthy`/`Recovering`/`Blocked`/`Invalid` remain authoritative in this plan's [Health status definitions](#health-status-definitions) and are synchronized into durable context by T06; they do not need to be duplicated as comments in the Rust type.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml scope_health`
  - Context synchronization: synced
  - Completed: 2026-09-19
  - Files changed: `cli/src/services/hooks/mutation_scope_health.rs`, `cli/src/services/hooks/mod.rs`
  - Result: Added the shared `MutationScopeHealthStatus` enum (`Healthy`/`Recovering`/`Blocked`/`Invalid`) and `MutationScopeAdapterHealth` record (adapter identity via the existing `ActorKind`, status, human-readable reason, optional machine detail) in a new `cli/src/services/hooks/mutation_scope_health.rs` module, registered in `hooks/mod.rs`. No adapter classification logic or doctor wiring was added.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml scope_health` — pass (3 tests).
  - Context impact: domain — no root context file currently describes mutation-scope health status; T06's context sync is where the shared status vocabulary migrates into `context/sce/agent-trace-hook-doctor.md` per the plan's context-sync section. This task introduces no new adapter-facing or user-facing contract by itself.

- [x] T02: `Classify Claude mutation-scope health` (status:done)
  - Task ID: T02
  - Scope: In — a read-only function in `cli/src/services/hooks/claude_mutation_scope/` that reads `claude-mutation-scope-state.json` via the existing `state::read_state` and maps it to the T01 status type using the proven rule from AC3 (`recovery_pending && attempts non-empty` → `Blocked`, because the recovery barrier's only self-healing path — `{"operation":"flush"}` — only runs when `attempts.is_empty()`, so a non-empty `attempts` list has no reachable future self-healing transition; `recovery_pending && attempts empty` → `Recovering`, because that flush path does run and does clear `recovery_pending`; neither → `Healthy`; a `read_state` error → `Invalid` with the error surfaced); absent-file handling (already `Ok(AdapterState::default())` in `read_state`) must classify as `Healthy`. Out — doctor wiring, other adapters, any change to the recovery barrier itself.
  - Dependencies: T01
  - Done when: unit tests cover all four resulting statuses, the absent-file case, and the AC3 regression scenario (stale non-empty-`attempts` state persists across two successive tracked `PreToolUse` calls without self-clearing).
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Context synchronization: synced
  - Completed: 2026-09-19
  - Files changed: `cli/src/services/hooks/claude_mutation_scope/health.rs` (new), `cli/src/services/hooks/claude_mutation_scope/mod.rs`, `cli/src/services/hooks/claude_mutation_scope/state.rs`
  - Result: Added `classify_health(git_dir) -> MutationScopeAdapterHealth` in a new `claude_mutation_scope/health.rs`, registered as `pub(crate) mod health;` in `mod.rs`. It calls the existing `state::read_state` and maps: a read/parse error → `Invalid` with the error surfaced via `.with_detail(...)`; `recovery_pending == false` (including the absent-file default) → `Healthy`; `recovery_pending == true` with empty `attempts` → `Recovering`; `recovery_pending == true` with non-empty `attempts` → `Blocked`. Made `state::state_path` `pub(crate)` (was module-private) so the malformed-file test could target the real state file path without duplicating the filename constant. No doctor wiring, other-adapter logic, or recovery-barrier behavior was touched.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope` — pass (119 tests, including the new `health::tests` module: absent-file, `recovery_pending=false` with live attempts, empty-attempts recovering, non-empty-attempts blocked, malformed-file invalid, and the AC3 regression driving a real failed-abandon seam call then asserting `apply_recovery_barrier` denies twice in a row with `classify_health` reporting `Blocked` throughout).
  - Context impact: domain — `context/cli/claude-mutation-scope-integration.md` now records the proven Healthy / Recovering / Blocked / Invalid mapping in a new "Mutation-scope health" section adjacent to "Abandonment cleanup signals" / "The recovery barrier", including the behavioral reason for Recovering (the recovery barrier's own flush + `clear_recovery_pending` path) versus Blocked (the flush path never runs when `attempts` is non-empty, so ordinary future `PreToolUse` calls keep denying without advancing recovery — the exact incident shape), the read-only boundary, and the observation-window nuance.

- [x] T03: `Investigate and classify Codex mutation-scope health` (status:done)
  - Task ID: T03
  - Scope: In — for every reachable `RecoveryState` value in `cli/src/services/hooks/codex_mutation_scope/state.rs` (`Clear`, `Pending { generation }`, `Flushing { generation }`) and its interaction with attempt state, answer for each: (1) is this combination actually reachable through production behavior; (2) is it valid persisted state; (3) once the hook call that created it has returned, what happens on the next ordinary tracked admission/lifecycle event; (4) does an existing code path advance recovery; (5) can it return to normal admission without manual state-file surgery; (6) is progress dependent on an event that can no longer occur; (7) does the next Start merely deny forever; (8) for owner-aware paths, can positive process-death evidence trigger existing recovery. Drive the adapter's real dispatch/barrier logic (the way its own existing test suite does) to answer these, not the enum names. A table-driven classification matrix covers every meaningful persisted `RecoveryState`/attempt-phase combination for Codex, and real-dispatch behavioral tests prove the future admission/recovery semantics for every distinct semantic equivalence class represented by that matrix — multiple matrix rows may share one behavioral proof when a demonstrated invariant makes them semantically equivalent; a classifier function implements exactly that proven mapping against the T01 status type. Out — OpenCode, Pi, doctor wiring.
  - Dependencies: T01
  - Done when: a table-driven classification matrix covers every meaningful persisted `RecoveryState`/attempt-phase combination for Codex; real-dispatch behavioral tests prove every distinct semantic equivalence class represented by that matrix, and multiple matrix rows may share one behavioral proof when equivalence follows from a demonstrated invariant; the classifier matches those proven semantics exactly; and absent/malformed state handling satisfies AC5.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex_mutation_scope`
  - Context synchronization: synced
  - Completed: 2026-09-19
  - Files changed: `cli/src/services/hooks/codex_mutation_scope/health.rs` (new), `cli/src/services/hooks/codex_mutation_scope/mod.rs`, `cli/src/services/hooks/codex_mutation_scope/state.rs`
  - Result: Investigated every reachable `(RecoveryState, attempts)` combination by reading and driving `codex_mutation_scope`'s real dispatch/recovery code (`admit_tracked_attempt`, `sweep_stale_lane_predecessors`, `cleanup_attempts_matching`, `normalize_recovery_after_boundary_lock_acquired`) and its existing test suite (notably `lifecycle_cleanup_with_a_failed_abandon_keeps_the_attempt_tracked_d12`, `recovery_barrier_flushes_once_quiescent_then_starts_ac12`, `test_i_orphaned_flushing_is_reclaimed_and_flush_is_retried_once`, `test20_recovery_pending_blocks_a_tracked_successor_until_recovery_succeeds_ac12`). Added `classify_health(git_dir) -> MutationScopeAdapterHealth` in a new `codex_mutation_scope/health.rs`, registered as `pub(crate) mod health;` in `mod.rs`: `RecoveryState::Clear` → `Healthy` (attempts alone are ordinary in-flight lifecycle state, not a recovery condition); `Pending{g}` with attempts empty → `Recovering`, proven by driving `admit_tracked_attempt` and observing `FlushClaimed`; `Flushing{g}` with attempts empty → `Recovering`, proven by driving `normalize_recovery_after_boundary_lock_acquired` then `admit_tracked_attempt` and observing the reclaimed generation retried as `FlushClaimed`; `Flushing{g}` with attempts non-empty is structurally impossible through production behavior (the only transition into `Flushing` requires attempts to already be empty, and `admit_tracked_attempt` refuses all new attempts while `Flushing`) and is classified `Invalid` if ever observed in a hand-seeded file; a read/parse error → `Invalid` with the error surfaced. Made `state::state_path` `pub(crate)` (was module-private) so `health.rs`'s malformed/hand-seeded-file tests could target the real state file path, mirroring T02.
    - **Correction (this task, applied after initial completion):** `Pending{g}` with attempts non-empty was initially classified `Blocked` on the theory that the global `RecoveryBlocked` gate in `admit_tracked_attempt` denies every tracked `PreToolUse` unconditionally. That theory was wrong: the real `PreToolUse` order is `with_boundary_lock -> normalize_recovery_after_boundary_lock_acquired -> sweep_stale_lane_predecessors -> admit_or_recover -> admit_tracked_attempt`, so `sweep_stale_lane_predecessors` — which retries the abandonment of a stale same-`(session_id, turn_id)`-lane predecessor — always runs *before* the `RecoveryBlocked` gate is reached for that same call. A same-lane successor can therefore retry and clear the stuck attempt, driving `Pending` to empty and self-admitting through `FlushClaimed`, entirely through existing ordinary lifecycle behavior. This is a proven normal self-healing route, so the correct classification is `Recovering`, not `Blocked`; unrelated admission (a different session, or the same session with a different `turn_id`) still denies fail-closed in the meantime, but that fail-closed behavior does not by itself make the state `Blocked` under this plan's [Health status definitions](#health-status-definitions). The classifier, its reason text, and the state-level `health.rs` test were corrected accordingly, and a new real same-lane driver regression (`same_lane_successor_retries_abandon_and_reaches_healthy_after_a_failed_lifecycle_abandon_ac4`) now proves the `abandon(A) -> flush -> start(C)` self-healing path end to end. No recovery/barrier production behavior changed; only the classifier's mapping and its supporting tests/docs were corrected.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex_mutation_scope` — pass, including the `health::tests` module (absent-file, Clear-with-attempts, Pending-empty-recovering, `pending_non_empty_recovery_denies_unrelated_admission_without_losing_the_recovery_path` [Recovering], orphaned-Flushing-recovering, hand-seeded Flushing-non-empty-invalid, malformed-file-invalid) and the `tests::driver` real-dispatch regressions: `pending_non_empty_recovery_denies_unrelated_admission_without_losing_the_recovery_path` (a failed `SessionEnd` abandon leaves the state `Recovering`, and two successive real `PreToolUse` calls from an unrelated session are both denied without the seam being invoked, while the classifier keeps reporting `Recovering` throughout), `health_classifies_recovering_then_healthy_once_the_next_pre_tool_use_flushes_ac4` (`Pending` + empty attempts self-heals to `Healthy`), and the new `same_lane_successor_retries_abandon_and_reaches_healthy_after_a_failed_lifecycle_abandon_ac4` (a same-`(session_id, turn_id)`-lane successor retries and clears a stale predecessor's abandon, then a further same-lane call drives the real `abandon(A) -> flush -> start(C)` seam-call ordering to `Healthy`).
  - Context impact: domain — `context/cli/codex-mutation-scope-health.md` is the canonical detailed owner of the proven Healthy/Recovering/Blocked/Invalid mapping and its reasoning (the global `RecoveryBlocked` gate, the same-lane sweep running *before* that gate for the current call, and the orphaned-flush reclaim path), corrected to remove the false claim that the recovery gate runs before the same-lane sweep and to reclassify non-empty `Pending` as `Recovering`; `context/cli/codex-mutation-scope-integration.md` links to it from its "Recovery and durable state" section rather than duplicating the mapping; `context/context-map.md` registers the health document.

- [x] T04: `Investigate and classify OpenCode mutation-scope health` (status:done)
  - Task ID: T04
  - Scope: In — the same investigation and classifier work as T03 (the same eight questions, the same "what future event gets this state out?" requirement, the same `Clear`/`Pending { generation }`/`Flushing { generation }` `RecoveryState` coverage), applied to `cli/src/services/hooks/opencode_mutation_scope/state.rs`, including its additional `PendingAbandon` attempt phase — investigate whether an unresolved `PendingAbandon` has any existing automatic successor path, or whether it can only be classified `Blocked` once its recovery boundary is proven to have no future normal advance. Out — Codex, Pi, doctor wiring.
  - Dependencies: T01
  - Done when: the full OpenCode classification matrix is table-driven and complete; every distinct semantic equivalence class underlying that matrix is proven through real adapter dispatch/recovery behavior; the classifier matches those proven semantics exactly; and absent/malformed state handling satisfies AC5.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml opencode_mutation_scope`
  - Context synchronization: synced
  - Completed: 2026-09-19
  - Files changed: `cli/src/services/hooks/opencode_mutation_scope/health.rs` (new), `cli/src/services/hooks/opencode_mutation_scope/mod.rs`, `cli/src/services/hooks/opencode_mutation_scope/state.rs`, `context/cli/opencode-mutation-scope-health.md` (new), `context/cli/opencode-mutation-scope-integration.md`, `context/context-map.md`
  - Result: Investigated every reachable `(RecoveryState, AttemptPhase)` combination by reading and driving `opencode_mutation_scope`'s real dispatch/recovery code (`admit_tracked_attempt`, `establish_tracked_start`, `resolve_recovery`, `normalize_recovery_after_boundary_lock_acquired`, `begin_terminal_cleanup`) and its existing regression suite (`regression_a_abandon_failure_preserves_terminal_intent_then_recovers`, `regression_b_ambiguity_flush_failure_blocks_new_starts_then_recovers`, `regression_c_rebaseline_flush_failure_is_recoverable_without_poison`, `a_close_before_start_confirmation_consumes_rather_than_closes`, `server_disposed_cannot_sweep_another_processes_attempt`). Added `classify_health(git_dir) -> MutationScopeAdapterHealth` in a new `opencode_mutation_scope/health.rs`, registered as `pub(crate) mod health;` in `mod.rs`: `RecoveryState::Pending{g}` and `RecoveryState::Flushing{g}` are always `Recovering` regardless of attempt state — unlike Codex's same-lane-scoped sweep, `admit_tracked_attempt`'s flush claim and `resolve_recovery`'s retry-every-`PendingAbandon`-attempt loop are not scoped to any particular session or call, so the very next tracked admission from *any* call always has a proven path to advance recovery, and an orphaned `Flushing` is always reclaimed to `Pending` by `normalize_recovery_after_boundary_lock_acquired` on the next boundary; `RecoveryState::Clear` with an attempt stuck in `PendingStart` is `Blocked` — this state sits outside the `RecoveryState` machine entirely, only that exact `(session_id, call_id)`'s own `ToolExecuteAfter`/`ToolError` (or a duplicate `Start` redelivery) retires it, no boundary sweeps a stale `PendingStart` on behalf of an unrelated call (proven inert for `ServerDisposed` by the adapter's own existing test), and OpenCode has no process-liveness detection analogous to Pi's `is_definitely_dead()`, so a crashed owning process leaves it a permanent wedge — the same "valid state + fail-closed admission + no reachable self-healing path" shape as the Claude incident this plan exists to surface; `RecoveryState::Clear` with a `PendingAbandon` attempt is structurally impossible (proven unreachable: `PendingAbandon` is only ever set atomically with `Flushing` in `begin_terminal_cleanup`, and recovery only returns to `Clear` after every `PendingAbandon` attempt has already been removed in `resolve_recovery`) and is classified `Invalid` if hand-seeded; a read/parse error → `Invalid` with the error surfaced. Made `state::state_path` `pub(crate)` (was module-private) so the malformed/hand-seeded-file tests could target the real state file path, mirroring T02/T03. No doctor wiring, other-adapter logic, or recovery-barrier production behavior was touched.
    - **Correction (this task, applied after initial completion):** the initial classifier's claim that `Pending`/`Flushing` are unconditionally `Recovering` "regardless of attempt state" was wrong. `resolve_recovery` only ever iterates and retires attempts already in `AttemptPhase::PendingAbandon`; it never inspects a `PendingStart` attempt. A reachable production sequence — A starts and is later abandoned via `ToolError` while B's own `Start` seam has already failed and left B `PendingStart` — persists `Pending` recovery (A `PendingAbandon`) alongside B's unrelated `PendingStart`, and the original classifier reported that `Recovering`. It is not: once recovery resolves A to `Clear`, the durable state becomes `Clear` + B `PendingStart`, which the classifier itself already (correctly) reports as `Blocked` — so the "Recovering" verdict was for a state with no ordinary future path back to normal admission, violating this plan's Recovering definition. The same shape recurs for orphaned `Flushing`. Fixed by reordering `classify_health`'s match so `has_pending_start` is checked as one condition spanning every `RecoveryState` value (`Clear`, `Pending`, and `Flushing` alike, whether or not a `PendingAbandon` is also outstanding), ahead of the `Pending`/`Flushing` → `Recovering` arms; only the `Clear` + `PendingAbandon` → `Invalid` structural-impossibility check still runs first. `Pending`/`Flushing` remain `Recovering` exactly when no `PendingStart` is outstanding. Two real-dispatch regressions were added: `pending_recovery_with_a_pending_start_attempt_is_blocked_even_though_an_unrelated_pending_abandon_can_still_clear_ac4` (the `Pending` critical regression: drives A through start/abandon with a failing ambiguity flush to leave `Pending` + mixed `PendingAbandon`/`PendingStart`, asserts `Blocked`, then drives an unrelated call C through real recovery resolution to `Clear` + `PendingStart` and asserts a further unrelated D is still denied and `Blocked` persists) and `orphaned_flushing_with_a_pending_start_attempt_is_blocked_not_recovering_ac4` (the `Flushing` analog, using a direct `begin_terminal_cleanup` call to simulate a crash before `resolve_recovery` ever ran, since that is the only way to observe a literal `Flushing` at rest). The full twelve-cell `(RecoveryState, has_pending_abandon, has_pending_start)` matrix — reachability, status, and what future event advances each cell — is recorded in `context/cli/opencode-mutation-scope-health.md`. No recovery/barrier production behavior changed; only the classifier's mapping, its reason text, and its supporting tests/docs were corrected.
    - **Correction (this task, second pass — AC4 completeness gap closed):** the prior completion record claimed the "table-driven behavioral tests in AC4 pass for OpenCode" but the suite only ever drove individually named real-dispatch scenarios; no test actually enumerated all twelve `(RecoveryState, has_pending_abandon, has_pending_start)` cells as a table. Added `health_classification_matrix_covers_all_twelve_recovery_and_attempt_phase_combinations` in `cli/src/services/hooks/opencode_mutation_scope/health.rs`: a `[(RecoveryState, bool, bool, MutationScopeHealthStatus); 12]` table (via two small test-only helpers, `matrix_attempt` and `write_matrix_state`) that hand-builds adapter state for every cell — always including one `Active` attempt to also prove active attempts never change the classification — and asserts `classify_health` against the documented matrix. This is a completeness proof over the classifier's output, not a substitute for the existing real-dispatch regressions, which remain and continue to prove *why* the `Blocked` (mixed `PendingStart`), `Recovering` (`Pending`/`Flushing` with no `PendingStart`), and `Invalid` (`Clear` + `PendingAbandon`) equivalence classes have those classifications by driving production code. Also corrected an overstated claim in the `Flushing` reason string and in `context/cli/opencode-mutation-scope-health.md`: the previous wording implied a single "next tracked event" both reclaims an orphaned `Flushing` to `Pending` *and* retries `resolve_recovery`. In production these are two distinct steps performed by two distinct mechanisms — `normalize_recovery_after_boundary_lock_acquired` (run by *any* tracked adapter boundary) performs only the `Flushing` → `Pending` reclaim, while `resolve_recovery` (run only as part of a recovery-capable tracked *admission*, i.e. `admit_tracked_attempt`'s `Pending{g}` → `Flushing{g}` claim followed by `resolve_recovery`) is what actually claims the generation and retries the outstanding `PendingAbandon` abandonment. The classifier's `Flushing` reason text and the context doc's table/prose were reworded to state both steps explicitly; no classifier logic, recovery/barrier production behavior, or classification result changed. No T05/Pi work, doctor wiring, `doctor --fix`, protocol behavior, stale-`PendingStart` cleanup, or attribution-semantics change was made.
    - **Correction (this task, third pass — AC4/Done-When wording alignment and matrix fixture cleanup):** the plan's AC4 and T04 Done When text still described the verification structure as one table-driven test proving "classifier result plus actual admission/recovery consequence per row," which no longer matched what the suite actually does (a classifier-completeness matrix over hand-built state, separate from representative real-dispatch regressions per semantic equivalence class). AC4 was rewritten to require, per adapter: (1) a table-driven classification matrix covering every meaningful reachable `RecoveryState`/attempt-phase combination; (2) real-dispatch behavioral tests proving the future admission/recovery semantics for every distinct semantic equivalence class used by that matrix, allowing multiple rows to share one behavioral proof under a demonstrated invariant; (3) the classifier matching those proven semantics exactly; (4) no classification inferred merely from enum/variant names — and now states explicitly that the matrix is the exhaustiveness layer and the real-dispatch regressions are the behavioral-justification layer, neither substituting for the other. T04's Done When was reworded to match: it no longer says "actual admission/recovery consequence per row" and instead requires the matrix be table-driven and complete, every distinct semantic equivalence class be proven through real adapter dispatch/recovery behavior, and absent/malformed state handling satisfy AC5. No wording elsewhere in this record overstated per-row behavioral execution, so no further correction to the Result/prior-correction prose was needed. Separately, `matrix_attempt` in `health.rs`'s test module was hand-building `scope_id` with a literal `format!("oc-tool-v1|s=8:ses-main|c=6:{call_id}")`, which produced invalid length prefixes for call IDs such as `call-active`/`call-abandon`/`call-start` (the classifier never inspects `scope_id`, so this did not affect any test result, but the fixture was not production-shaped). Changed `matrix_attempt` to build an `AttemptKey` and derive `scope_id` via the real production formatter, `super::super::format_opencode_scope_id(&key)`, instead of duplicating the encoding — no second test-only formatter was introduced. No classifier logic, recovery/barrier production behavior, or classification result changed by this pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml opencode_mutation_scope` — pass (103 tests, including the `health::tests` module: absent-file, clear-with-active-attempts healthy, `PendingStart`-blocked with two repeated unrelated denials proven via real dispatch, `Pending`-with-`PendingAbandon`-only-attempts recovering with repeated-denial-then-clearance proven via real dispatch, `Pending`-with-empty-attempts recovering, orphaned-`Flushing`-with-`PendingAbandon`-only recovering and reclaimed, the corrected `Pending`+mixed-`PendingStart` critical regression proven blocked before and after real recovery resolution, the corrected orphaned-`Flushing`+`PendingStart` regression proven blocked before and after real recovery resolution, hand-seeded `Clear`+`PendingAbandon` invalid, malformed-file invalid, and the `health_classification_matrix_covers_all_twelve_recovery_and_attempt_phase_combinations` table-driven completeness test, now built on production-shaped fixtures via `format_opencode_scope_id`). `cargo fmt --manifest-path cli/Cargo.toml --check` and `cargo clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` both clean.
  - Context impact: domain — `context/cli/opencode-mutation-scope-health.md` is the canonical detailed owner of the proven Healthy/Recovering/Blocked/Invalid mapping, corrected to state that `Pending`/`Flushing` are `Recovering` only when no `PendingStart` attempt is also outstanding (a stale `PendingStart` is `Blocked` regardless of `RecoveryState`, not just under `Clear`), including the full twelve-cell state matrix and reachability answers; the matrix section now also points at the new table-driven completeness test, and every `Flushing`-reclaim mention now distinguishes the next-boundary reclaim from the tracked-admission-driven `resolve_recovery` retry instead of conflating them into one "next tracked event"; `context/cli/opencode-mutation-scope-integration.md` links to it from its "Adapter lifecycle and recovery" section rather than duplicating the mapping; `context/context-map.md` registers the health document (unchanged by this correction).

- [x] T05: `Investigate and classify Pi mutation-scope health` (status:done)
  - Task ID: T05
  - Scope: In — the same investigation and classifier work as T03, applied to `cli/src/services/hooks/pi_mutation_scope/state.rs`, explicitly accounting for `AttemptPhase::PendingStart`/`Executed`/`PendingAbandon`, `RecoveryState::Clear`/`Pending { generation }`/`Flushing { generation }`, `ProcessOwner`, and `is_definitely_dead()`. A live persisted attempt is not automatically `Blocked`. A stale `PendingStart`/`Executed` attempt whose owner is positively dead (`is_definitely_dead`) may still be `Recovering` if the existing next-boundary stale-owner recovery path (the D10 sweep) can retire it automatically — prove this by driving that sweep, not by asserting it from the field name. A `PendingAbandon` is classified `Blocked` only if investigation proves that, once the event that created it has returned, no ordinary future boundary can resume and complete cleanup; otherwise it is `Recovering`. Out — Codex, OpenCode, doctor wiring.
  - Dependencies: T01
  - Done when: a complete table-driven Pi classification matrix covers every meaningful persisted `RecoveryState`/`AttemptPhase`/owner-liveness combination; real-dispatch behavioral tests prove every distinct semantic equivalence class represented by that matrix, including the required live-owner and definitely-dead-owner cases; multiple matrix rows may share one behavioral proof where a demonstrated invariant makes their future behavior equivalent; the classifier matches those proven semantics exactly; and absent/malformed state handling satisfies AC5.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`
  - Context synchronization: synced
  - Completed: 2026-09-19
  - Files changed: `cli/src/services/hooks/pi_mutation_scope/health.rs` (new), `cli/src/services/hooks/pi_mutation_scope/mod.rs`, `cli/src/services/hooks/pi_mutation_scope/state.rs`
  - Result: Investigated every reachable `(RecoveryState, AttemptPhase, owner-liveness)` combination by reading and driving `pi_mutation_scope`'s real dispatch/recovery code (`admit_or_recover`, `reconcile_stale_owners` — the unconditional D10 stale-owner sweep that runs on every tracked Start from any session, before that session's own admission is even considered — `resolve_recovery`, `normalize_recovery_after_boundary_lock_acquired`) and its existing D8/D10/D12 test suite (notably `a_pending_start_attempt_never_blocks_a_concurrent_new_admission`, `an_owner_that_cannot_be_positively_proven_dead_is_never_abandoned_by_an_unrelated_start`, `a_dead_pending_start_attempt_is_recovered_by_an_unrelated_fresh_session_start`, `an_interrupted_stale_owner_recovery_remains_pending_and_denies_the_triggering_start_until_resumed`, `a_terminal_recovery_flush_failure_leaves_a_pending_recovery_and_denies_new_admission`). Key finding, specific to Pi and different from Codex/OpenCode: Pi's `admit_tracked_attempt` never gates a new admission on another attempt's `PendingStart`/`Executed` phase (only on a co-existing `PendingAbandon`, via `UncertainAttemptBlocked`, which is itself proven structurally unreachable in production because `PendingAbandon` only ever coexists with `RecoveryState != Clear`), and the D10 sweep unconditionally retires any dead-owner `PendingStart`/`Executed` attempt on the very next tracked Start regardless of session — so, unlike OpenCode's stuck-`PendingStart` wedge, Pi has no persisted-state combination reachable through ordinary production dispatch that classifies `Blocked`; only hand-seeded, code-proven-impossible combinations classify `Invalid`. Added `classify_health(git_dir) -> MutationScopeAdapterHealth` in a new `pi_mutation_scope/health.rs`, registered as `pub(crate) mod health;` in `mod.rs`: a read/parse error → `Invalid` with the error surfaced; `RecoveryState::Clear` with a `PendingAbandon` attempt present → `Invalid` (structurally impossible: `resolve_recovery` always removes every `PendingAbandon` attempt before `complete_recovery_flush` can transition to `Clear`); `RecoveryState::Clear` with a dead-owner (`is_definitely_dead`) `PendingStart`/`Executed` attempt and no `PendingAbandon` → `Recovering` (the D10 sweep is unfinished recovery work with a proven automatic path, even though it does not currently block anything); `RecoveryState::Clear` otherwise (including live- or uncertain-owner `PendingStart`/`Executed` attempts, which are never swept and never block unrelated admission) → `Healthy`; `RecoveryState::Pending`/`Flushing` with any otherwise valid attempt composition → `Recovering`, because a subsequent recovery-capable tracked Start whose key is not already represented by a nonterminal attempt can claim and retry the recovery generation without manual intervention — not lane- or key-scoped, unlike Codex, so a fresh caller from any session can perform it; an orphaned `Flushing` is first reclaimed to `Pending` by the next tracked adapter boundary. A duplicate Start for an already-tracked `PendingStart`/`Executed` key may be idempotently reused before `RecoveryState` is ever inspected and therefore does not necessarily advance recovery on its own; live/uncertain `PendingStart`/`Executed` attempts do not block unrelated admission, so this remains `Recovering`, not `Blocked`. Made `state::state_path` `pub(crate)` (was module-private) and `process_owner` a `pub(crate) mod` (was module-private) so `health.rs`'s tests could target the real state file path and construct `ProcessOwner` fixtures, mirroring T02–T04. No recovery/barrier production behavior changed; only the new classifier and its supporting tests were added.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope` — pass (102 tests, including the new `health::tests` module: absent-file, clear-with-live-owner-attempt healthy, clear-with-uncertain-owner-attempt healthy with a real-dispatch proof that an unrelated Start never sweeps it, clear-with-dead-owner-`PendingStart` recovering with a real-dispatch D10-sweep proof reaching `Healthy`, clear-with-dead-owner-`Executed` recovering with a real-dispatch proof it is swept without a synthetic Close, pending-from-a-failed-terminal-abandon recovering with repeated-denial-then-self-heal proven via real dispatch, pending-from-an-interrupted-dead-owner-sweep recovering with the triggering call itself denied then resumed on retry, orphaned-Flushing recovering and reclaimed, hand-seeded `Clear`+`PendingAbandon` invalid, malformed-file invalid, a `health_classification_matrix_covers_all_twelve_recovery_and_attempt_condition_combinations` table-driven completeness test over `(RecoveryState, has_pending_abandon, has_dead_owner_attempt)`, and `pending_recovery_reuses_a_duplicate_start_for_an_existing_nonterminal_key_without_advancing_recovery_then_a_fresh_start_recovers` — proves via real dispatch that a `Pending` recovery with a live `PendingStart` key (B) and a `PendingAbandon` key (A) stays `Pending`/`Recovering` and its state untouched when a duplicate Start for B's own already-tracked key is reused idempotently, and only completes (`FlushClaimed` → `resolve_recovery` → `Clear` → `Healthy`) once a fresh Start for an untracked key (C) claims and retries the generation). `cargo fmt --manifest-path cli/Cargo.toml --check` and `SCE_CLI_PACKAGE_FALLBACK=1 cargo clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` both clean.
  - Context impact: domain — `context/cli/pi-mutation-scope-health.md` is now the canonical detailed owner of the Pi health mapping and its behavioral justification (the D10 sweep's unconditional any-session reach, the unreachability of `UncertainAttemptBlocked`/`Blocked` in production, and the `Pending`/`Flushing` recovery path); `context/cli/pi-mutation-scope-integration.md` links to it instead of duplicating that mapping; `context/context-map.md` registers the document. Corrected nuance: `Pending`/`Flushing` have a normal recovery-capable future tracked-Start path (a proven any-caller resolution once claimed), but not every duplicate Start is required to advance recovery — `admit_tracked_attempt` reuses a duplicate Start for an already-tracked nonterminal (`PendingStart`/`Executed`) key idempotently, before `RecoveryState` is ever inspected, so only a Start whose key is not already tracked can claim the pending generation.

- [ ] T06: `Report mutation-scope health in doctor JSON and human text output` (status:todo)
  - Task ID: T06
  - Scope: In — `cli/src/services/doctor/types.rs` (a new report field carrying one health record per detected/configured adapter, plus stable `DoctorProblem` metadata for non-healthy records: category — a new stable `ProblemKind`/category value scoped to mutation-scope health rather than overloading an unrelated existing kind — severity, fixability, summary/detail, remediation, and next_action); `cli/src/services/doctor/inspect.rs` (calling each of the four adapters' T02–T05 classifiers for the targets doctor already detects/configures, never writing to any state file, and mapping `recovering` to a `Warning`-severity `DoctorProblem` and `blocked`/`invalid` to an `Error`-severity `DoctorProblem` per AC6); `cli/src/services/doctor/render.rs` (JSON serialization and human-text rendering using `[PASS]`/`[WARN]`/`[FAIL]` per AC2, and the existing healthy-row-collapse convention); `cli/src/services/doctor/mod.rs` if aggregation/readiness wiring is needed there. Every `blocked`/`invalid` record's remediation is `manual_only` deterministic guidance (never a recommendation to delete the state file) unless a safe generic recovery command already exists for that specific case, and states plainly when no such command exists yet. Out — any change to the classifiers themselves (T01–T05 own that); any `doctor --fix` mutation of adapter recovery state.
  - Dependencies: T02, T03, T04, T05
  - Done when: AC1, AC2, and AC6 pass for all four adapters, including the collapsed-healthy-row case, an expanded row for each non-healthy status, and readiness/summary-count consistency for `recovering`-only and `blocked`/`invalid` repositories.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml doctor`
  - Context synchronization: pending

## Open questions

None. The scope, the shared status vocabulary, the precise `healthy |
recovering | blocked | invalid` semantics, the per-adapter ownership boundary,
the Codex/OpenCode/Pi behavioral-investigation requirement, and the doctor
problem/readiness wiring were all resolved during clarification.
