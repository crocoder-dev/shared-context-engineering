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

The fundamental mapping is **one independently mutation-capable Codex tool
execution = one SCE mutation `ScopeId`**. A Codex session, turn, or delegated
agent is never a scope; `session_id` / `turn_id` / any delegated-agent identity
are only inputs that distinguish tool executions.

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

### D1 — Scope = one independently mutation-capable Codex tool execution

A mutation scope is exactly one independently mutation-capable Codex tool
execution attempt. Not a session, not a turn, not a delegated agent. Sequential
tool calls are sequential scopes. Whether two Codex mutation-capable executions
can genuinely overlap (and therefore whether Codex alone can produce
`AiContended`) is **T01-GATED** — Codex has historically executed tools
serially within a turn. `AiContended` can still arise from a Codex scope
overlapping another harness's scope on the same worktree regardless of T01's
finding; see D14.

**T01 disposition (codex-cli 0.153.4): ASSUMPTION — PROBE (leaning serial).**
Codex executed every mutation-capable tool strictly serially in all 11 probes
(`PreToolUse → PostToolUse → PreToolUse → …`, never interleaved), including
across the parent/subagent boundary and when asked to parallelise. Codex-alone
`AiContended` is treated as not reachable for 0.153.4; the concurrency
regression must cross harnesses (D14). The adapter still never collapses two
executions into one `ScopeId`. Evidence: `fixtures/probe01…`, `probe02…`,
`probe08-subagent-delegation.*` + `fixtures/NOTES.md`.

### D2 — Codex tool classification — T01-GATED

The adapter classifies the raw Codex `tool_name` in Rust into:

- **Mutation-capable (establishes a scope):** at minimum `apply_patch` and the
  Codex shell/`Bash` tool. MCP tools and any **unknown** tool name are treated
  conservatively as mutation-capable (an unknown read-only tool only creates
  harmless scopes; the opposite default silently misses a new mutation-capable
  tool).
- **Read-only (never a scope):** the Codex file-read / list / search / web
  tools T01 enumerates.
- **Delegation (never a scope):** whatever tool Codex uses to spawn a delegated
  agent, if any — the delegated agent's own mutation-capable tool calls
  establish their own scopes.

T01 must enumerate Codex's actual tool-name vocabulary (`tool_name` values on
`PreToolUse`/`PostToolUse`, MCP tool naming, delegation tool name). The exact
membership of each list is frozen by T01 and recorded here.

**T01 disposition (codex-cli 0.153.4): PROVEN for the `codex exec` surface.**
- **Mutation-capable:** `apply_patch`, `Bash` (the shell tool — it also performs
  reads / listing / search via shell commands, so it is always treated
  mutation-capable; a read-only shell command merely creates a harmless scope).
  MCP tools and any unknown `tool_name` → conservatively mutation-capable
  (from upstream; no MCP server was configured live).
- **Read-only (never a scope):** none — this Codex surface has **no dedicated
  built-in read-only tool names**; reads go through `Bash`.
- **Delegation (never a scope):** `collaborationspawn_agent`,
  `collaborationwait_agent` (a `collaboration` namespace prefix, no separator).
  The delegated agent's own tool calls carry its `agent_id` and establish their
  own scopes.
Evidence: `fixtures/probe05-tool-vocabulary.*`, `probe08-subagent-delegation.*`,
`fixtures/NOTES.md`.

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
`(session_id, agent_id?, tool_use_id)`.
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

A mutation-capable tool must not execute after SCE has failed to establish its
mutation scope.

### D8 — Codex-native fail-closed PreToolUse — T01-GATED

A mutation-capable Codex `PreToolUse` is **fail-closed**: any failure to durably
establish the scope (state-allocation failure, seam `Start` failure,
unresolvable `cwd`, recovery-barrier denial) must **block the tool**, not let it
run un-scoped.

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

A read-only or delegation `PreToolUse` returns the neutral response, no scope.

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
terminal action. Evidence:
`fixtures/probe03-pre-tool-use-hook-decision-block.*`,
`fixtures/probe04-pre-tool-use-hook-hookspecificoutput-deny.*`.

