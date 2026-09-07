# T01 fixture capture notes — Codex mutation-scope integration

Raw Codex CLI hook-event payloads captured live by wiring a throwaway dump hook
into a **scratch** git repository's `.codex/hooks.json` (never the SCE repo's) and
driving scenarios with `codex exec`. Every `*.json` file in this directory is an
unmodified byte-for-byte copy of what the real `codex` binary wrote to the hook
script's STDIN, **except the `*.evidence.json` metadata files** (one for the
built-in probe 9, five for the MCP probes 13–17), which are clearly marked with a
leading `_comment` and contain only capture metadata (timestamps, git-status
observations, event ordering, upstream citations), not hook payloads. The
built-in captures (probes 1–11) predate the MCP lifecycle extension (probes
12–17); see the "T01 MCP lifecycle probe extension" section for that harness.

## Tested Codex version

- **`codex-cli 0.153.4`** (`codex --version`) — the version installed in this
  environment; "the version SCE chooses to support" per T01's scope, the same way
  #263's T01 pinned Claude Code `2.1.258`.
- Model reported in every payload: `gpt-5.6-sol`. `hooks` feature flag: `stable`,
  enabled. `multi_agent` feature flag: `stable`, enabled.
- **Inspected upstream `openai/codex` at tag `rust-v0.153.4`, commit
  `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`.** Authoritative source files:
  - `codex-rs/hooks/schema/generated/*.command.input.schema.json` /
    `*.command.output.schema.json` — the wire schema for every hook event.
  - `codex-rs/hooks/src/schema.rs` — `HookEventNameWire` enum (11 input variants;
    `SessionEnd` is a separate struct) and the `PreToolUse` output wire type.
  - `codex-rs/hooks/src/lib.rs` lines 96–108 — the normalized event-key labels
    (used for `$CODEX_HOME/config.toml` `[hooks.state]` keys and for
    `codex_hook_config::hook_event_key_label`), verified NOT to be a naive
    lowercase in every case (they happen to be snake_case here, but the mapping
    is explicit upstream — cite this file, do not lowercase — D22 / AC17a):

    | `hooks.json` key (PascalCase) | normalized label |
    | --- | --- |
    | `PreToolUse` | `pre_tool_use` |
    | `PermissionRequest` | `permission_request` |
    | `PostToolUse` | `post_tool_use` |
    | `PreCompact` | `pre_compact` |
    | `PostCompact` | `post_compact` |
    | `SessionStart` | `session_start` |
    | `SessionEnd` | `session_end` |
    | `UserPromptSubmit` | `user_prompt_submit` |
    | `SubagentStart` | `subagent_start` |
    | `SubagentStop` | `subagent_stop` |
    | `Stop` | `stop` |
    | `Interrupt` | `interrupt` |
  - `codex-rs/core/src/tools/registry.rs` ~line 674 — `PostToolUse` hooks run
    **only when `success_for_logging()` is true** for the tool result; a shell
    command that executed then exited non-zero still counts as a successful tool
    result (the exit code is data), while an `apply_patch` that fails verification
    does not.
  - `codex-rs/core/src/hook_runtime.rs` — `PreToolUseHookResult::Blocked`
    ("Command blocked by PreToolUse hook: …"), the `SessionEnd` and `Interrupt`
    transcript-flush points (both fire on interruption).

## Capture method

`cli/src/services/hooks/codex_mutation_scope/fixtures/` did not exist before this
task. In a scratch repo (`$SCRATCH/probe-repo`, throwaway; not the SCE checkout):

- `dump.sh` — reads raw STDIN, writes it verbatim to a per-event file plus a
  sequential `_sequence.log`, then exits 0 with empty stdout (neutral no-op).
  Registered as a handler on every candidate event.
- `block.sh` — a second `PreToolUse` handler (appended after `dump.sh` in group 0)
  that, only when the payload contains a unique marker, returns either
  `{"decision":"block","reason":…}` or
  `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":…}}`.
- Driven with `codex exec --dangerously-bypass-hook-trust
  --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check -C <repo>`.
  `--dangerously-bypass-hook-trust` is why no `$CODEX_HOME/config.toml` trust
  state was written for the scratch hooks; nothing was added to the SCE repo.

## Per-probe manifest and disposition

