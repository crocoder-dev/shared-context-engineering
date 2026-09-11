# Plan: pi-mutation-scope-integration

## Change summary

Add Pi as the fourth concrete mutation-scope producer, building on the validated Claude Code and Codex mutation-attribution adapters plus the mutation-scope-provenance work already on `main`. This plan is stacked on the still-open OpenCode integration (PR #276) rather than assuming it — see **Stack and base**.

The integration reuses the existing project-local Pi extension rather than introducing a second competing extension. The TypeScript extension remains a thin harness transport layer; a new Rust `pi_mutation_scope` adapter owns event parsing, tool classification, scope identity, durable attempt state, recovery, provenance normalization, and translation into the existing harness-neutral mutation-scope ingress.

One independently executing mutation-capable Pi tool call is one mutation scope. A Pi session, turn, agent loop, or process is not itself a scope.

The initial tracked built-in tool set is `bash`, `edit`, `write`. Known read-only built-ins (`read`, `grep`, `find`, `ls`) are untracked. Custom and unknown tools remain untracked in v1 unless exact Pi evidence establishes a sound capability contract — missed attribution is preferred over claiming AI attribution for an unknown tool.

Pi `!` / `!!` user Bash is human-initiated and never creates a Pi AI mutation scope.

The integration uses the existing generic mutation-scope runtime and existing `mutation_trace_*` storage. It introduces no new Agent Trace schema or mutation-trace SQL migration.

The plan is based on Pi `0.80.6`. T01 freezes the lifecycle evidence for that exact version. Changing the Pi version after T01 begins requires stopping the plan, updating the version policy, and rerunning the complete load-bearing probe matrix.

## Stack and base

- **Predecessor:** PR #276 `opencode-mutation-scope-integration` (head branch
  `opencode-mutation-scope-integration`, head commit `281a5290d35750f6952ea5646bd3aaefb08bb36c`;
  based on `mutation-scope-provenance`). **PR #276 is currently open and unmerged.**