### D9 — Terminal boundary on success — T01-GATED

For a successful mutation-capable tool with an `active` tracked attempt, the
terminal Codex hook maps to
`{ "operation":"close", "scope_id":<same>, "event_id":<scope>|close,
"actor_kind":"codex" }`. T01 must confirm **which** hook is the reliable
terminal signal for a successful mutation-capable execution (`PostToolUse` for
that tool, carrying the D3 identity that ties it to the `PreToolUse`). The
attempt is removed from adapter state only after durable `Close` success;
duplicate delivery after cleanup is a safe no-op.

**T01 disposition (codex-cli 0.153.4): PROVEN.** `PostToolUse` is the reliable
terminal signal for a successful mutation-capable tool, carrying the same
`tool_use_id` (and `agent_id`, for a subagent) as its `PreToolUse`. Evidence:
`fixtures/probe01-apply-patch-and-shell-success.*`,
`fixtures/probe08-subagent-delegation.agent-apply-patch.*`.

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

**T01 disposition (codex-cli 0.153.4): PROVEN — D10 becomes a `Close` mapping
like D9, and is cleaner than feared.**
- A **shell (`Bash`)** tool that wrote a file then exited non-zero **does** fire
  `PostToolUse` (same `tool_use_id`), so a partial mutation from a failed shell
  command is bounded by a terminal hook → map to `close` exactly like D9.
- An **`apply_patch`** that fails verification fires **no** `PostToolUse`, but
  Codex verifies the patch before touching the working tree, so a failed
  `apply_patch` writes nothing — there is no partial-mutation-without-terminal
  case for it.
- Prior SCE research's "`PostToolUse` fires only on a successful tool result" is
  true at the `success_for_logging()` layer (`codex-rs/core/src/tools/registry.rs`
  ~line 674 at `rust-v0.153.4`), but an executed shell command with a non-zero
  exit still counts as a successful tool result.
The adapter needs **no Close-on-failure path**. Evidence:
`fixtures/probe02-shell-partial-write-then-nonzero-exit.*` (PostToolUse fires),
`fixtures/probe06-apply-patch-verification-failure-no-post.*` (no PostToolUse,
no write).

### D10a — Failed-tool -> successor-tool in the same turn — T01-GATED

D10's "next lifecycle signal" is **not sufficient on its own** for the case where
a failed mutation-capable tool with no terminal hook is followed by **another
mutation-capable tool in the same turn**, before any `Stop` / `SessionEnd` /
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

**T01 disposition (codex-cli 0.153.4): PROVEN — Case A. The dangerous scenario
does not arise on 0.153.4, and no successor-barrier logic ships.**
- A failed **shell** tool always emits `PostToolUse` (terminal) before the next
  `PreToolUse` — Codex runs mutation-capable tools strictly serially (D1), so
  predecessor A is already terminal in bookkeeping when successor B's
  `PreToolUse` arrives.
- A failed **`apply_patch`** never mutates the working tree (atomic
  verification), so there is nothing to strand.
- A **hook-blocked** tool never executes (`PreToolUse` only, no `PostToolUse`) —
  no scope was established (D8 fail-closed happens before `start`).
- The only "partial mutation, no `PostToolUse`" case is **whole-turn
  interruption** (SIGINT), which emits `Interrupt` and then `SessionEnd` and
  ends the turn — there is no in-turn successor `PreToolUse` to race.
T02 records the D10a lane key as **N/A (Case A)**; T04 ships no successor
barrier; the D13 successor-Start invariant is still upheld trivially because a
terminal `PostToolUse` (or `Interrupt`/`SessionEnd`) always precedes the
successor. Evidence:
`fixtures/probe02-shell-partial-write-then-nonzero-exit.*`,
`fixtures/probe06-apply-patch-verification-failure-no-post.*`,
`fixtures/probe07-sigint-during-shell.*`,
`fixtures/probe11-interrupt-event-on-sigint.*`.

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
"next lifecycle signal" backstop is too late; its resolution (an intermediate
signal, a proven serial-lane successor barrier, or unsupported) is owned by
D10a, not this table.

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