| # | Probe | Result | Fixtures |
|---|---|---|---|
| 1 | `apply_patch` success + shell success, full lifecycle | captured | `probe01-apply-patch-and-shell-success.*` |
| 2 | shell writes a file then `exit 7` (partial-write failure), then a successor shell tool in the same turn | captured | `probe02-shell-partial-write-then-nonzero-exit.*` |
| 3 | another `PreToolUse` hook returns `{"decision":"block"}` | captured — **blocks the tool** | `probe03-pre-tool-use-hook-decision-block.*` |
| 4 | another `PreToolUse` hook returns `hookSpecificOutput.permissionDecision:"deny"` | captured — **also blocks the tool** | `probe04-pre-tool-use-hook-hookspecificoutput-deny.*` |
| 5 | tool vocabulary — read / list / search / edit | captured | `probe05-tool-vocabulary.*` |
| 6 | `apply_patch` that fails verification, then a successor shell tool | captured — **no `PostToolUse` for the failed patch** | `probe06-apply-patch-verification-failure-no-post.*` |
| 7 | SIGINT during a running shell tool (no `Interrupt` hook registered) | captured | `probe07-sigint-during-shell.*` |
| 8 | subagent delegation (`spawn_agent` / `wait_agent`) | captured | `probe08-subagent-delegation.*` |
| 9 | foreground shell spawns a self-detaching descendant that mutates the repo after `PostToolUse` | captured + Git-observability evidence | `probe09-self-detaching-descendant.*` |
| 10 | linked `git worktree` — hook `cwd` authority | captured | `probe10-linked-worktree-cwd.*` |
| 11 | SIGINT during a running shell tool, **with** an `Interrupt` hook registered | captured | `probe11-interrupt-event-on-sigint.*` |
| — | parallel mutation executions (built-in) | not reproduced — Codex ran every **built-in** tool serially in probes 1–11 | see D1/D14 |
| — | parallel mutation executions (MCP) | **REPRODUCED LIVE** — see probes 16/17 | see D1/D14 + MCP extension |
| — | `PermissionRequest` denial | not reachable from `codex exec` (non-interactive, bypass mode); documented from the upstream output schema | see D8/D12 |
| — | `PreCompact` / `PostCompact` | not triggered; documented from upstream as diagnostic-only | see D12 |
| 12 | MCP `mutate_success` — success lifecycle + `tool_name` shape | captured — `PreToolUse → PostToolUse`, same `tool_use_id` | `probe12-mcp-mutate-success.*` |
| 13 | MCP `mutate_then_error` — mutates git-visible file **then** returns `is_error:true` | captured — **NO `PostToolUse`**; mutation survives | `probe13-mcp-mutate-then-error.*` |
| 14 | failed MCP tool A → successor mutation-capable MCP tool B, same turn (the direct D10a probe) | captured — **no event of any kind between `PreToolUse(A)` and `PreToolUse(B)`** | `probe14-mcp-failed-then-successor.*` |
| 15 | MCP call blocked by a `PreToolUse` hook (`permissionDecision:"deny"`) | captured — `PreToolUse` only, no `PostToolUse`, no mutation | `probe15-mcp-blocked-call.*` |
| 16 | two mutation-capable MCP executions in parallel — server `supports_parallel_tool_calls = true` | captured — **genuinely concurrent**, two scopes live at once | `probe16-mcp-parallel-server-optin.*` |
| 17 | two mutation-capable MCP executions in parallel — via the tool's own `annotations.readOnlyHint` (no server opt-in) | captured — **genuinely concurrent** | `probe17-mcp-parallel-readonly-hint.*` |
| — | MCP tool naming | **PROVEN — `mcp__<server>__<tool>`** (`mcp__probe__mutate_success`); `tool_use_id` is `exec-<uuid>` (same shape as shell / `apply_patch`, not `call_<id>`) | `probe12…pre_tool_use.json` |

## Observed event sequences (from `_sequence.log`)

```
probe 1  : session_start → user_prompt_submit → PreToolUse(apply_patch) → PostToolUse(apply_patch)
           → PreToolUse(Bash) → PostToolUse(Bash) → Stop → SessionEnd
probe 2  : … → PreToolUse(Bash, exits 7) → PostToolUse(Bash)  ← fires on the failed tool
           → PreToolUse(Bash successor) → PostToolUse(Bash successor) → Stop → SessionEnd
probe 3/4: … → PreToolUse(blocked)  ← NO PostToolUse
           → PreToolUse(successor) → PostToolUse(successor) → Stop → SessionEnd
probe 6  : … → PreToolUse(apply_patch, verification fails)  ← NO PostToolUse
           → PreToolUse(Bash successor) → PostToolUse(Bash successor) → Stop → SessionEnd
probe 7  : … → PreToolUse(Bash) → SessionEnd          ← no PostToolUse, no Stop, no Interrupt hook wired
probe 8  : … → PreToolUse(spawn_agent) → PostToolUse(spawn_agent) → SubagentStart
           → PreToolUse(wait_agent) → PreToolUse(apply_patch, agent_id=A) → PostToolUse(apply_patch, agent_id=A)
           → SubagentStop(agent_id=A) → PostToolUse(wait_agent) → Stop → SessionEnd
probe 11 : … → PreToolUse(Bash) → Interrupt → SessionEnd   ← still no PostToolUse, no Stop
```

