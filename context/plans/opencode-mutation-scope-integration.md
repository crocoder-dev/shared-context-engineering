# Plan: opencode-mutation-scope-integration

## Change summary

Add the third concrete mutation-scope producer to SCE: an OpenCode adapter that
translates OpenCode's tool execution lifecycle into the generic mutation-scope
ingress already used by the shipped Claude Code and Codex adapters
(`cli/src/services/hooks/mutation_scope.rs` seam →
`mutation_trace::runtime::coordinate` / `abandon_scope`). This extends existing
behavior; it does not replace the generic ingress, the verified mutation
protocol, or the two shipped adapters, and it preserves the in-progress
`mutation-scope-provenance` (#275) work this branch is stacked on.

The first supported OpenCode mutation tools are intended to be:

```text
bash         -> TrackedMutation
write        -> TrackedMutation
edit         -> TrackedMutation
apply_patch  -> TrackedMutation

task         -> Delegation
MCP          -> Untracked
custom tools -> Untracked
unknown      -> Untracked
```

Each independently executing tracked OpenCode tool call receives one SCE
`ScopeId`. OpenCode sessions, turns, agents, and task/subagent sessions are
identity/provenance inputs, not mutation scopes themselves.

The implementation is stacked on PR #275 `Mutation scope provenance`, so every
admitted OpenCode scope can persist canonical SCE session identity
(`oc_<sessionID>`) and the model observed for that execution when reliable model
evidence is available. Missing model evidence is represented as `NULL`; it is
never guessed or backfilled later. `ActorKind::OpenCode`
(`cli/src/services/mutation_trace/types.rs`), the `"opencode"` mutation-ingress
wire value (`cli/src/services/hooks/mutation_scope.rs`), the `oc_` session
prefix, and the `mutation_trace_scope_provenance` persistence layer (migration
`005`) all already exist before this PR.

OpenCode differs from Codex in two important ways.

First, OpenCode executes plugin hooks sequentially. SCE's existing config merge
keeps non-SCE plugins before generated SCE plugins. The mutation-scope plugin
will therefore be generated as the final SCE plugin and must remain the final
plugin after setup merging. An earlier policy or user plugin that rejects a tool
consequently rejects it before the mutation-scope Start is established.

Second, OpenCode's current tool lifecycle does not provide a synchronous
failure-aware `tool.execute.after` boundary for ordinary registry tools.
`tool.execute.before` runs before `item.execute`, while `tool.execute.after` is
reached only after `item.execute` returns successfully. `write`, `edit`, and
`apply_patch` perform permission checks inside `item.execute`, so a permission
rejection or execution failure can occur after SCE Start but without an After
callback.

For that reason, OpenCode scopes use the same conservative principle introduced
for Codex: a write-ahead scope is not positive evidence that the tool executed.
Positive mutation attribution requires a confirmation boundary. The existing
Codex-specific confirmation rule (`isCodexScope` / `hasUnconfirmedCodexScope` in
`spec/mutation_cursor.qnt` and `is_codex_scope` / `has_unconfirmed_codex_scope`
in `cli/src/services/mutation_trace/protocol.rs`) is generalized to a
harness-independent "confirmation-required actor" rule covering Codex and
OpenCode.

For Bash, T01 must verify the stronger OpenCode-specific boundary already
visible in source: `shell.env` is invoked after Bash permission evaluation and
before process spawn. If verified, Bash Start uses `shell.env` rather than
`tool.execute.before`.

No new Agent Trace schema or mutation-trace SQL migration is required.

## Design

Numbered decisions the rest of the plan depends on. T01 exists to freeze the
lifecycle evidence each load-bearing decision below rests on; a contradictory
T01 result is a re-planning gate.

### D1 — Scope identity is one OpenCode tool execution

One independently executing tracked OpenCode tool call owns one SCE mutation
scope.

The initial identity candidate is:

```text
(sessionID, callID)
```

Do not treat the OpenCode session itself as a mutation scope. Concurrent tool
calls in the same session must remain distinguishable.

The intended `ScopeId` format is conceptually:

```text
oc-tool-v1|s=<len>:<sessionID>|c=<len>:<callID>
```

T01 must prove `callID` stability/uniqueness before this encoding is frozen. Do
not add turn or agent identity unless T01 demonstrates it is required.

### D2 — Tool classification is explicit

Initial intended classification:

```text
TrackedMutation:
  bash
  write
  edit
  apply_patch

Delegation:
  task

Untracked:
  MCP
  custom/plugin-defined tools
  unknown/future tools
  known read-only tools
```

`Untracked` does not mean read-only. It means:

```text
tool executes normally
tool may mutate
no mutation scope is created
no positive individual mutation attribution is claimed
```

Do not infer mutation capability from arbitrary descriptions, schemas,
annotations, or future tool names. T01 may reduce the tracked set if lifecycle
evidence shows one of the intended tools cannot satisfy the soundness contract.

### D3 — Confirmation-required attribution becomes generic

The current Codex-specific rule must become a generic actor property.
Conceptually replace:

```text
isCodexScope(...)
hasUnconfirmedCodexScope(...)
```

with something equivalent to:

```text
requiresBoundaryConfirmation(actor_kind)
```

where initially:

```text
Codex    -> true
OpenCode -> true
Claude   -> false
Pi       -> preserve current semantics
```

A live confirmation-required scope is unconfirmed until its exact own successful
`Close(scope)` boundary.

Boundary-aware attribution remains conceptually:

```text
if unhealthy / tainted / needs_rebaseline / no live scopes
  -> IneligibleUnscoped

else if ANY live confirmation-required scope is not confirmed by this exact boundary
  -> IneligibleUnscoped

else if exactly one live scope
  -> AiExclusive(scope)

else
  -> AiContended
```

The complete live scope set remains in `MutationEvent.active_scopes`. Preserve
the false-negative-over-false-positive policy.

Examples:

```text
OpenCode A live
Claude boundary
=> IneligibleUnscoped

OpenCode A live
Close(A)
=> AiExclusive(A)

OpenCode A + Claude B live
Close(A)
=> AiContended

OpenCode A + Codex C live
Close(A)
=> IneligibleUnscoped   (C remains unconfirmed)
```

### D4 — Bash uses the strongest available pre-execution boundary

T01 must prove the source-observed lifecycle around OpenCode Bash:

```text
tool.execute.before
-> OpenCode permission evaluation
-> shell.env
-> process spawn
-> process execution
-> tool.execute.after
```

If confirmed, Bash Start must use:

```text
shell.env -> Start(scope)
```

rather than `tool.execute.before`. The point is to establish Start:

```text
after OpenCode permission succeeded
before the child process can mutate
```

Rejected Bash commands must create no mutation scope. T01 must also probe
timeout, abort, non-zero exit, background/detached children, and whether
`tool.execute.after` still arrives for those cases.

### D5 — File mutation tools use write-ahead Start

For:

```text
write
edit
apply_patch
```

the intended lifecycle is:

```text
tool.execute.before
-> Start(scope)

successful tool.execute.after
-> Close(scope)
```

OpenCode may perform tool-specific permission or validation inside the actual
tool execution after `tool.execute.before`. Therefore Start does not prove
permission was granted or mutation occurred. A missing After must never itself
become positive execution evidence. D3 is the safety boundary for this
uncertainty.

### D6 — Generated plugin ordering is load-bearing

OpenCode executes plugin hooks sequentially. SCE setup merging preserves non-SCE
plugins and appends SCE-owned plugins. The final generated/merged order must
make the mutation-scope plugin the last plugin:

```text
<user/non-SCE plugins...>
./plugins/sce-bash-policy.ts
./plugins/sce-agent-trace.ts
./plugins/sce-mutation-scope.ts
```

This ordering is part of the correctness contract. A synchronous failure in an
earlier plugin must prevent the SCE mutation-scope Before callback from being
reached. Setup merge and doctor behavior must eventually verify this ordering.

### D7 — The TypeScript plugin is a thin transport adapter

Do not put mutation protocol business logic in TypeScript.

```text
OpenCode plugin hooks
        ↓
sce-mutation-scope.ts
        ↓
sce hooks opencode-mutation-scope
        ↓
Rust OpenCode adapter
        ↓
hooks::mutation_scope
        ↓
generic mutation runtime
```

TypeScript may own only harness-native concerns such as:

```text
hook registration
model observation
payload forwarding
fail-closed synchronous Start transport
best-effort terminal/error forwarding
```

Rust owns:

```text
strict parsing
classification
attempt identity
ScopeId/EventId
durable state
recovery
provenance normalization
mutation-scope ingress
```

### D8 — Model provenance is observed, not inferred

OpenCode tool execution hooks provide session/call identity but may not directly
carry the executing model.

T01 must determine whether synchronous `chat.params` reliably provides:

```text
sessionID
agent
provider
model
```

before tracked tool execution for:

```text
normal sessions
model switches
task/subagent sessions
```

If reliable, the plugin may keep an ephemeral:

```text
sessionID -> model candidate
```

map. At Start:

```text
provenance.session_id = oc_<sessionID>
provenance.model_id   = exact normalized observed model | NULL
```

If exact model evidence is unavailable, persist `NULL`. Never copy a model from
another session. Never guess. Never backfill provenance after Start.

### D9 — Legitimate OpenCode parallelism is preserved

Do not copy Codex's same-lane predecessor sweep. OpenCode may legitimately have:

```text
A active
B active
```

for different call identities in the same session. Starting B must not retire A
merely because:

```text
A.sessionID == B.sessionID
```

Any stale-attempt recovery mechanism needs stronger evidence than a successor
tool call. T01 must explicitly probe parallel execution.

### D10 — Asynchronous events may clean up but do not establish attribution

OpenCode's generic `event(...)` callback is asynchronous relative to the
synchronous tool trigger path. Events such as:

```text
message.part.updated
session.status
session.error
session disposal/deletion
```

may be useful for exact-attempt cleanup. They are not the write-ahead Start
boundary. Do not make positive attribution depend on an assumption that
asynchronous event delivery occurs before another mutation boundary unless T01
proves that ordering. An exact terminal failure may drive:

```text
Abandon(scope)
```

but D3 remains the correctness boundary while cleanup is delayed.

### D11 — No timeout-based correctness

Do not infer:

```text
scope older than N seconds
=> dead
```

Time is not proof that a tool execution ended. T01 must investigate:

```text
graceful shutdown
plugin disposal
session idle/error
Ctrl-C / interrupt
hard OpenCode process termination
restart behavior
background descendants
multiple OpenCode processes using one checkout
```

If a hard crash cannot safely distinguish an abandoned stale scope from another
process still legitimately executing that scope, retain conservative uncertainty
rather than introducing an unsafe TTL. The availability cost must be documented
rather than hidden.

### OpenCode/plugin version policy

T01's evidence is version-bound. The supported runtime version is evidence, not
a preference.

T01 begins against:

- the exact repo-pinned `@opencode-ai/plugin` version inherited from PR #275
  (currently `1.15.4`, pinned in `config/lib/package.json`); and
- one exact OpenCode CLI version selected and recorded before the first
  load-bearing probe.

Neither version may change after probing begins without stopping T01, updating
the plan/dependency as needed, and rerunning the complete load-bearing probe
matrix.

Evidence from one OpenCode CLI / plugin version may not be used to justify
load-bearing behavior on another version without explicit source/evidence
equivalence.

The open Dependabot branch for a newer plugin version is not itself a reason to
change the version used by T01.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: Exact OpenCode lifecycle evidence exists for the runtime/API version
  this integration supports, covering success, failure, permission rejection,
  interruption, concurrency, subagents, model identity, plugin ordering,
  synchronous plugin execution-barrier behavior, and process/session
  termination.
  - Validate: inspect the committed T01 probe fixtures/report under
    `cli/src/services/hooks/opencode_mutation_scope/fixtures/`; verify the
    `## Design` section defines D1 through D11 and that every load-bearing
    design decision cites an observed event sequence or a pinned upstream
    source reference; verify the `tool.execute.before` failure, `shell.env`
    failure, and earlier-plugin-failure execution-barrier properties are each
    recorded `PROVEN`.

- [ ] AC2: `bash`, `write`, `edit`, and `apply_patch` each create one
  independently identified mutation scope when their proven Start boundary is
  reached.
  - Validate: targeted adapter/plugin tests plus real temporary-worktree tests
    for all four tools (`cargo test -p sce opencode_mutation_scope`).

- [ ] AC3: `task`, MCP, custom tools, unknown tools, and known non-mutating
  tools create no mutation-scope state or runtime Start.
  - Validate: zero-footprint classification tests covering representative
    inputs (`cargo test -p sce opencode_mutation_scope::classify`).

- [ ] AC4: an OpenCode mutation scope cannot produce positive attribution before
  its own confirming successful Close boundary.
  - Validate: Rust protocol tests in
    `cli/src/services/mutation_trace/tests.rs` and Quint invariants in
    `spec/mutation_cursor.qnt` for an unconfirmed OpenCode scope.

- [ ] AC5: an unconfirmed OpenCode scope suppresses positive attribution at
  Claude, Codex, Pi, Flush, or another OpenCode scope's boundary.
  - Validate: protocol/runtime cross-harness tests plus Quint deterministic
    scenarios (`nix run .#quint -- test spec/mutation_cursor.qnt`).

- [ ] AC6: a successful `Close(OpenCode A)` can produce `AiExclusive(A)` when A
  is the only live scope and `AiContended` when all other live scopes are
  already confirmation-safe.
  - Validate: protocol/runtime tests plus Quint reachability witnesses in
    `spec/mutation_cursor.qnt`.

- [ ] AC7: permission rejection or tool failure after OpenCode Start cannot
  create false positive AI attribution, even if terminal cleanup is delayed.
  - Validate: failure-path regression with a mutation/other-harness boundary
    occurring before cleanup arrives.

- [ ] AC8: legitimate parallel OpenCode executions remain separate live scopes
  and are never retired merely because another tool starts in the same session.
  - Validate: parallel-attempt adapter/runtime regression derived from T01
    evidence.

- [ ] AC9: Bash uses a post-permission/pre-process Start boundary if T01
  confirms the `shell.env` ordering; rejected Bash commands create no scope.
  - Validate: live fixture and plugin regression asserting ordering and zero
    Start on rejected Bash.

- [ ] AC10: OpenCode Start provenance stores `oc_<sessionID>` and the exact
  observed normalized model when available; unavailable model evidence produces
  `NULL` rather than an inferred value.
  - Validate: DB-level provenance tests against a real repository Agent Trace DB
    plus the resulting Agent Trace regression.

- [ ] AC11: existing OpenCode Agent Trace and Bash policy behavior remains
  unchanged.
  - Validate: existing `config-lib-bun-tests` plus targeted regressions for
    `config/lib/agent-trace-plugin/` and `config/lib/bash-policy-plugin/`.

- [ ] AC12: the generated mutation-scope plugin is the final OpenCode plugin
  after setup merging, including configurations containing arbitrary user
  plugins, and doctor detects an ordering violation.
  - Validate: Pkl generation tests (`nix run .#pkl-check-generated`) plus
    `config_merge` / doctor fixtures.

- [ ] AC13: `IneligibleUnscoped` OpenCode intervals never enter
  `mutation_ai_patch`; confirmed exclusive OpenCode evidence does.
  - Validate: production mutation-attribution Git/DB tests in
    `cli/src/services/hooks/mod.rs`.

- [ ] AC14: no Agent Trace schema change or new mutation-trace SQL migration is
  introduced.
  - Validate: `git diff origin/mutation-scope-provenance -- config/schema/agent-trace.schema.json cli/migrations/agent-trace-repository/`
    is empty.

- [ ] AC15: the protocol/Quint diff is limited to replacing Codex-specific
  confirmation logic with the generalized confirmation-required actor rule and
  its OpenCode cases.
  - Validate: inspect the branch diff for `spec/mutation_cursor.qnt`,
    `spec/mutation_cursor.md`, `cli/src/services/mutation_trace/protocol.rs`,
    and `cli/src/services/mutation_trace/mbt/` — no unrelated protocol change.

### Full validation

- `nix run .#quint -- typecheck spec/mutation_cursor.qnt`
- `nix run .#quint -- test spec/mutation_cursor.qnt`
- Repository's configured deep Quint invariant verification
  (`checks.mutation-trace-quint-connect`).
- `nix run .#pkl-check-generated`
- `nix flake check`
- `git diff --check`

### Context sync

- `context/cli/mutation-scope-hook-ingress.md`
- `context/cli/mutation-scope-runtime.md`
- new `context/cli/opencode-mutation-scope-integration.md`
- `context/sce/generated-opencode-plugin-registration.md` and
  `context/sce/opencode-agent-trace-plugin-runtime.md` (OpenCode plugin/config
  ownership)
- `context/cli/mutation-trace-protocol.md` (generalized confirmation rule)
- `spec/mutation_cursor.md`
- `context/architecture.md`
- `context/context-map.md`
- `context/glossary.md`
- `context/overview.md`

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** OpenCode lifecycle evidence and probe fixtures; the OpenCode
  Rust adapter (`cli/src/services/hooks/opencode_mutation_scope/`, hidden
  `sce hooks opencode-mutation-scope`); a thin generated
  `config/lib/**` + `config/.opencode/plugins/sce-mutation-scope.ts` plugin;
  adapter durable state and recovery; the bounded generic
  confirmation-required-actor generalization in
  `spec/mutation_cursor.qnt`, `spec/mutation_cursor.md`,
  `cli/src/services/mutation_trace/protocol.rs`, and
  `cli/src/services/mutation_trace/mbt/`; Pkl generation
  (`config/pkl/base/opencode.pkl`, renderers), setup merge, and doctor
  ownership; scope-provenance integration; end-to-end regressions.
- **Out of scope:** Pi mutation-scope integration; Codex MCP attribution;
  exhaustive attribution for OpenCode MCP/custom tools; unrelated Agent Trace
  changes; OpenCode workflow/skill changes; any Agent Trace schema or
  mutation-trace SQL migration.
- **Constraints:** preserve the generic mutation-scope ingress as the shared
  runtime entry point; preserve #275's insert-once provenance semantics; false
  negatives are preferred to false-positive AI attribution; multiple legitimate
  OpenCode executions may not be collapsed or implicitly retired; the mutation
  protocol change is limited to which `ActorKind`s require boundary
  confirmation — no model/session field is added to `ProtocolState`,
  `ScopeState`, `MutationEvent`, `Attribution`, or the Quint scope state.
- **Non-goal:** infer mutation capability from arbitrary tool descriptions,
  annotations, or schemas.
- **Non-goal:** use TTL/staleness as proof that a live scope is dead.
- **Non-goal:** upgrade OpenCode merely to avoid this lifecycle design; the
  current upstream public plugin lifecycle still has the same
  successful-result-only After boundary.
- **Non-goal:** claim `AiExclusive` proves no human, MCP, custom plugin,
  detached process, or other untracked actor mutated concurrently.

## Assumptions

- New adapter code and generated TypeScript are comment-free per repository
  convention (`feedback_no_comments_in_code`).
- Repository verification prefers `nix flake check` and
  `nix run .#pkl-check-generated`; direct Cargo commands are secondary and used
  for targeted debugging (glossary `repo-level verification preference`).
- The hidden `sce hooks opencode-mutation-scope` command name and the
  `cli/src/services/hooks/opencode_mutation_scope/` module location follow the
  existing `claude_mutation_scope` / `codex_mutation_scope` naming already in
  the tree. This is a naming convention only; adapter structure (state-file
  shape, locking strategy, cleanup mechanism, model-candidate cache design) is
  a lifecycle-dependent decision left to T03/T04 against T01 evidence and
  D1–D11, not assumed here.

The OpenCode CLI and `@opencode-ai/plugin` versions are not assumptions: they
are governed by the **OpenCode/plugin version policy** in the Design section —
the plugin version inherited from PR #275, the CLI version selected and recorded
before the first load-bearing probe.

## Task stack

- [x] T01: `Freeze OpenCode mutation lifecycle evidence` (status:done)
  - Task ID: T01
  - Scope: In — reproduce the exact OpenCode lifecycle relevant to mutation
    attribution and commit the fixtures/report under
    `cli/src/services/hooks/opencode_mutation_scope/fixtures/`. Run the complete
    load-bearing probe matrix against the `@opencode-ai/plugin` version
    inherited from PR #275 and one exact OpenCode CLI version selected and
    recorded before the first load-bearing probe, per the **OpenCode/plugin
    version policy** in the Design section; do not silently switch either
    version once probing has begun. Record exact OpenCode CLI version,
    `@opencode-ai/plugin` version, upstream tag/commit, OS, and configuration.

    Probe at minimum:

    ```text
    bash success
    bash non-zero exit
    bash permission rejection
    bash timeout
    bash abort/interrupt
    bash shell.env failure prevents spawn
    bash detached/background descendant behavior

    write success
    write permission rejection
    write validation/tool failure where constructible
    write tool.execute.before failure prevents execution

    edit success
    edit permission rejection/failure

    apply_patch success
    apply_patch validation failure
    apply_patch permission rejection/failure

    parallel tracked execution
    task/subagent execution
    MCP/custom tool behavior
    plugin execution order
    earlier plugin failure prevents mutation-scope hook
    chat.params/model ordering
    terminal message/session events
    graceful shutdown
    hard process termination/restart
    ```

    Include three explicit fail-closed execution-barrier probes, each using a
    dedicated probe plugin:

    - **Probe A — `tool.execute.before` failure:** a probe plugin whose
      `tool.execute.before` throws/rejects before a tracked tool (at least one
      file mutation tool). Prove the Before hook is entered, the hook
      throws/rejects, and the actual tool execute function does NOT run. Record
      whether `tool.execute.after`, a `message.part.updated` error, a session
      error/status event, or any other terminal event subsequently occurs — do
      not assume their presence.
    - **Probe B — `shell.env` failure:** a probe plugin whose `shell.env`
      throws/rejects, exercised with a Bash command that has an observable
      filesystem side effect so non-execution is unambiguous. Prove `shell.env`
      is entered, the hook throws/rejects, and the Bash child process is NOT
      spawned. Record the terminal event sequence.
    - **Probe C — plugin ordering failure:** with
      `user plugin -> sce-bash-policy -> sce-agent-trace -> sce-mutation-scope`,
      prove a synchronous throw in an earlier plugin prevents later
      `tool.execute.before` plugins from executing (required for D6).

    Out — production adapter behavior; any Rust, TypeScript, Pkl, Quint, SQL,
    schema, or generated-file change.
  - Credential-blocked source-only evidence: for a lifecycle case that cannot be
    exercised live because the pinned OpenCode runtime requires unavailable
    provider credentials, T01 may accept `PROVEN-BY-SOURCE` evidence only when:

    1. the exact execution path is established from the pinned upstream source;
    2. the source version exactly matches the frozen OpenCode CLI/plugin version;
    3. the missing live probe is explicitly recorded in the T01 evidence report;
    4. the source evidence is sufficient for the lifecycle/design decision being
       frozen; and
    5. any acceptance criterion requiring later production-path or live
       integration coverage remains outstanding and is not considered satisfied
       by the source-only evidence.

    This exception is narrow. It applies only to cases genuinely blocked by
    external/provider credential availability and only where the exact
    pinned-source execution path is sufficient to prove the lifecycle property.
    It does not permit "source inspection may replace live probes whenever
    convenient", and it does not relax any acceptance criterion.
  - Dependencies: none
  - Done when: every load-bearing lifecycle assumption behind D1 through D11 in
    the `## Design` section has a `PROVEN`, `DOCUMENTED — NON-LOAD-BEARING`, or
    `UNSUPPORTED` disposition in a committed report; exact
    Start/terminal/model/concurrency sequences are recorded; the three
    execution-barrier properties (Probe A, Probe B, Probe C) are each recorded
    `PROVEN`; the maximal safe v1 tracked-tool set is confirmed. If the four
    intended tracked tools cannot satisfy the soundness contract, or the pinned
    `@opencode-ai/plugin` version / selected OpenCode CLI version cannot, stop
    and revise this plan before T02 rather than weakening attribution or
    silently changing a version.
  - Verify: replay/inspect all probe fixtures; compare load-bearing behavior
    against pinned upstream source and cite it in the report.
  - Completed: 2026-09-10
  - Files changed (vs baseline `cc2fe862`):
    - `cli/src/services/hooks/opencode_mutation_scope/fixtures/NOTES.md` (new — the report)
    - `cli/src/services/hooks/opencode_mutation_scope/fixtures/captures/*.jsonl`
      (new — 23 instrumented hook/event captures)
    - `cli/src/services/hooks/opencode_mutation_scope/fixtures/probe-plugins/`
      (new — `capture.ts`, `order-first.ts`, `order-last.ts`, `customtool.ts`,
      `opencode.json`; reference harness, comment-free)
  - Result: Probed OpenCode CLI `opencode-ai@1.15.4` + `@opencode-ai/plugin@1.15.4`
    (identical versions — no cross-version gap; CLI version selected and recorded
    per the version policy, mirroring the Codex T01 "installed version" precedent)
    against pinned upstream `sst/opencode` tag `v1.15.4`
    (`2b92c5677e830e95d34fc3d5664a69297d2d0b51`), Linux `x86_64` / NixOS 26.05.
    Probes driven with a throwaway git repo, an isolated `XDG_*` data/config tree
    (the operator's shared `opencode.db` had been migrated by OpenCode 1.18.x and
    threw `NOT NULL constraint failed: session_message.seq` against the 1.15.4
    binary — confirming OpenCode persistence is global-user-scoped and
    schema-version-coupled, not checkout-local), model `opencode/big-pickle`.
    All of D1–D11 are `PROVEN` (D5's `apply_patch` leg is PROVEN-by-source:
    identical `resolveTools` registry path to the live-proven `write`/`edit`,
    with the patch gate making `apply_patch` and `edit`/`write` mutually
    exclusive per session by model id). Probe A (`tool.execute.before` throw
    blocks the tool), Probe B (`shell.env` throw blocks the child spawn), and
    Probe C (earlier-plugin synchronous throw blocks later plugins + the tool)
    are each `PROVEN`. Maximal safe v1 tracked set confirmed:
    `{bash, write, edit, apply_patch}` as tool names (`write`/`edit` vs
    `apply_patch` mutually exclusive per session), `task` as Delegation,
    everything else (incl. MCP and plugin tools) Untracked. Bash Start boundary
    is `shell.env` (fires after OpenCode permission eval, before spawn — rejected
    bash never reaches it); file-tool Start is write-ahead `tool.execute.before`;
    `tool.execute.after` is the Close boundary and fires for success / non-zero
    exit / exit 127 / tool-enforced timeout but NOT for permission rejection,
    interrupt, or internal validation failure. SIGINT/SIGKILL leave the scope
    with zero terminal hook and zero cleanup event, and orphaned child processes
    keep mutating after OpenCode exits — so no TTL is safe (D11) and D3's
    confirmation-required rule is the correctness boundary. Concurrent bash
    scopes in one session (distinct `callID`) genuinely overlap — no same-session
    predecessor sweep (D9). Model provenance is available synchronously via
    `chat.params` per turn (`providerID` + `api.id`), keyed by `sessionID`
    (subagents get their own child-session `chat.params`); absent evidence →
    `NULL`. **No soundness failure and no version failure — no re-planning gate
    triggered.**
  - Verify outcomes:
    - Replay/inspect all probe fixtures — DONE: 23 committed captures, every line
      valid JSONL, every `captures/*.jsonl` referenced in `NOTES.md` present;
      barrier probes re-inspected (Probe A: `order-last` before-hook never runs +
      `probeA.txt` absent; Probe B: `order-last` `shell.env` never runs +
      `probeB.txt` absent; Probe C: only `order-first` before-hook runs +
      `probeC.txt` absent).
    - Compare load-bearing behavior against pinned upstream source and cite it —
      DONE: every D1–D11 disposition in `NOTES.md` cites `packages/...` paths at
      `v1.15.4` (`plugin/index.ts` trigger loop, `session/prompt.ts`
      `resolveTools`, `tool/shell.ts` L412/L482/L628, `tool/write.ts`,
      `tool/edit.ts`, `tool/apply_patch.ts`, `permission/index.ts`,
      `session/llm.ts` L162, `config/plugin.ts`, `cli/cmd/run/runtime.lifecycle.ts`).
    - `git diff --cached --check` — CLEAN (29 files, +2337, additions only, all
      under the fixtures dir).
  - Context impact: additive. New durable evidence artifact under
    `cli/src/services/hooks/opencode_mutation_scope/fixtures/`. No production
    code, no module wiring (`mod.rs` is T03), no `flake.nix` change (the dir is
    inert until T03 adds tests + the `workspaceSrc` fileset entry — noted in
    `NOTES.md`). New context doc
    `context/cli/opencode-mutation-scope-integration.md` is expected during
    synchronization; `context/cli/mutation-scope-hook-ingress.md`,
    `context/cli/mutation-trace-protocol.md`, `context/architecture.md`,
    `context/context-map.md`, `context/glossary.md`, `context/overview.md` to be
    verified.
  - Deviations / assumptions accepted:
    - OpenCode CLI version = `1.15.4` (installed into the probe runtime, matching
      the `@opencode-ai/plugin` pin), selected and recorded before the first
      load-bearing probe per the version policy.
    - `apply_patch` could not be exercised live: the patch gate needs a
      `gpt-`-class model id, and no such OpenCode credential works here
      (`openai/*` = ChatGPT/Codex account rejecting every model;
      `opencode-go/gpt-5.6-luna` = insufficient balance; free/ollama models have
      no `gpt-` id). Its lifecycle is PROVEN-by-source as identical to
      `write`/`edit`. Live `apply_patch` fixtures for AC2 are an outstanding item
      for T03–T06 before `/validate` — a credential gap, not a soundness gap.
  - Task result — `apply_patch` evidence exception (recorded under the
    *Credential-blocked source-only evidence* rule above):

    ```text
    apply_patch:
      live lifecycle probe: unavailable because the pinned v1.15.4 runtime exposes
      apply_patch only to an eligible gpt-* model and the probe environment had no
      working credential for such a model;

      lifecycle evidence: PROVEN-BY-SOURCE against pinned upstream
      sst/opencode v1.15.4;

      established path:
        tool.execute.before
        -> apply_patch validation / permission / mutation
        -> tool.execute.after only on successful completion;

      remaining requirement:
        live/production-path apply_patch coverage is still required before
        /validate where demanded by AC2 / T03-T06.
    ```
  - Context synchronization: synced

- [x] T02: `Generalize boundary-confirmed attribution to OpenCode` (status:done)
  - Task ID: T02
  - Scope: In — replace Codex-specific unconfirmed-scope logic with a generic
    confirmation-required-actor predicate covering Codex and OpenCode in
    `spec/mutation_cursor.qnt`, `spec/mutation_cursor.md`,
    `cli/src/services/mutation_trace/protocol.rs`, the runtime/MBT refinement in
    `cli/src/services/mutation_trace/mbt/`, invariants, deterministic cases, and
    reachability witnesses. Out — OpenCode adapter/plugin; SQL/schema changes;
    changes to the public `Attribution` variants.
  - Dependencies: T01
  - Done when: unconfirmed Codex/OpenCode scopes conservatively suppress
    positive attribution; each scope's own Close confirms only itself; existing
    Codex behavior is preserved bit-for-bit in outcomes; OpenCode
    exclusive/contended positive attribution remains reachable.
  - Verify: `cargo test -p sce mutation_trace`;
    `nix run .#quint -- typecheck spec/mutation_cursor.qnt`;
    `nix run .#quint -- test spec/mutation_cursor.qnt`;
    `checks.mutation-trace-quint-connect`.
  - Completed: 2026-09-10
  - Files changed (vs baseline `2201ce87`):
    - `cli/src/services/mutation_trace/protocol.rs` — `is_codex_scope` →
      `requires_boundary_confirmation(ActorKind)` + `scope_requires_confirmation`;
      `has_unconfirmed_codex_scope` → `has_unconfirmed_required_scope`;
      `attribution_for_boundary` calls the generalized predicate.
    - `spec/mutation_cursor.qnt` — `isCodexScope`/`hasUnconfirmedCodexScope` →
      `requiresBoundaryConfirmation`/`scopeRequiresConfirmation`/
      `hasUnconfirmedRequiredScope`; three `SafetyAttribution` invariants and the
      `AttributionMatchesObservedScopes` branch generalized; `HasOpenCode*`
      reachability witnesses added; `Scope5` (OpenCode on `WT0`) added to
      `ScopeId`/`SCOPES`/`scopeWorktree`/`scopeActor`/`singleScope`; five new
      `testOpenCode*` deterministic runs.
    - `spec/mutation_cursor.md` — "Unconfirmed Codex scopes" section and the
      attribution/verification prose generalized to confirmation-required actors.
    - `cli/src/services/mutation_trace/mbt/model.rs`,
      `cli/src/services/mutation_trace/mbt/driver.rs` — `WireScopeId::Scope5` and
      the `scope5 → OpenCode/wt0` partition entry.
    - `cli/src/services/mutation_trace/mbt/tests.rs` — three named replay
      wrappers for the new OpenCode scenarios.
    - `cli/src/services/mutation_trace/tests.rs` — `opencode_scope` helper and
      six OpenCode-actor protocol tests mirroring the Codex suite.
    - `cli/src/services/mutation_trace/runtime/coordinator.rs` — the
      `ac5-different-actor` contention case switched from `OpenCode` (now
      confirmation-required, so it suppresses) to `Pi`; two new OpenCode
      cross-harness coordinator tests.
  - Result: The unconfirmed-scope rule is now a harness-independent
    `requiresBoundaryConfirmation(actor)` property — `Codex` and `OpenCode` →
    `true`, `ClaudeCode` and `Pi` → `false` — in both the Quint model and the
    Rust kernel. `attribution_for_boundary` suppresses positive attribution to
    `IneligibleUnscoped` whenever any live confirmation-required scope is not
    confirmed by its own exact `Close`, exactly as the Codex-only rule did.
    Codex outcomes are unchanged: every pre-existing Codex `run` and Rust test
    keeps its original expectation and passes. OpenCode
    `AiExclusive`/`AiContended` positive attribution is reachable and witnessed
    (`HasOpenCodeConfirmedExclusiveEvidence`/`...ContendedEvidence`,
    `testOpenCodeCloseConfirms{Exclusive,Contended}Attribution`), and a mixed
    OpenCode+Codex live pair stays mutually unconfirmed at either `Close`. No
    change to the public `Attribution` variants, `ProtocolState`, `ScopeState`,
    `MutationEvent`, or the Quint scope state; no SQL/schema change.
  - Verify outcomes:
    - `cargo test -p sce mutation_trace` — run as
      `cargo test --manifest-path cli/Cargo.toml mutation_trace` with
      `SCE_CLI_PACKAGE_FALLBACK=1`: 370 passed, 0 failed (includes the 6 new
      OpenCode protocol tests, 2 new coordinator tests, 3 new MBT wrappers, the
      `all_named_scenarios` backstop replaying all 5 new `testOpenCode*` runs,
      and the 6-scope generated-trace refinement).
    - `nix run .#quint -- typecheck spec/mutation_cursor.qnt` — clean.
    - `nix run .#quint -- test spec/mutation_cursor.qnt` — 36 passing (all
      Codex runs preserved; 5 new OpenCode runs green).
    - `checks.mutation-trace-quint-connect` — `nix build
      .#checks.x86_64-linux.mutation-trace-quint-connect`: 16 passed, 0 failed.
    - Extra: `cargo clippy --all-targets` clean. The nightly deep symbolic
      check (`quint verify --invariant=SafetyAttribution`) is outside this
      task's required verify set and outside the required PR path; not run to
      completion here.
  - Context impact: additive + bounded refactor. The mutation-protocol change
    is limited to which `ActorKind`s require boundary confirmation, per the plan
    constraint. `context/cli/mutation-trace-protocol.md` (generalized
    confirmation rule) and `spec/mutation_cursor.md` need synchronization;
    `context/cli/mutation-scope-runtime.md`, `context/architecture.md`,
    `context/context-map.md`, `context/glossary.md`, `context/overview.md` to be
    verified. No new context doc for this task (the new
    `context/cli/opencode-mutation-scope-integration.md` is owned by later
    tasks' adapter/plugin work).
  - Deviations / assumptions accepted:
    - Predicate naming: `requires_boundary_confirmation` /
      `requiresBoundaryConfirmation`, `scope_requires_confirmation` /
      `scopeRequiresConfirmation`, `has_unconfirmed_required_scope` /
      `hasUnconfirmedRequiredScope` — follows existing snake/camel conventions.
    - Reachability modeling: added one representative scope (`Scope5`,
      OpenCode/`WT0`) rather than repointing an existing scope, so every
      pre-existing Codex/Claude `run` keeps its exact scope identities and
      expectations. `VERIFY_SCOPES` stays `= SCOPES` (now cardinality 6); the
      resulting increase in nightly symbolic-verification cost is accepted as it
      is outside the required PR path.
    - `cargo test` was run against the deterministic
      `SCE_CLI_PACKAGE_FALLBACK=1` build because the repo build requires a
      pre-generated Pkl payload; `cli/package-fallback` was already current.
    - New code and Quint runs are comment-free per
      `feedback_no_comments_in_code`.
  - Context synchronization: synced

- [x] T03: `Add OpenCode adapter identity and classification` (status:done)
  - Task ID: T03
  - Scope: In — add hidden `sce hooks opencode-mutation-scope` command routing
    (`cli_schema.rs`, `parse::command_runtime`, `services::hooks`), strict event
    parsing, explicit tool classification (`bash`/`write`/`edit`/`apply_patch`
    tracked, `task` delegation, everything else untracked), `AttemptKey`,
    length-prefixed hash-free `ScopeId`/`EventId` encoding, canonical
    `oc_<sessionID>` session-provenance construction, and model-candidate
    validation/normalization. Out — generated OpenCode plugin wiring; mutation
    runtime side effects; durable state persistence.
  - Dependencies: T01, T02
  - Done when: every proven OpenCode lifecycle input has a deterministic
    normalized representation; duplicate identity is stable; concurrent calls in
    one session remain distinguishable; untracked/delegation events are neutral
    (no Git resolution, no state, no seam call).
  - Verify: `cargo test -p sce opencode_mutation_scope` covering malformed
    payloads, duplicate events, parallel call IDs, task child-session
    identities, and model-present/model-absent provenance.
  - Completed: 2026-09-10
  - Files changed (vs baseline `ccd4cabd`):
    - `cli/src/services/hooks/opencode_mutation_scope/mod.rs` (new — event
      parsing, `classify_tool`, `AttemptKey`, `format_opencode_scope_id` +
      start/close event-id helpers, `opencode_scope_provenance`, the inert
      `run_opencode_mutation_scope_*` entry points, and 28 unit tests)
    - `cli/src/services/hooks/mod.rs` (`pub mod opencode_mutation_scope`;
      `HookSubcommand::OpenCodeMutationScope` variant + dispatch arm +
      `hook_runtime_invocation_name` arm; `normalize_opencode_model_id` + 2
      tests)
    - `cli/src/cli_schema.rs` (hidden `opencode-mutation-scope`
      `HooksSubcommand::OpenCodeMutationScope` with an explicit
      `name = "opencode-mutation-scope"`)
    - `cli/src/services/parse/command_runtime.rs` (mapping arm + 2 tests:
      parses to the hook subcommand; hidden from `sce hooks --help`)
  - Result: `sce hooks opencode-mutation-scope` is registered and hidden. The
    new adapter module turns a plugin→adapter wire event
    (`hook_event_name` discriminator over `ToolExecuteBefore` / `ShellEnv` /
    `ToolExecuteAfter` plus the T01-enumerated terminal signals `ToolError`,
    `SessionIdle`, `SessionError`, `SessionDeleted`, `ServerDisposed`) into a
    deterministic normalized representation. `classify_tool` is a closed
    allowlist — `{bash, write, edit, apply_patch}` → `TrackedMutation`, `task` →
    `Delegation`, everything else (read-only tools, plugin tools, MCP-shaped
    names, unknown/future, empty) → `Untracked` (D2). `AttemptKey` is
    `(session_id, call_id)` only (D1 — no turn/agent identity; child `task`
    sessions carry their own `sessionID`, D9). `format_opencode_scope_id`
    emits the D1-frozen `oc-tool-v1|s=<len>:<sid>|c=<len>:<callID>` with no
    attempt-sequence component; `EventId` is `<scope_id>|start` / `|close`.
    `opencode_scope_provenance` canonicalizes to `oc_<sessionID>` via the
    existing `prefixed_diff_trace_session_id` and normalizes the observed model
    through `normalize_opencode_model_id` (trim, `None` on blank) — absent
    evidence is `NULL`, never guessed (D8). No ingress seam call, no Git
    resolution, no durable state: `run_opencode_mutation_scope_from_payload`
    parses strictly (surfacing malformed input) and returns neutral output for
    every event; the Start/Close/Abandon lifecycle is T04.
  - Verify outcomes:
    - `cargo test -p sce opencode_mutation_scope` — run as the canonical
      `nix build .#checks.x86_64-linux.cli-tests` (per the repo verification
      preference; `cargo test` is bash-policy-blocked): PASS. Coverage includes
      malformed payloads (empty / non-JSON / non-object / unknown
      `hook_event_name` / missing / blank / wrong-typed fields), duplicate
      events reuse the same `ScopeId`, parallel `call_id`s stay distinguishable,
      `task` child-session identity flows through the `AttemptKey`, and
      model-present / model-absent (and already-`oc_`-prefixed) provenance. 32
      new tests total (28 adapter + 2 `command_runtime` routing + 2
      `normalize_opencode_model_id`).
    - `nix build .#checks.x86_64-linux.cli-clippy` — PASS (clean).
    - `nix build .#checks.x86_64-linux.cli-fmt` — PASS (`cargo fmt` applied).
    - `git diff --cached --check` — CLEAN (4 files, +907, additions only).
  - Context impact: additive. New leaf module under
    `cli/src/services/hooks/opencode_mutation_scope/` (picked up by
    `craneLib.fileset.commonCargoSources`; no `flake.nix` `workspaceSrc` entry
    needed because the tests use inline payloads, not `include_str!` fixtures).
    New hidden CLI surface `sce hooks opencode-mutation-scope`, inert until the
    T05 plugin (which depends on T04) routes real events to it.
    `context/cli/mutation-scope-hook-ingress.md`,
    `context/cli/opencode-mutation-scope-integration.md` (new, expected),
    `context/architecture.md`, `context/context-map.md`, `context/glossary.md`,
    `context/overview.md` to be verified during synchronization. No protocol,
    schema, Pkl, Quint, or SQL change.
  - Deviations / assumptions accepted:
    - Plugin↔adapter wire JSON (`hook_event_name` discriminator; `session_id`,
      `call_id`, `cwd`, `tool_name`, `model` fields; PascalCase event names) is
      an internal SCE interface defined in this task and consumed by T05,
      following the Codex adapter's payload-shape precedent.
    - `ScopeId` omits the Codex/Claude-style `n=<attempt_seq>` component: T01
      proved `callID` is collision-safe and never reused; a future generational
      need bumps the scheme to `oc-tool-v2`.
    - `run_opencode_mutation_scope_from_payload` is intentionally inert for
      tracked tools (parses, returns neutral) pending T04. Safe because the
      invoking T05 plugin depends on T04; nothing invokes the command in
      production between T03 and T05.
    - Terminal-event parsing (`ToolError`/`SessionIdle`/`SessionError`/
      `SessionDeleted`/`ServerDisposed`) is included now — T01 froze these
      signals — so T04 need not touch the parser; their field sets are the
      lightest defensible shape (`session_id` + `cwd`, or `cwd` only for
      `ServerDisposed`).
    - `normalize_opencode_model_id` mirrors `normalize_codex_model_id`
      (lenient trim / non-blank); the T05 plugin composes `providerID/api.id`
      before forwarding.
    - New code is comment-free per `feedback_no_comments_in_code`.
  - Context synchronization: synced

- [x] T04: `Implement OpenCode scope lifecycle and recovery` (status:done)
  - Task ID: T04
  - Scope: In — durable checkout-local OpenCode attempt bookkeeping in a state
    file under `<git-dir>/sce/` with its own lock never held across a seam call
    (exact file shape and locking strategy chosen in this task against T01
    evidence and D1–D11), write-ahead fail-closed `Start`, `Close`,
    `Abandon`/recovery, the recovery barrier, duplicate/replay behavior,
    multiple concurrently active attempts, and the T01-proven cleanup/restart
    signals. Drive only the in-process generic mutation-scope ingress seam.
    Out — TypeScript plugin generation/setup wiring.
  - Dependencies: T03
  - Done when: Start is durably established before a tracked tool's
    mutation-capable boundary returns; successful terminal evidence closes
    exactly that attempt; failure cleanup abandons exactly the affected attempt;
    terminal failures leave conservative recoverable state; one execution cannot
    retire a different concurrent execution; no unsafe timeout or same-session
    sweep exists.
  - Verify: `cargo test -p sce opencode_mutation_scope` state-machine tests
    covering Start replay, concurrent attempts, Close replay, Abandon, delayed
    error cleanup, recovery failure, process/session cleanup, and zero-footprint
    untracked events.
  - Completed: 2026-09-10
  - Files changed (vs baseline `4f95be85`):
    - `cli/src/services/hooks/opencode_mutation_scope/os_lock.rs` (new — OS
      advisory lock primitive, mirrors the Codex adapter)
    - `cli/src/services/hooks/opencode_mutation_scope/boundary_lock.rs` (new —
      per-`git-dir` boundary lock + 3 lock tests)
    - `cli/src/services/hooks/opencode_mutation_scope/state.rs` (new —
      `AdapterState`/`AdapterAttempt` keyed by `(session_id, call_id)`,
      `AttemptPhase`, generation-tracked `RecoveryState`, `admit_tracked_attempt`,
      `mark_active`, `remove_attempt`, `arm_recovery` /
      `complete_recovery_flush` / `relinquish_recovery_flush` /
      `normalize_recovery_after_boundary_lock_acquired`, durable
      temp-file+rename writer, and 25 unit tests)
    - `cli/src/services/hooks/opencode_mutation_scope/mod.rs` (wire
      `mod state/os_lock/boundary_lock`; `dispatch_opencode_hook_event`
      lifecycle, `establish_tracked_start`, `admit_or_recover` /
      `readmit_after_flush`, `establish_start`, `handle_close`, seam payload
      builders, `with_boundary_lock`; the ambiguity-consuming
      `abandon_and_consume` (see **T04 soundness correction**) replaced the
      first cut's `cleanup_attempts_matching` / `abandon_attempt`;
      `run_opencode_mutation_scope_from_payload` now
      resolves the checkout and drives the real generic seam;
      `run_opencode_mutation_scope_from_payload_at_state_root` +
      `_with_seams` test entrypoints; one T03 neutrality test replaced by a
      fail-closed test; `lifecycle_tests` + `runtime_seam_tests`)
  - Result: `sce hooks opencode-mutation-scope` now drives a full scope
    lifecycle over the generic in-process ingress seam
    (`hooks::mutation_scope::run_mutation_scope_from_payload`). Bash `Start` is
    anchored to `ShellEnv` (post-permission, pre-spawn — `ToolExecuteBefore` for
    `bash` is inert, so rejected bash creates no scope, D4); `write`/`edit`/
    `apply_patch` `Start` is write-ahead on `ToolExecuteBefore` (D5); `Close` is
    a successful `ToolExecuteAfter`; `ToolError` abandons exactly its
    `(session_id, call_id)` attempt and consumes that attempt's ambiguous
    filesystem interval before any surviving scope can confirm itself (see the
    **T04 soundness correction** below); the broad asynchronous events
    `SessionIdle`/`SessionError`/`SessionDeleted`/`ServerDisposed` are parsed
    for a stable T05 wire contract but drive **no** live-scope abandonment
    (D10/D11). Durable state is a
    checkout-local `<git-dir>/sce/opencode-mutation-scope-state.json` guarded by
    `opencode-mutation-scope-state.lock` (held only for individual file ops,
    never across a seam call) with all boundary processing serialised by
    `opencode-mutation-scope-boundary.lock`. Attempts are keyed solely by the
    D1-frozen `(session_id, call_id)` `ScopeId` — no turn/agent identity, no
    `attempt_seq`, and **no same-session predecessor sweep**: a second call in a
    live session is admitted alongside the first (D9). `admit` fails closed on
    its own recovery barrier (generation-tracked `Pending`/`Flushing`, Codex
    model) and on a lingering foreign `PendingStart` (crash residue). Any
    failure to durably establish `Start` — checkout resolution, admit denial,
    seam error, `mark_active` error — returns a non-zero `Err` carrying
    `FAIL_CLOSED_MESSAGE` so the T05 plugin throws and blocks the tracked tool;
    `Close`/terminal paths are best-effort and fall back to `Abandon` on seam
    failure. No TTL / staleness heuristic anywhere (D11). Duplicate `Start`,
    `Close`, and terminal deliveries are idempotent. Untracked/delegation events
    (`read`, `task`, MCP-shaped, unknown) never resolve a checkout, touch state,
    or call the seam.
  - Verify outcomes:
    - `cargo test -p sce opencode_mutation_scope` state-machine tests — run as
      the repo-canonical `nix build .#checks.x86_64-linux.cli-tests` (direct
      `cargo test` is bash-policy-blocked; per `project_sce_cargo_test_invocation`
      the fallback build is the real invocation): PASS. 73
      `opencode_mutation_scope` tests, all green (25 `state::tests`, 3
      `boundary_lock::tests`, 15 `lifecycle_tests`, 3 `runtime_seam_tests`, plus
      the pre-existing parser/classification suite). Coverage: write-ahead Start
      replay, bash Start anchored to `ShellEnv`, concurrent same-session scopes
      staying separate, Close replay as a no-op, and three real-runtime
      Start/Close/Abandon assertions against a repository Agent Trace DB. The
      abandonment / broad-event / ambiguity-consumption coverage was corrected
      and expanded — see **T04 soundness correction** (final suite: 1482 passed,
      0 failed, 1 ignored).
    - `nix build .#checks.x86_64-linux.cli-clippy` — PASS (clean).
    - `nix build .#checks.x86_64-linux.cli-fmt` — PASS (`cargo fmt` applied).
    - `git diff --cached --check` — CLEAN (4 files, +2243 / −9).
  - Context impact: additive. New leaf modules under
    `cli/src/services/hooks/opencode_mutation_scope/` (picked up by
    `craneLib.fileset.commonCargoSources`; no `flake.nix` `workspaceSrc` entry —
    tests use inline payloads and a temp repo, not `include_str!` fixtures). No
    new hidden CLI surface (T03's `sce hooks opencode-mutation-scope` is now
    live rather than inert). No protocol, schema, Pkl, Quint, or SQL change; no
    change to the generic ingress seam itself.
    `context/cli/mutation-scope-hook-ingress.md`,
    `context/cli/mutation-scope-runtime.md`,
    `context/cli/opencode-mutation-scope-integration.md` (new, expected),
    `context/architecture.md`, `context/context-map.md`, `context/glossary.md`,
    `context/overview.md` to be verified during synchronization.
  - Deviations / assumptions accepted:
    - State-file shape and locking strategy (delegated to this task by the plan)
      mirror the Codex adapter's structure and per-adapter file layout:
      `opencode-mutation-scope-state.json` / `.lock` /
      `-boundary.lock`. `os_lock` / `boundary_lock` are duplicated into the
      module rather than promoted to a shared location, matching the existing
      `claude_mutation_scope` / `codex_mutation_scope` layout.
    - Adapter↔plugin fail-closed contract: a non-zero adapter exit on a tracked
      `Start` event means "block the tool"; consumed by the T05 plugin. Internal
      SCE interface, following the Codex deny precedent adapted to OpenCode's
      throw-based transport.
    - Recovery uses generation-tracked `Pending`/`Flushing` (Codex model), not a
      bare boolean, for the D11 multi-writer case.
    - `ScopeId` keeps the T03 scheme with no `n=<attempt_seq>` component; a
      lingering `PendingStart` from a crashed invocation blocks new tracked
      Starts in that checkout until a terminal/session event clears it — a
      deliberate fail-closed availability cost per D11.
    - `apply_patch` still has no live end-to-end coverage here (T01 credential
      gap); its lifecycle is identical to `write`/`edit` in the adapter and the
      outstanding live-coverage item for `/validate` is unchanged.
    - New code is comment-free per `feedback_no_comments_in_code`.
  - Context synchronization: synced
  - **T04 soundness correction** (follow-up on `84dcd8d2`, same task):
    - **Root cause.** The first T04 cut used `abandon_scope()` alone to retire an
      uncertain scope. `abandon_scope()` transitions scope state and arms
      `needs_rebaseline` but does not itself consume the filesystem interval the
      abandoned scope's possible mutations occupy. With a concurrent survivor:
      `Start(A) Start(B) mutate(A) mutate(B) Abandon(A) Close(B)` — once A left
      the live set, B became the sole live scope and its own `Close` confirmed
      it, so the interval that could contain A's changes was attributable
      `AiExclusive(B)`. The adapter's own recovery barrier forbade the
      ambiguity-clearing `Flush` while any attempt remained live
      (`Pending + non-empty attempts ⇒ Flush forbidden`), which is exactly what
      allowed the contamination. Broad asynchronous events
      (`SessionIdle`/`SessionError`/`SessionDeleted` → sweep the session,
      `ServerDisposed` → sweep the checkout) were also unsafe: OpenCode's
      `event(...)` is fire-and-forget (D10), so a delayed event can retire a
      newer live call, and `ServerDisposed` carries only checkout identity so one
      process's disposal cannot be proven to refer to another's scopes.
    - **New recovery/Flush semantics.** `abandon_and_consume` (mod.rs), run under
      the per-`git-dir` boundary lock: (1) `arm_and_begin_recovery_flush` →
      `Flushing{gen}`; (2) ingress `flush` while the doomed scope **and any live
      siblings** are still registered — the generalized confirmation-required
      rule makes the runtime resolve it `IneligibleUnscoped` and advance the
      cursor past the ambiguous interval; (3) ingress `abandon` for each doomed
      scope, then `remove_attempt`; (4) a second ingress `flush` to consume the
      `needs_rebaseline` that step 3 arms, so surviving siblings keep their
      **future** intervals; (5) `complete_recovery_flush(gen)` → `Clear`.
      Surviving attempts stay `Active` and are never swept.
      `admit_tracked_attempt` no longer forbids the flush while attempts remain
      (`Pending ⇒ FlushClaimed` unconditionally): a new `Start` retries a
      previously-failed consume, and stays fail-closed until it succeeds. A
      failed `flush` `relinquish`es to `Pending` (recovery-required); generation
      ownership still prevents a stale `flush` completion from clearing a newer
      recovery requirement (`complete_recovery_flush` guards `owned == gen`).
    - **Broad async events now handled** as non-authoritative: `SessionIdle`,
      `SessionError`, `SessionDeleted`, and `ServerDisposed` are parsed (stable
      T05 wire contract) but abandon nothing. Lingering unconfirmed scopes after
      a crash / missing terminal event are accepted (D3 keeps them non-AI); no
      TTL. Only exact `ToolError` causal evidence retires an attempt.
    - **`ToolError` gained `tool_name`.** T01's `message.part.updated` tool part
      carries `part.tool` on the `error` transition
      (`captures/*-perm-ask.jsonl`), so the wire contract now carries
      `tool_name` on `ToolError`; the adapter classifies before `resolve_git_dir`
      / boundary lock / state access, so a `Delegation` or `Untracked`
      `ToolError` is genuinely zero-footprint.
    - **Tests added / rewritten** (`opencode_mutation_scope`): `state::tests` —
      `admit_claims_the_flush_even_while_attempts_remain_outstanding`,
      `arm_and_begin_recovery_flush_claims_flushing_regardless_of_pending_generation`,
      `a_stale_flush_completion_never_clears_a_newer_recovery_generation` (the
      gen-4-vs-gen-5 case); `lifecycle_tests` —
      `tool_error_retires_the_named_attempt_and_consumes_the_ambiguous_interval`,
      `exact_error_retires_only_the_named_sibling`,
      `a_close_before_start_confirmation_consumes_rather_than_closes`,
      `session_idle_is_non_destructive`,
      `delayed_session_idle_cannot_retire_a_newer_call`,
      `server_disposed_cannot_sweep_another_processes_attempt`,
      `a_failed_sibling_does_not_retire_survivors_or_block_new_starts`,
      `a_terminal_failure_consumes_the_interval_and_the_next_start_proceeds`,
      `a_failed_ambiguity_flush_stays_recovery_pending_and_fails_closed_starts`,
      `close_seam_failure_falls_back_to_consume`, plus `tool_error` untracked
      cases in the zero-footprint test; `runtime_seam_tests` (real temp Git repo
      + real Agent Trace DB) — `regression_a_failed_concurrent_scope_cannot_contaminate_a_survivor`
      (asserts an `ineligible_unscoped` event and **no** `ai_exclusive` for B),
      `regression_b_survivor_still_attributes_its_later_mutations` (B gets
      `ai_exclusive` for post-consume work),
      `regression_c_exact_error_does_not_sweep_siblings_through_the_real_runtime`,
      `regression_d_delayed_session_idle_cannot_kill_a_newer_call`,
      `regression_e_server_disposed_cannot_sweep_another_process`,
      `regression_f_untracked_tool_error_is_zero_footprint`.
    - **Verification.** `nix build .#checks.x86_64-linux.cli-tests` —
      `test result: ok. 1482 passed; 0 failed; 1 ignored`.
      `nix build .#checks.x86_64-linux.cli-clippy` / `.cli-fmt` — PASS.
      `nix run .#quint -- typecheck spec/mutation_cursor.qnt` — clean.
      `nix build .#checks.x86_64-linux.mutation-trace-quint-connect` —
      `16 passed; 0 failed`. `nix run .#pkl-check-generated` — 141 files,
      inventory sha256 `b5967aeccf044184f8e6aaab0a863254726e849664f95abdc733c77065dcb34e`.
      `git diff --check` — clean.
    - **No protocol / Quint / Pkl / SQL change.** The fix is entirely in the
      OpenCode adapter recovery layer plus the `ToolError` wire contract; the
      generic runtime's `flush` / `abandon` / `coordinate` semantics and the
      `requires_boundary_confirmation` predicate are unchanged, so no Quint
      semantic change was necessary — the existing Quint checks are re-run only
      to prove no regression.
    - **Files changed** (vs `84dcd8d2`):
      `cli/src/services/hooks/opencode_mutation_scope/mod.rs`,
      `cli/src/services/hooks/opencode_mutation_scope/state.rs`; context:
      `context/cli/opencode-mutation-scope-integration.md`,
      `context/cli/mutation-scope-hook-ingress.md`,
      `context/cli/mutation-scope-runtime.md`, `context/overview.md`,
      `context/context-map.md`, `context/plans/opencode-mutation-scope-integration.md`.
  - **T04 soundness/liveness correction — durable terminal-cleanup intent**
    (follow-up on `5995391a`, same task):
    - **Root cause.** `abandon_and_consume` removed the doomed attempt from
      adapter state even when the generic ingress `abandon` seam call failed —
      the abandon error was only logged, then `remove_attempt` ran
      unconditionally. That produced *adapter state: A forgotten / mutation
      protocol: A still Active + unconfirmed*: because OpenCode scopes are
      confirmation-required, the orphaned protocol scope suppressed positive
      attribution indefinitely (a permanent false negative) and — worse — the
      adapter had discarded the exact durable information needed to retry the
      terminal cleanup. A related gap: after a first ambiguity-`flush` failure,
      recovery state remembered only `Pending { generation }`, not "scope A is
      known terminal and still needs cleanup", so a later generic recovery flush
      could clear recovery and admit new work while silently dropping the
      responsibility to retire A.
    - **Final attempt state machine.** `AttemptPhase` is now
      `PendingStart → Active → PendingAbandon`. `PendingAbandon` means exact
      terminal evidence was observed, the OpenCode execution is finished, the
      protocol scope may still be live, and cleanup/rebaseline is outstanding;
      such an attempt never returns to `Active`, is never a reusable `Start`, and
      is not removed until the generic `abandon` definitely succeeds. No elapsed
      time, no TTL.
    - **`PendingAbandon` durability ordering.** On an exact tracked `ToolError`
      under the boundary lock: (1) `state::begin_terminal_cleanup` persists the
      doomed attempt(s) as `PendingAbandon` **and** the recovery generation
      (`Flushing{g}`) in a single durable write — before any seam call;
      (2) `resolve_recovery` drives the ineligible ambiguity `flush` while the
      doomed scope + siblings are still live; (3) drives the generic `abandon`
      for each doomed scope; (4) `state::remove_attempt` runs **only after** that
      `abandon` returns `Ok`; (5) drives the rebaseline `flush`;
      (6) `complete_recovery_flush(g)` clears recovery only if `g` still owns it.
      Terminal intent is persisted before any cleanup call that can fail; the
      attempt is removed only after `abandon` has definitely succeeded.
    - **First ambiguity `flush` fails.** `resolve_recovery` relinquishes
      `Flushing{g} → Pending{g}` and returns; the doomed attempt stays
      `PendingAbandon`, siblings stay `Active`, recovery stays unresolved,
      nothing is removed. A new tracked `Start` while recovery is unresolved
      hits `FlushClaimed{g}` and replays the whole `flush`/`abandon`/`flush`
      sequence before it can itself be admitted (fails closed until it
      completes). A replayed `Start` for the `PendingAbandon` identity itself is
      refused (`TerminalAttemptBlocked`) and fails closed.
    - **`abandon` fails.** `resolve_recovery` relinquishes to `Pending{g}` and
      returns before `remove_attempt`; the attempt stays `PendingAbandon` and
      stays in durable adapter state; the protocol scope is not falsely
      considered cleaned. Retry replays the idempotent sequence (repeating the
      ambiguity `flush` for safety) until `abandon` succeeds, then removes the
      attempt and clears recovery.
    - **Rebaseline `flush` fails after a successful `abandon`.** The doomed
      attempt was already removed (its `abandon` succeeded); recovery stays
      `Pending{g}`, which alone carries the "finish the rebaseline" obligation.
      The next recovery-capable boundary re-runs `resolve_recovery`: with no
      `PendingAbandon` attempts left it just replays `flush` (consuming the
      `needs_rebaseline` that `abandon` armed) and clears recovery. No permanent
      poison; the smaller state machine (recovery generation, not an extra
      per-attempt cleanup phase) is sufficient because `abandon` is idempotent.
    - **Recovery retries known terminal attempts.** `admit_or_recover`'s
      `FlushClaimed{g}` branch no longer does a bare `flush` + `complete` +
      admit. It calls `resolve_recovery(g)`, which reads the current
      `PendingAbandon` set and drives `flush` → `abandon` each → remove each →
      `flush` → `complete`. Recovery is "complete" only when every terminal
      protocol scope for that generation has been retired and the rebaseline
      `flush` has succeeded. `normalize_recovery_after_boundary_lock_acquired`
      still demotes an orphaned `Flushing{g}` (crashed mid-flush) to `Pending{g}`.
    - **Surviving parallel scopes preserved.** `resolve_recovery` only touches
      attempts whose phase is `PendingAbandon`; siblings stay `Active` and are
      never swept. The concurrent
      `Start(A) Start(B) mutate mutate ToolError(A)` scenario still consumes the
      A/B interval as `IneligibleUnscoped` and lets a later `Close(B)` produce
      `AiExclusive(B)` for B's post-recovery interval. No same-session
      predecessor sweep. Broad async `SessionIdle`/`SessionError`/
      `SessionDeleted`/`ServerDisposed` events remain non-destructive.
    - **Duplicate `ToolError` / late `ToolError` after `Close`.** Duplicate while
      `PendingAbandon` re-enters `resolve_recovery` for the same generation
      (idempotent: no new attempt, no new scope, generation not incremented, no
      sibling sweep). After a completed `Close` the attempt is gone, so
      `abandon_and_consume` finds nothing and returns a harmless no-op.
    - **State model / API.** `state.rs`: `AttemptPhase::PendingAbandon`;
      `AdmitDecision::TerminalAttemptBlocked`; `admit_tracked_attempt` checks the
      exact-key match *before* the recovery-state match (so a `PendingAbandon`
      identity is refused terminally and an `Active`/`PendingStart` duplicate is
      reused) and treats a lingering `PendingAbandon` like a `PendingStart` for
      the uncertain-attempt fail-closed guard; new `begin_terminal_cleanup`
      (atomic phase-mark + generation arm). `mod.rs`: `RecoveryResolution` enum;
      `resolve_recovery` replaces the inline flush/abandon/flush in both
      `abandon_and_consume` and the `FlushClaimed` branch; `handle_close` routes
      `PendingAbandon` (like `PendingStart`) to `abandon_and_consume`.
    - **Tests added** (`opencode_mutation_scope`): `lifecycle_tests` —
      `regression_a_abandon_failure_preserves_terminal_intent_then_recovers`,
      `regression_b_ambiguity_flush_failure_blocks_new_starts_then_recovers`,
      `regression_c_rebaseline_flush_failure_is_recoverable_without_poison`,
      `regression_e_duplicate_tool_error_is_idempotent`,
      `regression_f_start_replay_for_a_pending_abandon_identity_never_reactivates`,
      `regression_g_late_tool_error_after_close_is_a_harmless_no_op`,
      `regression_h_siblings_stay_active_through_a_transient_cleanup_failure`;
      `runtime_seam_tests` (real temp Git repo + real Agent Trace DB) —
      `regression_d_concurrent_survivor_stays_usable_after_a_transient_cleanup_failure`
      (injects one transient `abandon` seam failure, retries via a duplicate
      healthy `ToolError`, then asserts the A/B interval is `ineligible_unscoped`,
      A's protocol scope ends `abandoned`, and B's later mutation is
      `ai_exclusive`). `RecordingSeam` gained `failing_once_on` /
      `failing_on_nth_occurrence`; `OpenCodeRepo` gained
      `drive_failing_seam_operation_once`. The existing stale-generation
      regression (`a_stale_flush_completion_never_clears_a_newer_recovery_generation`)
      is retained.
    - **Verification.** `nix build .#checks.x86_64-linux.cli-tests` — ok (exit 0;
      `opencode_mutation_scope` suite 91 passed / 0 failed run directly).
      `nix build .#checks.x86_64-linux.cli-clippy` / `.cli-fmt` — PASS.
      `nix run .#quint -- typecheck spec/mutation_cursor.qnt` — clean;
      `nix run .#quint -- test spec/mutation_cursor.qnt` — exit 0 (no spec
      change). `nix build .#checks.x86_64-linux.mutation-trace-quint-connect` —
      exit 0. `nix run .#pkl-check-generated` — 141 files, inventory sha256
      `b5967aeccf044184f8e6aaab0a863254726e849664f95abdc733c77065dcb34e`
      (unchanged). `git diff --check` — clean.
    - **No protocol / Quint / Pkl / SQL / wire-contract change.** The fix is
      entirely OpenCode adapter recovery bookkeeping (`mod.rs` + `state.rs`); the
      `ToolError` wire contract from the previous correction is unchanged. The
      generic runtime's `flush` / `abandon` (idempotent) / `coordinate` semantics
      and `requires_boundary_confirmation` already suffice — existing Quint
      checks re-run only as regression verification.
    - **Files changed** (vs `5995391a`):
      `cli/src/services/hooks/opencode_mutation_scope/mod.rs`,
      `cli/src/services/hooks/opencode_mutation_scope/state.rs`; context:
      `context/cli/opencode-mutation-scope-integration.md`,
      `context/cli/mutation-scope-hook-ingress.md`,
      `context/cli/mutation-scope-runtime.md`, `context/architecture.md`,
      `context/overview.md`, `context/context-map.md`,
      `context/plans/opencode-mutation-scope-integration.md`.

- [x] T05: `Wire the OpenCode mutation-scope plugin` (status:done)
  - Task ID: T05
  - Scope: In — add canonical `config/lib/` TypeScript plugin support and the
    generated `config/.opencode/plugins/sce-mutation-scope.ts`; register it in
    `config/pkl/base/opencode.pkl` / renderer handoff as the final SCE plugin;
    hook registration, model observation via `chat.params`, synchronous
    fail-closed Start invocation, best-effort terminal/error forwarding; setup
    merge behavior keeping SCE mutation-scope last after arbitrary user plugins;
    doctor ordering expectations; generated inventories and the artifact-path
    count; relevant Bun/type tests. Preserve the existing `sce-bash-policy` and
    `sce-agent-trace` plugins. Out — changes to unrelated OpenCode
    workflows/skills.
  - Dependencies: T04
  - Done when: generated and installed OpenCode configurations route real
    lifecycle events to the Rust adapter; SCE mutation-scope is last after
    arbitrary user plugins through the config merge; Bash uses the T01-proven
    Start boundary; file mutation tools use their T01-proven boundary; failure
    to establish Start prevents tracked mutation execution; terminal failures do
    not fabricate a Close.
  - Verify: `config-lib-bun-tests`; TypeScript typecheck;
    `nix run .#pkl-check-generated`; setup merge/doctor tests; `nix flake check`.
  - Completed: 2026-09-11
  - Files changed (vs baseline `af975c77`):
    - `config/lib/mutation-scope-plugin/opencode-sce-mutation-scope-plugin.ts`
      (new — thin transport plugin: `chat.params` per-session `providerID/api.id`
      map ignoring the `title` agent, replacing rather than merging the cached
      model on every non-`title` event (see the **T05 correctness correction**
      below); `tool.execute.before` for `write`/`edit`/`apply_patch` →
      fail-closed `ToolExecuteBefore`; `shell.env` → fail-closed `ShellEnv` bash
      Start; `tool.execute.after` → best-effort `ToolExecuteAfter` Close; `event`
      → best-effort `ToolError` only, plus local `SessionDeleted` model-cache
      bookkeeping; `spawnSync sce hooks opencode-mutation-scope`, throw on any
      non-`"ok"` outcome — non-zero exit, spawn failure, timeout, or `ENOENT` —
      per the same correction)
    - `config/lib/mutation-scope-plugin/mutation-scope-runtime.test.ts` (new —
      10 Bun tests over a mocked `node:child_process`)
    - `config/lib/tsconfig.json` (`mutation-scope-plugin/**/*.ts` include)
    - `config/pkl/base/opencode.pkl` (`sce_mutation_scope_plugin` registration)
    - `config/pkl/renderers/common.pkl` (`sce_mutation_scope_plugin` appended
      last in `sceGeneratedOpenCodePlugins`)
    - `config/pkl/generate.pkl` (read source + emit
      `config/.opencode/plugins/sce-mutation-scope.ts`)
    - `config/pkl/renderers/generation-contract-check.pkl` (expected artifact
      path + `expectedArtifactPathCount` 141 → 142)
    - `config/pkl/check-generated.sh` (`required_paths` entry)
    - `config/pkl/generator-inputs.txt`, `cli/build.rs`
      (`CANONICAL_GENERATOR_INPUTS` + `create_fixture` write), `flake.nix`
      (`cliBuildInputFileset`, `cliGeneratedInputSrc`, `configLibBashPolicySrc`
      dir entry, `pklGeneratedCheckSrc`) — new `.ts` threaded through every
      generator-input list and Nix fileset
    - `scripts/test-check-generated.sh` (fake-pkl scaffold plugin file)
    - `cli/src/services/doctor/inspect.rs`
      (`inspect_opencode_plugin_ordering_health` + call from
      `inspect_opencode_integration_health` + test module import + a
      two-case unit test; reuses `ProblemKind::OpenCodePluginRegistryInvalid`)
    - `cli/src/services/setup/config_merge.rs` (`generated_opencode_config`
      helper adds the third plugin; count assertions 2/3 → 3/4;
      `opencode_merge_keeps_mutation_scope_last_after_arbitrary_user_plugins`
      test)
    - `cli/src/services/setup/mod.rs`
      (`install_merges_into_existing_opencode_config_json_and_stays_idempotent`
      asserts `sce-mutation-scope.ts` is the final installed plugin)
  - Result: A generated `config/.opencode/plugins/sce-mutation-scope.ts` (byte-
    identical to the `config/lib/` source) is registered as the final entry of
    the generated `opencode.json` `plugin` array. The existing OpenCode
    config merge (`merge_opencode_config`) already drops SCE-shaped entries and
    appends `generated.plugin` in order, so the merged/installed array is
    `[<user plugins…>, sce-bash-policy, sce-agent-trace, sce-mutation-scope]` for
    any user configuration — no merge-logic change was required, only the
    generated-array change plus coverage. `sce doctor` now flags an installed
    `opencode.json` that lists `./plugins/sce-mutation-scope.ts` anywhere other
    than last (`inspect_opencode_plugin_ordering_health`). The plugin is a pure
    transport adapter: it holds no mutation-protocol state, forwards the
    T03/T04 wire contract (`hook_event_name` discriminator + `session_id` /
    `call_id` / `cwd` / `tool_name` / `model`), anchors bash Start to
    `shell.env` (post-permission, pre-spawn) and `write`/`edit`/`apply_patch`
    Start to write-ahead `tool.execute.before`, and throws on **any** failure to
    establish a tracked Start — non-zero adapter exit, spawn failure, timeout,
    or a missing `sce` CLI (`ENOENT`, which also logs an install warning) — and
    on an unusable `shell.env` identity (missing/empty `sessionID`/`callID`,
    which never reaches the adapter at all) — so OpenCode always blocks the
    tool when Start cannot be established (T01 Probe A/B; see the **T05
    correctness correction** below for the `ENOENT` fix and the **T05
    shell.env identity correction** below for the identity fix).
    `tool.execute.after` is the only Close path and only fires on real tool
    success, so terminal failures never fabricate a Close. Model provenance is
    observed synchronously per turn via `chat.params` (`providerID/api.id`,
    keyed by `sessionID`, ignoring the internal `title` agent); every
    non-`title` event replaces the session's cached observation — a valid model
    overwrites it, an invalid/missing one clears it — so absent evidence always
    forwards `model: null` and never a stale prior value (see the correction
    below). Only exact `ToolError` is forwarded for terminal/error signals;
    `SessionIdle`/`SessionError`/`ServerDisposed` are not forwarded to the
    adapter (its dispatch is a no-op for them) and `SessionDeleted` only clears
    local model-cache state. `apply_patch` still has no live end-to-end coverage
    (T01 credential gap) — unchanged, outstanding for `/validate` AC2.
  - Verify outcomes:
    - `config-lib-bun-tests` — `nix build .#checks.x86_64-linux.config-lib-bun-tests`:
      PASS. Post-corrections: 26 tests across the two `config/lib` suites, 14 in
      the mutation-scope suite: write-ahead forward + payload shape, non-zero
      fail-closed throw, `ENOENT` fail-closed throw (corrected from an earlier
      fail-open assertion — see the **T05 correctness correction** below),
      `read`/`bash` ignored in `tool.execute.before`, `shell.env` bash Start,
      `shell.env` fail-closed on missing/empty `sessionID`/`callID` (four
      cases, corrected from an earlier fail-open no-op assertion — see the
      **T05 shell.env identity correction** below), `shell.env` fail-closed on
      adapter failure, best-effort Close never throws, tool-part `error` event
      → `ToolError`, a later `chat.params` with no valid model clears rather
      than reuses the cached session model, `title` agent model not observed.
    - TypeScript typecheck — `bunx tsc --noEmit` in `config/lib`: the new plugin
      source reports zero diagnostics (pre-existing `agent-trace` / `bash-policy`
      files carry their own long-standing relaxed-typing errors, unaffected).
      `nix build .#checks.x86_64-linux.config-lib-biome-check` /
      `.config-lib-biome-format`: PASS.
    - `nix run .#pkl-check-generated` — PASS: 142 files, inventory sha256
      `2e62b83d7568197c4ef02e518d30c11c38247e37505b84c2c109ae0277d1e2ef`
      (was 141, then `7803a2fb...` after the first correction). `nix build
      .#checks.x86_64-linux.pkl-generated` /
      `.cli-generated-input` / `.codex-hook-command`: PASS.
      `bash scripts/test-check-generated.sh` /
      `scripts/test-produce-cli-generated-input.sh`: PASS.
    - setup merge / doctor tests — `nix build .#checks.x86_64-linux.cli-tests`:
      PASS (includes the new `config_merge` ordering test, the `setup::mod`
      install assertion, and the `inspect` two-case ordering-health test).
      `.cli-clippy` / `.cli-fmt`: PASS.
    - `nix flake check` — `all checks passed!` (exit 0).
  - Context impact: additive + bounded. New generated OpenCode plugin surface
    and a new `config/lib/` plugin package; the hidden `sce hooks
    opencode-mutation-scope` command (T03/T04) is now reached in production. No
    protocol, schema, Pkl model, Quint, or SQL change; no change to the generic
    mutation-scope ingress or the OpenCode config-merge algorithm.
    `context/sce/generated-opencode-plugin-registration.md`,
    `context/sce/opencode-agent-trace-plugin-runtime.md`,
    `context/cli/opencode-mutation-scope-integration.md`,
    `context/cli/mutation-scope-hook-ingress.md`, `context/architecture.md`,
    `context/context-map.md`, `context/glossary.md`, `context/overview.md` to be
    verified during synchronization.
  - Deviations / assumptions accepted:
    - `shell.env` at `@opencode-ai/plugin@1.15.4` types `sessionID`/`callID` as
      optional. T01 D4 proved both are present on the pinned version, so the
      runtime is not expected to omit them; the plugin nonetheless throws the
      same fail-closed message when either is missing/empty rather than
      forwarding nothing, per the **T05 shell.env identity correction** below
      — a mutation-capable Bash execution must never silently proceed when
      SCE cannot establish its attribution scope, regardless of why the
      identity is unusable.
    - `spawnSync` timeout is 20s, above the adapter's 10s boundary-lock timeout,
      so legitimate contention resolves before the plugin fails closed.
    - The doctor ordering violation reuses `ProblemKind::OpenCodePluginRegistryInvalid`
      rather than adding a new kind (avoids threading a new
      `HealthProblemKind` mapping); the existing content-mismatch detection
      already catches a reordered SCE fragment, this adds an explicit,
      independently testable ordering signal.
    - New TypeScript, tests, and adapter code are comment-free per
      `feedback_no_comments_in_code`.
  - Context synchronization: synced
  - **T05 correctness correction** (follow-up on `4d6a98d4`, same task):
    - **Root cause 1 — fail-open on `ENOENT` contradicted the Done-when
      contract.** The first cut's `forwardFailClosed` only threw when
      `forwardToAdapter` returned `"failed"`; a `"cli-missing"` outcome
      (`ENOENT` spawning `sce`) returned normally, so a tracked `write`/`edit`/
      `apply_patch`/`bash` Start silently proceeded with **no** mutation scope
      whenever the `sce` CLI was not on `PATH` — exactly the "failure to
      establish Start prevents tracked mutation execution" property this task's
      Done-when clause requires, and the accepted deviation had this backwards
      by analogy by treating this Start boundary like `sce-bash-policy` /
      `sce-agent-trace`'s already-fail-open advisory hooks. Fixed:
      `forwardFailClosed` now throws whenever `forwardToAdapter` returns
      anything other than `"ok"` — `"failed"` (non-zero adapter exit, spawn
      failure, timeout) and `"cli-missing"` alike — so every transport failure
      on a tracked Start blocks the tool. `ENOENT` still logs the
      `sce CLI not found. Install it from ...` warning before throwing.
    - **Root cause 2 — stale model provenance.** The `chat.params` handler only
      ever called `observedModelBySessionId.set(...)` when the event carried a
      valid `providerID` + `api.id`; a later non-`title` `chat.params` event for
      the same session with no valid model left the previous turn's model
      cached, so a tracked `Start` on that session forwarded a stale
      `model_id` instead of `NULL` — a direct D8/AC10 violation ("unavailable
      model evidence must produce `NULL`... never guessed... never backfilled").
      Fixed: the handler now `delete`s the cached entry whenever a non-`title`
      event's model is invalid/missing, so every observation event *replaces*
      the session's current model state (set-or-clear), never merges into it.
    - **Unnecessary adapter spawns for no-op broad events.** T04's dispatch
      (`mod.rs` lines 480-483) already returns `Ok(String::new())` unconditionally
      for `SessionIdle`/`SessionError`/`SessionDeleted`/`ServerDisposed` — none of
      them resolve a checkout, touch state, or call the seam. The plugin was
      nonetheless synchronously `spawnSync`-ing the adapter process for each of
      these fire-and-forget OpenCode events for no behavioral effect. Fixed: the
      plugin no longer forwards `SessionIdle`, `SessionError`, or
      `ServerDisposed` to the adapter at all; `session.deleted` now only clears
      the plugin's own local `observedModelBySessionId` entry (pure in-process
      bookkeeping, no adapter spawn). Exact `ToolError` forwarding — which
      drives real `abandon_and_consume` recovery — is unchanged. The adapter's
      wire-format parser still accepts all four `hook_event_name` values
      unmodified (a stable contract for any future caller), so this is a
      plugin-side transport simplification, not a wire-contract change.
    - **Files changed** (vs `4d6a98d4`):
      `config/lib/mutation-scope-plugin/opencode-sce-mutation-scope-plugin.ts`
      (fail-closed on every non-`"ok"` transport outcome; `chat.params` clears
      instead of preserving a stale model; `AdapterPayload.hook_event_name`
      narrowed to the four values the plugin still emits;
      `SessionIdle`/`SessionError`/`ServerDisposed` forwarding removed;
      `session.deleted` reduced to local cache bookkeeping),
      `config/lib/mutation-scope-plugin/mutation-scope-runtime.test.ts`
      (renamed the `ENOENT` test to assert fail-closed throw instead of
      fail-open success; added
      `a later chat.params with no valid model clears the cached session model
      rather than reusing it`, replaying
      `chat.params(session A, valid model X)` →
      `chat.params(session A, model unavailable)` → tracked
      `tool.execute.before(session A)` → asserts `model: null`); context:
      `context/cli/opencode-mutation-scope-integration.md`,
      `context/overview.md`, `context/context-map.md`, `context/glossary.md`,
      `context/plans/opencode-mutation-scope-integration.md`. No Rust, Pkl,
      Quint, or SQL change — the correction is entirely in the T05 TypeScript
      transport layer; T03/T04's adapter and the generic mutation runtime are
      untouched.
    - **Verification.**
      `nix run nixpkgs#bun -- test config/lib` — 23 passed, 0 failed (was 22;
      +1 for the new model-replacement regression; the renamed `ENOENT` test
      now asserts a throw). `bunx tsc --noEmit` in `config/lib` — zero
      diagnostics against the mutation-scope-plugin source (the pre-existing
      `agent-trace`/`bash-policy` relaxed-typing errors are unaffected, as
      before). `nix run nixpkgs#biome -- check config/lib/mutation-scope-plugin`
      — clean. `nix run .#pkl-check-generated` — PASS, 142 files (inventory
      hash changed because the plugin source content changed; file count
      unchanged). `nix flake check` (x86_64-linux) — `all checks passed!`,
      including `cli-tests` (setup merge / doctor / targeted
      `opencode_mutation_scope` Rust suites, all unchanged and still green
      since no Rust code was touched), `cli-clippy`, `cli-fmt`,
      `mutation-trace-quint-connect`, `pkl-generated`, and
      `config-lib-bun-tests`.
    - **macOS `nix flake check` investigation (pre-existing, unrelated to T05).**
      The `Nix CI (macos-latest)` job on this PR (run `34543756402`, job
      `103091833801`, commit `4d6a98d4`) fails
      `checks.aarch64-darwin.mutation-trace-quint-connect` on
      `services::mutation_trace::mbt::tests::mutation_cursor_generated_traces_refine_rust_protocol`
      — the `#[quint_run(...)]`-generated-trace-refinement test that spawns 500
      random Quint traces (seed `0xe88248b8` on this run) and replays them
      through `protocol.rs`. The panic is a bare
      `Quint returned non-zero code.` with no counterexample/state-mismatch
      diagnostic printed — i.e. the `quint` subprocess itself is exiting
      non-zero on macOS, not producing a semantic Rust/Quint disagreement.
      Confirmed pre-existing and unrelated to T01-T05: the identical test fails
      the same way (different random seed `0x63518d39`, same
      `Quint returned non-zero code.` panic, no diagnostic) on the
      `Nix CI (macos-latest)` job for commit `cc2fe862` — this plan's own
      pre-T01 baseline, before any OpenCode mutation-scope work began. Every
      other macOS check on this PR's run passes
      (`config-lib-bun-tests`/`biome-check`/`biome-format`, `pkl-generated`,
      `codex-hook-command`, `cli-clippy`, `cli-fmt`, `cli-generated-input`), and
      `checks.x86_64-linux.mutation-trace-quint-connect` passes locally and in
      the `Nix CI (ubuntu-latest)` job on the same commit — so this is an
      aarch64-darwin-specific `quint`-runtime flake/incompatibility in the CI
      environment, not a code defect introduced by this plan. No code change
      was made for it; it is documented here rather than "fixed" because it
      predates and is out of scope for T05.
  - **T05 shell.env identity correction** (follow-up, same task):
    - **Root cause — `shell.env` fail-open on missing/empty identity
      contradicted the Done-when contract.** `@opencode-ai/plugin@1.15.4`
      types `shell.env`'s `sessionID`/`callID` as optional. The prior cut
      treated a missing or empty value the same as the previously-accepted
      deviation (a conservative "forward nothing" no-op), so a tracked Bash
      execution could reach OpenCode's shell spawn with **no** mutation scope
      whenever SCE could not construct `ShellEnv`'s identity — the same
      "failure to establish Start prevents tracked mutation execution"
      property T05's Done-when clause already requires, and which the
      `4d6a98d4` correction had already fixed for adapter-side transport
      failures (`"failed"` / `"cli-missing"`). The gap was upstream of the
      adapter: an unusable identity never reached `forwardFailClosed` at all.
      Fixed: `shell.env` now throws `FAIL_CLOSED_MESSAGE` — the same
      user-facing fail-closed message every other Start failure uses —
      whenever `sessionID` or `callID` is missing, non-string, or empty,
      before any attempt to spawn the adapter. Valid identity is unaffected:
      `shell.env` still forwards `ShellEnv` exactly as before and the Rust
      adapter's Start/spawn semantics, timeout, and `ENOENT` handling are
      untouched.
    - **Files changed:**
      `config/lib/mutation-scope-plugin/opencode-sce-mutation-scope-plugin.ts`
      (`shell.env`'s identity guard throws `FAIL_CLOSED_MESSAGE` instead of
      returning), `config/lib/mutation-scope-plugin/mutation-scope-runtime.test.ts`
      (replaced the single "shell.env without call identity forwards nothing"
      test with four fail-closed regressions — missing `sessionID`, empty
      `sessionID`, missing `callID`, empty `callID` — each asserting the
      `FAIL_CLOSED_MESSAGE` throw and zero adapter spawns; the existing
      "anchors the bash Start to shell.env" test continues to cover the valid-
      identity forwarding path), this plan document (deviation bullet updated
      to match — the plan and implementation now agree that no `shell.env`
      identity gap fails open). No Rust, Pkl, Quint, or SQL change — the
      pinned OpenCode v1.15.4 runtime is still expected to always supply both
      values (T01 D4); this closes the theoretical gap for when it doesn't,
      it does not change the supported-runtime assumption.
    - **Verification.** `nix run nixpkgs#bun -- test config/lib/mutation-scope-plugin`
      — 14 passed, 0 failed (was 11; net +3 for the fail-closed split of the
      former single no-op test into four regressions). `nix run nixpkgs#bun --
      test config/lib` — 26 passed, 0 failed across both suites (was 23).
      `bunx tsc --noEmit` in `config/lib` — zero diagnostics against the
      mutation-scope-plugin source (pre-existing `bash-policy-plugin` relaxed-
      typing errors unaffected). `nix run nixpkgs#biome -- check
      config/lib/mutation-scope-plugin` — clean. `nix run .#pkl-check-generated`
      — PASS, 142 files (inventory hash changed because the plugin source
      content changed; file count unchanged). `nix build
      .#checks.x86_64-linux.config-lib-bun-tests` — PASS. `nix flake check`
      (x86_64-linux) — `all checks passed!`, including `cli-tests`,
      `cli-clippy`, `cli-fmt`, `mutation-trace-quint-connect`, `pkl-generated`,
      `codex-hook-command`, `cli-generated-input`, `config-lib-biome-check`,
      `config-lib-biome-format`, `npm-bun-tests`, `npm-biome-check`,
      `npm-biome-format`, and `workflow-actionlint` (all unchanged and still
      green since no Rust/Pkl code was touched). The pre-existing
      aarch64-darwin `mutation-trace-quint-connect` flake documented above is
      unrelated and unaffected.

- [x] T06: `Add end-to-end OpenCode mutation attribution regressions` (status:done)
  - Task ID: T06
  - Scope: In — production-path Git/DB tests in `cli/src/services/hooks/mod.rs`
    from OpenCode lifecycle through the generic ingress, snapshot coordination,
    scope provenance, line attribution, `mutation_ai_patch`, and Agent Trace
    output. Cover each tracked tool, rejected/failed execution, concurrent
    OpenCode calls, child task sessions, OpenCode+Claude/Codex overlap,
    model-present/model-missing provenance, and untracked zero-footprint
    behavior. Out — new production semantics not already established by T02–T05.
  - Dependencies: T05
  - Done when: real repository mutations demonstrate that only confirmed
    exclusive OpenCode evidence reaches AI mutation lineage; ambiguous/
    unconfirmed intervals remain non-AI; provenance resolves to the correct
    session/model; cross-harness and concurrency cases preserve the formal
    semantics.
  - Verify: `cargo test -p sce hooks::` targeted Git/DB regression suite;
    Agent Trace schema validation for resulting traces; `nix flake check`.
  - Completed: 2026-09-11
  - Files changed (vs baseline `dca587ad`):
    - `cli/src/services/hooks/mod.rs` (extends the existing
      `services::hooks::tests::mutation_provenance_e2e` module — the same
      real-Git/real-DB harness already proving Claude/Codex production-path
      attribution — with 8 new OpenCode regressions and 3 small `ProvenanceE2eRepo`
      additions: `opencode_mutation_scope` import, a `mutation_events()` reader,
      and OpenCode wire-payload builders `opencode_before`/`opencode_shell_env`/
      `opencode_after`/`opencode_tool_error` + a `drive_opencode` helper)
  - Result: Eight new `hooks::tests::mutation_provenance_e2e` tests drive the
    real OpenCode adapter (`opencode_mutation_scope::run_opencode_mutation_scope_from_payload_at_state_root`)
    through real Git commits, the real post-commit intersection/Agent-Trace
    flow, and a real repository-scoped Agent Trace DB — the same production
    path already proven for Claude/Codex in this module, now proven for
    OpenCode: (1) `opencode_bash_mutation_persists_model_and_session_in_agent_trace`
    — bash Start on `ShellEnv`, Close on `ToolExecuteAfter`, full Agent Trace
    provenance (`oc_<sessionID>`, observed model); (2)
    `opencode_apply_patch_mutation_with_missing_model_persists_no_model_in_agent_trace`
    — `apply_patch` tool with no `model` field, asserting `contributor.model_id`
    is absent (never fabricated) while session provenance still resolves; (3)
    `opencode_write_mutation_persists_model_while_task_delegation_stays_zero_footprint`
    — a `task` Before/After pair creates zero `mutation_trace_scopes` rows, then
    a `write` call in the same session creates exactly one; (4)
    `opencode_unknown_tool_events_create_no_scope_or_mutation_state` — MCP/
    unknown/future tool names create zero scope or event rows across
    Before/After/Error; (5)
    `opencode_child_task_session_gets_its_own_independent_scope_and_provenance`
    — a parent's `task` delegation is neutral while a child (subagent) session's
    tracked `write` gets its own scope and its own `oc_<child sessionID>`
    provenance; (6)
    `opencode_concurrent_reject_and_confirm_keeps_only_the_confirmed_mutation_ai`
    — two concurrent OpenCode calls in one session (distinct `call_id`), one
    rejected (`ToolError`) and one surviving: the rejected scope's own mutation
    and the survivor's *pre-recovery* mutation (written before the abandoning
    scope's ambiguity-consuming flush, genuinely indistinguishable from the
    rejected scope's own filesystem effect) both correctly stay out of
    `mutation_ai_patch`, while the survivor's *later* mutation — made after the
    ambiguous interval was consumed and then confirmed by its own Close — is
    attributed AI; this reproduces and confirms the exact `Start(A) Start(B)
    mutate mutate Abandon(A) Close(B)` scenario from the T04 soundness
    correction's own written analysis, now proven end-to-end through
    `mutation_ai_patch`, not just at the adapter/runtime-seam level; (7)
    `opencode_and_codex_unconfirmed_overlap_stays_ineligible_until_codex_confirms`
    — a live unconfirmed Codex scope suppresses a confirming OpenCode Close to
    `ineligible_unscoped`; once Codex also confirms, a fresh solo OpenCode Close
    is `ai_exclusive` (asserted directly against `mutation_trace_events.attribution_kind`,
    since a coarse per-boundary event cannot itself carry file-level
    discrimination); (8) `opencode_and_claude_overlap_produces_ai_contended` — a
    live Claude scope (not confirmation-required) alongside a confirming
    OpenCode Close produces `ai_contended`, not suppression, per D3. Every test
    uses only the already-shipped T01–T05 adapter/plugin/protocol surface; no
    production semantics were introduced or changed.
  - Verify outcomes:
    - `cargo test -p sce hooks::` targeted Git/DB regression suite — direct
      `cargo test`/`cargo check` are bash-policy-blocked in this repo (per
      `project_sce_cargo_test_invocation`); run as the canonical
      `nix build .#checks.x86_64-linux.cli-tests`: PASS, `1500 passed; 0 failed;
      1 ignored` (includes the 8 new `mutation_provenance_e2e` tests). The
      first draft of test (6)
      exposed a genuine authoring error, not a product bug: it assumed the
      survivor's *concurrent* pre-recovery mutation would be cleanly attributed
      to it; the real runtime correctly swept that pre-recovery mutation into
      the same non-AI ambiguous interval as the rejected scope's own mutation
      (exactly per the T04 soundness-correction design), and the test was
      corrected to assert that true, more conservative behavior rather than
      the incorrect assumption.
    - Agent Trace schema validation for resulting traces — every
      `run_post_commit()`-driving test (tests 1, 2, 3, 5 above) passes through
      the same `validate_agent_trace_value` schema gate the production
      post-commit flow uses before persisting; no schema violation.
    - `nix flake check` — `all checks passed!` (x86_64-linux), including
      `cli-tests`, `cli-clippy`, `cli-fmt`, `mutation-trace-quint-connect`,
      `pkl-generated`, `cli-generated-input`, `codex-hook-command`,
      `config-lib-bun-tests`/`biome-check`/`biome-format`,
      `npm-bun-tests`/`biome-check`/`biome-format`, `workflow-actionlint`,
      `native-portability-audit`, `flatpak-static-validation`,
      `cargo-sources-parity`, `flatpak-manifest-parity`.
    - `git diff --check` — clean (1 file, +536, additions only).
  - Context impact: local. New test coverage only, in the same existing
    `hooks::tests::mutation_provenance_e2e` module and following its established
    pattern; no new adapter, plugin, protocol, Pkl, Quint, or SQL surface, and no
    change to any production code path. No context file names a specific test
    inventory for this module, so no context edit is expected beyond noting (if
    warranted during synchronization) that OpenCode now has the same
    production-path Agent Trace regression coverage as Claude and Codex.
  - Deviations / assumptions accepted:
    - The `apply_patch` case here is a Rust-adapter-level regression (`sce hooks
      opencode-mutation-scope` driven directly with an `apply_patch` tool name),
      not a live OpenCode CLI session — the Rust adapter has no knowledge of
      OpenCode's `gpt-`-model patch gate, so this is unaffected by the T01
      credential gap and it fully exercises the adapter's real classification,
      lifecycle, provenance, and Agent Trace code paths for `apply_patch`. It
      does not itself close the outstanding T01 credential gap for a live,
      real-OpenCode-CLI `apply_patch` probe/fixture, which remains a distinct,
      separately-tracked item for `/validate` (a credential gap, not a
      soundness gap, per T01/T04's existing notes).
    - Cross-harness overlap tests (7, 8) assert directly against
      `mutation_trace_events.attribution_kind` rather than walking all the way
      to Agent Trace JSON, because the line-level `mutation_ai_patch` consumer
      necessarily collapses `ai_contended` and `ineligible_unscoped` into the
      same non-AI line provenance (`MutationNonAi`) — the distinct protocol
      outcome AC5/AC6 describe is only observable at the `mutation_trace_events`
      row itself. The single-actor success/model tests (1, 2, 3, 5) and the
      concurrent-reject test (6) already prove the full stack down to Agent
      Trace/`mutation_ai_patch`.
    - `edit` and one additional `write`/`bash`/`apply_patch` combination are
      exercised across tests 1, 2, 3, 6 rather than one dedicated test per tool
      name; per-tool-name classification exhaustiveness is already covered by
      `cargo test -p sce opencode_mutation_scope` (T03/T04), which is AC2's
      named validation command, not this task's.
    - New test code is comment-free per `feedback_no_comments_in_code`.
  - Context synchronization: synced

## Open questions

- None. The harness lifecycle facts that could change the implementation are
  enumerated as D1–D11 in the Design section and owned by T01; a contradictory
  T01 result is a re-planning gate, not licence to weaken attribution. Per the
  **OpenCode/plugin version policy** in the Design section, the
  `@opencode-ai/plugin` version is fixed to the value inherited from PR #275 and
  one exact OpenCode CLI version is selected and recorded before the first
  load-bearing probe; the open Dependabot bump does not change that — changing
  either version after probing begins is a deliberate replan-and-reprobe step,
  not an open question.