### D13 — recovery_pending barrier and quiescent Flush

Carried verbatim from the Claude adapter (D19 there). Whenever an abandonment or
an uncertain lifecycle sets `recovery_pending = true`:

```text
recovery_pending == true AND known attempts still outstanding
  -> deny every new mutation-capable PreToolUse (D8 fail-closed shape)

recovery_pending == true AND attempts.is_empty()
  -> run one { "operation":"flush" } through the generic ingress
  -> clear recovery_pending ONLY on durable flush success
  -> a failed flush stays fail-closed
```

A failed abandonment leaves the attempt tracked and `recovery_pending = true`
(never silently allows a successor mutation execution).

**Successor-Start invariant.** A mutation-capable `PreToolUse` must never reach
its write-ahead `Start` while a known-stale predecessor attempt (D10a) remains
`active`/`pending_start` in bookkeeping. Where D10a Case A or Case B applies, the
predecessor is abandoned and — once quiescent — flushed before the successor's
`Start`. Where D10a Case C applies, the integration is unsupported and no
successor logic ships. This invariant does **not** license abandoning an attempt
that T01 proved can legitimately run concurrently with the successor (D10a
invariant (ii)); the sweep is always lane-scoped, never global.

### D14 — Concurrency and AiContended

Two simultaneously-live scopes (whether two Codex executions per D1, or a Codex
execution overlapping a Claude/OpenCode/Pi execution on the same worktree) carry
distinct `ScopeId`s, so the runtime can report `AiContended` for a tree
transition observed while both are live. The adapter never collapses two
executions into one `ScopeId`. If T01 proves Codex cannot overlap its own
mutation executions, the regression for AiContended (T06) exercises a Codex
scope overlapping a second harness's scope instead, and this plan records that
Codex-alone `AiContended` is not reachable.

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
- [ ] AC3: `classify_tool` classifies every Codex tool name T01 enumerated:
  mutation-capable (`apply_patch`, the shell tool, MCP tools, unknown names),
  read-only (the enumerated read/search tools — no scope), delegation (no
  scope). No `Start` boundary is emitted for `SessionStart`, `UserPromptSubmit`,
  `SubagentStart`, or any non-tool lifecycle event.
  - Validate: classification unit-test table; adapter mapping unit tests
    asserting processed-event keys.
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
- [ ] AC6: A tracked mutation-capable `PreToolUse` reaches durable
  generic-ingress `Start` before the hook returns its "continue" response to
  Codex (write-ahead `pending_start` -> ingress `Start` -> `active`), and the
  seam receives the raw hook `cwd` as `repository_root` (never `git_dir`).
  - Validate: adapter ordering unit test with an injected seam asserting the
    persisted phase from inside the seam call and the `repository_root`
    argument; T06 production-path confirmation.
- [ ] AC7: Any failure to establish adapter state or `Start` during a
  mutation-capable `PreToolUse` returns the exact Codex-native block response
  T01 froze (D8), never a silent success and never an explicit allow; the
  detailed error is logged via
  `sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed`.
  - Validate: failure-classification unit tests asserting the exact response
    JSON/exit; `RecordingLogger` assertion that the detail is logged and not
    leaked into the model-visible reason.
- [ ] AC8: A successful mutation-capable tool with an `active` attempt closes
  its scope: `PreToolUse` -> real filesystem mutation -> terminal Codex hook
  produces exactly one eligible tool interval and one terminal (`Closed`) scope
  with attribution `AiExclusive`.
  - Validate: T06 real-Git + real-Agent-Trace-DB regression.