## Payload shape (Codex 0.153.4, cross-checked against the generated schemas)

- **`PreToolUse` / `PostToolUse`** required: `hook_event_name`, `cwd`,
  `session_id`, `turn_id`, `model`, `permission_mode`, `tool_name`,
  `tool_use_id`, `tool_input`, `transcript_path` (nullable); `PostToolUse` also
  requires `tool_response` (an untyped value — a string in practice: `""` for a
  shell command with no stdout, `"Exit code: 0\nWall time: …\nOutput:\n…"` for
  `apply_patch`, command stdout when present). `agent_id` + `agent_type` appear
  **only for subagent** tool executions (not in the schema's `required` list).
- **`Stop`** required: `+ last_assistant_message` (nullable — `null` under
  `codex exec`), `stop_hook_active`. No `tool_use_id`.
- **`SessionStart`**: `session_id`, `transcript_path`, `cwd`, `model`,
  `permission_mode`, `source` ("startup"). No `turn_id`.
- **`SessionEnd`**: `session_id`, `transcript_path`, `cwd`, `reason` — `reason` is
  `const "other"` upstream (identical for a clean exit and a SIGINT). No output
  schema → cannot respond. No `turn_id`, `model`, or `permission_mode`.
- **`Interrupt`**: `session_id`, `turn_id`, `transcript_path`, `cwd`, `model`,
  `permission_mode`. Fires on SIGINT before `SessionEnd` (probe 11).
- **`SubagentStart`**: `session_id`, `turn_id` (the agent's), `agent_id`,
  `agent_type`, `transcript_path` (the agent's), `model`, `permission_mode`. No
  tool fields.
- **`SubagentStop`**: `+ agent_transcript_path`, `last_assistant_message`
  (string), `stop_hook_active`; `transcript_path` here is the parent's.
- The delegation tools' own `tool_name` values are `collaborationspawn_agent` and
  `collaborationwait_agent`; their `tool_use_id` is `call_<id>` (function-call
  style), whereas shell / `apply_patch` executions use `exec-<uuid>`.

## Design-decision dispositions (written back into the plan's Design section)

- **D1 / D14 — concurrency:** **scope-split by tool type.**
  - **Built-in `Bash` / `apply_patch`:** `ASSUMPTION — PROBE` (leaning serial).
    Codex executed every built-in mutation-capable tool strictly serially in all
    11 probes (`Pre → Post → Pre → Post …`, never interleaved), including when
    asked to parallelise and across the parent/subagent boundary.
  - **MCP:** `PROVEN (live)` — two mutation-capable MCP executions **do** overlap
    (probes 16/17). **Codex-alone `AiContended` IS reachable via MCP** on 0.153.4.
  The "serial / `AiContended` unreachable" conclusion is therefore correct **only
  for the built-in tools**. The AC10 regression still crosses harnesses; if MCP
  stays supported, T06 must add an MCP-overlap `AiContended` regression. The
  adapter never collapses two executions into one `ScopeId`. See the "T01 MCP
  lifecycle probe extension" section below.
- **D2 — tool classification:** `PROVEN` for the `codex exec` surface.
  Mutation-capable (establish a scope): `apply_patch`, `Bash` (the shell tool —
  it also performs reads/list/search via shell commands, so it is always treated
  mutation-capable and a read-only shell command simply creates a harmless
  scope). Delegation (never a scope): `collaborationspawn_agent`,
  `collaborationwait_agent`. There are **no dedicated built-in read-only tool
  names** in this surface. MCP tools and any unknown `tool_name` →
  conservatively mutation-capable. **MCP naming is now `PROVEN` live —
  `mcp__<server>__<tool>` (probes 12–17).** Classification conservatism
  (`unknown/MCP → mutation-capable`) is **only** for `Start` / fail-closed and
  confers **no lifecycle-support guarantee**: an unknown/MCP tool inherits none
  of the `Bash` / `apply_patch` terminal guarantees (probes 13/14). See the MCP
  extension section below.
- **D3 — execution identity:** `PROVEN`. Key = `(session_id, agent_id?,
  tool_use_id)`. `tool_use_id` is present and identical on the `PreToolUse` and
  `PostToolUse` for one call. `session_id` is stable across a whole session
  including subagents; `turn_id` differs per turn and per subagent; `agent_id`
  (a UUID) is present only on subagent events and distinguishes a delegated
  agent from the main thread. Codex **does** expose a delegated-agent identity —
  the plan must use `agent_id`, and must not invent one where it is absent
  (= main thread). Raw `tool_use_id`s are UUID-based and not observed to recur,
  but the D4 checkout-local `attempt_seq` guard is retained anyway.
- **D8 — fail-closed `PreToolUse` response:** `PROVEN`. **Both** shapes block the
  tool on 0.153.4 and are both in the generated output schema
  (`pre-tool-use.command.output.schema.json`): top-level
  `{"decision":"block","reason":…}` (enum `approve|block`) **and**
  `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":…}}`
  (enum `allow|deny|ask`). Recommend the adapter emit the `hookSpecificOutput`
  shape, matching the existing `sce hooks codex` `PreToolUse(Bash)` policy arm.
  A blocked tool fires `PreToolUse` only — **no `PostToolUse`** — so a fail-closed
  denial leaves no scope needing a terminal action.
- **D9 — terminal boundary on success:** `PROVEN`. `PostToolUse` is the reliable
  terminal signal for a successful mutation-capable tool, carrying the same
  `tool_use_id` (and `agent_id` for a subagent) as its `PreToolUse`.
- **D10 — failed-tool terminal observation:** **scope-split by tool type.**
  - **`Bash`:** `PROVEN` — a shell tool that wrote a file then exited non-zero
    **does** fire `PostToolUse` (probe 2), same `tool_use_id` — maps to `close`
    like D9; a partial mutation from a failed shell command is bounded by a
    terminal hook.
  - **`apply_patch`:** `PROVEN` — a verification failure fires **no**
    `PostToolUse` (probe 6), but Codex verifies before touching the tree, so
    nothing is written; no partial-mutation-without-terminal case.
  - **MCP:** `PROVEN` — a mutation-capable MCP tool that **mutates then returns
    `is_error:true`** fires **no terminal hook of any kind** (probe 13:
    `PreToolUse → Stop → SessionEnd`), and the mutation (`mcp_b.txt`) is
    git-visible afterwards. This is a real partial-mutation-without-terminal
    case. An external MCP server is not under Codex's atomicity control.
  - **unknown:** no tool-specific terminal guarantee; governed only by
    tool-generic lifecycle policy.
  Prior SCE research's "`PostToolUse` fires only on a successful tool result" is
  true at the `success_for_logging()` layer
  (`codex-rs/core/src/tools/registry.rs` ~674); a non-zero-exit shell command
  still counts as a successful tool result, but a `CallToolResult` with
  `is_error:true` does not (`McpToolOutput::success_for_logging` =
  `self.result.success()`). The adapter needs no Close-on-failure path for
  `Bash` / `apply_patch`; for MCP it has **no terminal signal at all**.
- **D10a — failed tool → successor tool in the same turn:** **scope-split by
  tool type.**
  - **Built-in `Bash` / `apply_patch`: `PROVEN — Case A`.** A failed shell tool
    always emits `PostToolUse` before the next `PreToolUse` (probe 2); a failed
    `apply_patch` never mutates; a hook-blocked tool never executes (probes 3/4);
    the only "partial mutation, no `PostToolUse`" case is whole-turn interruption
    (probes 7/11) which ends the turn. No serial-lane successor barrier is
    required for built-ins.
  - **MCP: `PROVEN — Case C`.** Probe 13: a mutation-capable MCP tool can mutate
    then fail with **no terminal hook**. Probe 14: the failed MCP tool A is
    followed **directly** by mutation-capable MCP tool B with **no event of any
    kind between them** — no positive stale/terminal evidence for A. Probes
    16/17: MCP executions **can overlap**, so `PreToolUse(B)` does **not** prove
    A stale. The adapter cannot distinguish `failed-and-dead A` from
    `still-running A`. This is D10a **Case C *if MCP is modeled as a scope***.
    **Resolved 2026-09-08 by re-planning direction B** — MCP/unknown are
    `Untracked` (no scope, no bookkeeping), so this lifecycle can never strand a
    scope. See the "Re-planning resolution" note at the end of this file and the
    plan's D23 / Open questions.
- **D12 — lifecycle cleanup signals:** `PROVEN`.
  - `Stop` — main-turn end, carries `session_id` + `turn_id`. Fires only on a
    clean turn end (not on interruption).
  - `SessionEnd` — the **load-bearing backstop**. Fires on a clean exit **and**
    on SIGINT (probes 7, 11), carries `session_id` + `cwd`. No `turn_id`/
    `agent_id` → a whole-session sweep.
  - `Interrupt` — fires on SIGINT **before** `SessionEnd` (probe 11), carries
    `session_id` + `turn_id`. A newly-observed signal the plan's D12 table does
    not list; usable as an earlier session/turn-scoped sweep, with `SessionEnd`
    still the backstop.
  - `SubagentStop` — carries `agent_id`; retires the ending delegated agent's
    outstanding attempts (probe 8).
  - `PermissionRequest` (denied) — `hookSpecificOutput.decision.behavior:"deny"`
    per the upstream output schema; not reachable from `codex exec`, so treated
    as `DOCUMENTED — NON-LOAD-BEARING`, with `SessionEnd` as the backstop.
  - `PreCompact` / `PostCompact` — not observed; `DOCUMENTED — NON-LOAD-BEARING`
    (diagnostic only).
- **D15 — raw hook `cwd` is authoritative:** `PROVEN`. Every payload's `cwd` was
  the `codex exec -C` directory; running against a linked `git worktree` (probe
  10) reported the worktree path in `cwd`, and the write landed in the worktree,
  not the main checkout. `checkout::resolve_git_dir(cwd)` resolves the
  worktree-specific `.git/worktrees/<name>` directory. Codex exposes **no**
  worktree-lifecycle event (no `WorktreeRemove` equivalent among the 12 event
  names); worktree-scoped cleanup relies on the `SessionEnd` / `Interrupt` /
  `SubagentStop` sweeps.
- **D16 — background / detached shell:** `PROVEN` (self-detaching descendant).
  The default `codex exec` shell tool has **no `run_in_background` parameter**
  (params are `command`, `workdir`, `timeout_ms`, `with_escalated_permissions`,
  `justification`), so there is no "explicit Codex-managed background execution"
  to deny in `PreToolUse` for this surface. A foreground shell command that
  `setsid`-detaches a descendant **does** leave a Git-observable mutation landing
  ~4s after `PostToolUse` (probe 9 + `probe09-…​.evidence.json`) — same class as
  the Claude adapter's D20. Recorded as an explicit unsupported boundary; the
  adapter adds no PID supervision, process-group tracking, shell static analysis,
  or staleness polling.
- **D17 inputs — command architecture:** the mutation-scope adapter needs
  registrations for at least `PreToolUse`, `PostToolUse`, `Stop`, `SessionEnd`,
  `SubagentStop` (and optionally `Interrupt`). The existing `sce hooks codex`
  dispatcher is fail-open; the mutation-scope adapter is fail-closed on
  `PreToolUse`. A **separate hidden `sce hooks codex-mutation-scope` command**
  (D17 option 1) remains the recommended default — its registrations use a
  distinct command so Codex invokes it as its own process, exactly as the
  existing `PreToolUse(Bash)` policy hook and a mutation-scope hook would run
  side by side. T02 makes the final call.

## Newly-discovered facts the plan did not anticipate

- Codex 0.153.4 has **12** hook events, not 11: the plan's list omits
  **`Interrupt`** (`hook_event_name: "Interrupt"`, label `interrupt`). D22 / the
  `hook_event_key_label` work must include it if the adapter registers it.
- `apply_patch`'s hook `tool_response` is a **string**
  (`"Exit code: 0\nWall time: …\nOutput:\n…"`), not the `{"success": true}`
  object some existing `sce hooks codex` tests assume. Not load-bearing for
  mutation-scope (the adapter does not parse `tool_response`), flagged for T05.
- The delegation tool names are `collaborationspawn_agent` /
  `collaborationwait_agent` (a `collaboration` namespace prefix with no
  separator), not a bare `spawn_agent`.

---

# T01 MCP lifecycle probe extension (probes 12–17)

The original probes 1–11 proved the failed-tool / serial-execution / successor-safety
story for the **built-in** `Bash` and `apply_patch` tools only. The plan then
generalised those conclusions to MCP / unknown mutation-capable tools **without
equivalent evidence**. This extension live-probes MCP against the same supported
version and finds the generalisation is **wrong**.

## Probe infrastructure

`cli/src/services/hooks/codex_mutation_scope/fixtures/mcp_probe/` (probe-only,
**not SCE runtime code**):

- `server.py` — a ~250-line zero-dependency stdio MCP server (MCP 2025-06-18,
  JSON-RPC over stdio). Tools, all deliberately mutation-capable (each writes a
  git-visible file into the scratch repo):
  - `mutate_success` — write a file, return success.
  - `mutate_then_error` — write a file **first**, then return `is_error:true`.
  - `slow_mutate` — write a file, sleep ~8 s (on its own thread), write a second
    file, return success. Long enough for overlap to be observable.
  - `read_only_liar` — same as `slow_mutate` but annotated
    `annotations.readOnlyHint = true` while still mutating.
- `dump.sh` / `block.sh` — the same neutral dump hook + marker-gated
  `permissionDecision:"deny"` hook as probes 1–11, wired onto all 12 events.
- `run-probes.sh` — builds a scratch git repo + a private `CODEX_HOME` (auth
  only), writes `config.toml` with two MCP servers (`probe`, and `probe_par`
  carrying `supports_parallel_tool_calls = true`), and drives `codex exec
  --dangerously-bypass-approvals-and-sandbox --dangerously-bypass-hook-trust
  --skip-git-repo-check` once per probe. Nothing touches the SCE checkout or the
  real `$CODEX_HOME`.
- `config.toml.sample` / `hooks.json.sample` — the generated config, path-sanitised.

## Observed MCP event sequences

```
probe 12 : session_start → user_prompt_submit
           → PreToolUse(mcp__probe__mutate_success) → PostToolUse(same tool_use_id)
           → Stop → SessionEnd
probe 13 : … → PreToolUse(mcp__probe__mutate_then_error)   ← MCP writes mcp_b.txt, returns is_error:true
           → Stop → SessionEnd                              ← NO PostToolUse at all
probe 14 : … → PreToolUse(A = mcp__probe__mutate_then_error)  ← writes mcp_c1.txt, is_error:true, tool_use_id exec-27777ab1
           → PreToolUse(B = mcp__probe__mutate_success)        ← nothing between A and B
           → PostToolUse(B) → Stop → SessionEnd               ← A's tool_use_id never recurs; A has no terminal hook
probe 15 : … → PreToolUse(mcp__probe__mutate_success)  ← blocked by permissionDecision:"deny"
           → Stop → SessionEnd                          ← no PostToolUse, no tools/call, no mutation
probe 16 : … → PreToolUse(A) 14:44:32.4187 → PreToolUse(B) 14:44:32.4199   ← both before any Post
           → PostToolUse(A) 14:44:40.4431 → PostToolUse(B) 14:44:40.4502
           server: slow_mutate[d1] begin :32.431, slow_mutate[d2] begin :32.439 (d1 still sleeping), both end :40.43x
probe 17 : same interleaving as 16, reached via annotations.readOnlyHint with NO server opt-in
```

## MCP design-decision dispositions

- **MCP tool naming (D2):** `PROVEN` — `mcp__<server>__<tool>`
  (`mcp__probe__mutate_success`, `mcp__probe_par__slow_mutate`). `tool_use_id` is
  `exec-<uuid>` — the **same shape** as shell / `apply_patch`, *not* the
  `call_<id>` form used by the `collaboration*` delegation tools. `tool_name` is
  identical on `PreToolUse` and `PostToolUse`; the execution key
  `(session_id, agent_id?, tool_use_id)` (D3) holds unchanged for MCP.
  Upstream contract: `MCP_TOOL_NAME_DELIMITER` / `join_tool_name` /
  `ensure_mcp_prefix` in `codex-rs/core/src/tools/handlers/mcp.rs` at
  `rust-v0.153.4`.

- **Successful MCP call (D9):** `PROVEN` — `PostToolUse` is the reliable terminal
  hook, same `tool_use_id` as its `PreToolUse` (probe 12). `tool_response` for MCP
  is a **structured object** `{"content":[…],"isError":false}`, not a string as
  for `apply_patch` — not load-bearing (the adapter does not parse it).

- **Blocked MCP call (D8):** `PROVEN` — a hook-blocked MCP `PreToolUse`
  (`hookSpecificOutput.permissionDecision:"deny"`) fires `PreToolUse` only, **no
  `PostToolUse`**, `tools/call` is never issued, nothing is written (probe 15).
  Identical to the built-in blocked-tool case (probes 3/4). A D8 fail-closed deny
  on a mutation-capable MCP `PreToolUse` therefore strands no scope.

- **Failed MCP call — `mutate_then_error` (D10):** `PROVEN` — a mutation-capable
  MCP tool that **writes a git-visible file and then returns `is_error:true`**
  receives **no terminal hook of any kind** (probe 13): `PreToolUse` → `Stop` →
  `SessionEnd`, no `PostToolUse`. `git status` after shows `mcp_b.txt`, mtime
  `16:43:33.836` — the mutation landed and survives the failed result. This is
  **not** the built-in `apply_patch` story (atomic pre-verification, nothing
  written) and **not** the built-in shell story (`PostToolUse` still fires on a
  non-zero exit). Upstream mechanism: `codex-rs/core/src/tools/registry.rs`
  ~line 674 — `let post_tool_use_payload = if success { … } else { None }` with
  `success = result.success_for_logging()`; for MCP,
  `McpToolOutput::success_for_logging()` = `self.result.success()`
  (`codex-rs/core/src/tools/context.rs:122-124`), which is false when the
  `CallToolResult` carries `is_error:true`. An external MCP server is not under
  Codex's atomicity control, so the write can precede the failure.
  Evidence: `probe13-mcp-mutate-then-error.{pre_tool_use,stop,session_end,evidence}.json`.

- **Failed MCP tool → successor tool, same turn (D10a):** `PROVEN — Case C for
  MCP *if modeled as a scope*` (resolved by direction B — MCP is `Untracked`).
  Probe 14: `PreToolUse(A = mutate_then_error)` is followed **directly** by
  `PreToolUse(B = mutate_success)` with **no intervening event** — no
  `PostToolUse(A)`, no `Interrupt`, no `Stop`, no `SubagentStop`, no
  `PermissionRequest`, no compaction event. A's `tool_use_id` (`exec-27777ab1…`)
  appears in exactly one hook delivery. Both `mcp_c1.txt` and `mcp_c2.txt` land.
  There is **no positive stale/terminal evidence for A** before B starts, and —
  because MCP executions *can* overlap (probes 16/17) — `PreToolUse(B)` does
  **not** prove A stale. The adapter cannot distinguish `failed-and-dead A` from
  `still-running A`. This is exactly D10a **Case C *if MCP is modeled as a
  scope***. **Resolved 2026-09-08 by re-planning direction B** — the adapter does
  not model MCP as a scope (MCP/unknown = `Untracked`), so no A attempt exists to
  strand. See the "Re-planning resolution" note at the end of this file.
  Evidence: `probe14-mcp-failed-then-successor.*`.

- **MCP concurrency (D1 / D14):** `PROVEN (live)` — two mutation-capable MCP tool
  executions run **genuinely concurrently** on 0.153.4. Probe 16 (server
  `supports_parallel_tool_calls = true`): `PreToolUse(A)` at `14:44:32.418731`,
  `PreToolUse(B)` at `14:44:32.419862` — 1.1 ms apart, both before either
  `PostToolUse`; the MCP server's own log shows `slow_mutate[d2]` begins while
  `slow_mutate[d1]` is still sleeping; both scopes are live between their
  `PreToolUse` and `PostToolUse` for ~8 s. Probe 17 reaches the same interleaving
  via the tool's own `annotations.readOnlyHint` with **no** server- or
  config-side opt-in. Upstream contract:
  `McpHandler::supports_parallel_tool_calls()` (`codex-rs/core/src/tools/handlers/mcp.rs:128-139`)
  = `tool_info.supports_parallel_tool_calls || annotations.read_only_hint`;
  `tool_info.supports_parallel_tool_calls` comes from `McpServerMetadata`
  (`codex-rs/codex-mcp/src/server.rs:395-421`) which reads the config.toml key
  `RawMcpServerConfig.supports_parallel_tool_calls`
  (`codex-rs/config/src/mcp_types.rs:362`). Therefore **Codex-alone `AiContended`
  IS reachable** on 0.153.4 whenever a mutation-capable MCP tool is
  parallel-eligible. The plan's blanket "Codex mutation-capable tools are serial /
  Codex-alone `AiContended` is unreachable" is true **only for the built-in
  `Bash` / `apply_patch` tools exercised by probes 1–11**.
  Evidence: `probe16-mcp-parallel-server-optin.*`, `probe17-mcp-parallel-readonly-hint.*`.

## Disposition terminology used above

- **PROVEN** — observed live in a captured fixture, or a deterministic structural
  fact that cannot differ at runtime.
- **DOCUMENTED — NON-LOAD-BEARING** — established from upstream source/schema, not
  load-bearing for adapter correctness, with a load-bearing backstop named.
- **ASSUMPTION — PROBE** — a leaning conclusion from limited observation, not
  proven.
- **UNSUPPORTED** — the lifecycle cannot be represented safely by the current
  mutation-scope contract; the adapter must fail closed / exclude / re-architect,
  and the plan stops for re-planning.

## Answers to the T01-extension questions

1. Successful MCP calls emit `PostToolUse`: **yes** (probe 12).
2. `mutate-then-error` MCP calls emit `PostToolUse`: **no** (probe 13).
3. An MCP side effect can survive a failed MCP result: **yes** — `mcp_b.txt` is
   git-visible after `is_error:true` with no terminal hook (probe 13).
4. A positive cleanup signal appears before a successor tool: **no** — nothing
   between `PreToolUse(A)` and `PreToolUse(B)` (probe 14).
5. MCP executions can overlap: **yes** — genuinely concurrent (probes 16, 17).
6. Codex-alone `AiContended` is reachable: **yes, via MCP** (probes 16/17);
   still **no** for built-in `Bash` / `apply_patch` (probes 1–11).
7. Final D10a disposition for MCP: **Case C *if MCP is modeled as a mutation
   scope***. Built-ins remain **Case A**. Re-planning (2026-09-08) resolved this
   by **not** modeling MCP as a scope — see the resolution note at the end of
   this file.
8. MCP remains supported operationally but is **outside Codex mutation-scope
   attribution coverage** in adapter v1 (re-planning **direction B**, chosen
   2026-09-08). MCP tools and unknown tool names are classified `Untracked`:
   allowed to execute, may mutate, no `Start`, no scope, no bookkeeping. The
   Case C evidence below is retained as the *reason* for the exclusion. Rejected:
   (A) deny MCP fail-closed. Deferred as future work: (C) a richer
   lifecycle/runtime mechanism for first-class MCP attribution.
9. Unknown tool names: classification conservatism (`unknown → mutation-capable`
   for `Start` / fail-closed) must be **separated** from a lifecycle-support
   guarantee. An unknown tool inherits **no** `Bash` / `apply_patch` terminal
   guarantee; its terminal/recovery behaviour may rely only on tool-generic
   lifecycle signals, else it is unsupported for trustworthy attribution.
10. Is T01 safe to mark done / proceed to T02: **yes, as of 2026-09-08** — the
    MCP D10a Case C finding was resolved by re-planning direction B (MCP/unknown
    are `Untracked`, outside coverage), which needs no protocol change. T01 is
    done; T02 is unblocked (not started). See the resolution note below.

## Re-planning resolution (2026-09-08) — direction B

The MCP D10a Case C finding above is **correct and retained**: *if a
mutation-capable MCP tool were represented as an SCE mutation scope, the Codex
0.153.4 hook lifecycle makes that scope's lifecycle unsafe* (probe 13:
mutate-then-error has no terminal hook; probe 14: a successor `PreToolUse` can
follow with no cleanup signal between; probes 16/17: parallel MCP execution is
real, so a successor cannot prove a predecessor stale).

**Resolution:** the Codex adapter v1 does **not** create mutation scopes for MCP
calls. `PreToolUse(mcp__…)` — and any unknown `tool_name` — is classified
`Untracked`: it executes normally, it may mutate, but the adapter emits no
`Start`, no `ScopeId`, no `EventId`, no attempt, and no `recovery_pending`, so no
`Close` / `Abandon` / `Flush` is ever needed for it. This is a deliberate
**attribution-coverage boundary**, not a lifecycle workaround — the adapter does
not claim MCP is read-only, does not guarantee MCP mutations are detected
immediately, and `AiExclusive` continues to mean "exactly one *tracked* scope was
live", not "sole authorship of the interval".

Coverage for Codex adapter v1:

| Class | Tools | Scope? |
| --- | --- | --- |
| `TrackedMutation` | `Bash`, `apply_patch` | yes — one execution → one `ScopeId` |
| `Delegation` | `collaborationspawn_agent`, `collaborationwait_agent` | no (the delegated agent's tracked tools get scopes) |
| `Untracked` | `mcp__*`, unknown `tool_name` | no — allowed, may mutate, outside coverage |

No `mutation_cursor.qnt` / mutation protocol / runtime-semantic / SQL-migration /
Agent Trace schema change is required. The plan's Design section (D1, D2, D8, D9,
D10, D10a, D12, D13, D14, new D23) and Open questions carry the full disposition.
The probe fixtures in this directory are unchanged evidence.
