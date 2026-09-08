# Plan: codex-mutation-scope-integration

## Change summary

Add the **second** concrete mutation-scope producer for SCE: a Codex adapter
that translates raw Codex hook lifecycle events into the normalized
mutation-scope contract already implemented by `sce hooks mutation-scope`
(`cli/src/services/hooks/mutation_scope.rs`, documented in
`context/cli/mutation-scope-hook-ingress.md`) and proven end-to-end by the
Claude adapter (`context/cli/claude-mutation-scope-integration.md`, PR #263).

Data flow:

```text
Codex raw hook event
  -> sce hooks codex-mutation-scope   (new, hidden — see T02 for the command decision)
  -> normalize Codex lifecycle + identity, classify tool
  -> hooks::mutation_scope generic ingress (in-process pub(crate) seam)
  -> coordinate() / abandon_scope()
  -> mutation cursor
```

The fundamental mapping is **one independently mutation-capable *tracked* Codex
tool execution = one SCE mutation `ScopeId`**. "Tracked" is load-bearing: the
Codex adapter v1 gives a mutation scope only to tool classes whose terminal
lifecycle it can safely observe (`Bash`, `apply_patch`). A Codex session, turn,
or delegated agent is never a scope; `session_id` / `turn_id` / any
delegated-agent identity are only inputs that distinguish tool executions.

**MCP calls and unknown/future Codex tools are deliberately *outside* Codex
mutation-scope attribution coverage in v1** (re-planning direction B, chosen
2026-09-08 — see D23 and Open questions). They execute normally; they may mutate;
the adapter creates no mutation scope, no bookkeeping, and no `Start` for them.
This is an attribution-coverage boundary, not a claim that MCP is read-only and
not a lifecycle workaround — the T01 evidence that MCP *cannot* be safely modeled
as a scope on Codex 0.153.4 (D10a Case C) is exactly why it is excluded.

This extends the mutation-scope stack. The generic ingress, the runtime
(`coordinate()` / `abandon_scope()`), and the `mutation-trace` protocol already
exist and are **unchanged in contract**. `ActorKind::Codex` and the
`"actor_kind":"codex"` wire value are already accepted by
`parse_mutation_scope_payload` today — the Codex adapter is a new caller of an
existing seam, not a protocol change. This change adds the Codex harness adapter
layer the ingress explicitly deferred, its checkout-local bookkeeping, its
hidden command/routing, and its generated `.codex/hooks.json` registrations
through the existing shared Codex hook-config ownership/merge/doctor system.

**The Claude adapter is a structural reference, not a lifecycle specification
for Codex.** Codex's hook surface differs materially from Claude's: it has no
`PostToolUseFailure`, no `PermissionDenied`, no `StopFailure`, and no
`WorktreeRemove` event (the eleven event names `codex_hook_config.rs` accepts
are `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`,
`PostCompact`, `SessionStart`, `SessionEnd`, `UserPromptSubmit`,
`SubagentStart`, `SubagentStop`, `Stop`). Every Codex lifecycle mapping in this
plan is therefore **gated on T01 evidence** rather than copied from Claude. T01
freezes the real Codex hook contract for the Codex version SCE chooses to
support before any mapping is designed.

The existing `sce hooks codex` integration (conversation tracing via
`UserPromptSubmit`/`Stop`, `PreToolUse(Bash)` policy, `PostToolUse(apply_patch)`
`diff_traces` evidence, `.codex/hooks.json` ownership/merge, doctor trust
diagnosis) is preserved untouched and additive. Mutation-scope events continue
to write only through the mutation runtime (`mutation_trace_*` tables); the
`apply_patch -> diff_traces -> post-commit intersection` evidence pipeline is a
complementary system and is not folded into mutation-scope storage.

No mutation protocol, Quint model, mutation-trace SQL migration,
mutation-attribution algorithm, or Agent Trace schema change is in scope. If T01
reveals Codex fundamentally cannot be represented by the current mutation-scope
contract, T01 stops and records the contradiction in this plan's Open questions
rather than modifying the protocol; any protocol change becomes a separate,
explicitly justified PR.

**T01 outcome (recorded 2026-09-08):** the built-in `Bash` / `apply_patch`
surface *is* representable by the current contract; the MCP mutation-scope
lifecycle *is not* (D10a Case C). Re-planning chose **direction B** — MCP and
unknown tools remain usable but are outside the Codex adapter's mutation-scope
coverage. This requires **no** protocol / Quint / mutation-trace SQL / Agent
Trace schema change (D23); it narrows the Codex adapter's coverage boundary
only. T01 is done; T02 is unblocked.

## Stack and base

- **Predecessor:** PR #263 `claude-mutation-scope-integration`
  (head branch `claude-mutation-scope-integration`, itself based on
  `mutation-scope-ingress` / #261).
- **This branch:** `codex-mutation-scope-integration`, already created at #263's
  head (no commits ahead of `claude-mutation-scope-integration` at plan time).
- **Base for the PR while the stack is unmerged:**
  `claude-mutation-scope-integration` (#263 head), **not** `main`.
- **Final branch comparison** is against `claude-mutation-scope-integration`,
  not `main`, for as long as #263 remains open. If the stack has changed by
  execution time (e.g. #263 merged to `main`, or a new predecessor inserted),
  re-check `gh pr list` and rebase onto the actual current predecessor, then
  update this section and every `origin/claude-mutation-scope-integration`
  reference below.

## Design

These are the design decisions the task stack and acceptance criteria reference
by number. Decisions whose correctness depends on a Codex lifecycle signal
actually firing (or on a payload field actually being present and stable) are
marked **T01-GATED** and carry no committed mapping until T01 records a
disposition (`PROVEN`, `DOCUMENTED — NON-LOAD-BEARING`, `ASSUMPTION — PROBE`,
or `UNSUPPORTED`). T01 writes each disposition back into this Design section.

### D1 — Scope = one independently mutation-capable *tracked* Codex tool execution

**"Capable of mutating" and "covered by SCE mutation-scope attribution" are two
different things for the Codex adapter.** A Codex tool execution can be
independently capable of mutating the checkout and still not be represented as an
SCE mutation scope, if the adapter cannot safely observe that tool class's
terminal lifecycle.

For the Codex adapter v1:

```text
one independently mutation-capable SUPPORTED / TRACKED Codex tool execution
=
one SCE mutation ScopeId
```

Concretely:

```text
Bash          -> scope   (TrackedMutation)
apply_patch   -> scope   (TrackedMutation)
MCP (mcp__*)  -> no scope (Untracked — still executes, may mutate)
unknown tool  -> no scope (Untracked — still executes, may mutate)
```

A scope is exactly one such tracked execution attempt. Not a session, not a
turn, not a delegated agent. Sequential tracked tool calls are sequential
scopes. Whether two Codex mutation-capable executions can genuinely overlap
(and therefore whether Codex alone can produce `AiContended`) is answered by
T01 below. `AiContended` can still arise from a Codex *tracked* scope
overlapping another harness's scope on the same worktree regardless; see D14.

This is a **Codex adapter coverage policy**. It does **not** broaden or narrow
the generic mutation-scope runtime contract, which already models exclusivity
among the tracked scopes it is told about, not exhaustive filesystem authorship
(D14, D23).

**T01 disposition (codex-cli 0.153.4): scope-split by tool type.**
- **Built-in `Bash` / `apply_patch`: ASSUMPTION — PROBE (leaning serial).**
  Codex executed every built-in mutation-capable tool strictly serially in all
  11 built-in probes (`PreToolUse → PostToolUse → PreToolUse → …`, never
  interleaved), including across the parent/subagent boundary and when asked to
  parallelise.
- **MCP: PROVEN — parallel-capable, live-reproduced.** The T01 MCP extension
  (probes 16/17) shows two mutation-capable MCP tool executions running
  **genuinely concurrently** on 0.153.4 — `PreToolUse(A)` and `PreToolUse(B)`
  ~1 ms apart, both before either `PostToolUse`, both scopes live for ~8 s,
  confirmed by the MCP server's own execution log. Enabled by the
  upstream-supported `[mcp_servers.<name>] supports_parallel_tool_calls = true`
  config key **or** the tool's own `annotations.readOnlyHint`
  (`McpHandler::supports_parallel_tool_calls()`,
  `codex-rs/core/src/tools/handlers/mcp.rs:128-139` at `rust-v0.153.4`).
- **Parallel MCP execution is a real operational fact and remains supported** —
  the adapter never blocks it. But because MCP tools are **Untracked** in Codex
  adapter v1 (D2, D23), two overlapping MCP executions produce **no MCP mutation
  scopes**, and therefore **no MCP-derived `AiContended`**. A tracked `Bash` /
  `apply_patch` scope overlapping an MCP execution may still yield
  `AiExclusive(Bash)` from the runtime — this means "exactly one *tracked*
  mutation scope was live", not "that scope authored every filesystem mutation
  in the interval" (D14).
- **Codex-alone `AiContended` from built-in tools only:** built-in
  `Bash` / `apply_patch` executed strictly serially across all 11 built-in
  probes, so built-in Codex-alone overlap was never observed. Cross-harness
  `AiContended` (a Codex tracked scope overlapping a Claude/OpenCode/Pi scope)
  remains reachable regardless; see D14.
The adapter still never collapses two executions into one `ScopeId`. Evidence:
`fixtures/probe01…`, `probe02…`, `probe08-subagent-delegation.*`,
`probe16-mcp-parallel-server-optin.*`, `probe17-mcp-parallel-readonly-hint.*` +
`fixtures/NOTES.md` ("T01 MCP lifecycle probe extension").

**v1 resolution (D23):** MCP is not modeled as a mutation scope, so D1's
"one execution = one `ScopeId`" rule simply does not range over MCP or unknown
tools. No contradiction with the parallel-MCP evidence remains, because the
adapter creates nothing for those executions.

### D2 — Codex tool classification — three semantic classes

The adapter classifies the raw Codex `tool_name` in Rust by **semantic intent**,
not by mutability alone. Terminology equivalent to:

```rust
enum ToolClassification {
    TrackedMutation,
    Delegation,
    Untracked,
}
```

(exact Rust naming may still be adjusted in T02).

#### TrackedMutation

```text
tool executes
+ adapter can safely observe its terminal lifecycle
+ adapter creates a mutation scope
```

v1 members: `Bash`, `apply_patch`. Each independently mutation-capable tracked
execution establishes exactly one `ScopeId` (D1/D4). Fail-closed on `PreToolUse`
(D8), terminal boundary on the proven terminal hook (D9/D10).

#### Delegation

```text
the delegation tool itself does not receive a mutation scope;
the delegated agent's own TrackedMutation tools do (carrying its agent_id).
```

v1 members: `collaborationspawn_agent`, `collaborationwait_agent` (a
`collaboration` namespace prefix, no separator). `PreToolUse` for a delegation
tool returns the neutral response, no scope.

#### Untracked

```text
tool executes normally
+ may mutate the checkout
+ adapter creates NO mutation scope
+ mutations are OUTSIDE SCE mutation-scope coverage (D23)
```

v1 members: `mcp__*` (any `mcp__<server>__<tool>`), and **any unknown /
unrecognised `tool_name`**.

`Untracked` does **not** mean "read-only". It means exactly:

```text
allowed to execute  +  does not participate in SCE mutation-scope accounting
```

MCP tools may mutate. Unknown tools may mutate. The adapter neither asserts they
are read-only nor guarantees their mutations are detected immediately — it simply
does not attribute them (D23). The classifier must **not** carry the earlier
language saying MCP/unknown are "mutation-capable therefore `Start`" — that
classification is precisely what produced the D10a Case C contradiction.

For `PreToolUse(mcp__…)` (and any unknown tool) the adapter conceptually does:

```text
classify as Untracked
  -> return Codex-neutral continue response
  -> no ScopeId, no EventId
  -> no adapter attempt, no Start, no recovery_pending
  -> no bookkeeping entry of any kind
```

Therefore successful, failed, interrupted, or parallel MCP/unknown executions
require no `Close`, `Abandon`, or `Flush` from the adapter — there is nothing to
retire. Existing normal Codex/SCE hooks unrelated to mutation-scope
(conversation tracing, `diff_traces`, policy) are unchanged and still run for
MCP calls.

**T01 disposition (codex-cli 0.153.4): PROVEN for the `codex exec` surface.**
- **TrackedMutation:** `apply_patch`, `Bash` (the shell tool — it also performs
  reads / listing / search via shell commands, so it is always treated
  mutation-capable; a read-only shell command merely creates a harmless scope).
- **Delegation:** `collaborationspawn_agent`, `collaborationwait_agent`. The
  delegated agent's own tool calls carry its `agent_id` and establish their own
  (tracked) scopes.
- **Untracked:** `mcp__<server>__<tool>` (T01 MCP extension, probes 12–17:
  `mcp__probe__mutate_success`, `mcp__probe_par__slow_mutate`), and every unknown
  `tool_name`. MCP `tool_use_id` is `exec-<uuid>` — the same shape as
  shell / `apply_patch` — but the adapter never keys any state on it. Upstream:
  `join_tool_name` / `MCP_TOOL_NAME_DELIMITER` / `ensure_mcp_prefix`,
  `codex-rs/core/src/tools/handlers/mcp.rs` at `rust-v0.153.4`.
- **Dedicated read-only tool names:** none — this Codex surface routes reads
  through `Bash`. There is no separate "read-only, never a scope" class in v1;
  the only never-a-scope classes are `Delegation` and `Untracked`.

**Why MCP and unknown are `Untracked` (not `TrackedMutation`):** T01 proved that
if MCP were modeled as a scope, Codex 0.153.4's lifecycle makes it unsafe — a
mutation-capable MCP tool can mutate a git-visible file then return
`is_error:true` with **no terminal hook** (probe 13), a successor `PreToolUse`
can follow with **no cleanup signal between them** (probe 14), and same-lane MCP
executions **genuinely overlap** (probes 16/17), so a successor cannot prove a
predecessor stale. That is D10a Case C. Rather than ship an unsafe scope
lifecycle, v1 does not create scopes for MCP at all (D10a, D23). Unknown tools
get the same compatibility-oriented default so a future Codex tool never becomes
unusable merely because SCE does not yet know its lifecycle; support is additive:

```text
new_tool: Untracked  --(lifecycle researched / proven)-->  TrackedMutation
```

without any protocol change.
Evidence: `fixtures/probe05-tool-vocabulary.*`, `probe08-subagent-delegation.*`,
`fixtures/probe12-mcp-mutate-success.*`, `probe13-mcp-mutate-then-error.*`,
`probe14-mcp-failed-then-successor.*`, `probe16-mcp-parallel-server-optin.*`,
`probe17-mcp-parallel-readonly-hint.*`, `fixtures/NOTES.md`.

### D3 — Codex execution identity — T01-GATED

The adapter needs the **smallest stable identity for one Codex tool execution**.
Candidate inputs, to be confirmed by T01 evidence only:

- `session_id` (present on Codex events today, stored `cx_`-prefixed elsewhere);
- `tool_use_id` (used today by the `apply_patch` `diff_traces` path to derive
  synthetic line identities — so present on at least `PostToolUse`);
- `turn_id` (present on Codex events today);
- a delegated-agent identifier **only if T01 proves Codex exposes one** — do not
  invent an `agent_id` abstraction if Codex has no equivalent.

T01 must establish: (a) which of these is present on `PreToolUse`, (b) which is
present on `PostToolUse` (and any terminal/failure event), (c) whether the same
execution identity appears in both the pre and post events for one tool call,
(d) whether a raw Codex tool identifier can recur after that execution is
terminal. T02 then freezes the execution key.

**T01 disposition (codex-cli 0.153.4): PROVEN.** Execution key =
`(session_id, agent_id?, tool_use_id)` — **for `TrackedMutation` tools only**.
`Untracked` (MCP, unknown) and `Delegation` tools get no execution key because
the adapter records no attempt for them (D2/D23).
- `tool_use_id` is present on **both** `PreToolUse` and `PostToolUse` and is
  identical for one call (`exec-<uuid>` for shell / `apply_patch`,
  `call_<id>` for the delegation tools). Not observed to recur (UUID-based); the
  D4 checkout-local `attempt_seq` guard is kept regardless.
- `session_id` is stable for a whole session including subagents; `turn_id`
  differs per turn and per subagent.
- `agent_id` (a UUID) is present **only on subagent tool/lifecycle events** and
  distinguishes a delegated agent from the main thread (absent = main thread).
  Codex **does** expose a delegated-agent identity — use `agent_id`; do not
  invent one where it is absent. `agent_type` ("default") is diagnostic only.
Evidence: `fixtures/probe01…`, `probe08-subagent-delegation.*`, and the
`pre-tool-use` / `post-tool-use` / `subagent-stop` generated schemas at
`openai/codex` `rust-v0.153.4` (`agent_id`/`agent_type` present but not in
`required`).

### D4 — ScopeId / EventId derivation — depends on D3

A raw Codex tool identifier that can recur after terminal execution forces a
checkout-local monotonic attempt sequence, exactly as in the Claude adapter: the
adapter keeps `next_attempt_seq` in its bookkeeping store and each new attempt
draws a fresh `attempt_seq`. A terminal SCE `ScopeId` is **never reused**.

`ScopeId` is a length-prefixed, hash-free encoding (no crypto dependency) in a
Codex-specific versioned namespace. The conceptual shape, **not frozen until D3
is resolved by T01**:

```text
cx-tool-v1|n=<attempt_seq>|<length-prefixed identity fields from D3>
```

`EventId`s derive deterministically from the `ScopeId`: `<scope-id>|start` and
`<scope-id>|close`. Replaying the same hook event for one live attempt yields
the same `ScopeId` and `EventId` (the runtime's replay/idempotency key). After
an attempt is terminal, a later hook event for the same raw Codex tool
identifier draws a new `attempt_seq` and a new `ScopeId`.

### D5 — Checkout-local adapter bookkeeping — reasoned, not assumed

Codex hooks are invoked as **independent OS processes** — the generated
`.codex/hooks.json` command is `... exec bash "$root/.codex/hooks/
run-sce-or-show-install-guidance.sh" sce hooks codex-mutation-scope`, a fresh
process per hook event. A `PreToolUse` process and the later `PostToolUse`
process for the same tool call share no in-memory state. Therefore the adapter
**requires** durable cross-process bookkeeping to know which Codex-created
scopes may still need a terminal action — the same conclusion the Claude adapter
reached, for the same reason. T02/T03 must confirm this holds for the supported
Codex version (Codex does not, for example, run all hooks for one turn in a
single persistent process); if it does not, T03 records why the store shape
changes.

The store lives at `<git-dir>/sce/codex-mutation-scope-state.json` with lock
`<git-dir>/sce/codex-mutation-scope-state.lock` (`<git-dir>` via
`checkout::resolve_git_dir(cwd)` — worktree-specific for linked worktrees). It
holds a versioned `{version, next_attempt_seq, recovery_pending, attempts[]}`,
each attempt carrying `attempt_seq`, `scope_id`, the D3 identity fields,
`tool_name`, and `phase` (`pending_start | active`). This is **adapter
bookkeeping, never attribution evidence**: not exported, not synced, not part of
Agent Trace, not authoritative for attribution. A malformed or wrong-version
file is rejected, never fabricated.

### D6 — Durable persistence and a separate state lock

State writes follow the `checkout::persist_checkout_id_inner` durability pattern
(lock, temp file, `sync_data`, atomic rename, best-effort parent-dir `sync_all`
on Unix). The adapter-state lock protects bookkeeping only and is **never held
across a `hooks::mutation_scope` seam invocation**, so no
`adapter lock -> WorktreeLock` order can form. The adapter may call
`checkout::resolve_git_dir` but not `read_checkout_id` /
`get_or_create_checkout_id`, and never constructs a `WorktreeId`.

### D7 — Write-ahead Start ordering

Unless T01 proves a different safe ordering from Codex's hook semantics, the
adapter preserves the write-ahead property for a new tracked mutation-capable
tool:

```text
parse event -> resolve raw cwd -> resolve git_dir (bookkeeping only)
  -> acquire state lock -> allocate attempt_seq -> persist phase=pending_start -> release lock
  -> invoke generic ingress seam with { "operation":"start", "scope_id":<derived>,
       "event_id":<scope>|start, "actor_kind":"codex" }, passing the raw cwd as repository_root
  -> reacquire state lock -> phase pending_start -> active -> release lock
  -> return (Codex-native "continue" — see D8)
```

A `TrackedMutation` tool must not execute after SCE has failed to establish its
mutation scope. (`Untracked` and `Delegation` tools never reach this path — no
`Start` is attempted for them; D2/D8.)

### D8 — Codex-native fail-closed PreToolUse — T01-GATED

D8 fail-closed behaviour applies **only** when SCE is trying to establish a
**`TrackedMutation`** scope:

```text
TrackedMutation PreToolUse
  -> failure to establish a durable Start  -> deny (block the tool)

Untracked PreToolUse (mcp__*, unknown)
  -> neutral continue
  -> never attempts Start, so there is nothing to fail closed on
  -> never denied merely because it is untracked

Delegation PreToolUse
  -> neutral continue, no scope
```

A `TrackedMutation` Codex `PreToolUse` is **fail-closed**: any failure to durably
establish the scope (state-allocation failure, seam `Start` failure,
unresolvable `cwd`, recovery-barrier denial) must **block the tool**, not let it
run un-scoped. Do **not** deny MCP. Do **not** deny an unknown tool. The adapter
also never emits an explicit **allow** for an `Untracked` tool — it returns the
normal neutral / no-op hook result and lets Codex's own permission handling
proceed unchanged.

The exact Codex-native denial response and exit semantics are **T01-GATED**. Do
**not** assume Claude's `{"hookSpecificOutput":{...,"permissionDecision":
"deny",...}}` shape. Candidate shapes to disambiguate in T01 for the supported
version: the Claude-identical `hookSpecificOutput.permissionDecision: "deny"`
(the shape the existing `sce hooks codex` `PreToolUse(Bash)` policy arm returns
today, per `openai/codex` issue #28437), or a `{"decision":"block","reason":
"..."}` shape seen in some Codex versions, or a non-zero exit code. T01 records
which one blocks the tool for the supported version; T04 emits exactly that.

The detailed error is logged via `Logger::warn`
(`sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed`); the model-visible
denial reason never carries it. The adapter never emits an explicit **allow**
that bypasses Codex's own permission system — success returns Codex's neutral
"continue" (empty stdout, or whatever T01 shows is the no-op response), and only
failure returns the block.

An `Untracked` or `Delegation` `PreToolUse` returns the neutral response, no
scope, no bookkeeping.

**T01 disposition (codex-cli 0.153.4): PROVEN.** **Both** denial shapes block
the tool on 0.153.4 and both appear in the generated
`pre-tool-use.command.output.schema.json` at `openai/codex` `rust-v0.153.4`:
top-level `{"decision":"block","reason":"…"}` (`decision` enum `approve|block`),
**and**
`{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"…"}}`
(`permissionDecision` enum `allow|deny|ask`). T04 should emit the
`hookSpecificOutput` shape, matching the existing `sce hooks codex`
`PreToolUse(Bash)` policy arm. A blocked tool fires `PreToolUse` only —
**no `PostToolUse`** — so a fail-closed denial leaves no scope needing a
terminal action. This holds for a **blocked MCP call** too (probe 15:
`permissionDecision:"deny"` on `mcp__probe__mutate_success` → `PreToolUse` only,
no `PostToolUse`, `tools/call` never issued, nothing written). **v1 note:**
because re-planning chose direction B (D23), the adapter does **not** deny MCP —
probe 15 stays relevant only as evidence that a blocked `PreToolUse` (from any
source) strands no scope, and as the shape a rejected direction (A) would have
used. Evidence:
`fixtures/probe03-pre-tool-use-hook-decision-block.*`,
`fixtures/probe04-pre-tool-use-hook-hookspecificoutput-deny.*`,
`fixtures/probe15-mcp-blocked-call.*`.

### D9 — Terminal boundary on success — T01-GATED

Terminal boundary rules apply **only to `TrackedMutation` tools** (`Bash`,
`apply_patch`). MCP and unknown tools have no scope, so they have no terminal
boundary and no `Close`.

For a successful `TrackedMutation` tool with an `active` tracked attempt, the
terminal Codex hook maps to
`{ "operation":"close", "scope_id":<same>, "event_id":<scope>|close,
"actor_kind":"codex" }`. T01 must confirm **which** hook is the reliable
terminal signal for a successful mutation-capable execution (`PostToolUse` for
that tool, carrying the D3 identity that ties it to the `PreToolUse`). The
attempt is removed from adapter state only after durable `Close` success;
duplicate delivery after cleanup is a safe no-op.

**T01 disposition (codex-cli 0.153.4): PROVEN.** `PostToolUse` is the reliable
terminal signal for a **successful `TrackedMutation`** tool, carrying the same
`tool_use_id` (and `agent_id`, for a subagent) as its `PreToolUse`. A
**successful MCP call** also emits `PostToolUse` (probe 12), but **the adapter
ignores it** — no scope was created, so there is nothing to close. The MCP
lifecycle research (probes 12–17) is retained as the *reason MCP is excluded*
(D2/D10a/D23), not as an MCP terminal-boundary mapping. Evidence:
`fixtures/probe01-apply-patch-and-shell-success.*`,
`fixtures/probe08-subagent-delegation.agent-apply-patch.*`,
`fixtures/probe12-mcp-mutate-success.*`.

### D10 — Failed-tool terminal observation — T01-GATED, likely no reliable signal

Prior SCE research (`context/plans/codex-cli-integration.md` Assumptions) found
Codex calls `PostToolUse` **only after a successful tool result**. Codex has
**no `PostToolUseFailure` event**. If T01 confirms this for the supported
version, then a mutation-capable tool that **wrote files then failed** produces
**no terminal hook**, and the adapter must **not** design a Close-on-failure
path.

In that case the failed-tool partial-mutation interval is bounded conservatively
by the next lifecycle signal (D11/D12): the stale `pending_start`/`active`
attempt is `abandon`ed and, once quiescent, one `flush` re-baselines the
worktree. The partial mutation is then attributed to nothing
(`IneligibleUnscoped`) rather than misattributed. **False-negative attribution
is preferable to false-positive attribution.**

If — and only if — T01 proves Codex emits a reliable final observation for a
failed mutation-capable tool (a `PostToolUse` with a failure-indicating
`tool_response`, or another event carrying the D3 identity), D10 becomes a
`Close` mapping like D9. T01 records which.

**T01 disposition (codex-cli 0.153.4): scope this conclusion to the tool types
actually proven — it does NOT hold globally.**
- **`Bash`: PROVEN — bounded by a terminal hook.** A shell tool that wrote a file
  then exited non-zero **does** fire `PostToolUse` (same `tool_use_id`) → map to
  `close` exactly like D9 (probe 2).
- **`apply_patch`: PROVEN — verification failure, no mutation.** Fires **no**
  `PostToolUse`, but Codex verifies the patch before touching the working tree,
  so nothing is written — there is no partial-mutation-without-terminal case for
  it (probe 6).
- **MCP: PROVEN — partial mutation with NO terminal hook.** A mutation-capable
  MCP tool that **writes a git-visible file and then returns `is_error:true`**
  receives **no terminal hook of any kind** (probe 13: `PreToolUse → Stop →
  SessionEnd`); `git status` afterwards shows the file. An external MCP server is
  not under Codex's atomicity control, so the side effect precedes the failure.
  Upstream mechanism: `codex-rs/core/src/tools/registry.rs` ~line 674 —
  `post_tool_use_payload = if success { … } else { None }` with
  `success = result.success_for_logging()`; for MCP
  `McpToolOutput::success_for_logging()` = `self.result.success()`
  (`codex-rs/core/src/tools/context.rs:122-124`), false when
  `CallToolResult.is_error == true`.
  **v1 consequence (D23):** since MCP is `Untracked`, **no scope exists**, so
  whether `PostToolUse` fires or not is **irrelevant to scope cleanup** — there
  is no `pending_start`/`active` attempt to strand and nothing to `abandon` or
  `flush`. This probe-13 lifecycle is exactly *why* MCP is excluded (D10a), not a
  gap the adapter must bound.
- **unknown: no tool-specific terminal guarantee, and no scope** — an unknown
  tool is `Untracked` (D2), so like MCP it has no scope and its `PostToolUse`
  presence/absence is irrelevant to scope cleanup.
Prior SCE research's "`PostToolUse` fires only on a successful tool result" is
true at the `success_for_logging()` layer; a non-zero-exit shell command still
counts as a successful tool result, an `is_error:true` MCP result does not. The
adapter needs **no Close-on-failure path for `Bash` / `apply_patch`** (the shell
`PostToolUse` still fires; `apply_patch` verification failure writes nothing),
and **no failure path for MCP / unknown** because it creates no scope for them.
Evidence: `fixtures/probe02-shell-partial-write-then-nonzero-exit.*`,
`fixtures/probe06-apply-patch-verification-failure-no-post.*`,
`fixtures/probe13-mcp-mutate-then-error.*`.

### D10a — Failed-tool -> successor-tool in the same turn — T01-GATED

This decision applies only to **`TrackedMutation`** tools. `Untracked` tools
(MCP, unknown) never enter `attempts[]`, so a failed `Untracked` tool followed
by any successor creates no zombie scope — there is nothing in adapter state to
strand (see the MCP disposition below).

D10's "next lifecycle signal" is **not sufficient on its own** for the case where
a failed `TrackedMutation` tool with no terminal hook is followed by **another
`TrackedMutation` tool in the same turn**, before any `Stop` / `SessionEnd` /
`UserPromptSubmit`:

```text
PreToolUse(A) -> Start(A) -> A mutates Git-visible state -> A fails -> NO terminal hook
PreToolUse(B) -> ... (adapter state still: A.phase = active, recovery_pending = false)
```

At `PreToolUse(B)` the D13 barrier does nothing (nothing armed it), so if B
starts normally SCE has a **zombie live scope A** alongside the real scope B.
This can produce a false `AiContended` for any tree transition observed while
both look live, or attribute B's (or later) mutations to A.

Two invariants must both hold, and they are in tension:

```text
(i)  a mutation-capable successor must never Start while an older attempt that
     may already be terminal remains "live" solely because Codex omitted its
     terminal event;
(ii) a still-legitimately-running parallel attempt must never be abandoned just
     because another PreToolUse arrived.
```

Reconciling them requires a **T01-proven seriality boundary**. T01's new
"failed tool followed by another tool in the same turn" probe (see T01 scope)
records exactly one disposition:

- **Case A — a reliable intermediate cleanup signal exists.** Codex emits
  positive stale/terminal evidence for A between A's failure and `PreToolUse(B)`.
  The adapter routes it through D12: signal -> `abandon` A -> `recovery_pending`
  -> quiescent `flush` -> then `PreToolUse(B)` proceeds. T01 records which signal
  is load-bearing. No successor-barrier logic is needed.
- **Case B — no intermediate signal, but successor `PreToolUse` proves the
  predecessor stale.** T01 proves Codex executes mutation-capable tools
  **serially within a narrow, identity-defined lane** (candidate lane keys:
  session, turn, or a proven delegated-agent identity — **not frozen until T01
  proves the concurrency semantics**). Only then may `PreToolUse(B)` itself count
  as positive evidence that an older outstanding attempt **in B's same proven
  lane** cannot still be running. The adapter then does, on `PreToolUse(B)`,
  before allocating B:

  ```text
  inspect existing attempts in B's proven-serial lane
    -> a stale predecessor A found (same lane, not terminal in bookkeeping)
    -> arm recovery_pending
    -> abandon A
    -> when quiescent, one flush through the seam
    -> only then write-ahead Start(B)
  ```

  This is a **lane-scoped** sweep, never a global sweep. An attempt outside B's
  proven lane (a legitimately concurrent execution, if T01 shows any exist) is
  left untouched — invariant (ii).
- **Case C — neither is safe.** No reliable intermediate signal **and** the
  successor `PreToolUse` does not prove the predecessor stale because parallel
  executions in the same lane are possible. The adapter cannot distinguish
  `failed-and-dead A` from `still-running A`. T01 **marks this an architectural
  contradiction / unsupported lifecycle and stops the plan for re-planning** —
  it does not guess.

T01 must define "same lane" using only identity/concurrency facts it actually
established. T02 freezes the lane key and the successor-barrier design (if
Case B); T04 implements it; T06 proves it.

**T01 disposition (codex-cli 0.153.4): scope-split by tool type. Case A for
built-ins; Case C for MCP *if MCP were modeled as a scope* — resolved in v1 by
NOT modeling MCP as a scope (D23, re-planning direction B).**

**Built-in `Bash` / `apply_patch`: PROVEN — Case A.** The dangerous scenario does
not arise, and no successor-barrier logic ships for built-ins.
- A failed **shell** tool always emits `PostToolUse` (terminal) before the next
  `PreToolUse` — Codex runs built-in mutation-capable tools serially (D1), so
  predecessor A is already terminal in bookkeeping when successor B's
  `PreToolUse` arrives.
- A failed **`apply_patch`** never mutates the working tree (atomic
  verification), so there is nothing to strand.
- A **hook-blocked** tool never executes (`PreToolUse` only, no `PostToolUse`) —
  no scope was established (D8 fail-closed happens before `start`).
- The only built-in "partial mutation, no `PostToolUse`" case is **whole-turn
  interruption** (SIGINT), which emits `Interrupt` then `SessionEnd` and ends the
  turn — there is no in-turn successor `PreToolUse` to race.
Evidence: `fixtures/probe02-*`, `probe06-*`, `probe07-*`, `probe11-*`.

**MCP: the T01 finding is correct and stands — `MCP is D10a Case C *if modeled
as a scope*`.** The T01 MCP extension establishes all three conditions of Case C
simultaneously, and this evidence is **not** weakened by the v1 resolution:
- **A mutation-capable MCP tool can mutate then fail with no terminal hook**
  (probe 13: writes `mcp_b.txt`, returns `is_error:true`, then `PreToolUse →
  Stop → SessionEnd` — no `PostToolUse`; the mutation survives). D10 above.
- **No positive cleanup signal appears before a successor.** Probe 14:
  `PreToolUse(A = mutate_then_error)` is followed **directly** by
  `PreToolUse(B = mutate_success)` with **no event of any kind between them** —
  no `PostToolUse(A)`, no `Interrupt`/`Stop`/`SubagentStop`/`SessionEnd`/
  `PermissionRequest`/compaction. A's `tool_use_id` never recurs.
- **Same-lane MCP executions genuinely overlap** (probes 16/17), so
  `PreToolUse(B)` does **not** prove A stale — there is no narrower serial lane
  than `(session_id, turn_id)` and executions overlap within it. Case B is
  unavailable.
If MCP were a scope, the adapter could not distinguish `failed-and-dead A` from
`still-running A`, and no safe successor barrier exists. **That lifecycle did not
become safe — the v1 resolution is to not put MCP in the scope model at all.**

**v1 resolution (re-planning direction B, D23): the contradiction is resolved by
not modeling MCP executions as scopes.** `PreToolUse(mcp__…)` is classified
`Untracked` (D2): no `Start`, no bookkeeping. So the probe-13/14 sequence
becomes:

```text
PreToolUse(A MCP)   -> Untracked -> no Start, no attempt recorded
A mutates and fails -> no PostToolUse -> nothing in adapter state to strand
PreToolUse(B)       -> normal classification
                       (if B is Bash/apply_patch it Starts on its own merits;
                        if B is MCP it is also Untracked)
```

No successor barrier is needed for MCP because **no MCP attempt exists in adapter
state** — the D10a tension (invariants (i)/(ii)) only arises for tracked scopes,
and there are none for MCP. This is a deliberate attribution-coverage boundary
(D23), not a lifecycle workaround: the adapter does **not** claim the MCP
lifecycle is safe, does **not** silently downgrade MCP/unknown to read-only, and
does **not** pretend an MCP mutation was attributed.

T02 records the D10a lane key as **N/A** (confirmed 2026-09-08) — Case A for
built-ins ships no barrier; MCP/unknown are `Untracked` and ship no barrier and
no scope. The T02 event model therefore carries no lane-key field and
`classify_tool` has no successor-sweep hook. Evidence:
`fixtures/probe13-mcp-mutate-then-error.*`,
`fixtures/probe14-mcp-failed-then-successor.*`,
`fixtures/probe15-mcp-blocked-call.*`,
`fixtures/probe16-mcp-parallel-server-optin.*`,
`fixtures/probe17-mcp-parallel-readonly-hint.*`,
`fixtures/NOTES.md` ("T01 MCP lifecycle probe extension").

### D11 — Uncertain-boundary abandonment rules

Carried verbatim from the Claude adapter (D11/D12 there), because they are
runtime-contract properties, not Claude lifecycle specifics:

- **`pending_start` + terminal/cleanup signal -> abandon, not late-Start.** The
  adapter cannot prove `Start` committed; a late `Start` after the tool ran
  would observe the post-tool tree and misattribute the interval. `abandon` on
  a committed `Start` is normal abandonment; `abandon` on a `Start` that never
  committed hits the runtime's `MissingScope` / `NeverSeen` recovery path.
- **Failed `Close` -> abandon + `recovery_pending`, not a replayed `Close`.**
  The original observation time is lost; a later tree must never be presented as
  the tool-completion tree. The two ingress carried-success variants
  (`MarkerClearAfterCommit` / `MarkerClearAfterCompletion`) are durable success
  and do not enter this path.

### D12 — Codex lifecycle cleanup signals — T01-GATED

The adapter retires outstanding attempts on positive staleness evidence only —
**never** on absence of activity, and **never** inferred from `ActorKind`. The
candidate Codex signals, each **T01-GATED** on actually firing with the identity
fields the mapping needs:

| Candidate Codex event | Would retire | T01 must establish |
| --- | --- | --- |
| `Stop` | outstanding attempts for that `session_id` (main turn) | fires on every turn end; carries `session_id` |
| `SessionEnd` | every outstanding attempt for the session | fires on session/process termination; carries `session_id` |
| `SubagentStop` | outstanding attempts owned by the ending delegated agent | fires; carries a delegated-agent identity distinguishable from the main thread (else it cannot be used for a scoped sweep) |
| `PermissionRequest` (denied) | the one live attempt for that tool call | whether a denied `PermissionRequest` leaves a durably-established `Start` with no terminal event |
| next `UserPromptSubmit` | stale main-turn attempts (interruption fallback) | whether Codex emits `Stop` on user interruption, or only the next prompt |
| `PreCompact` / `PostCompact` | *(diagnostic only unless T01 shows a lifecycle gap)* | whether compaction can strand an attempt |

Only signals T01 marks `PROVEN` (or `DOCUMENTED — NON-LOAD-BEARING` with a
load-bearing backstop named) become adapter behavior. T01 must identify which
signals provide **positive staleness evidence** suitable for `abandon`, and
which single signal is the load-bearing backstop (the Claude adapter's backstop
is `SessionEnd`).

The failed-tool -> successor-tool sequence (D10a) is the one case where the
"next lifecycle signal" backstop is too late **for a tracked tool**; for
built-ins it is Case A (a terminal `PostToolUse` always precedes the successor),
and for MCP/unknown it does not arise because they are `Untracked` (no scope, no
attempt). Owned by D10a, not this table.

**T01 disposition (codex-cli 0.153.4): PROVEN.**

| Signal | Fires | Identity | Adapter use |
| --- | --- | --- | --- |
| `SessionEnd` | clean exit **and** SIGINT | `session_id`, `cwd` (no `turn_id`/`agent_id`) | **load-bearing backstop** — whole-session sweep of every outstanding attempt |
| `Stop` | clean main-turn end only (not interruption) | `session_id`, `turn_id` | main-turn sweep (`agent_id` absent) |
| `Interrupt` | SIGINT, **before** `SessionEnd` | `session_id`, `turn_id` | earlier session/turn-scoped sweep — **newly discovered; not in the plan's original list** |
| `SubagentStop` | delegated agent ends | `agent_id`, `agent_type` | sweep attempts owned by that `agent_id` |
| `PermissionRequest` (deny) | interactive approval flows only; not reachable from `codex exec` | — | `DOCUMENTED — NON-LOAD-BEARING`; `SessionEnd` backstop covers it |
| `PreCompact` / `PostCompact` | not observed | — | `DOCUMENTED — NON-LOAD-BEARING` (diagnostic only) |

`SessionEnd` is the single load-bearing backstop (the Codex analogue of the
Claude adapter's `SessionEnd`). Evidence:
`fixtures/probe01-…​.stop.json` / `.session_end.json`,
`fixtures/probe07-sigint-during-shell.session_end.json`,
`fixtures/probe11-interrupt-event-on-sigint.{interrupt,session_end}.json`,
`fixtures/probe08-subagent-delegation.subagent_stop.json`, and the generated
`session-end` / `stop` / `interrupt` / `subagent-stop` / `permission-request`
schemas at `openai/codex` `rust-v0.153.4`.

**MCP caveat:** `Stop` and `SessionEnd` still fire at turn/session end for an
MCP-only turn (probes 13/14: `PreToolUse → Stop → SessionEnd`). This backstop is
**whole-turn-late** — it does not fire between a failed MCP tool and an in-turn
successor `PreToolUse` (probe 14 shows *nothing* there). That gap is precisely
why MCP cannot be safely modeled as a scope on 0.153.4 (D10a). In v1 there is
**no stranded failed-MCP attempt** for any signal to retire, because MCP is
`Untracked` and the adapter records no attempt for it (D2/D23). The D12 sweeps
operate only over adapter-owned tracked attempts.

### D13 — recovery_pending barrier and quiescent Flush

Carried verbatim from the Claude adapter (D19 there). The recovery barrier
operates **only over adapter-owned tracked attempts** — `Untracked` (MCP,
unknown) executions never enter `attempts[]`, never set `recovery_pending`, and
never participate in the abandon/flush lifecycle. Whenever an abandonment or an
uncertain lifecycle of a **`TrackedMutation`** attempt sets
`recovery_pending = true`:

```text
recovery_pending == true AND known (tracked) attempts still outstanding
  -> deny every new TrackedMutation PreToolUse (D8 fail-closed shape)
  -> an Untracked PreToolUse is NOT denied by the barrier (it never Starts)

recovery_pending == true AND attempts.is_empty()
  -> run one { "operation":"flush" } through the generic ingress
  -> clear recovery_pending ONLY on durable flush success
  -> a failed flush stays fail-closed
```

A failed abandonment leaves the attempt tracked and recovery armed
(never silently allows a successor tracked-mutation execution).

**Successor-Start invariant.** A `TrackedMutation` `PreToolUse` must never reach
its write-ahead `Start` while a known-stale predecessor **tracked** attempt
(D10a) remains `active`/`pending_start` in bookkeeping. For built-ins this is
Case A (a terminal `PostToolUse` precedes the successor), so no successor-barrier
logic ships. MCP/unknown are `Untracked`, so they create no predecessor attempt
and no barrier is needed. This invariant does **not** license abandoning an
attempt that can legitimately run concurrently with the successor (D10a
invariant (ii)).

**D13a — inter-process concurrency semantics (T04 follow-up, 2026-09-08).**
Codex runs every hook as an independent OS process, so the barrier and
tracked-attempt admission must be atomic across processes, not merely
lock-serialized field writes. The initial T04 driver composed the decision from
an unlocked `read_state` recovery check followed by a separate `allocate_attempt`
(no recovery re-check) plus a plain-boolean `recovery_pending` — a TOCTOU gap
where a second process could arm recovery in between, a duplicate quiescent
`flush`, and a stale `flush` completion clearing a newer recovery. The follow-up
makes recovery **generation-aware** and moves admission into one locked
transition:

```text
recovery state = Clear | Pending(generation) | Flushing(generation)
                 + a monotonic next_recovery_generation

admit_tracked_attempt(key, tool)  -- one adapter-state-lock transition:
  Flushing(_)                        -> RecoveryBlocked
  Pending(g) AND attempts non-empty  -> RecoveryBlocked
  Pending(g) AND attempts empty      -> persist Flushing(g); return FlushClaimed(g)
  Clear AND key already tracked       -> reuse that attempt (idempotent)
  Clear AND an unrelated PendingStart -> UncertainAttemptBlocked
  Clear otherwise                     -> persist a fresh PendingStart; return Admitted

arm_recovery()  -- one transition:
  Clear        -> Pending(next_recovery_generation++)
  Pending(g)   -> Pending(g)                     (kept; not a downgrade)
  Flushing(g)  -> Pending(next_recovery_generation++)   (supersedes the in-flight flush)

complete_recovery_flush(g)  -- one transition, run AFTER the flush seam:
  Flushing(g) -> Clear      (only when the generation still matches)
  otherwise   -> no-op      (a newer recovery armed while the flush ran survives)

relinquish_recovery_flush(g)  -- flush seam failed:
  Flushing(g) -> Pending(g)   so a later PreToolUse re-claims and retries
```

The adapter-state lock is still **never** held across a `hooks::mutation_scope`
seam call (I6). The driver drives at most one quiescent `flush` per
`PreToolUse`: `FlushClaimed(g)` -> flush seam -> `complete_recovery_flush(g)` ->
one re-entrant `admit_tracked_attempt`; a re-armed generation observed on
re-entry is relinquished and the tool denied (the next `PreToolUse` owns it).

**D13b — unresolved `PendingStart` is a conservative barrier (Problem 4).**
`Start` seam success followed by a failed `mark_active` leaves a durable
`PendingStart` attempt whose runtime `Start` may have committed. That attempt now
blocks a successor tracked admission (`UncertainAttemptBlocked`) until a positive
cleanup signal abandons it -> arms recovery -> quiescent `flush`. `PendingStart`
means "the adapter cannot prove whether `Start` committed", never "`Start`
definitely failed". A duplicate delivery for the same `AttemptKey` still reuses
the same attempt and `ScopeId` (no second scope).

### D14 — Concurrency and AiContended

Two simultaneously-live **tracked** scopes (two tracked Codex executions per D1,
or a tracked Codex execution overlapping a Claude/OpenCode/Pi execution on the
same worktree) carry distinct `ScopeId`s, so the runtime can report `AiContended`
for a tree transition observed while both are live. The adapter never collapses
two executions into one `ScopeId`.

**The generic mutation-scope semantic is preserved exactly and needs no
protocol or Quint change:**

```text
AiExclusive(scope) == exactly one tracked mutation scope was live in the interval
```

It does **not** mean:

```text
that scope authored every filesystem mutation in the interval
```

An MCP call, a human editor, or any other `Untracked` actor may mutate the
worktree during an `AiExclusive` interval. This is already the generic runtime
contract — the Codex adapter's `Untracked` policy does not change it and does not
require a protocol or Quint change.

**T01 disposition (codex-cli 0.153.4):**
- **Built-in `Bash` / `apply_patch`: observed serial** (probes 1–11) — no
  Codex-alone tracked-scope overlap observed.
- **MCP: parallel MCP execution is real** (probes 16/17: two MCP executions
  overlap for ~8 s via `supports_parallel_tool_calls` or the tool's own
  `annotations.readOnlyHint`). But MCP is `Untracked` in v1, so:
  - `MCP + MCP` overlap -> no MCP mutation scopes -> **no MCP-derived
    `AiContended`**;
  - `Bash + MCP` overlap -> only `Bash` is a tracked scope -> the runtime may
    still report `AiExclusive(Bash)`, which per the semantic above means "the
    only *tracked* scope live", **not** "MCP did not mutate". T06 documents this
    explicitly (see the tracked-tool-plus-MCP-overlap regression).
- **Cross-harness: `AiContended` remains reachable** — a tracked Codex scope
  overlapping a Claude/OpenCode/Pi scope on the same worktree.
The T06 concurrency regression (AC10) crosses harnesses. There is **no**
MCP-overlap `AiContended` regression because MCP produces no tracked scopes; T06
instead adds a `Bash`-overlapping-MCP regression that asserts the
`AiExclusive` = tracked-scope-exclusivity (not sole-authorship) semantic.

### D15 — Raw Codex hook cwd is authoritative — T01-GATED

The mutation runtime's repository root is the raw Codex hook payload's `cwd`
(Codex runs command hooks with `.current_dir(cwd)` and the event carries `cwd`).
T01 must confirm the raw hook `cwd` is authoritative for the actual checkout
being mutated (Codex has no known Claude-style `isolation: worktree` subagent,
but T01 verifies, and checks whether Codex exposes any separate
worktree-lifecycle event or path). The adapter passes the raw `cwd` to the seam
as `repository_root`; the runtime derives `WorktreeId`. The adapter never
accepts, derives, stores, or constructs a `WorktreeId`, and passes no
`worktree_id` key to the seam. There is no Codex `WorktreeRemove` equivalent;
worktree-scoped cleanup relies on the D12 session/agent signals.

**T01 disposition (codex-cli 0.153.4): PROVEN.** Every hook payload's `cwd` was
the `codex exec -C` directory. Running against a linked `git worktree` reported
the worktree path in `cwd`, and the write landed inside the worktree, not the
main checkout; `checkout::resolve_git_dir(cwd)` resolves the worktree-specific
`.git/worktrees/<name>` directory. Codex exposes **no** worktree-lifecycle event
(the 12 event names are `PreToolUse`, `PermissionRequest`, `PostToolUse`,
`PreCompact`, `PostCompact`, `SessionStart`, `SessionEnd`, `UserPromptSubmit`,
`SubagentStart`, `SubagentStop`, `Stop`, `Interrupt` — no `WorktreeRemove`).
Evidence: `fixtures/probe10-linked-worktree-cwd.*`, and every other probe's
`cwd`.

### D16 — Background / detached shell is a correctness boundary — T01-GATED

T01 must separate **Codex-managed background execution** (if Codex exposes a
"run in background" tool option or a background-task lifecycle) from a
**foreground shell command that spawns a self-detaching descendant** (already
known to be fundamentally difficult without process supervision — the Claude
adapter's D20 confirmed this is real and Git-observable).

- If Codex exposes explicit background execution, and T01 shows its lifecycle
  does **not** reliably bound the mutations, the adapter **denies** it in
  `PreToolUse` (D8 shape) with a Codex-specific unsupported-execution message,
  exactly as the Claude adapter denies `run_in_background = true`.
- The self-detaching-descendant boundary is recorded, with **Codex-specific
  evidence from T01**, as an explicit unsupported boundary: `PostToolUse` (or
  whatever terminal signal exists) does not necessarily prove that every
  descendant stopped mutating. The adapter adds **no** PID supervision,
  process-group tracking, background-process ownership, shell-command static
  analysis, or staleness polling.

Do not claim support for any execution pattern unless T01 demonstrates its
lifecycle actually bounds the mutations.

**T01 disposition (codex-cli 0.153.4): PROVEN (self-detaching descendant);
no Codex-managed background execution in this surface.**
- The default `codex exec` shell tool has **no `run_in_background` parameter**
  (params: `command`, `workdir`, `timeout_ms`, `with_escalated_permissions`,
  `justification`). There is no explicit Codex-managed background execution to
  deny in `PreToolUse` for this surface, so the adapter ships **no
  background-execution classifier / deny** (unlike the Claude adapter's
  `run_in_background = true` deny). If a future Codex surface adds one, revisit.
- A foreground shell command that `setsid`-detaches a descendant **does** leave a
  Git-observable mutation landing ~4s after `PostToolUse`
  (`fixtures/probe09-self-detaching-descendant.{pre_tool_use,post_tool_use,evidence}.json`)
  — same class as the Claude adapter's D20. Recorded as an explicit unsupported
  boundary; the adapter adds no PID supervision, process-group tracking,
  shell-command static analysis, or staleness polling, and does not treat
  `PostToolUse` as proof every descendant stopped mutating. Not generalised to
  `nohup` / double-fork / daemonize, which this probe did not exercise.

### D17 — Command architecture: separate hidden command (recommended) — decided in T02

Codex today funnels **every** registered hook event through the single
`sce hooks codex` dispatcher, which is **fail-open** (errors -> empty stdout),
and `codex_hook_config.rs` decides SCE ownership structurally by matching the
exact trailing command tokens `["sce", "hooks", "codex"]`
(`CODEX_COMMAND_WORDS`).

The mutation-scope adapter is **non-fail-open** (fail-closed on `PreToolUse`,
never-silently-drop on terminal boundaries). Two viable architectures:

1. **Separate hidden command `sce hooks codex-mutation-scope`** (recommended
   default). Its `.codex/hooks.json` registrations use a distinct command, so
   Codex invokes the mutation-scope hook as its own process, independent of the
   `sce hooks codex` policy/diff process for the same event — exactly how Claude
   runs `sce policy bash` and `sce hooks claude-mutation-scope` side by side on
   `PreToolUse`. `codex_hook_config.rs` gains a **second command contract**
   (`CODEX_COMMAND_WORDS` becomes a set; `REQUIRED_EVENTS` records which command
   owns each registration; ownership/merge/doctor become command-aware). This
   keeps each evidence system's failure posture and diagnosability independent,
   and no single `sce hooks codex` invocation has to do two jobs with two
   failure postures.
2. **Extend the `sce hooks codex` dispatcher** with mutation-scope arms, keyed
   by matched tool groups so no event is double-invoked, and restructure the
   dispatcher so mutation-scope arms propagate errors while conversation/diff
   arms stay fail-open. Smaller `codex_hook_config.rs` change
   (`CODEX_COMMAND_WORDS` unchanged, only `REQUIRED_EVENTS` grows), but couples
   the two evidence systems' fate inside one process for `PreToolUse(shell)` and
   `PostToolUse(apply_patch)`.

T02 makes the final call against T01 findings and the code, defaulting to (1),
and records the decision here. Every subsequent task's wording assumes (1); if
T02 chooses (2), T02 revises D17, D8's routing, and T04/T05 scope accordingly.

**T01 inputs (codex-cli 0.153.4):** the mutation-scope adapter needs
registrations for at least `PreToolUse`, `PostToolUse`, `Stop`, `SessionEnd`,
`SubagentStop` (optionally `Interrupt` as an earlier interruption sweep). The
existing `sce hooks codex` dispatcher funnels 4 events (`UserPromptSubmit`,
`Stop`, `PreToolUse` matcher `Bash`, `PostToolUse` matcher `apply_patch`) and is
fail-open; the mutation-scope adapter is fail-closed on `PreToolUse` and
never-silently-drop on terminal boundaries. `PreToolUse` and `Stop` would be
double-registered (once per command). Live probes confirmed Codex runs each
registered handler as its own process and that two `PreToolUse` handlers in one
group both execute (dump + block). A **separate hidden
`sce hooks codex-mutation-scope` command** (option 1) remains the recommended
default; nothing in T01 argues against it. Evidence: `fixtures/NOTES.md`,
`.codex/hooks.json` two-handler `PreToolUse` group used across probes 3–11.

**T02 decision (2026-09-08): option 1 — a separate hidden
`sce hooks codex-mutation-scope` command.** Confirmed against T01 and the code:

- The mutation-scope adapter is fail-closed on `PreToolUse` and
  never-silently-drop on terminal boundaries; the existing `sce hooks codex`
  dispatcher (`hooks::codex`) is fail-open (errors -> empty stdout). Folding the
  two into one process (option 2) would couple their failure postures for
  `PreToolUse(shell)` and `PostToolUse(apply_patch)` — the exact events both
  systems care about.
- T01 proved Codex runs each registered handler as its own OS process and that
  two `PreToolUse` handlers in one matcher group both execute (dump + block
  across probes 3–11), so a distinct command registered alongside
  `sce hooks codex` runs independently — the direct analogue of Claude running
  `sce policy bash` and `sce hooks claude-mutation-scope` side by side.
- `#263`'s Claude adapter set the precedent: a dedicated non-fail-open
  `sce hooks claude-mutation-scope` command, not an arm of the fail-open
  `sce hooks claude` dispatcher.

Consequences for later tasks (unchanged from each task's current wording, which
already assumes option 1): T04 adds the hidden `HooksSubcommand::CodexMutationScope`
routed unwrapped like `mutation-scope`; T05 makes `codex_hook_config.rs`
command-aware (`CODEX_COMMAND_WORDS` becomes a set; `REQUIRED_EVENTS` records
which command owns each registration) and appends the mutation-scope
registrations position-stably after the existing `sce hooks codex` ones (D20).
D8's denial routing is unaffected — the mutation-scope command owns its own
`PreToolUse` registration.

### D18 — Adapter depends on hooks::mutation_scope only

Dependency direction is strictly
`codex_mutation_scope -> hooks::mutation_scope -> mutation_trace::runtime`. The
Codex adapter's production code (everything in
`cli/src/services/hooks/codex_mutation_scope/` outside `#[cfg(test)]`) imports
no `crate::services::mutation_trace::{runtime,protocol,store}` and names no
`RepositoryAgentTraceDb`, `WorktreeId`, `GitSnapshotService`, or
`RepositoryAgentTraceDb`. Its only dependency into the mutation stack is the
single `super::mutation_scope::run_mutation_scope_from_payload` seam import
(already `pub(crate)` since #263's T05), reused verbatim by building the generic
wire payload as a JSON string — no second `RuntimeBoundary` path, no spawned
`sce` subprocess, no invocation of `coordinate()` / `abandon_scope()` directly.

### D19 — Existing Codex integration stays additive

The `sce hooks codex` pipeline (`UserPromptSubmit`/`Stop` conversation capture,
`PreToolUse(Bash)` policy, `PostToolUse(apply_patch)` `diff_traces` evidence,
`.codex/hooks.json` ownership/merge, doctor trust/policy diagnosis,
`sce setup --codex`, `sce doctor`) is not replaced or redesigned. The
`apply_patch -> diff_traces -> post-commit intersection` pipeline is a
complementary evidence system and is **not** folded into mutation-scope storage.
The new adapter writes only `mutation_trace_*` rows through the runtime. The raw
Agent Trace tables `diff_traces`, `post_commit_patch_intersections`,
`agent_traces`, `messages`, and `parts` are never written by the new adapter.
Generated `.codex/hooks.json` must preserve every existing SCE-owned and
user-owned registration; setup merge stays idempotent.

### D20 — Existing Codex hook trust identity must survive the upgrade

Codex hook-trust identity is not just handler bytes. Current SCE/Codex trust
logic (`codex_hook_config.rs` + `codex_hook_policy.rs`, and upstream
`hooks::version_for_toml` / state keying) identifies a hook by
`event key label` + `matcher-group index` + `handler index` +
`normalized handler contents/hash`. So **adding a mutation-scope handler can
invalidate an existing hook's trust even though its JSON bytes are unchanged**:

```text
before:  Stop / group 0 / handler 0 -> sce hooks codex          (key stop:0:0, trusted)
after (bad merge):
         Stop / group 0 / handler 0 -> sce hooks codex-mutation-scope
         Stop / group 0 / handler 1 -> sce hooks codex          (key stop:0:1 — re-trust needed)
```

T05 must preserve, for every existing SCE Codex registration, the tuple
`(event, matcher, matcher-group index, handler index, handler contents/hash)`
when upgrading a canonical four-registration document to one containing
mutation-scope registrations. Insertion is **additive and position-stable**:

- an existing handler keeps its index; a new mutation-scope handler is appended
  **after** the existing handler in its group;
- an existing matcher group keeps its index; a new matcher group is appended
  **after** the existing groups.

New SCE-owned handlers/groups are never prepended in a way that renumbers an
already-trusted hook. If position preservation is genuinely impossible for some
event/matcher structure (T01/T05 must say which, if any), the plan records it and
`sce doctor` must surface that **re-trust is needed** — trust is never silently
invalidated, and `sce doctor --fix` never writes, grants, or changes Codex trust
or managed policy.

### D21 — Doctor: three-dimension health for mutation-scope registrations

A structurally-current mutation-scope hook is useless if Codex never loads it —
and unlike a fail-open diff/conversation hook, an unloaded mutation-scope hook
means SCE **silently cannot fail closed**. So each mutation-scope registration
participates in the full existing Codex health model, exactly as the four
`sce hooks codex` registrations do:

```text
healthy  ==  structurally current  AND  trusted/enabled  AND  effective project-hook policy allows it
```

Never healthy: `PresentAndCurrent + Untrusted`, `+ Modified`, `+ Disabled`,
`+ PolicyBlocked` (Error severity, manual-only), `+ PolicyUnknown` (Warning
severity, manual-only). Doctor reports the actual readiness of each registration
rather than flattening the mutation-scope hooks and the `sce hooks codex` hooks
into one generic `.codex/hooks.json` status. The single per-invocation
`configRequirements/read` policy probe (`codex_hook_policy.rs`) is reused, not
re-run per registration.

### D22 — New event key labels come from upstream, not from lowercasing

`codex_hook_config::hook_event_key_label` currently maps only the four
SCE-owned events. Every **newly** SCE-owned mutation-scope event (T01 decides the
set — candidates `SessionEnd`, `SubagentStop`, `PermissionRequest`, an unmatched
`PreToolUse` group, etc.) must be added with the **exact upstream Codex key
label**, verified against `openai/codex` source (T01/T05), not by lowercasing the
event name. Each newly registered event gets a `hook_event_key_label` test.

**T01 disposition (codex-cli 0.153.4): PROVEN.** The upstream label map is
`codex-rs/hooks/src/lib.rs` lines 96–108 at tag `rust-v0.153.4`
(commit `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`):
`PreToolUse→pre_tool_use`, `PermissionRequest→permission_request`,
`PostToolUse→post_tool_use`, `PreCompact→pre_compact`,
`PostCompact→post_compact`, `SessionStart→session_start`,
`SessionEnd→session_end`, `UserPromptSubmit→user_prompt_submit`,
`SubagentStart→subagent_start`, `SubagentStop→subagent_stop`, `Stop→stop`,
`Interrupt→interrupt`. **Codex 0.153.4 has 12 hook events, not the 11 this plan
lists — it also has `Interrupt`** (fires on SIGINT before `SessionEnd`). T05
must add a `hook_event_key_label` entry + test for every newly-registered
mutation-scope event, citing this source file. The `$CODEX_HOME/config.toml`
`[hooks.state]` keys observed live use exactly these labels
(`…:pre_tool_use:0:0`, `…:post_tool_use:0:0`, `…:stop:0:0`,
`…:user_prompt_submit:0:0`).

### D23 — Codex mutation-scope attribution v1 is partial by tool surface

**Decision (2026-09-08, re-planning direction B).** Codex mutation-scope
attribution v1 is **deliberately partial**, split by tool surface:

```text
Covered (TrackedMutation — one execution -> one ScopeId):
  Bash
  apply_patch

Delegation (no scope for the tool itself; the delegated agent's tracked tools get scopes):
  collaborationspawn_agent
  collaborationwait_agent

Allowed but NOT covered (Untracked — executes, may mutate, no scope, no bookkeeping):
  mcp__*  (any MCP tool)
  unknown / future Codex tool names, until their lifecycle is explicitly researched
```

**Coverage meaning.** Codex mutation-scope attribution describes **exclusivity
among the tracked scopes the adapter is told about**, not exhaustive authorship
of every filesystem mutation. `AiExclusive(scope)` means exactly one tracked
scope was live in the interval — an MCP call, a human editor, or another
`Untracked` actor may have mutated the same worktree in that interval (D14).

**Why MCP is `Untracked`, not a scope.** T01 (probes 12–17, codex-cli 0.153.4)
proved that if MCP were represented as a mutation scope, the Codex 0.153.4 hook
lifecycle makes it unsafe (D10a Case C):

- **mutate-then-error has no terminal hook** — a mutation-capable MCP tool can
  write a git-visible file and return `is_error:true` with no `PostToolUse`
  (probe 13);
- **a successor can start immediately** — `PreToolUse(A)` -> `PreToolUse(B)` with
  no cleanup signal of any kind between them (probe 14);
- **parallel MCP execution is real** — two mutation-capable MCP executions
  genuinely overlap (probes 16/17), so a successor `PreToolUse` cannot prove a
  predecessor stale.

The adapter cannot safely retire a failed-MCP zombie scope, and there is no
narrower serial lane than `(session_id, turn_id)`. **The resolution is to not
model MCP executions as scopes at all.** MCP calls execute normally and are
explicitly outside attribution coverage. This is a first-class coverage
boundary, not a lifecycle workaround — the adapter does not claim the MCP
lifecycle is safe, does not assert MCP is read-only, and does not guarantee MCP
mutations are detected immediately.

**What v1 must NOT do:**

- must **not** deny an MCP or unknown `PreToolUse` merely because it is untracked
  (D8);
- must **not** silently downgrade MCP/unknown to "read-only";
- must **not** silently pretend an MCP mutation was attributed;
- must **not** create any `Start`, `attempt`, `recovery_pending`, `Close`,
  `Abandon`, or `Flush` for an MCP or unknown execution.

**No protocol / formal change.** This choice requires **no**
`mutation_cursor.qnt` change, **no** mutation-trace protocol change, **no**
mutation runtime semantic change, **no** SQL migration, and **no** Agent Trace
schema change — the existing runtime already models exclusivity among tracked
scopes, not global filesystem authorship. Direction B changes only the Codex
adapter's coverage boundary.

**Rejected / deferred alternatives (historical rationale):**

- **A — deny mutation-capable MCP `PreToolUse` fail-closed.** Rejected: too
  disruptive (MCP tools become unusable inside Codex under SCE), and it needs a
  rule to tell "mutation-capable MCP" from "read-only MCP" that cannot trust the
  server's own `readOnlyHint`.
- **B — allow MCP/unknown untracked.** **Chosen for Codex adapter v1.**
- **C — a richer lifecycle/runtime mechanism** (e.g. a per-`tool_use_id` MCP
  scope retired only by `PostToolUse` or an overlap-tolerant turn-boundary
  sweep, plus an `AiContended`-aware successor policy). Deferred as possible
  **future work** in a separate, explicitly justified PR — "investigate
  first-class MCP mutation attribution using a richer lifecycle mechanism".

**Durable-context requirement.** `context/cli/codex-mutation-scope-integration.md`
(authored by T07) must state, in public/durable language, that **Codex MCP calls
remain usable but their filesystem mutations are not individually attributed by
the Codex mutation-scope adapter**, and must document why (the three T01
probe findings above), avoiding any wording that implies SCE knows MCP is
read-only, that MCP mutations are necessarily detected immediately, or that
`AiExclusive` proves sole authorship.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: `sce hooks codex-mutation-scope` exists, is hidden from `sce --help`
  and `sce hooks --help`, and routes through the normal hook command stack
  (`HooksSubcommand::CodexMutationScope` -> `convert_hooks_subcommand_request`
  -> `HookSubcommand::CodexMutationScope` -> `run_hooks_subcommand_in_repo`,
  **unwrapped / non-fail-open** like `mutation-scope`).
  - Validate: `sce hooks codex-mutation-scope </dev/null` exits with the strict
    parser's error (not "unknown subcommand"); `sce --help` and
    `sce hooks --help` do not list it; routing test in `command_runtime.rs`.
    (If T02 chooses D17 option 2, this AC instead asserts the new
    `CodexDispatchArm` mutation-scope variants and their non-fail-open handling
    inside `sce hooks codex`.)
- [ ] AC2: The raw Codex mutation-scope event parser strictly validates the
  fields T01 freezes as required for a tracked `PreToolUse` and rejects an empty
  payload, non-object JSON, and missing/blank/wrong-typed fields with a
  `Invalid Codex hook event payload from STDIN: <detail>.` diagnostic, never
  fabricating an identity.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope` parser unit tests,
    including a fixture per T01 probe.
- [ ] AC3: `classify_tool` returns exactly one of the three classes (D2) for
  every Codex tool name T01 enumerated: `TrackedMutation` (`apply_patch`, the
  shell/`Bash` tool), `Delegation` (`collaborationspawn_agent` /
  `collaborationwait_agent` — no scope), `Untracked` (any `mcp__<server>__<tool>`
  matching the proven shape, and any unknown/unrecognised `tool_name`). There is
  no "read-only" class on the `codex exec` surface. No `Start`, `attempt`, or
  bookkeeping entry is created for a `Delegation` or `Untracked` tool, nor for
  `SessionStart` / `UserPromptSubmit` / `SubagentStart` / any non-tool lifecycle
  event. The classifier must **not** contain language treating MCP/unknown as
  "mutation-capable therefore Start".
  - Validate: classification unit-test table (each tool name -> class); adapter
    mapping unit tests asserting that `Untracked`/`Delegation` events produce no
    processed-event keys.
- [ ] AC9b (MCP / unknown pass-through — direction B): A `PreToolUse(mcp__…)`
  and a `PreToolUse(<unknown tool>)` are classified `Untracked` and produce a
  **Codex-neutral continue response** — no `ScopeId`, no `EventId`, no
  mutation-scope `Start`, no adapter attempt, no `recovery_pending`, no
  bookkeeping row of any kind. The adapter never denies them for being untracked,
  never emits an explicit allow, never downgrades them to "read-only", and never
  records that their mutations were attributed. A successful MCP call produces
  **no** mutation-scope rows or events attributable to that MCP execution.
  - Validate: adapter unit tests — `PreToolUse(mcp__…)` and `PreToolUse(unknown)`
    return the neutral response with the state store untouched; a full
    successful-MCP lifecycle (`PreToolUse → PostToolUse`, probe 12 fixture)
    leaves zero mutation-scope rows/events; the recorded D23 decision in this
    plan.
- [ ] AC9c (MCP mutate-then-error leaves no stale state — direction B): Driving
  the probe-13 lifecycle (`PreToolUse(mcp__…)` mutates a git-visible file, tool
  returns an error, **no `PostToolUse`**, then `Stop`/`SessionEnd`) through the
  adapter leaves **no stale attempt, no `recovery_pending`, no `abandon`, and no
  zombie scope** — because no `Start` ever occurred. The subsequent `Stop` /
  `SessionEnd` sweeps find nothing to retire.
  - Validate: adapter unit test replaying the probe-13 fixture sequence and
    asserting the state store is empty throughout and after; T06 row-count
    assertion.
- [ ] AC9d (failed MCP A -> successor B — direction B): Using the probe-14
  sequence (`A` = untracked MCP that mutates then errors, immediately followed by
  `PreToolUse(B)` where `B` is a tracked `Bash`/`apply_patch` **or** another
  MCP), A leaves **no mutation-scope bookkeeping** that can interfere with B: if
  B is tracked it `Start`s normally as the only live scope; if B is MCP it is
  also `Untracked`. No successor barrier runs because no MCP attempt exists.
  - Validate: adapter unit test over the probe-14 fixture asserting B (tracked)
    is the only live scope at its `Start` and no false `AiContended`; T06
    regression.
- [ ] AC9e (parallel MCP — direction B): Using probes 16/17 (two MCP executions
  genuinely overlap), neither MCP `PreToolUse` creates a mutation scope, there is
  **no MCP/MCP `AiContended`**, and the adapter state store shows no leak (empty
  before, during, and after).
  - Validate: adapter unit test over the probe-16/17 fixtures; T06 regression
    asserting zero mutation-scope rows for the overlapping MCP pair.
- [ ] AC9f (tracked tool overlapping MCP — semantics documented): With
  `Start(Bash A)` live, an MCP call mutates the worktree, then `Close(Bash A)`.
  The runtime may report `AiExclusive(A)` for the interval. The test and the
  durable context must state this means **A was the only tracked scope live**,
  **not** that MCP did not mutate. The generic protocol is **not** changed to
  force this into `AiContended`.
  - Validate: T06 regression (`Bash` scope + real MCP mutation via
    `fixtures/mcp_probe/`) asserting the `AiExclusive` result and a comment/doc
    line recording the tracked-scope-exclusivity (not sole-authorship) reading;
    AC22 confirms no protocol/Quint change.
- [ ] AC4: The Codex execution key is exactly the field tuple T02 froze from
  T01 evidence (recorded in D3). Duplicate delivery of the same live
  `PreToolUse` reuses the same `attempt_seq`, `ScopeId`, and `Start` `EventId`.
  - Validate: identity/formatter unit tests; state unit tests; T06 duplicate-
    delivery regression.
- [ ] AC5: A later execution attempt of the same raw Codex tool identifier,
  after the previous attempt became terminal, receives a new `attempt_seq` and a
  new `ScopeId`; a terminal `ScopeId` is never reused; replaying a live
  attempt's event is `ScopeId`/`EventId`-stable.
  - Validate: state unit tests (terminal attempt then fresh `attempt_seq`);
    formatter determinism tests; T06 reused-identifier regression.
- [ ] AC6: A `TrackedMutation` `PreToolUse` reaches durable
  generic-ingress `Start` before the hook returns its "continue" response to
  Codex (write-ahead `pending_start` -> ingress `Start` -> `active`), and the
  seam receives the raw hook `cwd` as `repository_root` (never `git_dir`).
  - Validate: adapter ordering unit test with an injected seam asserting the
    persisted phase from inside the seam call and the `repository_root`
    argument; T06 production-path confirmation.
- [ ] AC7: Any failure to establish adapter state or `Start` during a
  `TrackedMutation` `PreToolUse` returns the exact Codex-native block response
  T01 froze (D8), never a silent success and never an explicit allow; the
  detailed error is logged via
  `sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed`. A `Delegation` or
  `Untracked` `PreToolUse` is never subject to this fail-closed path.
  - Validate: failure-classification unit tests asserting the exact response
    JSON/exit; `RecordingLogger` assertion that the detail is logged and not
    leaked into the model-visible reason.
- [ ] AC8: A successful `TrackedMutation` tool with an `active` attempt closes
  its scope: `PreToolUse` -> real filesystem mutation -> terminal Codex hook
  produces exactly one eligible tool interval and one terminal (`Closed`) scope
  with attribution `AiExclusive`.
  - Validate: T06 real-Git + real-Agent-Trace-DB regression.
- [ ] AC9: A `TrackedMutation` tool that partially mutated the checkout then
  failed is handled per D10's T01 disposition:
  - **`Bash`:** the terminal `PostToolUse` closes the scope (partial mutation
    attributed to that scope).
  - **`apply_patch`:** verification failure writes nothing — no scope to close.
  - **MCP / unknown:** not applicable — they are `Untracked` (D2/D23), no scope
    exists, so there is nothing to close, abandon, or flush; see AC9c.
  - Validate: T06 failed-`Bash` and failed-`apply_patch` regressions whose
    assertions match the D10 disposition recorded by T01.
- [ ] AC9a: A partially-mutating failed `TrackedMutation` tool A with **no
  terminal event**, followed by another `TrackedMutation` `PreToolUse(B)` in the
  same turn (Case A): a terminal `PostToolUse` (or `Interrupt`/`SessionEnd`)
  always precedes the successor `PreToolUse` for built-ins, so no
  successor-barrier logic ships and B never `Start`s alongside a zombie A. The
  MCP form of this scenario is covered by AC9d, not here, because MCP creates no
  attempt.
  - Validate: T06 built-in failed-A-then-B regression (assert A `Abandoned` or
    `Closed` per tool, B the only live scope at its `Start`, no false
    `AiContended`).
- [ ] AC10: Two simultaneously-live **tracked** scopes produce `AiContended` for
  a tree transition observed while both are live; the adapter never assigns them
  one shared `ScopeId`. Per D14 the exercised form is a **tracked Codex scope
  overlapping a second harness's scope** on the same worktree. There is **no**
  MCP-derived `AiContended` (MCP is `Untracked`); the `Bash`-overlapping-MCP case
  is AC9f, and asserts `AiExclusive` = tracked-scope exclusivity, not sole
  authorship.
  - Validate: T06 cross-harness concurrency regression; the plan records the D14
    form exercised.
- [ ] AC11: An outstanding **tracked** execution with no terminal hook is
  retired by exactly the Codex lifecycle signals T01 marked load-bearing (D12),
  via `abandon_scope`, leaving the worktree `needs_rebaseline`. The D12 sweeps
  operate only over adapter-owned tracked attempts; `Untracked` (MCP, unknown)
  executions never enter `attempts[]` and are never swept.
  - Validate: T06 regressions for each proven cleanup signal; adapter cleanup
    unit tests including one asserting an `Untracked` execution left no attempt
    for a sweep to touch.
- [ ] AC12: While `recovery_pending` is armed and known **tracked** attempts
  remain outstanding, every new `TrackedMutation` `PreToolUse` is denied (D8
  shape); an `Untracked` `PreToolUse` is not affected by the barrier; once
  quiescent, exactly one `{"operation":"flush"}` runs through the seam and
  `recovery_pending` clears only on durable flush success. (No D10a successor
  barrier ships — built-ins are Case A and MCP/unknown are `Untracked`.)
  - Validate: adapter recovery-barrier unit tests (deny-while-outstanding,
    flush-then-proceed, flush-failure-stays-closed); T06 recovery-barrier
    regression.
- [ ] AC13: A failed abandonment leaves the attempt tracked and
  `recovery_pending = true` (no successor mutation `Start` is allowed until
  recovery succeeds).
  - Validate: adapter unit test (failed `abandon` -> attempt retained, barrier
    armed, next `PreToolUse` denied, seam not re-driven).
- [ ] AC14: A hook process whose raw payload `cwd` names checkout B drives
  mutation state for checkout B only (its `WorktreeId`/cursor advances; another
  checkout's cursor is unchanged); the adapter constructs no `WorktreeId` and
  passes no `worktree_id` key.
  - Validate: T06 worktree-isolation regression (real linked worktree);
    dependency-boundary grep for `worktree_id` key construction.
- [ ] AC15: The background/detached execution boundary matches T01's finding
  (D16): any Codex-managed background execution T01 shows is unbounded is denied
  in `PreToolUse` with the recorded message; the self-detaching-descendant
  boundary is documented with Codex-specific T01 evidence and the adapter adds
  no detection or supervision.
  - Validate: adapter classification unit test (if a deny applies); T06
    documented unsupported-case regression; inspection of the T01 evidence
    fixture and the D16 disposition.
- [ ] AC16: Generated `.codex/hooks.json` after `sce setup --codex` still
  contains the four existing SCE registrations (`UserPromptSubmit`, `Stop`,
  `PreToolUse` matcher `Bash`, `PostToolUse` matcher `apply_patch`) routed to
  `sce hooks codex`, byte-for-byte, plus the new mutation-scope registrations;
  user-owned Codex handlers and unrelated valid event groups are preserved.
  - Validate: `nix run .#pkl-check-generated`; `codex_hook_config.rs` merge
    tests (existing + new); inspection of the rendered `config/.codex/hooks.json`.
- [ ] AC16a: Upgrading a realistic already-installed, already-trusted SCE Codex
  document (exactly the canonical four registrations) through the same
  merge/setup path that adds mutation-scope hooks preserves, for every existing
  registration, the tuple `(event, matcher, matcher-group index, handler index,
  handler contents/hash)` — so its Codex trust identity stays valid (D20). New
  mutation-scope handlers/groups are appended after existing ones, never
  prepended in a way that renumbers a trusted hook; each new mutation-scope hook
  appears exactly once; user-owned hooks are untouched; a second merge is
  byte-identical. Where position preservation is genuinely impossible for some
  structure, the plan says which and doctor surfaces that re-trust is needed.
  - Validate: `codex_hook_config.rs` "canonical-four -> plus-mutation-hooks"
    upgrade regression asserting each existing registration's identity tuple and
    computed trust key are unchanged, the new hooks are appended once, user hooks
    unchanged, and a second run is idempotent.
- [ ] AC17: `sce setup --codex` merge is idempotent for the mutation-scope
  registrations (a second run produces byte-identical output) and a
  structurally invalid existing `.codex/hooks.json` fails before the atomic swap
  with the existing file untouched.
  - Validate: `codex_hook_config.rs` idempotency + malformed-input tests.
- [ ] AC17a: Every newly SCE-owned mutation-scope event has a
  `codex_hook_config::hook_event_key_label` entry whose label is the **exact
  upstream Codex key label** (verified against `openai/codex` source in T01/T05,
  not derived by lowercasing), with a dedicated test per newly registered event
  (D22).
  - Validate: `services::codex_hook_config` label tests, one per new event;
    a comment or `NOTES.md` citation of the upstream source for each label.
- [ ] AC18: `sce doctor` gives each SCE-owned mutation-scope registration the
  full three-dimension Codex health model (D21), reported independently of the
  four existing `sce hooks codex` registrations, proving all of:
  1. structural diagnosis (`PresentAndCurrent` / `Missing` / `Stale`,
     `Malformed` for the whole document);
  2. normal Codex trust diagnosis (`Trusted` / `Untrusted` / `Modified` /
     `Disabled`);
  3. effective project-hook policy diagnosis (`ProjectHooksAllowed` /
     `PolicyBlocked` / `PolicyUnknown`), reusing the single per-invocation
     `configRequirements/read` probe;
  4. `PresentAndCurrent` combined with `Untrusted` / `Modified` / `Disabled` /
     `PolicyBlocked` / `PolicyUnknown` is **never** reported healthy;
  5. `sce doctor --fix` changes only SCE-owned `.codex/hooks.json` structure;
  6. no `$CODEX_HOME/config.toml` or any trust-state / managed-policy mutation
     occurs on any doctor path.
  - Validate: `services::doctor::` tests covering a missing, a stale, an
    untrusted, a modified, a disabled, a policy-blocked, and a policy-unknown
    mutation-scope registration, plus the `--fix` path; an assertion that the
    existing trusted `sce hooks codex` hooks and the new (untrusted) mutation
    hooks are reported with distinct readiness rather than one flattened
    `.codex/hooks.json` status; a filesystem assertion that no `$CODEX_HOME`
    write occurs.
- [ ] AC19: Production Codex-adapter code (everything in
  `cli/src/services/hooks/codex_mutation_scope/` outside `#[cfg(test)]`)
  contains no `use` or qualified-path reference naming
  `crate::services::mutation_trace::{runtime,protocol,store}`,
  `RepositoryAgentTraceDb`, `WorktreeId`, or `GitSnapshotService`, and its only
  dependency into the mutation stack is the single seam import from
  `crate::services::hooks::mutation_scope`.
  - Validate: `rg -n --type rust
    '^\s*use\s+crate::services::mutation_trace::(runtime|protocol|store)|::(RepositoryAgentTraceDb|WorktreeId|GitSnapshotService)\b'
    cli/src/services/hooks/codex_mutation_scope/` returns no match outside a
    `#[cfg(test)]` module; manual check confirms exactly one `use` reaching
    `crate::services::hooks::mutation_scope`.
- [ ] AC20: Codex mutation-scope-only regressions leave `diff_traces`,
  `post_commit_patch_intersections`, `agent_traces`, `messages`, and `parts`
  unchanged (before/after row-count assertions), and adapter state lives only
  below `<git-dir>/sce/`.
  - Validate: T06 regression with row-count assertions; state-module inspection.
- [ ] AC21: Crash/recovery invariants hold against the real runtime: (a) a
  `pending_start` attempt whose `Start` never committed is abandoned then
  recovered by the quiescent flush; (b) a `pending_start` attempt whose `Start`
  did commit is abandoned as a real runtime abandonment, not a late `Start`;
  (c) a `Close` that committed durably before local bookkeeping caught up is
  replay-safe on redelivery (no second transition, revision unchanged).
  - Validate: T06 regressions Test-crash-a/b/c driving real events after
    simulating each crash point via the adapter's own bookkeeping helpers only.
- [ ] AC22: The diff against
  `origin/claude-mutation-scope-integration` is empty for
  `spec/mutation_cursor.qnt`, `cli/src/services/mutation_trace/protocol.rs`,
  `cli/migrations/agent-trace-repository/`, and
  `config/schema/agent-trace.schema.json`; no mutation-trace SQL migration and
  no Quint/protocol/attribution-algorithm change is introduced.
  - Validate: `git diff origin/claude-mutation-scope-integration -- <those
    paths>` is empty.
- [ ] AC23: Durable context clearly separates the generic mutation-scope
  ingress, the Codex mutation adapter, and the mutation runtime, and records:
  the tool-execution scope model, the **three-class** Codex tool classification
  (`TrackedMutation` / `Delegation` / `Untracked`) and the **partial-by-tool-
  surface coverage boundary** (D23), execution identity,
  `ScopeId`/`EventId` derivation, the fail-closed `PreToolUse` (tracked only) and
  its exact Codex-native response, the terminal-boundary mappings, the
  failed-tool handling, why MCP/unknown are `Untracked` (the three T01 probe
  findings), the Codex lifecycle cleanup signals and the load-bearing backstop,
  the recovery barrier, worktree/cwd ownership, the concurrency story (including
  that `AiExclusive` is tracked-scope exclusivity, not sole authorship), the
  exact background/detached execution limitations, and the Codex hook-config
  coexistence contract (existing-registration trust-identity preservation, the
  three-dimension doctor health model, upstream-verified event key labels) —
  each stated as Codex-proven, Codex-documented, or Codex-unsupported, with the
  tested Codex version. Durable context must include the coverage table:
  `Tracked: Bash, apply_patch` / `Delegation: collaborationspawn_agent,
  collaborationwait_agent` / `Allowed but untracked: mcp__*, unknown tools`, and
  the sentence that **Codex MCP calls remain usable but their filesystem
  mutations are not individually attributed by the Codex mutation-scope
  adapter**.
  - Validate: inspection of `context/cli/codex-mutation-scope-integration.md`
    and the updated cross-reference files.
- [ ] AC24: The plan's exact unsupported / out-of-coverage limitations are
  enumerated in durable context:
  - **MCP tools and unknown/future Codex tools are outside Codex mutation-scope
    attribution coverage** (D23, direction B). They execute and may mutate; the
    adapter creates no scope. Durable context records *why*: on codex-cli
    0.153.4, if MCP were modeled as a scope it would be D10a Case C
    (mutate-then-error has no terminal hook; a successor can start with no
    cleanup signal between; parallel MCP execution is real). This is a coverage
    boundary, not a shipped-then-broken feature. Wording must not imply SCE knows
    MCP is read-only, that MCP mutations are detected immediately, or that
    `AiExclusive` proves sole authorship.
  - No line-level attribution for mutations from a failed tool with no terminal
    hook; the failed-tool -> successor-tool guarantee is Case A for built-in
    `Bash` / `apply_patch` only.
  - No attribution guarantee for self-detaching descendant processes; no
    Codex-managed background execution SCE cannot bound; and whatever else T01
    marks `UNSUPPORTED`.
  - Future work is explicitly named: investigate first-class MCP mutation
    attribution using a richer lifecycle mechanism in a separate PR.
  - Validate: inspection of the "Unsupported / Coverage boundary" section of
    `context/cli/codex-mutation-scope-integration.md`.

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::codex_mutation_scope`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::codex`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::codex_hook_config`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::doctor::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix run .#pkl-check-generated`
- `nix flake check`
- `git diff origin/claude-mutation-scope-integration -- spec/mutation_cursor.qnt cli/src/services/mutation_trace/protocol.rs cli/migrations/agent-trace-repository/ config/schema/agent-trace.schema.json` must be empty.

Final branch comparison is against `claude-mutation-scope-integration`
(#263 head), not `main`, while this PR remains stacked on #263.

### Context sync

- New: `context/cli/codex-mutation-scope-integration.md` (owns the Codex adapter
  domain — see AC23/AC24). Must document, in public/durable language, the
  **partial-by-tool-surface coverage boundary** (D23):

  ```text
  Tracked:            Bash, apply_patch
  Delegation:         collaborationspawn_agent, collaborationwait_agent
  Allowed but untracked:  mcp__*, unknown tools

  Coverage meaning:  mutation-scope attribution describes tracked-scope
                     exclusivity, not exhaustive authorship of every
                     filesystem mutation.
  ```

  and *why* MCP is untracked, citing the T01 probes: mutate-then-error has no
  terminal hook (probe 13); a successor can start immediately with no cleanup
  signal (probe 14); parallel MCP execution is real (probes 16/17). Plus the
  sentence: "Codex MCP calls remain usable, but their filesystem mutations are
  not individually attributed by the Codex mutation-scope adapter."
- Update: `context/cli/mutation-scope-runtime.md` (a second concrete adapter now
  exists; Codex is no longer "unwired"),
  `context/cli/mutation-scope-hook-ingress.md` (a second in-process seam
  consumer now exists),
  `context/sce/agent-trace-hooks-command-routing.md` (new
  `codex-mutation-scope` route, or the new `sce hooks codex` mutation-scope
  arms if T02 chooses D17 option 2),
  `context/sce/codex-integration-runtime.md` (the Codex hook runtime now also
  drives mutation-scope; keep the existing conversation/diff pipeline
  description intact and additive),
  `context/context-map.md`, `context/overview.md`, `context/architecture.md`
  (line 135's hook-runtime paragraph names the new adapter).
- Not a target: `context/sce/generated-opencode-plugin-registration.md`,
  `context/cli/claude-mutation-scope-integration.md` (Claude-only — unchanged),
  and any `context/decisions/2026-08-23-codex-*` ADR unless T02/T05's ownership
  extension materially changes the accepted non-destructive-ownership contract
  (in which case a **new dated** ADR is written, never an edit to the existing
  one).
- `context/sce/doctor-human-text-contract.md` — update only if T05's
  three-dimension mutation-scope health rows change the documented `sce doctor`
  human text layout.
- ADR: only if T01/T02/T05 reveals a genuinely new system-wide architectural
  constraint meeting the repository's ADR threshold (e.g. the Codex hook-config
  ownership model must become permanently multi-command, or existing-hook trust
  identity must be a first-class merge invariant). A routine extension of
  `REQUIRED_EVENTS` / `CODEX_COMMAND_WORDS` / `hook_event_key_label` does not
  qualify.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/cli_schema.rs`,
  `cli/src/services/parse/command_runtime.rs`,
  `cli/src/services/hooks/mod.rs`,
  `cli/src/services/hooks/mutation_scope.rs` (no change expected beyond the
  already-`pub(crate)` seam; touch only if T04 finds a genuine gap),
  `cli/src/services/hooks/codex_mutation_scope/mod.rs` (new — event model, tool
  classification, identity, `ScopeId`/`EventId` derivation, adapter driver),
  `cli/src/services/hooks/codex_mutation_scope/state.rs` (new — durable
  checkout-local bookkeeping),
  `cli/src/services/hooks/codex_mutation_scope/fixtures/` (new — T01 raw Codex
  hook-event fixtures + `NOTES.md`),
  `cli/src/services/codex_hook_config.rs` (command-aware ownership/merge per
  D17; additive position-stable insertion preserving existing trust identity per
  D20; `hook_event_key_label` coverage for new events per D22),
  `config/pkl/renderers/codex-content.pkl` (mutation-scope `.codex/hooks.json`
  registrations, appended after the existing SCE registrations),
  `cli/src/services/doctor/` (three-dimension structural + trust + effective-
  policy diagnosis for each mutation-scope registration per D21),
  and the context files listed under Context sync.
- **Out of scope:** OpenCode/Pi mutation-scope adapters; a generic
  adapter-framework extraction; any change to the mutation protocol, Quint
  model, `mutation_trace/protocol.rs`, `mutation_trace/store.rs`, or
  `mutation_trace/runtime/`; any mutation-trace SQL migration; any Agent Trace
  schema migration or `config/schema/agent-trace.schema.json` change; any new
  mutation-attribution algorithm; the existing `sce hooks codex`
  conversation/diff pipeline behavior (`UserPromptSubmit`/`Stop`/
  `PreToolUse(Bash)`/`PostToolUse(apply_patch)` slices — unchanged); folding
  `apply_patch -> diff_traces` into mutation-scope storage; PID/process-group
  supervision, background-process ownership, shell-command static analysis,
  staleness polling; a Codex App Server / `codex exec --json` integration for
  mutation-scope; writing Codex hook-trust or auto-trust state.
- **Constraints:** the adapter depends only on `hooks::mutation_scope`, never on
  `mutation_trace::runtime` / `::protocol` / `::store` directly
  (`codex_mutation_scope -> mutation_scope -> mutation_trace::runtime`); it may
  call `checkout::resolve_git_dir(cwd)` but not `read_checkout_id` /
  `get_or_create_checkout_id` / `resolve_checkout_id_for_repo` and must not
  construct a `WorktreeId`; the adapter-state lock is never held across a
  `hooks::mutation_scope` invocation; `ScopeId` uses a length-prefixed tuple
  encoding, no hashing / no crypto dependency; latest deps pinned exactly,
  node24 for any new JS work per `context/plans/feedback_deps.md` (no new deps
  expected here); reuse the shared `codex_hook_config.rs` ownership/merge and
  the shared doctor structural/trust/policy diagnosis — no second Codex
  hook-config implementation; reuse `ActorKind::Codex` /
  `"actor_kind":"codex"`, already accepted by the generic ingress.
- **Non-goal:** copying Claude's `PostToolUseFailure` / `PermissionDenied` /
  `StopFailure` / `WorktreeRemove` mappings into Codex without T01 evidence that
  the equivalent Codex signal exists and fires; designing a Close-on-failure
  path around an event Codex does not reliably emit; treating any terminal hook
  as proof that every descendant process has stopped mutating; turning `abandon`
  into a `RuntimeBoundary`; a long-lived Codex "session" or "agent" scope;
  inventing a Codex `agent_id` abstraction if Codex exposes no delegated-agent
  identity; **modeling an MCP call or an unknown Codex tool as a mutation scope**
  (D23 — they are `Untracked` in v1: allowed, may mutate, no scope, no
  bookkeeping); **denying an MCP or unknown `PreToolUse` for being untracked**;
  **claiming MCP is read-only or that its mutations are individually attributed**;
  first-class MCP mutation attribution (direction C — deferred to a separate PR).

## Assumptions

- Task numbering is `T01..T07`. T04 and T05 of the change request's suggested
  shape (driver, and command/routing) are combined into this plan's T04, because
  the D17 command decision is made in T02 and the Claude adapter's own precedent
  combined driver + command wiring in one task (#263 T06).
- The generic seam is the existing
  `hooks::mutation_scope::run_mutation_scope_from_payload(repository_root,
  stdin_payload, logger) -> Result<String>`, already `pub(crate)` since #263's
  T05, reused verbatim by constructing the `{"operation":...}` JSON wire string.
  No new payload operation and no `sce` subprocess.
- `ActorKind::Codex` and the `"actor_kind":"codex"` wire value are already
  accepted by `parse_mutation_scope_payload` (per
  `context/cli/mutation-scope-hook-ingress.md`), so the Codex adapter needs no
  ingress change to identify itself.
- Adapter state path is `<git-dir>/sce/codex-mutation-scope-state.json` with
  lock `<git-dir>/sce/codex-mutation-scope-state.lock`, following the
  `checkout::persist_checkout_id_inner` durability pattern.
- Generated Codex mutation-scope hook registrations carry the matchers T01
  proves Codex requires (an unmatched group if Codex supports it, else
  per-tool matched groups); the exact set of registered events is the minimum
  the adapter actually uses after T01, not Claude's ten.
- The user has explicitly allowed assumptions for ordinary local choices
  (module naming, test-helper shape, fixture layout) — these follow the Claude
  adapter's precedent and are not blocking.
- The Codex version SCE supports is pinned by T01 (`codex --version` plus the
  inspected upstream commit), the same way #263's T01 pinned Claude Code
  `2.1.258`.

## Task stack

- [x] T01: `Freeze the real Codex hook and lifecycle contract` (status:done)
  - Task ID: T01
  - Built-in probes completed: 2026-09-07
  - MCP lifecycle extension completed: 2026-09-07 (probes 12–17)
  - Re-planning resolution recorded: 2026-09-08
  - **Re-planning resolution (2026-09-08) — direction B chosen.** T01 discovered
    D10a **Case C for MCP** on codex-cli 0.153.4: a mutation-capable MCP tool can
    mutate a git-visible file then return `is_error:true` with **no terminal
    hook** (probe 13); no cleanup signal reaches the adapter before a successor
    `PreToolUse` (probe 14); same-lane MCP executions **overlap** (probes 16/17).
    Re-planning chose **direction B**: MCP tools and unknown/future Codex tools
    remain usable but are **outside the Codex adapter's mutation-scope
    attribution coverage** (D23) — classified `Untracked`, no scope, no
    bookkeeping. The Case C finding is **retained** and is exactly *why* MCP is
    excluded: "if MCP were represented as a mutation scope, Codex 0.153.4 makes
    the lifecycle unsafe." No protocol / Quint / mutation-trace SQL / Agent Trace
    schema change is required (D23). Design decisions updated: D1, D2, D8, D9,
    D10, D10a, D12, D13, D14, new D23; ACs updated: AC3, AC9, AC9a, AC9b (now
    "MCP/unknown pass-through"), AC9c–AC9f (new), AC10, AC11, AC12, AC23, AC24;
    tasks updated: T02 unblocked, T06 matrix. The built-in `Bash` / `apply_patch`
    evidence and all non-MCP dispositions (D3, D8, D9-built-in, D10-built-in,
    D10a-built-in, D12, D15, D16, D17-inputs, D22) remain valid and are **not**
    re-opened.
  - **T01 → done; T02 → unblocked** (not started as part of this re-planning).
  - Files changed:
    - `cli/src/services/hooks/codex_mutation_scope/fixtures/` (new — 35 raw
      byte-for-byte built-in hook-event captures across 11 probes + one
      `probe09-*.evidence.json`; **plus 21 raw MCP hook-event captures + 5
      `probe1[3-7]-*.evidence.json` files across MCP probes 12–17**; `NOTES.md`)
    - `cli/src/services/hooks/codex_mutation_scope/fixtures/mcp_probe/` (new —
      probe-only, not runtime: `server.py` zero-dep stdio MCP server, `dump.sh`,
      `block.sh`, `run-probes.sh` driver, `config.toml.sample`,
      `hooks.json.sample`)
    - `flake.nix` (`./cli/src/services/hooks/codex_mutation_scope/fixtures`
      already in `workspaceSrc`; `mcp_probe/` is under it — no change needed)
    - `context/plans/codex-mutation-scope-integration.md` (T01 dispositions
      written into D1, D2, D3, D8, D9, D10, D10a, D12, D15, D16, D17-inputs,
      D22; MCP extension dispositions written into D1/D2/D8/D9/D10/D10a/D14;
      **2026-09-08 re-planning direction B written into the Change summary, D1,
      D2, D8, D9, D10, D10a, D12, D13, D14, new D23, AC3/AC9/AC9a/AC9b/
      AC9c–AC9f/AC10/AC11/AC12/AC23/AC24, T02/T06, Open questions**; this task
      record)
  - Result: Froze the Codex hook/lifecycle contract for **codex-cli 0.153.4**
    (model `gpt-5.6-sol`), cross-checked against upstream `openai/codex` tag
    `rust-v0.153.4` (commit `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`) —
    generated hook JSON schemas, `codex-rs/hooks/src/{schema.rs,lib.rs}`,
    `codex-rs/core/src/{tools/registry.rs,hook_runtime.rs}`. All 11 live probes
    captured (normal apply_patch + shell success, shell partial-write-then-fail,
    two `PreToolUse`-hook denial shapes, tool vocabulary, apply_patch
    verification failure, SIGINT with/without an `Interrupt` hook, subagent
    delegation, self-detaching descendant with Git-observability evidence,
    linked-worktree cwd). Built-in key findings: **D10a is Case A for
    `Bash` / `apply_patch`** (a failed shell tool still fires `PostToolUse`; a
    failed `apply_patch` writes nothing; interruption ends the turn);
    **`SessionEnd` is the load-bearing cleanup backstop**, `Interrupt` is a
    newly-discovered earlier interruption signal (**Codex has 12 hook events,
    not 11**); **both** `{"decision":"block"}` and
    `hookSpecificOutput.permissionDecision:"deny"` block a tool; Codex runs
    **built-in** mutation-capable tools serially; Codex **does** expose a
    delegated-agent identity (`agent_id`, subagent events only); raw hook `cwd`
    is authoritative including for linked worktrees; the default `codex exec`
    shell tool has **no `run_in_background` parameter** but a self-detaching
    descendant is an unsupported boundary as for Claude.

    **MCP lifecycle extension (probes 12–17, codex-cli 0.153.4, cross-checked
    against `codex-rs/core/src/tools/{registry.rs,context.rs,handlers/mcp.rs}`
    and `codex-rs/config/src/mcp_types.rs` at `rust-v0.153.4`)** — a tiny local
    stdio MCP server (`fixtures/mcp_probe/server.py`) exposing deliberately
    mutation-capable tools was wired into a scratch repo and driven with
    `codex exec`. Findings: MCP tool naming is `mcp__<server>__<tool>` with an
    `exec-<uuid>` `tool_use_id` (D2/D3 unchanged for MCP); a **successful** MCP
    call emits `PostToolUse` (D9); a **blocked** MCP call emits `PreToolUse`
    only (D8); **but a mutation-capable MCP tool that mutates a git-visible file
    and then returns `is_error:true` receives NO terminal hook** (probe 13), a
    failed MCP tool is followed **directly** by a successor MCP tool with **no
    intervening cleanup signal** (probe 14), and **two mutation-capable MCP
    executions run genuinely concurrently** (probes 16/17,
    `supports_parallel_tool_calls` config key / `annotations.readOnlyHint`).
    **This is D10a Case C for MCP if MCP were modeled as a scope.** Re-planning
    (2026-09-08) chose **direction B**: MCP and unknown tools are `Untracked` —
    they execute and may mutate, but the adapter creates no scope, so the Case C
    lifecycle can never produce a zombie scope (D23). Built-in `Bash` /
    `apply_patch` evidence is unaffected. Parallel MCP execution remains real
    operationally but produces no tracked scopes and therefore no
    MCP-derived `AiContended`.
  - Verify:
    - `nix run .#pkl-check-generated` — **passed** (built-in probes; re-run
      after the MCP-extension fixtures land).
    - `nix flake check` — **passed** ("all checks passed!"; incompatible
      non-Linux systems omitted as usual; re-run after the MCP-extension
      fixtures land).
    - Built-in fixtures committed under
      `cli/src/services/hooks/codex_mutation_scope/fixtures/`; `NOTES.md` lists
      the manifest and per-probe disposition; all JSON fixture files parse; CLI
      build input list already includes the fixtures directory (`flake.nix`).
    - MCP-extension: 21 raw MCP hook payloads + 5 `evidence.json` files + the
      `mcp_probe/` probe harness committed under the same fixtures tree; all
      parse; the six MCP probes were driven live against `codex exec`
      (`codex-cli 0.153.4`).
  - Context impact: domain — a new adapter-domain fixture corpus + frozen Codex
    hook-contract facts now exist; no code, no user-visible behavior, no public
    interface yet. Durable Codex-adapter context (`context/cli/
    codex-mutation-scope-integration.md`) is authored by T07 once behavior
    ships; T01's facts live in the plan's Design section and the fixtures
    `NOTES.md` until then.
  - Scope: In — capture raw Codex hook-event fixtures from the Codex version SCE
    chooses to support, commit them under
    `cli/src/services/hooks/codex_mutation_scope/fixtures/` (one file per probe,
    named for the probe) with a `NOTES.md` recording the tested `codex --version`
    and the inspected `openai/codex` commit; inspect current upstream Codex
    source/documentation where a live probe is not possible. For each probe
    record, in `NOTES.md` and back into this plan's Design section, a
    disposition of `PROVEN` / `DOCUMENTED — NON-LOAD-BEARING` /
    `ASSUMPTION — PROBE` / `UNSUPPORTED`. Probes:
    - **Normal mutation-capable execution** — `apply_patch` success, shell/`Bash`
      command success, and any other checkout-mutating Codex tool: capture
      `PreToolUse` and the terminal hook, recording `tool_use_id`,
      `session_id` / `turn_id`, `cwd`, `tool_name`, `tool_input`,
      `tool_response`, `model`, and any timestamp. Establish whether the same
      stable execution identity appears in both the pre and post events (D3/D9).
    - **Tool failure** — a tool that writes files then exits non-zero / fails:
      determine the exact event sequence, whether `PostToolUse` fires at all on
      failure, and whether any event carries a reliable final mutation
      observation. Do **not** assume success and failure use the same sequence
      (D10).
    - **Failed tool followed by another tool in the same turn** (D10a — the
      hard case): (1) execute mutation-capable tool A; (2) make A mutate
      Git-visible repository state; (3) make A fail; (4) do **not** end the
      turn; (5) cause mutation-capable tool B to execute; (6) capture every
      hook/lifecycle event between A's failure and `PreToolUse(B)`. Establish
      whether Codex emits any **positive** stale/terminal evidence for A before
      B, and record exactly one disposition:
      - **Case A** — a reliable intermediate cleanup signal exists between A's
        failure and `PreToolUse(B)`; record which signal is load-bearing.
      - **Case B** — no intermediate signal, but Codex is proven to execute
        mutation-capable tools **serially within a narrow, identity-defined
        lane** (candidate lane keys: session / turn / a proven delegated-agent
        identity), so `PreToolUse(B)` itself proves an older same-lane attempt
        cannot still be running. Record the exact concurrency evidence and the
        lane key it licenses. Codex tools have historically run serially — this
        case needs a positive proof of the seriality boundary, not an
        assumption.
      - **Case C** — neither: no reliable intermediate signal **and** same-lane
        parallel executions are possible, so the adapter cannot distinguish
        `failed-and-dead A` from `still-running A`. Mark this an architectural
        contradiction / unsupported lifecycle, record it in Open questions, and
        **stop the plan for re-planning** — do not guess.
    - **Tool denial** — SCE `PreToolUse` policy denies; Codex itself denies;
      `PermissionRequest` denied; another hook denies (if applicable):
      determine whether a `Start` could have been durably established with no
      corresponding terminal event, and the exact Codex-native denial/block
      response shape and exit semantics for the supported version (D8/D12).
    - **Interrupted execution / turn / session lifecycle** — user interruption,
      turn completion, session termination, process termination: which of
      `Stop` / `SessionEnd` / `SubagentStop` / `PreCompact` fire, with what
      identity fields, and which provide **positive** staleness evidence
      suitable for `abandon` (not absence of activity) (D12).
    - **Parallel execution** — whether Codex can have two mutation-capable
      executions overlapping; if yes, capture evidence of two coexisting
      independent tool executions (D1/D14). **Done for MCP** — probes 16/17
      reproduce genuine overlap; the plan may not conclude seriality from the
      built-in probes alone.
    - **MCP lifecycle (probes 12–17)** — a local MCP server exposing
      mutation-capable tools (`mutate_success`, `mutate_then_error`,
      `slow_mutate`, `read_only_liar`): capture the full hook lifecycle for a
      successful MCP call, a blocked MCP call, a mutate-then-error MCP call, a
      failed MCP call followed by a successor MCP call in the same turn, and two
      MCP calls forced to run in parallel; record git-observable mutation
      evidence (not just the MCP result) distinguishing "tool did not mutate"
      from "tool mutated then returned failure"; determine whether `PostToolUse`
      fires and whether any positive cleanup signal precedes a successor; pin the
      Codex version and cite the upstream `success_for_logging` /
      `supports_parallel_tool_calls` source. Record the D10a MCP disposition
      (Case A / B / C) explicitly and, for Case C, stop the plan for re-planning.
    - **Subagents / delegated execution** — whether Codex exposes nested/
      delegated agent execution and whether hook payloads carry enough identity
      to distinguish a delegated agent from the main thread (D3). Do not invent
      an `agent_id` if Codex has none.
    - **Worktrees / cwd** — whether the raw hook `cwd` is authoritative for the
      actual checkout being mutated, and whether Codex exposes any separate
      worktree-lifecycle event or path (D15).
    - **Background / detached shell** — separate Codex-managed background
      execution (if any) from a foreground command spawning a self-detaching
      descendant; capture Git-observable evidence for the descendant case, as
      #263's T04 did for Claude, before making any Codex-specific claim (D16).
    - **Tool vocabulary** — enumerate every `tool_name` Codex emits on
      `PreToolUse` / `PostToolUse`, including MCP tool naming and the delegation
      tool name, for the D2 classification table.
    Out — any production code, any Rust module, any settings change, any Design
    decision that is not backed by a captured fixture or an upstream source
    citation.
  - Dependencies: none
  - Done when: fixtures exist for every probe correctness depends on (or an
    upstream-source citation where a live probe is impossible), `NOTES.md`
    records the tested Codex version and per-probe dispositions, and every
    T01-GATED decision (D1, D2, D3, D8, D9, D10, D10a, D12, D15, D16,
    D17-inputs) carries an explicit disposition written back into this plan's
    Design section — D10a specifically records Case A / Case B / Case C with its
    evidence. **Met:** all fixtures committed; `NOTES.md` complete; D10a records
    Case A (built-ins) and Case C *if modeled as a scope* (MCP). The Case C
    finding triggered the re-planning that chose direction B (D23) — MCP/unknown
    are `Untracked` and need no protocol change, so the plan proceeds to T02.
  - Verify (planned): fixtures committed and referenced from this plan;
    `NOTES.md` lists the manifest and per-probe disposition;
    `nix run .#pkl-check-generated` and `nix flake check` still pass (fixtures
    are inert data — confirm the CLI build input list includes the new fixtures
    directory, as #263's T01 needed for Claude).
  - Context synchronization: synced
    - T01 ships **no code, no public interface, and no user-visible behaviour** —
      only the fixture corpus, the frozen Design-section dispositions, and (as of
      2026-09-08) the recorded re-planning direction B (D23). The durable
      cross-reference files (`context/cli/...`, `context/sce/...`) are authored
      by **T07** once behaviour ships; there is nothing for T01 to sync into them
      now, exactly as recorded under "Context impact". The re-planning facts live
      in this plan's Design section, Open questions, and
      `cli/src/services/hooks/codex_mutation_scope/fixtures/NOTES.md`.

- [x] T02: `Command architecture, Codex event model, classification, and identity` (status:done)
  - Task ID: T02
  - Completed: 2026-09-08
  - **Unblocked (2026-09-08).** The T01 D10a Case C blocker is resolved by
    re-planning direction B (D23): MCP tools and unknown tool names are
    `Untracked` — they execute and may mutate, but the adapter creates no scope,
    no `Start`, and no bookkeeping for them, so no zombie-scope lifecycle exists.
    `classify_tool` is now a three-way decision (`TrackedMutation` /
    `Delegation` / `Untracked`, D2) with a frozen membership; no protocol change
    is required. T02 has not been started.
  - Scope: In — (1) decide the D17 command architecture against T01 + the code,
    defaulting to a separate hidden `sce hooks codex-mutation-scope` command,
    and write the decision into D17; (2) `cli/src/services/hooks/
    codex_mutation_scope/mod.rs`: the strict raw Codex mutation-scope event
    parser (rejecting empty/non-object/missing/blank/wrong-typed with
    `Invalid Codex hook event payload from STDIN: <detail>.`), the supported
    mutation-scope hook-event enum (only the events T01 proved), the D2
    **three-class** `classify_tool` (`TrackedMutation` = `Bash` / `apply_patch`;
    `Delegation` = `collaborationspawn_agent` / `collaborationwait_agent`;
    `Untracked` = `mcp__*` and any unknown `tool_name`) with no
    "mutation-capable therefore Start" language for MCP/unknown, the D3
    execution-key type frozen from T01 evidence (tracked tools only), the D4
    length-prefixed `cx-tool-v1|n=..|...` `ScopeId` formatter, and the
    `<scope-id>|start` / `<scope-id>|close` `EventId` formatters, plus any
    background-execution classifier T01 shows is needed (model/classify only —
    the denial is T04's). Out — any durable state, any runtime/ingress call, any
    CLI wiring, any generated settings; **no D10a successor-barrier / lane-key
    work** (Case A for built-ins ships none; MCP/unknown are `Untracked`).
  - Dependencies: T01
  - Done when: the module compiles behind the existing `hooks` module tree; the
    D17 decision is recorded in this plan; the D10a lane key is explicitly N/A
    (recorded in D10a); unit tests prove AC2, AC3 (the three-class table,
    including `Untracked` for `mcp__*` and unknown names, producing no
    processed-event keys), AC4 (formatter determinism), AC5 (formatter is a
    function of `attempt_seq`), against T01's tool vocabulary.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope`; `clippy` /
    `fmt` clean.
  - Files changed:
    - `cli/src/services/hooks/codex_mutation_scope/mod.rs` (new — event model,
      strict parser, three-class `classify_tool`, D3 `AttemptKey`, D4
      `cx-tool-v1` `ScopeId` / `EventId` formatters, 23 unit tests over the T01
      fixture corpus)
    - `cli/src/services/hooks/mod.rs` (one line — `pub mod codex_mutation_scope;`
      module-tree declaration; no CLI wiring)
    - `context/plans/codex-mutation-scope-integration.md` (D17 T02 decision =
      option 1; D10a lane-key N/A confirmation; this task record)
  - Result: Froze the Codex mutation-scope adapter's foundation layer. D17
    decided as **option 1** — a separate hidden `sce hooks codex-mutation-scope`
    command — recorded in D17 with the failure-posture, process-isolation, and
    `#263`-precedent rationale. `codex_mutation_scope/mod.rs` contains:
    `parse_codex_hook_event` (strict — empty / non-object / invalid-JSON /
    unsupported `hook_event_name` / missing / blank / wrong-typed all rejected
    with `Invalid Codex hook event payload from STDIN: <detail>.`, no fabricated
    identity); `CodexHookEvent` limited to the six events the adapter registers
    (`PreToolUse`, `PostToolUse`, `Stop`, `Interrupt`, `SubagentStop`,
    `SessionEnd`); identity types carrying Codex's `turn_id` and subagent-only
    `agent_id` / `agent_type`; `classify_tool` -> `TrackedMutation`
    (`Bash`, `apply_patch`) / `Delegation` (`collaborationspawn_agent`,
    `collaborationwait_agent`) / `Untracked` (`mcp__*` + any unknown name), with
    no "mutation-capable therefore Start" path; `AttemptKey`
    `(session_id, agent_id?, tool_use_id)` (turn_id excluded); `format_codex_scope_id`
    (`cx-tool-v1|n=<seq>|s=<len>:<sid>|a=<len>:<aid>|t=<len>:<tuid>`, length-prefixed,
    hash-free) and `codex_scope_{start,close}_event_id`. No durable state, no
    ingress call, no CLI wiring, no generated settings, no D10a successor-barrier
    (D10a lane key confirmed **N/A**). No background-execution classifier — T01
    D16 proved the `codex exec` shell tool has no `run_in_background` parameter.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
      cli/Cargo.toml services::hooks::codex_mutation_scope` — **passed** (23
      tests: AC2 parser/fixtures, AC3 classification table + delegation/untracked
      fixtures, AC4 formatter determinism + length-prefix disambiguation, AC5
      fresh-seq / turn_id-excluded).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
      cli/Cargo.toml services::hooks::` — **passed** (354 tests; existing
      `sce hooks codex` / mutation-scope suites unaffected).
    - `clippy --all-targets -- -D warnings` — **clean**.
    - `cargo fmt -- --check` — **clean** (after `cargo fmt`).
  - Context impact: domain — new adapter-domain Rust module (event model,
    classifier, identity/formatter contract) plus the D17 command-architecture
    decision and the D10a N/A confirmation now exist in the plan's Design
    section. No user-visible behaviour, no public CLI interface, no generated
    config yet (all deferred to T04/T05). Durable Codex-adapter context
    (`context/cli/codex-mutation-scope-integration.md`) is authored by T07 once
    behaviour ships; the frozen contract lives in the plan's Design section
    (D2/D3/D4/D17) and this record until then.
  - Context synchronization: synced
    - T02 ships an internal Rust foundation module with **no non-test caller**
      (no CLI wiring, no ingress call, no `sce setup` registration), no
      user-visible behaviour, no public interface, and no generated config. The
      D17 / D10a decisions are plan-internal design state, not durable context.
      The durable Codex-adapter context
      (`context/cli/codex-mutation-scope-integration.md` and the cross-reference
      edits to `context/overview.md`, `context/context-map.md`,
      `context/cli/mutation-scope-hook-ingress.md`, etc.) is authored by **T07**
      once behaviour ships — exactly as recorded for T01. Mandatory five-root
      pass done: `overview.md`, `architecture.md`, `glossary.md`, `patterns.md`,
      `context-map.md` all read and confirmed not contradicted (each still
      correctly states Codex has no wired mutation-scope adapter). No
      architecture decision qualified for an ADR.

- [x] T03: `Durable checkout-local Codex adapter state and recovery bookkeeping` (status:done)
  - Task ID: T03
  - Completed: 2026-09-08
  - Scope: In — `cli/src/services/hooks/codex_mutation_scope/state.rs`: the
    versioned `{version, next_attempt_seq, recovery_pending, attempts[]}` store
    at `<git-dir>/sce/codex-mutation-scope-state.json`, the
    `checkout::persist_checkout_id_inner`-style durable write with its own lock
    at `<git-dir>/sce/codex-mutation-scope-state.lock` (never held across a seam
    call), the `pending_start | active` phase model, `allocate_attempt` /
    `mark_active` / `remove_attempt` / `mark_recovery_pending` /
    `clear_recovery_pending` helpers (the same set #263's Claude adapter needed
    after its follow-up), malformed/wrong-version rejection, and the D5 reasoning
    (confirm Codex hook invocations are independent processes for the supported
    version and record it). Out — any event parsing, any runtime/ingress call,
    any driver logic.
  - Dependencies: T01, T02
  - Done when: unit tests prove attempt allocation is monotonic and
    checkout-local, a terminal attempt is followed by a fresh `attempt_seq` (AC5
    state half), the store round-trips durably, a malformed/wrong-version file is
    rejected not fabricated, and the state lock is provably released before any
    external call boundary (helper-level test).
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope`.
  - Files changed:
    - `cli/src/services/hooks/codex_mutation_scope/state.rs` (new — the versioned
      `{version, next_attempt_seq, recovery_pending, attempts[]}` store at
      `<git-dir>/sce/codex-mutation-scope-state.json` with its own lock at
      `<git-dir>/sce/codex-mutation-scope-state.lock`; `pending_start | active`
      phase model; `allocate_attempt` / `mark_active` / `remove_attempt` /
      `mark_recovery_pending` / `clear_recovery_pending` helpers; the
      `checkout::persist_checkout_id_inner`-style lock → temp file → `sync_data`
      → atomic rename → best-effort parent-dir `sync_all` durable write;
      malformed / wrong-version rejection; 20 unit tests)
    - `cli/src/services/hooks/codex_mutation_scope/mod.rs` (one line —
      `pub(crate) mod state;`)
  - Result: Added the Codex adapter's durable checkout-local bookkeeping layer,
    structurally mirroring #263's `claude_mutation_scope::state` (post-follow-up
    helper set). The store is `AdapterState` (`version` = 1, `next_attempt_seq`,
    `recovery_pending`, `attempts: Vec<AdapterAttempt>`); each `AdapterAttempt`
    carries `attempt_seq`, the D4 `scope_id` (from `format_codex_scope_id`), the
    D3 identity fields (`session_id`, `agent_id?`, `tool_use_id`), `tool_name`,
    and `phase` (`PendingStart | Active`). `allocate_attempt` reuses a live
    attempt on a duplicate key (same `attempt_seq` / `ScopeId`, no counter
    advance) and otherwise draws a fresh monotonic `attempt_seq`; a removed
    (terminal) attempt is never reused — a later same-`tool_use_id` execution
    draws a new `attempt_seq` and a new `ScopeId` (AC5 state half). Writes go
    through a `try_lock`-based `AdapterStateLock` (separate `.lock` file, D6) and
    the durable temp-file/`sync_data`/atomic-rename pattern; the lock is released
    before each helper returns and is never held across an external boundary
    (D6). Malformed JSON and any `version != 1` file are rejected, never
    fabricated (D5). **D5 process-isolation reasoning confirmed and recorded
    here:** T01 proved Codex runs every registered hook handler as its own
    short-lived OS process (plan line ~972), so a `PreToolUse` process and the
    later `PostToolUse` process share no memory — the durable cross-process
    store is required, exactly as for Claude; the store shape does not change.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
      cli/Cargo.toml services::hooks::codex_mutation_scope` — **passed** (43
      tests: 20 new `state::tests` covering monotonic + checkout-local
      allocation, duplicate-key reuse, terminal→fresh non-reuse, phase
      transition, unknown-`scope_id` rejection, durable round-trip,
      malformed / wrong-version rejection, interrupted-before-rename atomicity,
      leftover-lock-file, parallel writers, lock contention, lock-released-
      between-helpers, and the `<git-dir>/sce/` path boundary; plus the 23
      pre-existing T02 tests).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
      cli/Cargo.toml services::hooks::` — **passed** (374 tests; existing
      `sce hooks codex` / mutation-scope / Claude-adapter suites unaffected).
    - `clippy --all-targets -- -D warnings` — **clean**.
    - `cargo fmt -- --check` — **clean** (after `cargo fmt`).
  - Context impact: domain — a new adapter-domain Rust module (durable state
    store + helpers) with **no non-test caller** (no event parsing, no
    runtime/ingress call, no driver, no CLI wiring — all deferred to T04). No
    user-visible behaviour, no public interface, no generated config. The D5/D6
    durability contract lives in the plan's Design section and the module doc
    comment; durable Codex-adapter context
    (`context/cli/codex-mutation-scope-integration.md`) is authored by T07 once
    behaviour ships, as for T01/T02.
  - Context synchronization: synced
    - T03 ships an internal Rust state module (`codex_mutation_scope::state`)
      with **no non-test caller** — no event parsing wiring, no runtime/ingress
      call, no driver, no CLI command, no `sce setup` registration — and no
      user-visible behaviour, public interface, or generated config. The D5/D6
      durability and process-isolation contract is plan-internal design state,
      not durable context. The durable Codex-adapter
      context (`context/cli/codex-mutation-scope-integration.md` and the
      cross-reference edits to `context/overview.md`, `context/context-map.md`,
      `context/cli/mutation-scope-hook-ingress.md`, etc.) is authored by **T07**
      once behaviour ships — exactly as recorded for T01/T02. Mandatory
      five-root pass done: `overview.md` (its "Codex … still have no adapter"
      statement remains accurate — nothing is wired), `architecture.md` (its
      `sce hooks codex` description is untouched and additive), `glossary.md`,
      `patterns.md`, `context-map.md` all read and confirmed not contradicted.
      No architecture decision qualified for an ADR — a durable adapter-state
      file mirroring the existing `claude_mutation_scope::state` pattern is a
      routine, reversible implementation detail with no boundary, interface,
      persistence-contract, or security-posture change (the store is explicitly
      not attribution evidence, not exported, not synced).

- [x] T04: `Codex mutation-scope driver + hidden command routing` (status:done)
  - Task ID: T04
  - Completed: 2026-09-08
  - Scope: In — per the D17 decision (default: separate command),
    `cli_schema::HooksSubcommand::CodexMutationScope` (hidden via
    `#[command(hide = true)]`), the `convert_hooks_subcommand_request` arm,
    `services::hooks::HookSubcommand::CodexMutationScope`, the
    `run_hooks_subcommand_in_repo` dispatch arm **unwrapped / non-fail-open**
    (mirroring the `MutationScope` and `ClaudeMutationScope` arms),
    `hook_runtime_invocation_name`, and the adapter driver in
    `codex_mutation_scope/mod.rs`: read one raw Codex hook JSON object from
    STDIN via `super::read_hook_stdin()`; resolve the raw `cwd` as
    `repository_root` and `git_dir` (bookkeeping only) as two independent
    parameters, never substituted; map each proven event —
    **`TrackedMutation`** `PreToolUse` -> D7 write-ahead `Start` + D8 fail-closed
    Codex-native block on any failure + D16 background-execution deny if
    applicable; **`Untracked`** (`mcp__*`, unknown) and **`Delegation`**
    `PreToolUse` -> Codex-neutral continue response, no `Start`, no attempt, no
    bookkeeping (D2/D8/D23); the proven success terminal hook for a tracked tool
    -> `Close` (D9); the proven tracked failure disposition from D10 (built-in
    Case A, no Close-on-failure path); the proven denial signal(s) -> `abandon`
    (D12); the proven session/turn/agent cleanup signals -> scoped `abandon`
    sweeps over adapter-owned tracked attempts only (D12, never global, never
    touching `Untracked` executions — there are none in state); D11
    uncertain-boundary rules and the D13 successor-Start invariant; the D13
    recovery barrier + one quiescent `flush`. **No D10a successor-barrier /
    lane-key code ships** (built-ins are Case A; MCP/unknown are `Untracked`).
    The driver reaches the runtime
    **only** through
    `super::mutation_scope::run_mutation_scope_from_payload` by building the
    generic wire payload as a string (D18/AC19). Inject the git-dir resolver and
    the seam as `&dyn Fn` parameters so every mapping is unit-testable without a
    real repo or DB. Fail-closed `PreToolUse` failures log via
    `sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed`. Out — generated
    `.codex/hooks.json` / `sce setup` wiring (T05), real Git/DB regressions
    (T06). If T02 chose D17 option 2, this task instead adds the mutation-scope
    `CodexDispatchArm` variants and the non-fail-open handling inside
    `run_codex_subcommand`, and the AC1/AC7 assertions target that surface.
  - Dependencies: T02, T03
  - Done when: focused unit tests with an injected seam cover every proven
    event-to-operation mapping, fail-closed `TrackedMutation` `PreToolUse` (exact
    Codex-native response JSON/exit, AC7), the `Untracked`/`Delegation`
    neutral-pass-through with an untouched state store (AC9b), the probe-13/14
    MCP sequences leaving no stale state / no zombie scope (AC9c/AC9d),
    write-ahead ordering (AC6), `pending_start` + terminal -> abandon (D11),
    failed `Close` -> abandon + `recovery_pending` (D11/AC13), the recovery
    barrier's branches (AC12), and the built-in Case A failed-A-then-B path
    (AC9a) — and, if T01 proved a deny applies, the background-execution deny
    (AC15). AC1 routing test passes;
    `sce hooks codex-mutation-scope </dev/null` shows the strict-parser error
    and `sce hooks --help` omits it. AC19 dependency-boundary grep is clean.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope`; `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    services::hooks::mutation_scope`; `services::parse::` routing tests; live
    binary check of the hidden command; AC19 grep.
  - Files changed:
    - `cli/src/cli_schema.rs` (new hidden `HooksSubcommand::CodexMutationScope`
      variant, `#[command(hide = true)]`)
    - `cli/src/services/parse/command_runtime.rs`
      (`convert_hooks_subcommand_request` arm →
      `HookSubcommand::CodexMutationScope`; two routing/hidden-help tests)
    - `cli/src/services/hooks/mod.rs` (`HookSubcommand::CodexMutationScope`
      variant; unwrapped / non-fail-open `run_hooks_subcommand_in_repo` dispatch
      arm mirroring `MutationScope` / `ClaudeMutationScope`;
      `hook_runtime_invocation_name` arm)
    - `cli/src/services/hooks/codex_mutation_scope/mod.rs` (adapter driver:
      `GitDirResolver` / `IngressSeam` injected `&dyn Fn` aliases, `ACTOR_KIND_CODEX`,
      `run_codex_mutation_scope_subcommand` / `_from_payload` / `_from_payload_with`
      (+ `#[cfg(test)] _at_state_root`), `dispatch_codex_hook_event`,
      `handle_pre_tool_use`, `admit_or_recover` / `readmit_after_flush` /
      `Admission` (superseding the initial `apply_recovery_barrier` /
      `BarrierOutcome` — see the concurrency follow-up),
      `establish_start`, `handle_close`, `cleanup_attempts_matching`,
      `abandon_attempt`, `attempt_matches_key`, `scope_boundary_payload` /
      `abandon_payload` / `flush_payload`, `pre_tool_use_deny_json`,
      `log_pre_tool_use_fail_closed`; `mod driver` unit tests including the
      inter-process concurrency regressions Test A–F)
    - `cli/src/services/hooks/codex_mutation_scope/state.rs` (T03's durable store,
      materially revised by the concurrency follow-up — see below)
  - Result: Wired the hidden `sce hooks codex-mutation-scope` command through the
    normal hook stack unwrapped (non-fail-open), exactly as `mutation-scope` /
    `claude-mutation-scope`, and implemented the Codex adapter driver as a close
    structural mirror of #263's `claude_mutation_scope` driver, adapted to Codex's
    six-event surface (`PreToolUse`, `PostToolUse`, `Stop`, `Interrupt`,
    `SubagentStop`, `SessionEnd` — no `PermissionDenied` / `StopFailure` /
    `UserPromptSubmit` / `WorktreeRemove`). Mappings: `TrackedMutation`
    (`Bash` / `apply_patch`) `PreToolUse` → D13 recovery barrier → D7 write-ahead
    `Start` (`pending_start` → ingress `start` with `actor_kind:"codex"` and the
    raw hook `cwd` as `repository_root` → `active`) → D8 fail-closed
    `hookSpecificOutput` `permissionDecision:"deny"` on any failure (resolver,
    barrier, allocation, seam), detail logged via
    `sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed`, never leaked to the
    model; `Untracked` (`mcp__*`, unknown) and `Delegation`
    (`collaborationspawn_agent` / `collaborationwait_agent`) `PreToolUse` →
    neutral empty continue, no `Start`, no attempt, no bookkeeping, never denied
    for being untracked, never affected by the recovery barrier (D2/D8/D23);
    `PostToolUse` for a tracked `active` attempt → `close` then `remove_attempt`,
    a `pending_start` attempt → `abandon`, a failed `close` → `abandon` +
    `recovery_pending` (D9/D11); no `Close`-on-failure path (built-in Case A);
    `Stop` → main-turn (`agent_id` none) scoped abandon sweep; `Interrupt` →
    whole-session sweep (SIGINT tears down the session, strictly covered by the
    `SessionEnd` backstop); `SubagentStop` → `agent_id`-scoped sweep; `SessionEnd`
    → whole-session sweep — all sweeps over adapter-owned tracked attempts only
    (D12); D13 recovery barrier denies new `TrackedMutation` `PreToolUse` while
    tracked attempts remain and runs exactly one quiescent `flush`, clearing
    `recovery_pending` only on durable flush success. No D10a successor-barrier /
    lane-key code ships (T02 recorded the lane key N/A). The driver's only
    mutation-stack dependency in non-test code is
    `super::mutation_scope::run_mutation_scope_from_payload`, wire payload built
    as a JSON string (D18/AC19). Injected git-dir resolver + seam make every
    mapping unit-testable without a real repo or DB. No background-execution
    classifier/deny ships (D16 — the `codex exec` shell tool has no
    `run_in_background`); AC15's live-check for that path is therefore N/A for
    this Codex surface.
  - Verify:
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
      cli/Cargo.toml services::hooks::codex_mutation_scope` — **passed** (73
      tests: 30 new `tests::driver` covering the `Untracked` / `Delegation`
      pass-through with an untouched state store (AC9b), a full successful-MCP
      lifecycle leaving no scope (AC9b), write-ahead `Start` ordering + `cwd` as
      `repository_root` observed from inside the seam (AC6), duplicate-delivery
      `ScopeId` reuse (AC4), resolver/`Start`-seam fail-closed with the exact
      deny JSON + logged detail + never-allow (AC7), `Delegation`/`Untracked`
      never fail-closed (AC7), successful `close` (AC8), `pending_start` → abandon
      and failed `close` → abandon + `recovery_pending` (D11/AC13), failed
      `close` + failed `abandon` keep the attempt tracked (D11), `PostToolUse`
      with no matching attempt is a no-op, `Stop` / `Interrupt` / `SubagentStop`
      / `SessionEnd` scoped sweeps + a failed-abandon sweep keeping the attempt
      tracked (D12), recovery-barrier deny-while-outstanding / untracked-unaffected
      / flush-then-start / flush-failure-stays-closed (AC12), probe-13
      mutate-then-error leaves no stale state (AC9c), probe-14 failed-MCP →
      tracked successor starts clean (AC9d), probes-16/17 parallel MCP creates no
      scopes (AC9e), built-in failed-A-then-B never leaves a zombie (AC9a),
      malformed / unsupported-event payloads propagate as real errors; plus the
      43 pre-existing T02/T03 tests).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
      cli/Cargo.toml services::hooks::mutation_scope` — **passed** (36 tests;
      generic ingress unaffected).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
      cli/Cargo.toml services::parse::` — **passed** (15 tests, incl. the new
      `codex_mutation_scope_hook_parses_to_hook_subcommand` and
      `codex_mutation_scope_hook_is_hidden_from_hooks_help`).
    - `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
      cli/Cargo.toml services::hooks::` — **passed** (404 tests; existing
      `sce hooks codex` / Claude-adapter / mutation-scope suites unaffected and
      additive).
    - Live binary (AC1): `sce hooks codex-mutation-scope </dev/null` →
      `Error [SCE-ERR-RUNTIME]: Invalid Codex hook event payload from STDIN:
      expected a JSON object, got an empty payload.` exit 4 (strict parser, not
      "unknown subcommand", not fail-open); `sce hooks --help` lists `codex` and
      `mutation-scope` but **not** `codex-mutation-scope`; `sce --help` does not
      list it.
    - AC19 grep
      (`^\s*use\s+crate::services::mutation_trace::(runtime|protocol|store)|::(RepositoryAgentTraceDb|WorktreeId|GitSnapshotService)\b`
      over `cli/src/services/hooks/codex_mutation_scope/`) — **clean**; the only
      non-test mutation-stack import is
      `super::mutation_scope::run_mutation_scope_from_payload`.
    - `clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` —
      **clean**.
    - `cargo fmt --manifest-path cli/Cargo.toml -- --check` — **clean** (after
      `cargo fmt`).
  - Follow-up (2026-09-08 — inter-process concurrency correctness, PR #268):
    The initial T04 driver implementation exposed an inter-process TOCTOU gap
    between recovery-barrier inspection and tracked-attempt allocation, plus
    duplicate quiescent `flush` and stale-`flush`-clear risks. `apply_recovery_barrier`
    did an unlocked `read_state` recovery check and then a separate
    `allocate_attempt` that never re-checked recovery, and `recovery_pending` was a
    bare boolean. The follow-up moved recovery ownership and tracked admission into
    **generation-aware atomic adapter-state transitions** (see D13a/D13b):
    `state.rs` bumps `ADAPTER_STATE_VERSION` to `2`, replaces `recovery_pending: bool`
    with `recovery: RecoveryState` (`Clear | Pending{generation} | Flushing{generation}`)
    + a monotonic `next_recovery_generation`, and replaces `allocate_attempt` /
    `mark_recovery_pending` / `clear_recovery_pending` with `admit_tracked_attempt`
    (one locked transition: recovery check + duplicate-key reuse + unresolved-
    `PendingStart` barrier + fresh `PendingStart` persist, returning
    `Admitted | RecoveryBlocked | UncertainAttemptBlocked | FlushClaimed{generation}`),
    `arm_recovery` (`Clear→Pending(new)`, `Pending(g)` kept, `Flushing(g)→Pending(new)`),
    `complete_recovery_flush(g)` (clears only while still `Flushing(g)`), and
    `relinquish_recovery_flush(g)` (failed flush → `Pending(g)` for retry). The
    driver's `admit_or_recover` runs at most one quiescent `flush` per `PreToolUse`
    with the state lock never held across the seam (I6). A `Start`-succeeded-then-
    `mark_active`-failed attempt stays `PendingStart` and conservatively blocks a
    successor Start until a cleanup signal abandons it → recovery → `flush`
    (Problem 4 / D13b).
    - New durable state shape: `{ version:2, next_attempt_seq, next_recovery_generation,
      recovery: RecoveryState, attempts[] }`; a v1-shaped or unknown-version file is
      rejected, never fabricated.
    - `state.rs` state-machine tests: `Clear→Pending(g)`, `Pending(g)→Flushing(g)`,
      one-claimer-per-generation (concurrent), `Flushing(g)→Clear` only on matching
      generation, stale/superseded completion is a safe no-op, `Flushing(g1)` +
      newer recovery survives completion(g1), recovery survives serialization/reload,
      atomic admission refuses recovery state and persists `PendingStart` before
      returning, blocks an unrelated `PendingStart`, allows a new key alongside an
      `Active` attempt; retained: parallel admission converges without lost updates,
      monotonic `attempt_seq`, checkout-local, durable write, malformed/version
      rejection, OS lock behaviour + lock-released-between-helpers.
    - `mod.rs` inter-process concurrency regressions (deterministic, channel-gated
      seams — no probabilistic sleeps): **Test A** recovery armed mid-abandon blocks
      a concurrent tracked `PreToolUse` and no `start` payload for it reaches the
      seam; **Test B** two quiescent recovery callers emit exactly one `flush(g)`;
      **Test C** recovery re-armed while `flush(g1)` is in flight → final state
      `Pending(g2)`, the stale completion cannot clear it; **Test D** `Start` ok +
      `mark_active` fails → successor denied, then cleanup → abandon → recovery →
      one `flush` then `start`; **Test E** duplicate `AttemptKey` reuses the same
      `ScopeId`; **Test F** `mcp__*` / unknown / delegation stay neutral while
      recovery is `Pending`/`Flushing`.
    - Re-validated: `services::hooks::codex_mutation_scope` — **passed** (87 tests);
      `services::hooks::mutation_scope` — **passed** (36); `services::hooks::` —
      **passed** (418); `services::hooks::codex` — **passed** (216, existing
      dispatcher unaffected); `services::mutation_trace::` — **passed** (323, no
      protocol/runtime change); `clippy --all-targets -- -D warnings` — **clean**;
      `cargo fmt -- --check` — **clean**. AC19 boundary still clean (only
      `super::mutation_scope::run_mutation_scope_from_payload`); no
      `spec/mutation_cursor.qnt` / `mutation_trace/protocol.rs` /
      `mutation_trace/runtime/` / `mutation_trace/store.rs` /
      `cli/migrations/agent-trace-repository/` / `agent-trace.schema.json` change;
      MCP Option B classification unchanged; no daemon, PID supervision, polling, or
      new DB; the adapter-state lock is never held across the generic ingress seam.
      Changes are confined to `codex_mutation_scope/{state,mod}.rs` and this plan.
  - Context impact: domain — a new adapter-domain driver plus a new hidden CLI
    route. User-visible surface: one hidden `sce hooks codex-mutation-scope`
    subcommand (hidden from `sce --help` / `sce hooks --help`); no visible-help
    or public-API change. No generated config yet (`.codex/hooks.json` / `sce
    setup` wiring is T05), no real Git/DB path yet (T06). The generic
    mutation-scope ingress, runtime, protocol, Quint model, and Agent Trace
    schema are unchanged (AC22 territory — no edits to those paths). Durable
    Codex-adapter context (`context/cli/codex-mutation-scope-integration.md` and
    the cross-reference edits to `context/overview.md`,
    `context/architecture.md`, `context/cli/mutation-scope-runtime.md`,
    `context/cli/mutation-scope-hook-ingress.md`,
    `context/sce/agent-trace-hooks-command-routing.md`,
    `context/sce/codex-integration-runtime.md`, `context/context-map.md`) is
    authored by **T07** once the full adapter ships, exactly as recorded for
    T01/T02/T03.
  - Context synchronization: synced
    - T04 wires an internal Rust adapter driver plus one hidden, **unregistered**
      CLI route (`sce hooks codex-mutation-scope`). No `sce setup` registration
      (T05), no `.codex/hooks.json` generation (T05), and no real Git/DB path
      (T06) — so no real Codex session can reach the adapter yet. Consistent with
      the plan's "Context sync" section and the T01/T02/T03 precedent, the durable
      Codex-adapter context (`context/cli/codex-mutation-scope-integration.md`)
      and the root/domain cross-reference edits (`context/overview.md`,
      `context/architecture.md` line ~135,
      `context/cli/mutation-scope-runtime.md`,
      `context/cli/mutation-scope-hook-ingress.md`,
      `context/sce/agent-trace-hooks-command-routing.md`,
      `context/sce/codex-integration-runtime.md`, `context/context-map.md`) are
      authored by **T07** once the adapter is registered and proven. Mandatory
      five-root pass done: `overview.md` (its "Codex, OpenCode, and Pi still have
      no adapter" sentence stays accurate — the driver is inert until T05),
      `architecture.md` (its `hooks/mod.rs` line-135 enumeration will name the
      new arm at T07, matching how the `mutation-scope` / `claude-mutation-scope`
      arms were documented at their integration task), `glossary.md`,
      `patterns.md`, `context-map.md` all read and confirmed not contradicted by
      an inert, unreachable adapter. No architecture decision qualified for an
      ADR — T04 is a new caller of the existing
      `hooks::mutation_scope::run_mutation_scope_from_payload` seam under the
      already-recorded D17 decision (separate hidden command, decided at T02),
      structurally mirroring `claude_mutation_scope`; the Codex hook-ownership
      ADR (`2026-08-23-codex-nondestructive-hook-ownership.md`) is untouched
      (that is T05's `codex_hook_config.rs` scope).
    - Concurrency follow-up (2026-09-08, PR #268) re-checked the five roots and
      remains `no_context_change`: the generation-aware recovery state machine
      (D13a/D13b) is adapter-internal inter-process synchronisation with no
      protocol, Quint, runtime-semantic, SQL, or Agent Trace schema change (AC22
      still holds) and no user-visible surface change — the adapter is still
      inert and unregistered. Durable adapter context (D13a/b, the coverage
      table, the concurrency story) is authored by T07 as already recorded.

- [ ] T05: `Generated .codex/hooks.json registrations, setup merge, and doctor` (status:todo)
  - Task ID: T05
  - Scope: In —
    (a) `config/pkl/renderers/codex-content.pkl`: add the minimum mutation-scope
    `.codex/hooks.json` registrations the adapter uses (per T01's matcher
    findings), routed to the D17 command, leaving the four existing
    `sce hooks codex` registrations byte-for-byte unchanged **and positionally
    stable** (D20 — appended after, never prepended).
    (b) `cli/src/services/codex_hook_config.rs`: extend `REQUIRED_EVENTS` and
    the ownership predicate to be **command-aware** (recognize both the existing
    `["sce","hooks","codex"]` contract and the new
    `["sce","hooks","codex-mutation-scope"]` contract), so the merge preserves
    every existing SCE-owned and user-owned handler, replaces only the matching
    command's stale/duplicate handlers, and stays idempotent; the merge inserts
    new SCE-owned handlers/groups **additively after** existing ones so an
    already-trusted registration keeps its
    `(event, matcher, matcher-group index, handler index, handler
    contents/hash)` identity and its computed Codex trust key (D20). If some
    event/matcher structure genuinely cannot preserve position, the code records
    which and doctor surfaces that re-trust is needed — trust is never silently
    invalidated.
    (c) Extend `codex_hook_config::hook_event_key_label` for **every** newly
    SCE-owned mutation-scope event with the exact upstream Codex key label
    (verified against `openai/codex` source, cited in a comment / `NOTES.md`,
    never lowercased), with a dedicated test per new event (D22).
    (d) Extend the doctor Codex-hook diagnosis so each mutation-scope
    registration gets the full **three-dimension** health model (D21): structural
    (`PresentAndCurrent` / `Missing` / `Stale` / `Malformed`), normal Codex trust
    (`Trusted` / `Untrusted` / `Modified` / `Disabled`), and effective
    project-hook policy (`ProjectHooksAllowed` / `PolicyBlocked` /
    `PolicyUnknown`) reusing the single per-invocation `configRequirements/read`
    probe; healthy requires all three; `PresentAndCurrent` + any of
    `Untrusted` / `Modified` / `Disabled` / `PolicyBlocked` / `PolicyUnknown` is
    never healthy; mutation-scope registrations are reported with readiness
    distinct from the `sce hooks codex` registrations, not flattened into one
    `.codex/hooks.json` status. `sce doctor --fix` repairs only SCE-owned
    `.codex/hooks.json` structure and **never** writes `$CODEX_HOME/config.toml`,
    grants trust, or changes managed policy.
    Out — any adapter behavior change; any non-Codex renderer; any change to the
    trust/policy probe mechanism itself (`codex_hook_policy.rs` — reused as-is).
  - Dependencies: T04
  - Done when: `nix run .#pkl-check-generated` passes with the new registrations
    (its exact file-count artifact updated if the count legitimately changes);
    `codex_hook_config.rs` tests prove AC16, AC16a (the canonical-four ->
    plus-mutation-hooks **upgrade regression**: starting from a realistic
    already-installed, already-trusted document with exactly
    `UserPromptSubmit`/`Stop`/`PreToolUse Bash`/`PostToolUse apply_patch` ->
    `sce hooks codex`, the same merge/setup path adds mutation-scope hooks while
    every existing registration's identity tuple and trust key are unchanged,
    new hooks appear exactly once, user hooks are untouched, and a second merge
    is byte-identical), AC17 (idempotency + malformed-input untouched-file),
    AC17a (one `hook_event_key_label` test per new event); doctor tests prove
    AC18's six points, including that the existing trusted `sce hooks codex`
    hooks and the new untrusted mutation-scope hooks are reported with distinct
    readiness and no `$CODEX_HOME` write occurs on any path; a fresh
    `sce setup --codex` into a repo with a user-owned Codex handler preserves it.
  - Verify: `nix run .#pkl-check-generated`; `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    services::codex_hook_config`; `nix develop -c ./scripts/run-cli-cargo.sh
    test --manifest-path cli/Cargo.toml services::doctor::`; `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    services::setup::`.
  - Context synchronization: pending

- [ ] T06: `Real Git/DB regressions through the production Codex path` (status:todo)
  - Task ID: T06
  - Scope: In — regressions using real temporary Git repositories and real
    repository Agent Trace DBs, driven through the production Codex-adapter ->
    generic-ingress -> real runtime path (no manual `mutation_trace_*` inserts;
    the only permitted injection is the adapter's own `state.rs` bookkeeping
    helpers to simulate a crash point, exactly as #263's T08 did). Add a
    `#[cfg(test)]` state-root variant of the real adapter entry point mirroring
    `mutation_scope::run_mutation_scope_from_payload_at_state_root`. The matrix,
    adapted to T01 findings and re-planning direction B (D23):
    1. `Bash` successful mutation -> tracked / `AiExclusive` / `Closed` (AC8);
    2. `Bash` partial mutation + non-zero exit -> tracked / `Closed` (partial
       mutation attributed to that scope) (AC9);
    3. `apply_patch` success -> tracked / `Closed` (AC8);
    4. `apply_patch` verification failure -> no mutation, safe cleanup, no scope
       to close (AC9);
    5. duplicate tracked lifecycle (`Pre`/terminal redelivery) -> idempotent, no
       second transition (AC4);
    6. interrupted tracked execution -> abandonment / recovery via the proven
       D12 signal (AC11);
    7. subagent tracked tool -> independent scope identity (its `agent_id`);
    8. linked worktree -> correct `WorktreeId` / cursor, other worktree
       unchanged (AC14);
    9. **MCP success -> allowed, no scope** — full `PreToolUse → PostToolUse`
       MCP lifecycle (probe-12 shape) driven live via `fixtures/mcp_probe/`
       leaves zero mutation-scope rows/events attributable to the MCP execution
       and an untouched adapter state store (AC9b);
    10. **MCP mutate-then-error -> allowed, no scope, no zombie state** — the
        probe-13 lifecycle (MCP mutates a git-visible file, returns error, **no
        `PostToolUse`**, then `Stop`/`SessionEnd`): no stale attempt, no
        `recovery_pending`, no `abandon`, no zombie scope, because no `Start`
        occurred (AC9c);
    11. **failed MCP A -> successor tracked B** (probe-14 shape) -> B (`Bash` /
        `apply_patch`) `Start`s normally as the only live scope; no stale MCP
        state exists to interfere; no false `AiContended` (AC9d);
    12. **parallel MCP executions** (probe-16/17 shape) -> both allowed, neither
        creates a scope, **no MCP-derived `AiContended`**, no adapter state leak
        (AC9e);
    13. **tracked `Bash`/`apply_patch` overlapping an MCP mutation** -> the
        runtime result (`AiExclusive` on the tracked scope) is asserted **and**
        documented as *tracked-scope exclusivity, not sole authorship* — MCP did
        mutate in the interval; the generic protocol is not changed to force
        `AiContended` (AC9f);
    14. **unknown tool -> allowed untracked** — `PreToolUse(<unknown name>)`
        returns the neutral response, no scope, no bookkeeping (AC9b);
    15. raw Agent Trace tables (`diff_traces`,
        `post_commit_patch_intersections`, `agent_traces`, `messages`, `parts`)
        remain untouched by the mutation-scope adapter (before/after row counts)
        (AC20).
    Plus the crash/recovery rows: crash before `Start` commit -> conservative
    recovery (AC21a); `Start` committed before bookkeeping settlement ->
    abandonment recovery, not late-`Start` (AC21b); terminal transition committed
    before bookkeeping cleanup -> replay-safe (AC21c); `recovery_pending` blocks
    a tracked successor until recovery succeeds (AC12); reused raw Codex tool
    identifier after terminal -> new `ScopeId` (AC5); any unsupported background
    execution is rejected / documented (AC15); denied tracked execution -> no
    mutation under an untracked `Start` (AC7 production half); cross-harness
    overlap -> `AiContended` (AC10).
    The mutation-scope regressions use **real** temporary Git repos and real
    Agent Trace DBs. The live MCP fixtures (`fixtures/mcp_probe/`, probes 12–17)
    remain evidence fixtures and are **reused** to drive rows 9–13 rather than
    re-running `codex exec` in ordinary unit tests.
    Each applicable test asserts scope status, processed-event keys, revision,
    `cursor_tree`, mutation-event count, attribution kind, `needs_rebaseline`,
    and adapter state. Out — new production behavior; any process supervision or
    detached-child detection; any test that inserts the event it means to prove;
    any regression asserting an MCP execution itself produces mutation-scope
    attribution.
  - Dependencies: T04 (T05 only for a test that installs generated settings —
    prefer driving the adapter entry point directly).
  - Done when: the whole matrix passes and collectively satisfies AC4, AC5,
    AC7 (production half), AC8, AC9, AC9a, AC9b, AC9c, AC9d, AC9e, AC9f, AC10,
    AC11, AC12, AC14, AC15, AC20, AC21.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope`; `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    services::mutation_trace::`.
  - Context synchronization: pending

- [ ] T07: `Author the durable Codex mutation-scope context` (status:todo)
  - Task ID: T07
  - Scope: In — create `context/cli/codex-mutation-scope-integration.md` owning
    the Codex adapter domain (the tool-execution scope model, the D2 **three-class**
    classification table (`TrackedMutation` / `Delegation` / `Untracked`), the
    **D23 partial-by-tool-surface coverage boundary** — the coverage table and
    the "MCP calls remain usable but are not individually attributed" statement
    plus *why* (the T01 probe findings), D3 execution identity, D4
    `ScopeId`/`EventId` derivation, D5/D6 bookkeeping store, D7 write-ahead, D8
    exact Codex-native fail-closed response (tracked only), D9/D10 terminal
    boundaries, D10a the failed-tool -> successor-tool handling (Case A for
    built-ins; Case C *if MCP were modeled as a scope*, resolved by not modeling
    it), D12 cleanup signals + the load-bearing backstop, D13 recovery barrier,
    D15 worktree/cwd ownership, D14 concurrency (including that `AiExclusive` is
    tracked-scope exclusivity, not sole authorship), D16 background/detached
    limitations, D17 command architecture, D18 dependency direction, and
    D20/D21/D22 the Codex hook trust-identity preservation, three-dimension
    doctor health model, and upstream-verified event key labels), with an
    explicit **Unsupported / Coverage boundary** section (AC24) naming the future
    work (first-class MCP attribution via a richer lifecycle mechanism in a
    separate PR) and the tested Codex version; and update
    `context/cli/mutation-scope-runtime.md`,
    `context/cli/mutation-scope-hook-ingress.md`,
    `context/sce/agent-trace-hooks-command-routing.md`,
    `context/sce/codex-integration-runtime.md`, `context/context-map.md`,
    `context/overview.md`, and `context/architecture.md` (line 135) to record
    that a second concrete harness adapter now exists — each edit additive,
    naming Codex as wired/registered and leaving OpenCode/Pi as still-unwired,
    and keeping the existing `sce hooks codex` conversation/diff description
    intact. Write a new dated ADR **only if** T01/T02 established a genuinely
    new system-wide constraint (e.g. permanently multi-command Codex hook-config
    ownership). Out — describing behavior not actually shipped by T02–T06; any
    edit to `context/cli/claude-mutation-scope-integration.md`; any edit to an
    existing ADR.
  - Dependencies: T05, T06
  - Done when: `context/cli/codex-mutation-scope-integration.md` exists and
    satisfies AC23/AC24; every cross-reference file names the second adapter;
    `context/` files stay within the repository's per-file line budget (split a
    file rather than overrun); `nix flake check` and `nix run
    .#pkl-check-generated` still pass.
  - Verify: inspection of the new file and each updated cross-reference against
    AC23/AC24; `git diff origin/claude-mutation-scope-integration --
    spec/mutation_cursor.qnt cli/src/services/mutation_trace/protocol.rs
    cli/migrations/agent-trace-repository/ config/schema/agent-trace.schema.json`
    is empty (AC22); `nix flake check`.
  - Context synchronization: pending

## Open questions

The change's value is not in doubt: the mutation-scope stack exists to attribute
mutations per independently-capable execution across every harness, and Codex is
the second of four planned producers. The generic ingress and runtime were
built specifically so this adapter would be additive. There is no smaller
version worth naming — an adapter that does not establish `Start` before the
tool runs, or does not fail closed, is not a correct adapter.

**T01 outcome + re-planning (codex-cli 0.153.4, upstream `openai/codex`
`rust-v0.153.4` / `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`): built-in
`Bash` / `apply_patch` is representable by the current mutation-scope contract.
The MCP mutation-scope lifecycle is D10a Case C *if MCP is modeled as a scope*.
Re-planning (2026-09-08) chose direction B (D23): MCP and unknown tools are
`Untracked` — usable, may mutate, but outside the Codex adapter's mutation-scope
coverage; no protocol change. T01 is done; T02 is unblocked.** Every built-in
risk is resolved by a captured fixture or an upstream schema citation; the MCP
risk is resolved by exclusion, with the Case C evidence retained as the reason
(see the Design section dispositions, D23, and
`cli/src/services/hooks/codex_mutation_scope/fixtures/NOTES.md`). Headline
resolutions:

- **D10a is Case A for built-in `Bash` / `apply_patch` only.** A failed shell
  tool still fires `PostToolUse`; a failed `apply_patch` writes nothing;
  interruption ends the turn. No serial-lane successor barrier ships for
  built-ins.
- **D10a is Case C for MCP *if MCP were modeled as a scope* — resolved by NOT
  modeling MCP as a scope (direction B, D23).** The T01 MCP
  extension (probes 12–17) proves, live, all three Case C conditions at once,
  and this evidence stands unchanged:
  1. a mutation-capable MCP tool can **mutate a git-visible file and then return
     `is_error:true` with NO terminal hook** (probe 13:
     `PreToolUse → Stop → SessionEnd`, `mcp_b.txt` present in `git status`).
     Upstream: `registry.rs` ~674 gates `PostToolUse` on
     `success_for_logging()`; MCP's is `CallToolResult.success()`, false on
     `is_error:true`.
  2. **no positive cleanup signal precedes a successor** — probe 14:
     `PreToolUse(A = mutate_then_error)` → `PreToolUse(B = mutate_success)` with
     **no event of any kind between them**; A's `tool_use_id` never recurs.
  3. **same-lane MCP executions can overlap** — probes 16/17: two
     mutation-capable MCP executions run genuinely concurrently (~8 s window,
     `PreToolUse`s ~1 ms apart, confirmed by the MCP server's own log). Enabled
     by the config key `[mcp_servers.<name>] supports_parallel_tool_calls = true`
     **or** the tool's own `annotations.readOnlyHint`
     (`McpHandler::supports_parallel_tool_calls()`). So `PreToolUse(B)` cannot
     prove A stale, and there is no narrower serial lane than
     `(session_id, turn_id)`.
  If MCP were a scope, the adapter could not distinguish `failed-and-dead A` from
  `still-running A` — exactly the stale-scope problem D10a was added to prevent.
  **Re-planning decision (2026-09-08):**
  - **A — deny mutation-capable MCP `PreToolUse` fail-closed. REJECTED.** Too
    disruptive (MCP tools unusable inside Codex under SCE); also needs a rule to
    tell "mutation-capable MCP" from "read-only MCP" that cannot trust the
    server's own `readOnlyHint`.
  - **B — MCP (and unknown) tools are `Untracked`: allowed, may mutate, no
    `Start`, no scope, explicitly outside mutation-scope coverage. CHOSEN for
    Codex adapter v1** (D23). The un-attributed-MCP-mutation gap is an explicit,
    documented coverage boundary, never a silent gap. No protocol / Quint /
    SQL / schema change.
  - **C — a richer lifecycle/runtime mechanism** (per-`tool_use_id` MCP scope
    retired only by `PostToolUse`, or an overlap-tolerant turn-boundary sweep,
    plus an `AiContended`-aware successor policy). DEFERRED as possible future
    work in a separate, explicitly justified PR — "investigate first-class MCP
    mutation attribution using a richer lifecycle mechanism".
  The adapter must **not** silently downgrade MCP/unknown to read-only; direction
  B allows them *explicitly* untracked, documented as a coverage boundary.
- **Parallel MCP execution is real** (probes 16/17) but produces **no tracked
  scopes** under direction B, so there is **no MCP-derived `AiContended`** and
  **no MCP-overlap `AiContended` regression** in T06. `Bash`-overlapping-MCP is
  covered by AC9f/T06 row 13, asserting `AiExclusive` = tracked-scope
  exclusivity (not sole authorship). Cross-harness `AiContended` remains
  reachable and is the AC10/T06 form.
- **`SessionEnd` is the load-bearing cleanup backstop** (fires on clean exit and
  on SIGINT); `Interrupt` is an additional earlier interruption signal the plan
  did not know about (Codex has **12** hook events, not 11). For MCP,
  `SessionEnd` would be a **whole-turn-late** backstop — which is exactly why MCP
  cannot be modeled as a scope; under direction B (D23) no MCP attempt is created
  for any signal to retire.
- **The fail-closed `PreToolUse` response**: both `{"decision":"block"}` and
  `hookSpecificOutput.permissionDecision:"deny"` block the tool (probes 3/4/15);
  T04 emits the `hookSpecificOutput` shape for a **`TrackedMutation`** failure
  only — MCP/unknown are never denied for being untracked (D8/D23).
- **MCP tool naming** is `mcp__<server>__<tool>` with an `exec-<uuid>`
  `tool_use_id` (D2/D3 unchanged for MCP).
- **Codex exposes a delegated-agent identity** (`agent_id`, on subagent events
  only) — the plan uses it and does not invent one.
- **Command architecture**: T01 found nothing against the recommended separate
  hidden `sce hooks codex-mutation-scope` command; T02 still decides.
- **Existing-hook trust identity**: the upstream label map is
  `codex-rs/hooks/src/lib.rs` 96–108; a position-stable additive merge is a T05
  implementation constraint, not an unknown.

The real risks were empirical, not architectural. T01 resolved the built-in ones
with fixtures and the MCP one by an explicit coverage boundary:

- **RESOLVED — Failed tool with no terminal event, then another tool in the same
  turn.** (D10a — the single highest-risk correctness question.) For built-in
  `Bash` / `apply_patch`: **Case A** — a failed shell tool still fires
  `PostToolUse`, a failed `apply_patch` writes nothing, and interruption ends the
  turn; no barrier ships. For **MCP: Case C *if modeled as a scope*** — probes
  13/14/16/17 prove a mutation-capable MCP tool can mutate-then-fail with no
  terminal hook, no cleanup signal reaches the adapter before the successor
  `PreToolUse`, and same-lane MCP executions can overlap. **Resolved by direction
  B (D23):** MCP/unknown are `Untracked`, so no MCP attempt or scope exists, the
  D10a tension never arises for them, and the un-attributed MCP mutation is a
  documented coverage boundary. The MCP lifecycle did **not** become safe — it is
  simply out of scope for the v1 adapter.
- **Is there any reliable terminal signal for a failed mutation-capable tool at
  all?** (D10.) Even outside the successor case, the failed-partial-mutation
  interval with no `Close` is bounded only by the next lifecycle signal + a
  re-baselining `flush` — a deliberate false-negative.
- **Which Codex lifecycle signal is the load-bearing cleanup backstop?** (D12.)
  Claude's is `SessionEnd`. Codex's must be one T01 proves fires on process/
  session termination with a usable `session_id`. If none does, outstanding
  attempts can only be retired on the next turn's `PreToolUse` via the recovery
  barrier — acceptable but weaker.
- **What is the exact Codex-native fail-closed response for the supported
  version?** (D8.) `permissionDecision: "deny"`, `{"decision":"block"}`, or a
  non-zero exit — version-dependent, and the adapter must emit exactly the one
  that blocks the tool.
- **RESOLVED — Can Codex overlap its own mutation-capable executions?** (D1/D14.)
  **Yes, via MCP** (probes 16/17). Built-in `Bash` / `apply_patch` remained
  serial across all 11 built-in probes. Under direction B, MCP executions are
  `Untracked` and produce no tracked scopes, so there is **no MCP-derived
  `AiContended`**; the T06 concurrency regression exercises the **cross-harness**
  form only, and a separate `Bash`-overlapping-MCP regression documents the
  `AiExclusive` = tracked-scope-exclusivity (not sole-authorship) semantic
  (AC9f).
- **Does Codex expose any delegated-agent identity?** (D3.) If not, there is no
  per-agent cleanup sweep and no `agent_id` in the `ScopeId` — and the plan
  must not invent one.
- **Command architecture** (D17): separate `sce hooks codex-mutation-scope`
  (recommended, decided in T02) versus extending the `sce hooks codex`
  dispatcher. Not blocking — T02 decides against T01 + the code with a recorded
  rationale.
- **Can the mutation-scope registrations be added without disturbing the
  existing hooks' Codex trust identity?** (D20.) Codex trust keys on
  `event label + matcher-group index + handler index + handler hash`, so a
  non-additive merge would silently un-trust the four working `sce hooks codex`
  hooks. T05 must merge additively (append, never prepend/renumber) and prove
  every existing identity tuple + trust key is unchanged; if some structure
  genuinely cannot preserve position, doctor must say re-trust is needed. Not
  blocking — this is a T05 implementation constraint, not an unknown.

No Quint / mutation-cursor protocol / mutation-trace SQL migration / Agent Trace
schema / attribution-algorithm change is expected or required: the generic
mutation-scope contract already models `Start`/`Advance`/`Close`/`Flush`/
`abandon`, already accepts `ActorKind::Codex`, and already handles replay
idempotency, conservative recovery, and `AiContended` — the Claude adapter proved
the contract is sufficient for a concrete harness without touching any of those.
The T01 MCP D10a Case C finding did **not** force a protocol change: re-planning
direction B (D23) resolves it entirely within the Codex adapter's coverage
boundary — MCP and unknown tools are `Untracked`. The runtime already models
exclusivity among the tracked scopes it is told about, not global filesystem
authorship, so nothing formal changes. Future work (direction C — first-class MCP
attribution via a richer lifecycle mechanism) would be a separate, explicitly
justified PR.
