# T01 fixture capture notes — Codex mutation-scope integration

Raw Codex CLI hook-event payloads captured live by wiring a throwaway dump hook
into a **scratch** git repository's `.codex/hooks.json` (never the SCE repo's) and
driving scenarios with `codex exec`. Every `*.json` file in this directory is an
unmodified byte-for-byte copy of what the real `codex` binary wrote to the hook
script's STDIN, except the two synthesised `*.evidence.json` metadata files, which
are clearly marked and contain only capture metadata (timestamps, tree/ignore
observations), not hook payloads.

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
| — | parallel mutation executions | not reproducible — Codex ran every tool strictly serially in probes 1–11 | see D1/D14 |
| — | `PermissionRequest` denial | not reachable from `codex exec` (non-interactive, bypass mode); documented from the upstream output schema | see D8/D12 |
| — | `PreCompact` / `PostCompact` | not triggered; documented from upstream as diagnostic-only | see D12 |
| — | MCP tool naming | no MCP server configured; documented from upstream (`<server>__<tool>` namespaced names) + the conservative "unknown → mutation-capable" default | see D2 |

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

- **D1 / D14 — concurrency:** `ASSUMPTION — PROBE` (leaning serial). Codex executed
  every mutation-capable tool strictly serially in all 11 probes (`Pre → Post →
  Pre → Post …`, never interleaved), including when asked to parallelise and
  across the parent/subagent boundary. Codex-alone `AiContended` is treated as
  not reachable for 0.153.4; the AC10 regression must cross harnesses (a Codex
  scope overlapping a second harness's scope on the same worktree). The adapter
  still never collapses two executions into one `ScopeId`.
- **D2 — tool classification:** `PROVEN` for the `codex exec` surface.
  Mutation-capable (establish a scope): `apply_patch`, `Bash` (the shell tool —
  it also performs reads/list/search via shell commands, so it is always treated
  mutation-capable and a read-only shell command simply creates a harmless
  scope). Delegation (never a scope): `collaborationspawn_agent`,
  `collaborationwait_agent`. There are **no dedicated built-in read-only tool
  names** in this surface. MCP tools and any unknown `tool_name` →
  conservatively mutation-capable (documented from upstream; none live here).
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
- **D10 — failed-tool terminal observation:** `PROVEN`, and **cleaner than the
  plan feared**. A shell (`Bash`) tool that wrote a file then exited non-zero
  **does** fire `PostToolUse` (probe 2), same `tool_use_id` — it maps to `close`
  exactly like D9, so a partial mutation from a failed shell command is bounded
  by a terminal hook. An `apply_patch` that fails verification fires **no**
  `PostToolUse` (probe 6) — but Codex verifies the patch before touching the
  working tree, so a failed `apply_patch` writes nothing; there is no
  partial-mutation-without-terminal case for it. Prior SCE research
  (`context/plans/codex-cli-integration.md`) said "`PostToolUse` fires only on a
  successful tool result" — that is true at the `success_for_logging()` layer,
  but an executed shell command with a non-zero exit still qualifies as a
  successful tool result. The adapter needs **no Close-on-failure path**.
- **D10a — failed tool → successor tool in the same turn:** `PROVEN — Case A`
  (the dangerous scenario does not arise on 0.153.4). A failed shell tool always
  emits `PostToolUse` before the next `PreToolUse` (serial execution, probe 2);
  a failed `apply_patch` never mutates; a hook-blocked tool never executes
  (probes 3/4). The only "partial mutation, no `PostToolUse`" case is whole-turn
  interruption (probes 7/11), which emits `Interrupt` and/or `SessionEnd` and
  ends the turn — there is no in-turn successor to race. **No serial-lane
  successor barrier is required**; T02 records the lane key as N/A.
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