- [ ] AC9: A mutation-capable tool that partially mutated the checkout then
  failed is handled per D10's T01 disposition: if Codex provides a reliable
  final observation, the scope closes and the partial mutation is attributed to
  that scope; if it does not, the stale attempt is abandoned and a subsequent
  `flush` re-baselines the worktree so the partial mutation is
  `IneligibleUnscoped`, never misattributed.
  - Validate: T06 failed-tool regression whose assertions match the D10
    disposition recorded by T01.
- [ ] AC9a: A partially-mutating failed tool A with **no terminal event**,
  followed by another mutation-capable `PreToolUse(B)` in the same turn, is
  handled per D10a's T01 disposition: **either** a proven intermediate lifecycle
  signal retires A before B starts (Case A), **or** a proven serial-lane
  successor barrier retires A (arm `recovery_pending` -> abandon A -> flush when
  quiescent) before B's write-ahead `Start` (Case B), **or** T01 marked the
  lifecycle unsupported and implementation did not proceed (Case C). B must never
  reach `Start` while a known-stale A is still `active`/`pending_start` in
  bookkeeping, and no false `AiContended` event is produced merely because A
  lingered in adapter bookkeeping.
  - Validate: T06 failed-A-then-B regression (real Git + real Agent Trace DB):
    assert A is `Abandoned`, B is the only live scope at its `Start`, the
    worktree re-baselined between them, and the mutation-event stream contains
    **no** `AiContended` row attributable to the A/B overlap; adapter unit test
    for the Case B lane-scoped successor barrier (a same-lane stale predecessor
    is swept, an out-of-lane concurrent attempt is not). If Case C, validate by
    the recorded T01 disposition and the plan's stop-for-re-planning note.