- **This branch:** `pi-mutation-scope-integration` (PR #278) is already stacked
  directly on `opencode-mutation-scope-integration`: PR #276's head
  (`281a5290d`) is the merge base and the direct parent commit of this branch,
  which is exactly one plan commit ahead of it. Confirmed at planning time:
  the OpenCode mutation-scope adapter and the generalized per-`ActorKind`
  confirmation-required predicate (covering `ActorKind::Codex` and
  `ActorKind::OpenCode`) are already present in this branch's history via that
  stacked base.
- **Stack invariant, not a plan task:** PR #276's current head must remain an
  ancestor of this branch for as long as #278 is stacked on it. Before
  beginning a task, if PR #276's head has moved (amended or rebased), update
  this branch onto the new predecessor before continuing. If PR #276 merges,
  rebase/retarget #278 onto the branch that now contains the landed OpenCode
  work — normally `main`. This is a Git operation performed outside the task
  stack, not a numbered task; every task below assumes the invariant currently
  holds.
- **Base for the PR while the stack is unmerged:** `opencode-mutation-scope-integration`
  (#276 head), **not** `main`.
- **Final branch comparison** is against `opencode-mutation-scope-integration`,
  not `main`, for as long as #276 remains open. If #276 changes before
  execution time (rebased, amended, or merged to `main`), re-check
  `gh pr view 276` and the actual branch ancestry, rebase onto the current
  predecessor, and update this section plus every task below that assumes a
  specific pre-existing OpenCode/Codex confirmation-required shape.

## Dependency and version policy

Independently of the branch stacking above, this plan also pins:

* `@earendil-works/pi-coding-agent` `0.80.6` as pinned by `config/lib/package.json`
  (confirmed at planning time);
* upstream Pi tag `v0.80.6`, commit `2b3fda9921b5590f285165287bd442a25817f17b`.

No Pi package upgrade belongs in this PR.

If the pinned Pi runtime behaves differently from the lifecycle assumptions below, T01 is a re-planning gate. Do not weaken attribution semantics to make the implementation fit an unexpected lifecycle.

## Design

### D1 — One tool execution is one scope

A mutation scope represents one independently executing mutation-capable tool call.

```text
Pi session
  |
  +-- toolCall A: bash   -> scope A
  +-- toolCall B: write  -> scope B
  +-- toolCall C: read   -> no scope
```

Parallel tool executions must remain distinct live scopes.

A session, turn, agent loop, or Pi process must never be collapsed into a single mutation scope.

The adapter maintains a monotonic checkout-local attempt sequence so a reused Pi `toolCallId` can never reactivate a terminal SCE scope.

Canonical identity:

```text
pi-tool-v1|n=<attempt-seq>|s=<len>:<session-id>|c=<len>:<tool-call-id>
```

The exact live-attempt key is:

```text
(session-id, tool-call-id)
```

A replay while that attempt is live resolves to the existing attempt. A new execution after terminal cleanup receives a new attempt sequence and therefore a new `ScopeId`.

Boundary event IDs derive deterministically from the scope:

```text
<scope-id>|start
<scope-id>|close
```

No timestamp, random UUID, model identifier, PID, or tool argument participates in attribution identity.

### D2 — Conservative tool classification

Pi `0.80.6` has three SCE-supported built-in mutation-capable tools: `bash`, `edit`, `write`. These establish scopes.

Known read-only tools (`read`, `grep`, `find`, `ls`) create no mutation-scope state.

Custom and unknown tool names are untracked in v1. Their schemas or descriptions are not sufficient evidence of mutation capability.

This is deliberately asymmetric:

```text
unknown mutating tool -> possible false negative
unknown read-only tool -> never fabricates an AI scope
```

False negatives are preferred to false-positive AI attribution.

If Pi allows a built-in mutation-capable name to be replaced with behavior that invalidates this classification, T01 must record that and the plan must be revised before T02.

### D3 — Start is write-ahead and fail-closed

The candidate Pi Start boundary is `tool_call`.

The pinned API defines it as occurring before execution and permits the handler to block the tool.

Within the existing SCE Pi extension, handler ordering must be:

```text
bash policy
    ↓
mutation-scope Start
    ↓
existing edit/write diff pre-image capture
    ↓
tool execution
```

For Bash, an SCE bash-policy denial therefore occurs before mutation-scope admission and creates no scope.

For a tracked tool, the mutation-scope handler synchronously invokes:

```text
sce hooks pi-mutation-scope
```

and does not allow the tool to proceed unless the Rust adapter has durably established `Start`.

Transport failure, missing `sce`, timeout, malformed identity, durable-state failure, recovery failure, provenance identity conflict, or generic Start failure all return Pi's normal `{ block: true, reason: ... }` shape.

A tracked mutation must never proceed merely because mutation attribution could not be established.

### D4 — Pi becomes confirmation-required

Pi changes from `requires_boundary_confirmation(Pi) = false` to `requires_boundary_confirmation(Pi) = true`.

The reason is extension ordering.

A successful SCE `tool_call` handler does not itself prove the tool will execute. A later Pi extension can still return `block: true`.

Therefore `Start(Pi A)` followed by a later extension rejecting A must never make A eligible for positive attribution.

Until Pi A reaches its own confirmed post-execution Close, any boundary while A remains unconfirmed resolves to `IneligibleUnscoped`.

A successful Pi Close confirms its own scope exactly like the generalized Codex/OpenCode confirmation rule, already in place via the stacked base (see **Stack and base**).

This change must remain bounded to the existing confirmation-required predicate and corresponding Quint/MBT cases. It must not add Pi-specific fields to `ProtocolState`, `ScopeState`, `MutationEvent`, `Attribution`, or the Quint scope model.

### D5 — Execution start is lifecycle evidence, not a mutation boundary

Pi exposes `tool_execution_start`.

The adapter uses this to distinguish "Start admitted, tool never executed" from "Start admitted, tool execution actually began," but it does not send another generic mutation boundary.

Adapter state moves conceptually:

```text
PendingStart
    ↓ generic Start succeeds
AwaitingExecution
    ↓ tool_execution_start
Active
```

This distinction is needed for later-plugin rejection, interruption, shutdown, and recovery.

T01 must prove exact event ordering against Pi `0.80.6`.

### D6 — `tool_execution_end` is the candidate confirming Close

Pi exposes `tool_execution_end` with `toolCallId`, `toolName`, `result`, `isError`, and describes it as firing when tool execution finishes.

Subject to T01 confirmation, both successful and failed tracked executions map to the same observed Close boundary:

```text
tool_execution_end(success) -> Close
tool_execution_end(error)   -> Close
```

A failed tool may already have changed the filesystem, so `isError: true` must not cause SCE to discard its final observation.

The existing `tool_result` integration remains responsible for edit/write diff tracing. It is not used as the mutation boundary unless T01 demonstrates that `tool_execution_end` lacks a required lifecycle guarantee.

Do not produce both boundaries for one execution.

### D7 — A Start followed by no execution must be abandoned, never closed

If SCE established Start but Pi subsequently proves the execution never began — for example because a later extension blocked the tool — there is no legitimate successful Close.

The adapter must conservatively resolve:

```text
Start
no execution
exact terminal evidence
    ↓
consume ambiguous interval as non-AI
abandon scope
rebaseline
```

It must not fabricate a Close.

The exact signal proving that an `AwaitingExecution` attempt can no longer execute is owned by T01. Candidate evidence includes the actual blocked-tool result sequence and `agent_settled`, but no broad lifecycle event may be treated as terminal until the pinned runtime proves it.

### D8 — Recovery follows the soundness-first flush/abandon/flush pattern

Pi is confirmation-required and may overlap another live scope, so terminal recovery must preserve the same safety invariant already established for OpenCode.

When an exact terminal condition requires abandoning A:

```text
1. durably record A as PendingAbandon
2. Flush while A and siblings are still live
3. abandon A
4. remove A only after durable abandon succeeds
5. Flush again to consume needs_rebaseline
6. clear recovery only after every step succeeds
```

The first Flush makes the ambiguous interval ineligible while A is still an unconfirmed live scope.

Surviving scopes are not swept:

```text
Start(A)
Start(B)
mutations
A becomes unrecoverably terminal
    ↓
ambiguous interval -> non-AI
abandon(A)
rebaseline
B remains live
B makes later mutation
Close(B)
    ↓
later B interval can still become AI
```

Failed recovery remains fail-closed for subsequent tracked Pi Starts.

Never replay an old Close later merely because its original delivery failed. The current Git tree would no longer represent the original observation time.

### D9 — Transport failure after tool execution cannot be repaired by pretending the observation is current

Start transport is fail-closed because the tool has not executed yet.

Terminal transport is different: once Pi reports `tool_execution_end`, the mutation may already exist.

If the terminal event reaches the Rust adapter but the generic Close fails before durable completion, Rust owns the recovery sequence from D8.

If the TypeScript extension cannot reach the Rust adapter at all after execution:

* retain an in-process unresolved-terminal marker for that exact attempt;
* deny subsequent tracked Starts while the terminal state remains unresolved;
* once the adapter becomes reachable, recover the old scope through abandonment/rebaseline, not a delayed Close;
* if the Pi process dies before that can happen, process-staleness recovery owns it.

Existing conversation-trace and diff-trace delivery remains fail-open. Mutation-scope delivery does not inherit their advisory semantics.

### D10 — Process death is positive staleness evidence; elapsed time is not

Each durable Pi attempt records enough process ownership information to determine whether the Pi process that owned it is definitely gone.

On a later adapter invocation, an attempt owned by a positively dead process may be recovered through abandonment/rebaseline.

A live PID alone is not enough to prove ownership after PID reuse; the implementation should use the strongest process-instance evidence available without weakening portability. If exact cross-platform process-instance identity cannot be established, a possibly-reused live PID is treated as alive and the stale scope remains conservative.

No timeout or TTL proves death. Do not abandon scopes because they are old. Do not abandon every Pi scope at startup. Do not use `ActorKind::Pi` as evidence of staleness.

### D11 — Pi session/model provenance is admission-time metadata

Every tracked Start carries:

```text
session_id = pi_<Pi session ID>
model_id   = <ctx.model.provider>/<ctx.model.id> | NULL
```

Canonical session prefixing is idempotent.

Model provenance comes from the exact `ctx.model` observed for that Start.

Missing or unusable model evidence yields `NULL`.

Never: infer the model from another session; reuse stale model state; backfill a `NULL` provenance row later; update provenance after model switching; make model absence itself block a tracked tool.

Provenance remains insert-once metadata outside protocol state.

### D12 — Multi-process Pi is normal concurrency

Pi has no need for a special "subagent scope."

If another Pi process/session works on the same checkout, its tracked tools naturally receive their own scopes:

```text
Pi process/session A -> pi-tool scope A
Pi process/session B -> pi-tool scope B
```

Their overlap becomes normal mutation-scope contention.

If an extension/custom tool launches another Pi process, the launching custom tool remains untracked unless explicitly classified; the child Pi's own tracked tools are attributed independently if that child loads the SCE extension.

No parent scope absorbs child mutations.

### D13 — `!` / `!!` user Bash is not AI attribution

Pi's `user_bash` lifecycle is user-initiated. It must never establish an `ActorKind::Pi` scope.

Ordinary user Bash mutations occurring while no agent scope is active are naturally observed as unscoped by the next mutation boundary.

T01 must explicitly determine whether `user_bash` can execute concurrently with an active agent tool. If it can, the plan must stop and add a sound explicit unscoped/taint boundary before T02. Do not silently allow user Bash mutations to become eligible inside a Pi scope.

This PR does not need to solve the separately deferred Bash-policy behavior for `!` / `!!` unless doing so is required to maintain mutation-attribution soundness.

### D14 — Detached descendants remain an explicit limitation

A foreground Pi Bash tool can potentially launch a child process that survives the Bash tool's own completion.

If the pinned Pi runtime provides no structured lifecycle proving all descendants are dead, `tool_execution_end` cannot prove that a self-detached descendant has stopped mutating.

Do not attempt to infer this from Bash command text. Do not build a shell parser or static background-process detector in this PR.

Record the exact observed behavior in T01 and document the residual attribution boundary.

## Acceptance criteria

- [ ] AC1: Exact lifecycle evidence exists for Pi `0.80.6`, covering `tool_call`, `tool_execution_start`, `tool_execution_end`, `tool_result`, blocking, handler failure, tool failure, interruption, session lifecycle, process death, model observation, extension ordering, and concurrency.
  - Validate: committed T01 fixtures/report with exact Pi version, upstream commit, environment, and event sequences.
- [ ] AC2: `bash`, `edit`, and `write` each establish one independently identified Pi mutation scope before their mutation-capable execution begins.
  - Validate: adapter tests plus live/runtime fixtures.
- [ ] AC3: `read`, `grep`, `find`, `ls`, `user_bash`, and representative unknown/custom tools create no Pi mutation scope.
  - Validate: zero-footprint classification and runtime tests.
- [ ] AC4: failure to establish a tracked Pi Start blocks the tool before execution.
  - Validate: live probe where the adapter fails and an observable filesystem mutation never occurs.
- [ ] AC5: a Pi scope cannot create positive mutation attribution until its own confirming post-execution Close.
  - Validate: Rust protocol tests plus Quint Pi confirmation-required cases.
- [ ] AC6: an unconfirmed Pi scope suppresses positive attribution at Claude, Codex, OpenCode, Pi, and Flush boundaries.
  - Validate: protocol/MBT/Quint cross-harness tests.
- [ ] AC7: a confirming Pi Close can produce `AiExclusive(Pi)` when it is the only safe live scope and `AiContended` when overlapping confirmation-safe scopes remain.
  - Validate: Rust/Quint reachability tests.
- [ ] AC8: an earlier extension or SCE bash policy rejecting a tool before SCE Start creates no scope; a later extension rejecting after SCE Start cannot create positive attribution and is eventually conservatively recovered.
  - Validate: pinned-runtime ordering fixtures plus adapter/runtime regression.
- [ ] AC9: a tracked tool that executes and then reports `isError` still observes its final Git tree through the same terminal boundary as success.
  - Validate: partial-mutation-then-error regression.
- [ ] AC10: simultaneous or overlapping Pi calls remain separate scopes and terminal cleanup of one never implicitly retires another.
  - Validate: concurrency adapter/runtime test.
- [ ] AC11: a lost or failed terminal boundary cannot later be replayed as if its observation happened at recovery time.
  - Validate: injected terminal seam failure followed by another filesystem mutation; recovery must discard/rebaseline the ambiguous interval instead of attributing it.
- [ ] AC12: stale-process cleanup requires positive process-death evidence and never uses TTL, age, session identity, or ActorKind alone.
  - Validate: live-owner vs dead-owner durable-state tests.
- [ ] AC13: Pi Start provenance stores canonical `pi_<sessionID>` plus the exact observed normalized model, or `NULL` when unavailable.
  - Validate: real repository Agent Trace DB and final Agent Trace regressions.
- [ ] AC14: existing Pi Bash policy, conversation tracing, edit/write diff tracing, generated extension installation, and doctor behavior remain intact.
  - Validate: existing Pi/config-lib tests, setup smoke, doctor smoke, and generated-output validation.
- [ ] AC15: only confirmed exclusive Pi evidence reaches `mutation_ai_patch`; blocked, ambiguous, unconfirmed, abandoned, custom/unknown, and recovery intervals do not.
  - Validate: real Git/DB production-path tests.
- [ ] AC16: cross-harness Pi overlap obeys the generalized mutation protocol, at minimum Pi+Claude, Pi+Codex, Pi+OpenCode.
  - Validate: production-path tests against the OpenCode adapter already present in the stacked base (see **Stack and base**), plus Rust/Quint cross-harness tests.
- [ ] AC17: no new Agent Trace schema or mutation-trace SQL migration is introduced.
  - Validate: baseline diff over schema/migration paths is empty.
- [ ] AC18: the protocol/Quint semantic change is limited to adding the Pi case to the already-generalized confirmation-required predicate.
  - Validate: targeted baseline diff over `protocol.rs`, `spec/mutation_cursor.qnt`, its documentation, and MBT/refinement surface, showing only Pi-shaped additions.

### Full validation

Run from the repository's prescribed Nix environment.

```text
nix run .#quint -- typecheck spec/mutation_cursor.qnt
nix run .#quint -- test spec/mutation_cursor.qnt
nix build .#checks.x86_64-linux.mutation-trace-quint-connect

nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope
nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace
nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml hooks::

nix run nixpkgs#bun -- test config/lib

nix run .#pkl-check-generated
nix flake check
git diff --check
```

Also verify that the baseline diff introduces no mutation-trace schema migration:

```text
git diff <base> -- \
  config/schema/agent-trace.schema.json \
  cli/migrations/agent-trace-repository/
```

Expected: empty. `<base>` is `opencode-mutation-scope-integration` (#276 head) for as long as that PR remains open — see **Stack and base**.

### Context sync

Expected durable-context impact:

```text
context/cli/mutation-scope-hook-ingress.md
context/cli/mutation-scope-runtime.md
context/cli/mutation-scope-provenance.md
context/cli/mutation-trace-protocol.md
context/cli/pi-mutation-scope-integration.md
context/sce/agent-trace-hooks-command-routing.md
context/architecture.md
context/context-map.md
context/glossary.md
context/overview.md
spec/mutation_cursor.md
```

Pi generation/setup ownership documentation should be updated only where mutation-scope behavior materially changes the existing extension contract.

Each completed task must finish context synchronization as `synced` before the next task begins.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** pinned Pi lifecycle evidence (`cli/src/services/hooks/pi_mutation_scope/fixtures/`);
  the Pi mutation-scope adapter (`cli/src/services/hooks/pi_mutation_scope/`) and
  its hidden `sce hooks pi-mutation-scope` route; adding the Pi case to the
  existing confirmation-required protocol predicate and its Quint/MBT
  scenarios; Pi scope identity and durable adapter state; conservative
  recovery/stale-process handling; Pi provenance; existing Pi extension wiring
  (`config/lib/pi-plugin/sce-pi-extension.ts`); generated extension parity;
  setup/doctor preservation; production Git/DB/Agent Trace regressions;
  cross-harness attribution tests against the Claude, Codex, and OpenCode
  adapters already present in the stacked base.
- **Out of scope:** redesigning the generic mutation runtime; a new mutation
  protocol; Agent Trace schema changes; mutation-trace SQL migrations;
  arbitrary custom-tool capability inference; comprehensive attribution for
  third-party custom Pi tools; implementing a native Pi subagent framework;
  changing conversation-trace or diff-trace semantics; redesigning Pi Bash
  policy; policy support for `!` / `!!` unless required for attribution
  soundness; parsing Bash to detect detached descendants; upgrading Pi;
  refactoring Claude/Codex/OpenCode adapters merely to deduplicate Pi code;
  **redoing the OpenCode confirmation-required generalization** — that is
  PR #276's work, which this plan's stacked base already supplies rather than
  reimplements.
- **Constraints:** PR #276's commits must remain an ancestor of this branch
  for as long as #278 is stacked on it (see the stack invariant in **Stack
  and base**); no Pi package upgrade; attribution safety outranks preserving attribution coverage; do not
  resolve an uncertain lifecycle by broadening positive AI attribution; reuse
  `ActorKind::Pi` / `"actor_kind":"pi"`, already accepted by the generic
  ingress; production adapter code reaches mutation semantics only through the
  existing `hooks::mutation_scope` ingress seam, never a second `coordinate()`
  path; the adapter-state lock is never held across a `hooks::mutation_scope`
  invocation.
- **Non-goal:** treating `AiExclusive(Pi)` as proof no human edited the
  worktree; inferring staleness from `ActorKind::Pi`, TTL, or age; replaying an
  old Close at recovery time; a long-lived Pi "session" or "agent" scope; a
  Bash-text detached-process detector.

## Assumptions

- Task numbering follows T01..T06 as given in the original change request, one
  task per design-and-acceptance slice already scoped above.
- File and command naming (`cli/src/services/hooks/pi_mutation_scope/`,
  `sce hooks pi-mutation-scope`) follows the existing `claude_mutation_scope` /
  `codex_mutation_scope` and `claude-mutation-scope` / `codex-mutation-scope`
  precedent exactly, per repository convention.

## Task stack

- [ ] T01: `Freeze Pi mutation lifecycle evidence` (status:todo)
  - Task ID: T01
  - Scope: In — probing the exact SCE-pinned Pi `0.80.6` runtime and committing
    reproducible evidence under `cli/src/services/hooks/pi_mutation_scope/fixtures/`,
    recording Pi package version, upstream tag/commit, OS/platform, runtime
    mode, test configuration, and extension ordering. Out — writing any
    adapter, protocol, or extension code.
  - Dependencies: none
  - Done when: every load-bearing assumption in D1–D14 has a recorded
    disposition — `PROVEN`, `PROVEN-BY-PINNED-SOURCE`, `DOCUMENTED — NON-LOAD-BEARING`,
    or `UNSUPPORTED` — covering at minimum: bash success/non-zero
    failure/timeout/abort/partial-mutation-then-failure; write success/failure;
    edit success/failure; `tool_call` ordering including handler block/throw
    and SCE Start failure; earlier-extension-blocks-before-SCE and
    later-extension-blocks-after-SCE-Start; `tool_execution_start` ordering;
    `tool_execution_end` success/`isError`; `tool_result` success/`isError`;
    whether blocked calls receive execution/result events; multiple/overlapping
    tool calls and two Pi processes on one checkout; session
    startup/resume/fork/reload/switch/shutdown, `agent_end`, `agent_settled`,
    hard process termination; model available/switch/missing at `tool_call`;
    `user_bash` (`!`/`!!`) and whether it can overlap an active agent tool;
    custom read-only/mutating tools and built-in-name replacement; a foreground
    Bash tool spawning a detached descendant. Probe A (Start transport failure
    proves fail-closed block with no execution and no filesystem side effect),
    Probe B (later-extension rejection after a successful SCE Start proves no
    positive attribution is possible while the scope is unconfirmed), and
    Probe C (exact `tool_call`/`tool_execution_start`/`tool_execution_end`/`tool_result`
    ordering for both success and mutate-then-fail) are explicitly load-bearing
    and must each have recorded evidence. The plan may not proceed to T02 if
    `tool_call` cannot reliably block before mutation execution, no sound
    confirming post-execution boundary exists, later-extension rejection
    invalidates the confirmation-required design, Pi user Bash can overlap AI
    execution in a way the current protocol cannot soundly distinguish, or
    process/recovery semantics cannot conservatively preserve false-positive
    safety — any such finding requires revising this plan rather than
    weakening attribution.
  - Verify: replay/inspect committed captures and compare every load-bearing
    claim with upstream Pi `v0.80.6` source.
  - Context synchronization: pending

- [ ] T02: `Make Pi a confirmation-required protocol actor` (status:todo)
  - Task ID: T02
  - Scope: In — adding the Pi case to the existing generalized confirmation
    predicate (`ClaudeCode -> false`, `Codex -> true`, `OpenCode -> true`,
    `Pi -> true`; the Codex and OpenCode entries already exist in the stacked
    base — see **Stack and base** — so this task's actual diff is adding Pi) in
    `cli/src/services/mutation_trace/protocol.rs`,
    `cli/src/services/mutation_trace/tests.rs`,
    `cli/src/services/mutation_trace/mbt/`, `spec/mutation_cursor.qnt`, and
    `spec/mutation_cursor.md`; adding explicit Pi scenarios (`Start(Pi A)` +
    mutation + another actor's boundary => `IneligibleUnscoped`; `Start(Pi A)` +
    mutation + `Close(Pi A)` => `AiExclusive(A)`; `Start(Pi A)` + `Start(Claude B)`
    + mutation + `Close(Pi A)` => `AiContended`; `Start(Pi A)` + `Start(Codex B)`
    + mutation + `Close(Pi A)` => `IneligibleUnscoped` until Codex B confirms).
    Out — touching Claude/Codex/OpenCode's existing confirmation behavior;
    adding session/model/process fields to protocol state.
  - Dependencies: T01
  - Done when: Rust and Quint agree that Pi needs its own confirming Close and
    all existing Claude/Codex/OpenCode behavior remains unchanged.
  - Verify: `nix run .#quint -- typecheck spec/mutation_cursor.qnt`;
    `nix run .#quint -- test spec/mutation_cursor.qnt`;
    `nix build .#checks.x86_64-linux.mutation-trace-quint-connect`.
  - Context synchronization: pending

- [ ] T03: `Add the Pi mutation-scope adapter` (status:todo)
  - Task ID: T03
  - Scope: In — `cli/src/services/hooks/pi_mutation_scope/` and the hidden
    `sce hooks pi-mutation-scope` route, owning strict Pi wire parsing,
    tracked/read-only/untracked classification, `ScopeId`/`EventId`
    derivation, canonical `pi_` session identity, admission-time model
    provenance, checkout-local attempt state (`PendingStart` ->
    `AwaitingExecution` -> `Active` -> `PendingAbandon`), `tool_call` Start,
    `tool_execution_start` phase transition, and `tool_execution_end` Close.
    Durable state under `<git-dir>/sce/pi-mutation-scope-state.json` with a
    versioned schema, persisted with the same lock/write-temp/sync/atomic-rename/
    best-effort-parent-sync discipline as other adapters, never holding the
    adapter-state lock while invoking the generic mutation runtime. Out — any
    recovery/stale-process handling (T04); wiring into the actual TypeScript
    extension (T05).
  - Dependencies: T02
  - Done when: the Rust adapter correctly drives the frozen happy-path Pi
    lifecycle through the generic mutation-scope runtime, with durable
    provenance and no recovery shortcuts, reaching mutation semantics only
    through the existing `hooks::mutation_scope` ingress seam (no second
    direct `coordinate()` path); focused tests cover parser rejection,
    classification, identity stability/replay, terminal `ScopeId` non-reuse,
    session separation, model present/absent, `tool_execution_start` state
    transition, successful Close, failed-execution-still-Close, and untracked
    zero footprint.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`.
  - Context synchronization: pending

- [ ] T04: `Add sound terminal and stale-process recovery` (status:todo)
  - Task ID: T04
  - Scope: In — the conservative recovery obligations frozen by T01: Start
    committed then execution later blocked; Start committed then process dies
    before execution; execution begins then process dies before terminal
    event; Close/abandon seam failure; first-ambiguity-Flush and
    rebaseline-Flush failure; duplicate/late terminal events; adapter crash
    during recovery; multiple live Pi siblings; a Pi sibling plus another
    harness. Durable terminal intent (mark -> Flush ambiguous interval ->
    abandon -> remove only after abandon succeeds -> Flush rebaseline -> clear
    recovery), with a failed step leaving recovery pending and a new tracked
    Start remaining fail-closed until pending recovery resolves. Positive
    process-staleness handling based only on exact process-death evidence from
    the frozen Pi lifecycle — no TTL, no broad session sweep, no
    same-session predecessor sweep. Out — any change to the TypeScript
    extension (T05).
  - Dependencies: T03
  - Done when: no tested crash, rejection, missing terminal event, or
    transient seam failure can turn an uncertain Pi interval into positive AI
    attribution, while unrelated surviving scopes retain future attribution
    capability.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml pi_mutation_scope`;
    `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml mutation_trace`.
  - Context synchronization: pending

- [ ] T05: `Wire mutation scope into the existing Pi extension` (status:todo)
  - Task ID: T05
  - Scope: In — modifying the canonical Pi extension source
    `config/lib/pi-plugin/sce-pi-extension.ts` (not a second project-local SCE
    extension) to register mutation lifecycle handlers in the frozen T01 order
    (bash policy -> mutation Start -> edit/write diff pre-image) plus the
    frozen execution/terminal lifecycle handlers, with synchronous fail-closed
    Start transport, terminal transport that never pretends the tool did not
    run on post-execution transport failure (D9's unresolved-terminal guard),
    and preservation of existing Bash policy, conversation trace, edit/write
    diff trace, message trace, Pi session prefix behavior, and tool-version
    resolution. Using the existing generated Pi extension pipeline
    (`config/lib` / Pkl sources) — no hand-edited generated copies. Out — any
    Rust adapter change beyond what T03/T04 already produced.
  - Dependencies: T04
  - Done when: a real `sce setup --pi` installation routes Pi's mutation
    lifecycle through the Rust adapter while all existing Pi integration
    behavior remains intact; Bun tests (mocked subprocess transport) cover
    bash-policy-denial-means-no-Start, tracked-Start-success,
    tracked-Start-adapter-failure-blocks, missing-`sce`-blocks,
    read-only/unknown-tool-means-no-adapter-call, `tool_execution_start`
    forwarding/state, successful/failed `tool_execution_end`, terminal
    transport failure, future-tracked-tool-blocked-while-unresolved,
    model present/absent, session canonicalization, and unchanged
    edit/write-diff and conversation tracing.
  - Verify: `nix run nixpkgs#bun -- test config/lib`; `nix run .#pkl-check-generated`;
    `nix flake check`; plus scratch setup/doctor smoke.
  - Context synchronization: pending

- [ ] T06: `Add production-path and live Pi attribution regressions` (status:todo)
  - Task ID: T06
  - Scope: In — extending the existing mutation-provenance production test
    harness with Pi, driving real temporary Git repositories and real
    repository-scoped Agent Trace databases through the Pi adapter, generic
    mutation ingress, snapshot coordinator, scope provenance,
    `mutation_trace_events`, `mutation_ai_patch`, post-commit intersection, and
    Agent Trace JSON, covering: Pi bash/write/edit confirmed mutation with
    `pi_<session>` + model in Agent Trace; missing model preserving session
    with `model` `NULL`; read/grep/find/ls and custom/unknown tools with zero
    scope footprint; later-extension rejection after Start producing no
    `mutation_ai_patch`; mutate-then-error still observing the final Git tree
    through the confirmed Close; two overlapping Pi calls as independent
    scopes with correct contended/confirmation behavior; one overlapping call
    failing while the surviving scope's later confirmed interval remains
    attributable; Pi+Claude, Pi+Codex, and Pi+OpenCode overlap under
    confirmation-safe semantics; stale/dead Pi process recovery discarding the
    old ambiguous interval while later fresh Pi work remains usable; and
    `user_bash` creating no Pi AI scope. Also a pinned real-Pi smoke covering
    bash, write, edit, SCE Start failure, later extension rejection, execution
    error, and model provenance. Out — weakening any regression because the
    conservative runtime produces less positive attribution than expected —
    fix the expectation or the lifecycle design per the formal semantics
    instead.
  - Dependencies: T05
  - Done when: Pi has the same production-path mutation-attribution confidence
    as Claude, Codex, and OpenCode — confirmed exclusive evidence can reach
    final Agent Trace provenance; uncertain, blocked, failed-to-observe, or
    ambiguous execution cannot.
  - Verify: the complete **Full validation** section, run after context
    synchronization for this task.
  - Context synchronization: pending

## Open questions

None. T01 owns every lifecycle fact that could invalidate this design, and a contradictory finding there is a re-planning gate, not a deferred guess. The one dependency question this plan started with — whether to wait for PR #276 to merge, redo its protocol generalization here, or stack directly on its branch — was resolved during planning: this plan stacks on PR #276's head per **Stack and base**.
