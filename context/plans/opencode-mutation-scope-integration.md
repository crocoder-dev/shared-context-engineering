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

- [ ] T02: `Generalize boundary-confirmed attribution to OpenCode` (status:todo)
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
  - Context synchronization: pending

- [ ] T03: `Add OpenCode adapter identity and classification` (status:todo)
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
  - Context synchronization: pending

- [ ] T04: `Implement OpenCode scope lifecycle and recovery` (status:todo)
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
  - Context synchronization: pending

- [ ] T05: `Wire the OpenCode mutation-scope plugin` (status:todo)
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
  - Context synchronization: pending

- [ ] T06: `Add end-to-end OpenCode mutation attribution regressions` (status:todo)
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
  - Context synchronization: pending

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