- [ ] AC10: Two simultaneously-live scopes (per D14 — two Codex executions if
  T01 proves overlap is possible, otherwise a Codex scope overlapping a second
  harness's scope) produce `AiContended` for a tree transition observed while
  both are live; the adapter never assigns them one shared `ScopeId`.
  - Validate: T06 concurrency regression; the plan records which D14 form was
    exercised.
- [ ] AC11: An outstanding mutation-capable execution with no terminal hook is
  retired by exactly the Codex lifecycle signals T01 marked load-bearing (D12),
  via `abandon_scope`, leaving the worktree `needs_rebaseline`. A concurrent
  execution T01 proved can legitimately still be running is **not** retired by
  the same sweep (D10a invariant (ii)) — sweeps are always lane-scoped, never
  global.
  - Validate: T06 regressions for each proven cleanup signal; adapter cleanup
    unit tests including one asserting an out-of-lane attempt survives a sweep.
- [ ] AC12: While `recovery_pending` is armed and known attempts remain
  outstanding, every new mutation-capable `PreToolUse` is denied (D8 shape);
  once quiescent, exactly one `{"operation":"flush"}` runs through the seam and
  `recovery_pending` clears only on durable flush success. The same barrier is
  what a D10a Case B successor arms before abandoning a known-stale predecessor,
  so a successor B is denied its own `Start` until predecessor A's abandonment
  and the quiescent flush have durably completed.
  - Validate: adapter recovery-barrier unit tests (deny-while-outstanding,
    flush-then-proceed, flush-failure-stays-closed, successor-blocked-until-
    predecessor-recovered); T06 recovery-barrier regression.
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
  the tool-execution scope model, Codex tool classification, execution identity,
  `ScopeId`/`EventId` derivation, the fail-closed `PreToolUse` and its exact
  Codex-native response, the terminal-boundary mappings, the failed-tool ->
  successor-tool handling and its recorded case + successor-Start invariant,
  the Codex lifecycle cleanup signals and the load-bearing backstop, the
  recovery barrier, worktree/cwd ownership, the concurrency story, the exact
  background/detached execution limitations, and the Codex hook-config coexistence
  contract (existing-registration trust-identity preservation, the
  three-dimension doctor health model, upstream-verified event key labels) —
  each stated as Codex-proven, Codex-documented, or Codex-unsupported, with the
  tested Codex version.
  - Validate: inspection of `context/cli/codex-mutation-scope-integration.md`
    and the updated cross-reference files.
- [ ] AC24: The plan's exact unsupported limitations are enumerated in durable
  context: no line-level attribution for mutations from a failed tool with no
  terminal hook (if D10 lands that way); the failed-tool -> successor-tool
  guarantee only as strong as T01's D10a case (and, if Case C, the integration
  is not shipped and durable context records the unsupported lifecycle); no
  attribution guarantee for self-detaching descendant processes; no
  Codex-managed background execution SCE cannot bound; and whatever else T01
  marks `UNSUPPORTED`.
  - Validate: inspection of the "Unsupported" section of
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
  domain — see AC23/AC24).
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
  identity.

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
  - Completed: 2026-09-07
  - Files changed:
    - `cli/src/services/hooks/codex_mutation_scope/fixtures/` (new — 35 raw
      byte-for-byte Codex hook-event captures across 11 probes + one
      `probe09-*.evidence.json` capture-metadata file + `NOTES.md`)
    - `flake.nix` (add `./cli/src/services/hooks/codex_mutation_scope/fixtures`
      to `workspaceSrc` so the fixtures reach the build sandbox, mirroring the
      Claude adapter's T01 line)
    - `context/plans/codex-mutation-scope-integration.md` (T01 dispositions
      written into D1, D2, D3, D8, D9, D10, D10a, D12, D15, D16, D17-inputs,
      D22; T01 outcome added to Open questions; this task record)
  - Result: Froze the Codex hook/lifecycle contract for **codex-cli 0.153.4**
    (model `gpt-5.6-sol`), cross-checked against upstream `openai/codex` tag
    `rust-v0.153.4` (commit `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`) —
    generated hook JSON schemas, `codex-rs/hooks/src/{schema.rs,lib.rs}`,
    `codex-rs/core/src/{tools/registry.rs,hook_runtime.rs}`. All 11 live probes
    captured (normal apply_patch + shell success, shell partial-write-then-fail,
    two `PreToolUse`-hook denial shapes, tool vocabulary, apply_patch
    verification failure, SIGINT with/without an `Interrupt` hook, subagent
    delegation, self-detaching descendant with Git-observability evidence,
    linked-worktree cwd). Key findings: **D10a is Case A** (a failed shell tool
    still fires `PostToolUse`; a failed `apply_patch` writes nothing;
    interruption ends the turn — no successor-barrier logic ships);
    **`SessionEnd` is the load-bearing cleanup backstop**, `Interrupt` is a
    newly-discovered earlier interruption signal (**Codex has 12 hook events,
    not 11**); **both** `{"decision":"block"}` and
    `hookSpecificOutput.permissionDecision:"deny"` block a tool; Codex runs
    mutation-capable tools **strictly serially** (Codex-alone `AiContended`
    unreachable); Codex **does** expose a delegated-agent identity (`agent_id`,
    subagent events only); raw hook `cwd` is authoritative including for linked
    worktrees; the default `codex exec` shell tool has **no `run_in_background`
    parameter** (no background-execution deny needed) but a self-detaching
    descendant is an unsupported boundary as for Claude. No probe showed Codex
    cannot be represented by the current mutation-scope contract — the plan
    proceeds to T02.
  - Verify:
    - `nix run .#pkl-check-generated` — **passed** ("Ephemeral Pkl generation
      passed: 141 files, inventory sha256
      dcbd28041c3587156510bdb3a6c76e5a9ec4851c140b4c50785c764d95ebfd5c").
    - `nix flake check` — **passed** ("all checks passed!"; incompatible
      non-Linux systems omitted as usual).
    - Fixtures committed under
      `cli/src/services/hooks/codex_mutation_scope/fixtures/` and referenced
      from this plan; `NOTES.md` lists the manifest and per-probe disposition;
      all 36 JSON fixture files parse; CLI build input list updated (`flake.nix`).
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
      independent tool executions (D1/D14).
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
    evidence and, for Case B, the exact lane key. If any probe (D10a Case C
    included) shows Codex cannot be represented by the current mutation-scope
    contract, that is recorded in Open questions and the plan stops for
    re-planning rather than proceeding to T02.
  - Verify (planned): fixtures committed and referenced from this plan;
    `NOTES.md` lists the manifest and per-probe disposition;
    `nix run .#pkl-check-generated` and `nix flake check` still pass (fixtures
    are inert data — confirm the CLI build input list includes the new fixtures
    directory, as #263's T01 needed for Claude).
  - Context synchronization: pending

- [ ] T02: `Command architecture, Codex event model, classification, and identity` (status:todo)
  - Task ID: T02
  - Scope: In — (1) decide the D17 command architecture against T01 + the code,
    defaulting to a separate hidden `sce hooks codex-mutation-scope` command,
    and write the decision into D17; (2) `cli/src/services/hooks/
    codex_mutation_scope/mod.rs`: the strict raw Codex mutation-scope event
    parser (rejecting empty/non-object/missing/blank/wrong-typed with
    `Invalid Codex hook event payload from STDIN: <detail>.`), the supported
    mutation-scope hook-event enum (only the events T01 proved), the D2
    `classify_tool` table, the D3 execution-key type frozen from T01 evidence,
    the D4 length-prefixed `cx-tool-v1|n=..|...` `ScopeId` formatter, and the
    `<scope-id>|start` / `<scope-id>|close` `EventId` formatters, plus any
    background-execution classifier T01 shows is needed (model/classify only —
    the denial is T04's); (3) if T01 recorded D10a Case B, freeze the
    **serial-lane key** (the exact identity-field subset from T01's evidence
    that defines "same lane" for the successor-barrier) as a typed accessor on
    the parsed event, with unit tests — no adapter logic yet, just the key. Out
    — any durable state, any runtime/ingress call, any CLI wiring, any generated
    settings.
  - Dependencies: T01
  - Done when: the module compiles behind the existing `hooks` module tree; the
    D17 decision is recorded in this plan; the D10a lane key is frozen (Case B)
    or explicitly N/A (Case A) or the plan is already stopped (Case C); unit
    tests prove AC2, AC3, AC4 (formatter determinism), AC5 (formatter is a
    function of `attempt_seq`), and the full classification table against T01's
    tool vocabulary.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope`; `clippy` /
    `fmt` clean.
  - Context synchronization: pending

- [ ] T03: `Durable checkout-local Codex adapter state and recovery bookkeeping` (status:todo)
  - Task ID: T03
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
  - Context synchronization: pending

- [ ] T04: `Codex mutation-scope driver + hidden command routing` (status:todo)
  - Task ID: T04
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
    mutation-capable `PreToolUse` -> D7 write-ahead `Start` + D8 fail-closed
    Codex-native block on any failure + D16 background-execution deny if
    applicable; the proven success terminal hook -> `Close` (D9); the proven
    failure disposition from D10; the D10a failed-tool -> successor-tool
    handling per T01's recorded case — Case A routes the intermediate signal
    through D12; Case B implements the **lane-scoped** successor barrier on
    `PreToolUse(B)` (inspect same-lane attempts using T02's frozen lane key ->
    if a stale predecessor is found, arm `recovery_pending` -> abandon it ->
    flush when quiescent -> only then write-ahead `Start(B)`; an out-of-lane
    attempt is never touched); Case C ships nothing; the proven denial
    signal(s) -> `abandon` (D12); the proven session/turn/agent cleanup signals
    -> scoped `abandon` sweeps (D12, always lane/identity-scoped, never global);
    D11 uncertain-boundary rules and the D13 successor-Start invariant; the D13
    recovery barrier + one quiescent `flush`. The driver reaches the runtime
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
    event-to-operation mapping, fail-closed `PreToolUse` (exact Codex-native
    response JSON/exit, AC7), write-ahead ordering (AC6), `pending_start` +
    terminal -> abandon (D11), failed `Close` -> abandon + `recovery_pending`
    (D11/AC13), the recovery barrier's branches including
    successor-blocked-until-predecessor-recovered (AC12), the D10a handling for
    T01's recorded case — Case B's lane-scoped successor barrier proven to sweep
    a same-lane stale predecessor and leave an out-of-lane concurrent attempt
    untouched (AC9a, AC11) — and, if T01 proved a deny applies, the
    background-execution deny (AC15). AC1 routing test passes;
    `sce hooks codex-mutation-scope </dev/null` shows the strict-parser error
    and `sce hooks --help` omits it. AC19 dependency-boundary grep is clean.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope`; `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    services::hooks::mutation_scope`; `services::parse::` routing tests; live
    binary check of the hidden command; AC19 grep.
  - Context synchronization: pending

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
    adapted to T01 findings — expect at least:
    1. successful mutation-capable tool -> `AiExclusive` + `Closed` (AC8);
    2. failed tool with partial mutation -> the D10 disposition's assertion
       (Close-and-attribute, or abandon-then-`IneligibleUnscoped` after flush)
       (AC9);
    2a. failed tool A (partial mutation, no terminal event) followed by
        `PreToolUse(B)` in the same turn -> A is `Abandoned` and the worktree
        re-baselined before B's `Start`; B is the only live scope at its
        `Start`; the mutation-event stream contains **no** `AiContended` row
        attributable to the A/B overlap; and an out-of-lane concurrent attempt
        (if T01's model allows one) is not swept (AC9a) — asserted per T01's
        recorded D10a case; if Case C, this row is replaced by a comment
        pointing at the stopped-for-re-planning disposition;
    3. two overlapping mutation executions -> `AiContended` (the D14 form T01
       allows) (AC10);
    4. duplicate `Pre`/terminal delivery -> idempotent, no second transition
       (AC4);
    5. denied execution -> no mutation runs under an untracked `Start`
       (AC7 production half);
    6. interrupted / stale execution -> conservative abandonment via the proven
       D12 signal (AC11);
    7. reused raw Codex tool identifier after terminal -> new `ScopeId` (AC5);
    8. cwd/worktree isolation -> correct `WorktreeId`/cursor, other worktree
       unchanged (AC14);
    9. crash before `Start` commit -> conservative recovery (AC21a);
    10. `Start` committed before bookkeeping settlement -> abandonment recovery,
        not late-`Start` (AC21b);
    11. terminal transition committed before bookkeeping cleanup -> replay-safe
        (AC21c);
    12. `recovery_pending` blocks a successor until recovery succeeds (AC12);
    13. any unsupported background execution is rejected / documented (AC15);
    14. mutation-scope path does not alter `diff_traces`,
        `post_commit_patch_intersections`, `agent_traces`, `messages`, `parts`
        (before/after row counts) (AC20).
    Each applicable test asserts scope status, processed-event keys, revision,
    `cursor_tree`, mutation-event count, attribution kind, `needs_rebaseline`,
    and adapter state. Out — new production behavior; any process supervision or
    detached-child detection; any test that inserts the event it means to prove.
  - Dependencies: T04 (T05 only for a test that installs generated settings —
    prefer driving the adapter entry point directly).
  - Done when: the whole matrix passes and collectively satisfies AC4, AC5,
    AC7 (production half), AC8, AC9, AC9a, AC10, AC11, AC12, AC14, AC15, AC20,
    AC21.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::hooks::codex_mutation_scope`; `nix develop -c
    ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    services::mutation_trace::`.
  - Context synchronization: pending

- [ ] T07: `Author the durable Codex mutation-scope context` (status:todo)
  - Task ID: T07
  - Scope: In — create `context/cli/codex-mutation-scope-integration.md` owning
    the Codex adapter domain (the tool-execution scope model, the D2
    classification table, D3 execution identity, D4 `ScopeId`/`EventId`
    derivation, D5/D6 bookkeeping store, D7 write-ahead, D8 exact Codex-native
    fail-closed response, D9/D10 terminal boundaries, D10a the failed-tool ->
    successor-tool handling and the recorded case (A/B/C) with the successor-Start
    invariant and the lane key, D12 cleanup signals + the load-bearing backstop,
    D13 recovery barrier, D15 worktree/cwd ownership, D14 concurrency, D16
    background/detached limitations, D17 command architecture, D18 dependency
    direction, and D20/D21/D22 the Codex hook trust-identity preservation,
    three-dimension doctor health model, and upstream-verified event key
    labels), with an explicit **Unsupported** section (AC24) and the tested Codex
    version; and update
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

**T01 outcome (codex-cli 0.153.4, upstream `openai/codex` `rust-v0.153.4` /
`3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`): Codex is representable by the
current mutation-scope contract — the plan proceeds to T02.** Every empirical
risk below is resolved by a captured fixture or an upstream schema citation
(see the Design section dispositions and
`cli/src/services/hooks/codex_mutation_scope/fixtures/NOTES.md`). Headline
resolutions:

- **D10a is Case A / a non-issue on 0.153.4.** A failed shell tool still fires
  `PostToolUse`; a failed `apply_patch` writes nothing; interruption ends the
  turn. No serial-lane successor barrier ships.
- **`SessionEnd` is the load-bearing cleanup backstop** (fires on clean exit and
  on SIGINT); `Interrupt` is an additional earlier interruption signal the plan
  did not know about (Codex has **12** hook events, not 11).
- **The fail-closed `PreToolUse` response**: both `{"decision":"block"}` and
  `hookSpecificOutput.permissionDecision:"deny"` block the tool; T04 emits the
  `hookSpecificOutput` shape.
- **Codex runs mutation-capable tools strictly serially** → Codex-alone
  `AiContended` is not reachable; the AC10 regression crosses harnesses.
- **Codex exposes a delegated-agent identity** (`agent_id`, on subagent events
  only) — the plan uses it and does not invent one.
- **Command architecture**: T01 found nothing against the recommended separate
  hidden `sce hooks codex-mutation-scope` command; T02 still decides.
- **Existing-hook trust identity**: the upstream label map is
  `codex-rs/hooks/src/lib.rs` 96–108; a position-stable additive merge is a T05
  implementation constraint, not an unknown.

The real risks are empirical, not architectural, and every one is deliberately
deferred to T01 evidence rather than guessed here:

- **Failed tool with no terminal event, then another tool in the same turn.**
  (D10a — the single highest-risk correctness question.) Codex has no
  `PostToolUseFailure` and prior SCE research found `PostToolUse` fires only on
  success, so a mutation-capable tool A that mutated then failed can leave no
  terminal hook. If `PreToolUse(B)` then arrives before any `Stop`/`SessionEnd`,
  the `recovery_pending` barrier is unarmed and B would `Start` alongside a
  zombie live A — false `AiContended` or misattribution. T01's dedicated probe
  records one of: **Case A** (a reliable intermediate signal retires A),
  **Case B** (Codex is proven to run mutation-capable tools serially within a
  narrow identity-defined lane, so `PreToolUse(B)` proves same-lane A stale and
  the adapter runs a lane-scoped abandon+flush before `Start(B)`), or **Case C**
  (neither is safe -> unsupported lifecycle, stop for re-planning). The
  invariant: B must never `Start` while a known-stale A is live; and a
  legitimately-concurrent out-of-lane attempt must never be swept for it.
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
- **Can Codex overlap its own mutation-capable executions?** (D1/D14.) If not,
  Codex-alone `AiContended` is unreachable and the concurrency regression must
  cross harnesses.
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
schema / attribution-algorithm change is expected: the generic mutation-scope
contract already models `Start`/`Advance`/`Close`/`Flush`/`abandon`, already
accepts `ActorKind::Codex`, and already handles replay idempotency, conservative
recovery, and `AiContended` — the Claude adapter proved the contract is
sufficient for a concrete harness without touching any of those. If T01 proves
Codex genuinely cannot be represented by it, T01 stops and records the
contradiction here for re-planning; it does not quietly modify the protocol.
