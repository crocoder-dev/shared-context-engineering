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
- [ ] AC4: For each of Codex, OpenCode, and Pi, table-driven behavioral tests
      cover every meaningful reachable `RecoveryState`/attempt-phase
      combination and prove — from actual future admission/recovery behavior,
      not only the classifier's output — why each is `healthy | recovering |
      blocked | invalid`. The adapter's classifier implements exactly that
      proven mapping before `doctor` is allowed to report `blocked` for that
      adapter. Classifications are never inferred from enum/variant names.
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

- [ ] T02: `Classify Claude mutation-scope health` (status:todo)
  - Task ID: T02
  - Scope: In — a read-only function in `cli/src/services/hooks/claude_mutation_scope/` that reads `claude-mutation-scope-state.json` via the existing `state::read_state` and maps it to the T01 status type using the proven rule from AC3 (`recovery_pending && attempts non-empty` → `Blocked`, because the recovery barrier's only self-healing path — `{"operation":"flush"}` — only runs when `attempts.is_empty()`, so a non-empty `attempts` list has no reachable future self-healing transition; `recovery_pending && attempts empty` → `Recovering`, because that flush path does run and does clear `recovery_pending`; neither → `Healthy`; a `read_state` error → `Invalid` with the error surfaced); absent-file handling (already `Ok(AdapterState::default())` in `read_state`) must classify as `Healthy`. Out — doctor wiring, other adapters, any change to the recovery barrier itself.
  - Dependencies: T01
  - Done when: unit tests cover all four resulting statuses, the absent-file case, and the AC3 regression scenario (stale non-empty-`attempts` state persists across two successive tracked `PreToolUse` calls without self-clearing).
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml claude_mutation_scope`
  - Context synchronization: pending

- [ ] T03: `Investigate and classify Codex mutation-scope health` (status:todo)
  - Task ID: T03
  - Scope: In — for every reachable `RecoveryState` value in `cli/src/services/hooks/codex_mutation_scope/state.rs` (`Clear`, `Pending { generation }`, `Flushing { generation }`) and its interaction with attempt state, answer for each: (1) is this combination actually reachable through production behavior; (2) is it valid persisted state; (3) once the hook call that created it has returned, what happens on the next ordinary tracked admission/lifecycle event; (4) does an existing code path advance recovery; (5) can it return to normal admission without manual state-file surgery; (6) is progress dependent on an event that can no longer occur; (7) does the next Start merely deny forever; (8) for owner-aware paths, can positive process-death evidence trigger existing recovery. Drive the adapter's real dispatch/barrier logic (the way its own existing test suite does) to answer these, not the enum names. A table-driven test module records, per row, the semantic answer to "what future event gets this state out?" alongside the resulting status; a classifier function implements exactly that proven mapping against the T01 status type. Out — OpenCode, Pi, doctor wiring.
  - Dependencies: T01
  - Done when: the table-driven behavioral tests in AC4 pass for Codex — each asserting both the classifier result and the actual admission/recovery consequence that justifies it (e.g. seed/create the state, assert `classify_health(...)`, then invoke the next normal boundary and assert it either advances recovery or remains denied) — the classifier matches them exactly, and absent/malformed state files classify per AC5.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml codex_mutation_scope`
  - Context synchronization: pending

- [ ] T04: `Investigate and classify OpenCode mutation-scope health` (status:todo)
  - Task ID: T04
  - Scope: In — the same investigation and classifier work as T03 (the same eight questions, the same "what future event gets this state out?" requirement, the same `Clear`/`Pending { generation }`/`Flushing { generation }` `RecoveryState` coverage), applied to `cli/src/services/hooks/opencode_mutation_scope/state.rs`, including its additional `PendingAbandon` attempt phase — investigate whether an unresolved `PendingAbandon` has any existing automatic successor path, or whether it can only be classified `Blocked` once its recovery boundary is proven to have no future normal advance. Out — Codex, Pi, doctor wiring.
  - Dependencies: T01
  - Done when: the table-driven behavioral tests in AC4 pass for OpenCode (classifier result plus actual admission/recovery consequence per row, as in T03), the classifier matches them exactly, and absent/malformed state files classify per AC5.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml opencode_mutation_scope`
  - Context synchronization: pending

- [ ] T05: `Investigate and classify Pi mutation-scope health` (status:todo)
  - Task ID: T05
  - Scope: In — the same investigation and classifier work as T03, applied to `cli/src/services/hooks/pi_mutation_scope/state.rs`, explicitly accounting for `AttemptPhase::PendingStart`/`Executed`/`PendingAbandon`, `RecoveryState::Clear`/`Pending { generation }`/`Flushing { generation }`, `ProcessOwner`, and `is_definitely_dead()`. A live persisted attempt is not automatically `Blocked`. A stale `PendingStart`/`Executed` attempt whose owner is positively dead (`is_definitely_dead`) may still be `Recovering` if the existing next-boundary stale-owner recovery path (the D10 sweep) can retire it automatically — prove this by driving that sweep, not by asserting it from the field name. A `PendingAbandon` is classified `Blocked` only if investigation proves that, once the event that created it has returned, no ordinary future boundary can resume and complete cleanup; otherwise it is `Recovering`. Out — Codex, OpenCode, doctor wiring.
  - Dependencies: T01
  - Done when: the table-driven behavioral tests in AC4 pass for Pi (classifier result plus actual admission/recovery consequence per row, as in T03, including at least one dead-owner and one live-owner case), the classifier matches them exactly, and absent/malformed state files classify per AC5.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`
  - Context synchronization: pending

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
