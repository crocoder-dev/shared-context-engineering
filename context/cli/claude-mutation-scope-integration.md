# Claude mutation-scope integration: the first concrete harness adapter

`sce hooks claude-mutation-scope` is the first concrete harness lifecycle
adapter targeting the mutation-scope runtime. It translates raw Claude Code
tool/lifecycle hook events into the normalized mutation-scope contract
implemented by
[`mutation-scope-hook-ingress.md`](mutation-scope-hook-ingress.md), reaching the
runtime only through that ingress's in-process seam — never `coordinate()` /
`abandon_scope()` directly, never a second `sce` subprocess. Built by the
`claude-mutation-scope-integration` plan; lives in
`cli/src/services/hooks/claude_mutation_scope/` (`mod.rs` — event model, tool
classification, `ScopeId`/`EventId` derivation, adapter driver; `state.rs` —
durable checkout-local bookkeeping). The generic ingress and the
[`mutation-scope-runtime.md`](mutation-scope-runtime.md) contract are unchanged.

## Command routing

Routes through the normal hook stack (`cli_schema::HooksSubcommand::
ClaudeMutationScope` -> `convert_hooks_subcommand_request` ->
`services::hooks::HookSubcommand::ClaudeMutationScope` ->
`run_hooks_subcommand_in_repo` -> `run_claude_mutation_scope_subcommand`), hidden
from help (`#[command(hide = true)]` on the nested clap variant), the dispatch
arm **unwrapped** like `mutation-scope` (no `Ok(...)` fail-open shim). It takes
no `repository_root` parameter — every repository root is read from the parsed
event payload (see [Raw cwd is authoritative](#raw-cwd-is-authoritative)). STDIN
is one raw Claude hook JSON object via `super::read_hook_stdin()`; success emits
**empty stdout**, except the deliberate `PreToolUse` permission-decision object
below.

## Scope model and tool classification

**One independently mutation-capable Claude tool execution = one SCE mutation
`ScopeId`.** A session, prompt, main agent, or subagent is never a scope;
`session_id` / `agent_id` are only identity inputs distinguishing tool
executions. Two parallel mutation-capable tools produce two simultaneously live
scopes and may correctly yield `AiContended`; sequential tool calls are
sequential scopes. `SessionStart`, `UserPromptSubmit`, and `SubagentStart`
establish no scope.

`classify_tool(tool_name)`:

- **Mutation-capable (establishes a scope):** any name not in the two lists
  below — `Bash`, `PowerShell`, `Write`, `Edit`, `NotebookEdit`, `MultiEdit`,
  every `mcp__*` tool, and any **unknown** name. Unknown-means-mutation-capable
  is deliberate: an unknown read-only tool only creates harmless scopes, whereas
  the opposite default would silently miss a new mutation-capable tool.
- **Read-only (never a scope):** `Read`, `Glob`, `Grep`, `WebFetch`,
  `WebSearch`, `AskUserQuestion`.
- **`Agent` (delegation, never a scope):** the subagent's own mutation-capable
  tool calls establish their own scopes (carrying its `agent_id`); a parent
  scope would fold every child mutation into it.

`is_explicit_background_shell(tool_name, run_in_background)` is a separate
model-only predicate (`true` only for `Bash`/`PowerShell` with
`run_in_background == true`); its denial is in the driver — see
[Background shell is unsupported](#background-shell-is-unsupported).

## Identity and ScopeId / EventId derivation

A tracked `PreToolUse` requires `session_id`, `cwd`, `tool_name`,
`tool_use_id`. Optional: `agent_id` (absent = main thread, present = subagent),
`prompt_id` / `agent_type` (diagnostics only). `parse_claude_hook_event` is
strict — an empty payload, non-object JSON, or a missing/blank/wrong-typed field
is rejected with `Invalid Claude hook event payload from STDIN: <detail>.`, and
no identity is ever fabricated.

The tool-execution key is `(session_id, agent_id?, tool_use_id)`. A raw
`tool_use_id` can recur (a deferred execution resumed) and a terminal SCE
`ScopeId` must never be reused, so `ScopeId` is **not** a pure function of
`tool_use_id`: the adapter keeps a monotonic checkout-local `next_attempt_seq`,
and each new attempt draws a fresh `attempt_seq`. `ScopeId` is a length-prefixed,
hash-free encoding (no crypto dependency), and `EventId`s derive
deterministically from it as `<scope-id>|start` / `<scope-id>|close`:

```text
cc-tool-v1|n=<attempt_seq>|s=<byte-len>:<session_id>|a=<byte-len>:<agent_id-or-empty>|t=<byte-len>:<tool_use_id>
```

Replaying the same hook event for one live attempt yields the same `ScopeId` and
`EventId` (the runtime's replay/idempotency key). After an attempt is terminal a
later `PreToolUse` for the same `tool_use_id` draws a new `attempt_seq` and a
new `ScopeId`; otherwise-identical tool IDs under main / `agent_id=A` /
`agent_id=B` produce three distinct `ScopeId`s; no `ScopeId` derives from
`agent_id` alone, so a resumed subagent reusing an `agent_id` is safe.

## Checkout-local adapter state

`state.rs` keeps cross-hook-process state at
`<git-dir>/sce/claude-mutation-scope-state.json` (`<git-dir>` via
`checkout::resolve_git_dir(cwd)` — worktree-specific for linked worktrees): a
versioned `{version, next_attempt_seq, recovery_pending, attempts[]}`, each
attempt carrying `attempt_seq`, `scope_id`, the identity fields, `tool_name`, and
`phase` (`pending_start | active`). This is **adapter bookkeeping, never
attribution evidence** — not exported, synced, or authoritative; its only job is
knowing which Claude-created scopes may still need a terminal action. A malformed
or wrong-version file is rejected, never fabricated.

Writes follow the checkout-identity durability pattern
(`checkout::persist_checkout_id_inner`: lock at
`<git-dir>/sce/claude-mutation-scope-state.lock`, temp file, `sync_data`, atomic
rename, best-effort parent `sync_all` on Unix). That lock is **never held across
a `mutation_scope` seam invocation**, so no `adapter lock -> WorktreeLock` order
can form. The adapter may call `checkout::resolve_git_dir` but not
`read_checkout_id` / `get_or_create_checkout_id`, and never constructs a
`WorktreeId`.

## PreToolUse: write-ahead Start, fail-closed

For a tracked mutation-capable tool, `handle_pre_tool_use` runs: explicit-background-shell check -> resolve `git_dir` -> recovery barrier ->
write-ahead `Start` — persist `phase=pending_start`, resolve the canonical
`cc_<session>` and the exact model-state key (`agent_id = ""` for the main
agent), call the seam with the optional provenance snapshot, persist
`pending_start -> active`, return empty success. The resolver is injectable for tests; production reads `claude_model_state_by_session_and_agent` from the
repository Agent Trace DB. Missing state or resolver errors become a null model
and never deny the mutation-capable tool. The seam receives the raw `cwd` as its
`repository_root` and SCE derives the `WorktreeId`, so durable generic-ingress
`Start` is reached before the hook returns success to Claude.

**Fail-closed via Claude's deny decision.** Claude treats ordinary non-2 hook failures as non-blocking, so a generic non-zero exit would let the tool run
without its `Start`. Therefore **any** failure in the mutation-capable
`PreToolUse` path — state-allocation failure, seam `Start` failure, unresolvable
`cwd`, or a barrier denial — returns (`Ok`, never `Err`):

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"SCE could not establish mutation attribution for this tool execution."}}
```

The detailed error is logged via `Logger::warn`
(`sce.hooks.claude_mutation_scope.pre_tool_use_fail_closed`); Claude's deny
reason never carries it. Model-state lookup failures are logged separately as
model-unavailable metadata and do not enter this deny path. The adapter never
returns `allow`, so SCE cannot bypass Claude's permission system. A read-only or
`Agent` `PreToolUse` returns empty stdout, no scope.

## PostToolUse / PostToolUseFailure: close the scope

For an `active` tracked attempt, both events map to a `close` operation
(`event_id = <scope>|close`, `actor_kind = claude_code`). On Claude Code
`2.1.258` exactly one of the two fires per attempt, never both (T01, D10), and a
failed tool that already changed files still gets its final tree captured. The
attempt is removed from adapter state only after durable `Close` success; a
duplicate `PostToolUse` after cleanup is a safe no-op. Two uncertain-boundary
rules:

- **`pending_start` + terminal signal -> abandon, not late-Start (D11).** The
  adapter cannot prove `Start` committed, and a late `Start` after the tool ran
  would observe the post-tool tree and misattribute the interval — normal
  abandonment on a committed `Start`, the runtime's `MissingScope` / `NeverSeen`
  recovery path otherwise.
- **Failed `Close` -> abandon, not a replayed `Close` (D12).** The original
  observation time is lost, so the adapter must not retry that `Close` later as
  the original observation; it abandons and arms `recovery_pending`. The ingress
  carried-success variants (`MarkerClearAfterCommit` /
  `MarkerClearAfterCompletion`) are durable success and do not enter this path.

## Abandonment cleanup signals

Every abandonment shares one helper: arm `recovery_pending`, call the seam
`abandon` operation, then remove the attempt on success. A failed abandon leaves
`recovery_pending = true` and the attempt tracked, so the next mutation-capable
`PreToolUse` is denied by the barrier.

| Event | Retires |
| --- | --- |
| `PermissionDenied` (D13) | the one live attempt for that `tool_use_id` — an auto-mode-classifier denial signal only; manual/other-hook denial relies on the sweeps below |
| `Stop` / `StopFailure` (D14/D15) | every outstanding **main-thread** attempt (`session_id` match, `agent_id` absent) |
| `UserPromptSubmit` (D16) | the same main-thread sweep — fallback for a user-interrupted main turn, which emits no `Stop` |
| `SubagentStop` (D17) | outstanding attempts owned by `(session_id, agent_id = event.agent_id)` |
| `SessionEnd` (D18) | every outstanding attempt for the session, regardless of `agent_id` |
| `WorktreeRemove` (D22) | every attempt under the Git directory resolved from the event's own `worktree_path` (not the process cwd) — best-effort |

On Claude Code `2.1.258`, `StopFailure` and `WorktreeRemove` were not observed
to fire (T01); their handlers/registrations are kept best-effort but correctness
depends only on `Stop`, `UserPromptSubmit`, `SubagentStop`, and `SessionEnd`.

**The recovery barrier.** While `recovery_pending` is armed and known attempts
are still outstanding, new mutation-capable `PreToolUse` is denied (fail-closed
shape). When `recovery_pending == true AND attempts.is_empty()`, the adapter
runs one `{"operation":"flush"}` through the seam — one worktree-level
recovery/rebaseline boundary. Only a successful `flush` clears
`recovery_pending`; a failed `flush` stays fail-closed.

## Raw cwd is authoritative

The runtime's repository root is the raw payload's `cwd`, never
`$CLAUDE_PROJECT_DIR` (`WorktreeRemove` cleanup uses the event's
`worktree_path`). An `isolation: worktree` subagent's tool executions run inside
the isolated worktree and drive the runtime from that worktree's `cwd`, so a
hook process launched from checkout A with a payload `cwd = checkout B` drives
checkout B's state and only that worktree's cursor. The adapter never accepts,
derives, stores, or constructs a `WorktreeId`, and passes no `worktree_id` key
to the seam.

## Background shell is unsupported

An explicit `Bash.run_in_background = true` / `PowerShell.run_in_background =
true` is denied in `PreToolUse` (fail-closed shape) with:

```text
SCE mutation attribution does not yet support detached background shell execution. Run this command in the foreground.
```

A detached shell can keep mutating the repository after `PostToolUse` returns and
can outlive a session; the generic contract has no process supervisor or stable
background-execution terminal signal. This is a deliberate correctness boundary,
not a Bash security policy. Background **subagents** are not excluded — their
internal mutation-capable tool calls still establish their own scopes.

**Self-detaching descendants are a separate, explicit unsupported boundary
(D20).** A `run_in_background = false` call can still leave a repository-mutating
descendant running after `PostToolUse` returns when the invoked command detaches
a child (`command &`, `nohup`, `setsid`, double-fork, `start_new_session=True`).
T04 proved this live against Claude Code `2.1.258`: a foreground `setsid`
command returned `PostToolUse` in `duration_ms: 13` and its descendant's write
landed ~3s later, changing the Git tree an SCE snapshot would capture — outside
the tool's closed scope. This is not solvable by inspecting the command string;
the integration adds no detection, supervision, or static scan, and simply does
not treat `PostToolUse` as proof that every descendant has stopped mutating. See
the T04 addendum and `probe17-*` fixtures under
`cli/src/services/hooks/claude_mutation_scope/fixtures/`.

## Generated settings

`sce setup` (`config/pkl/renderers/claude-content.pkl`) registers the adapter
for all ten handled events, each with **no** `matcher` (the adapter classifies
tools in Rust), matching the existing unmatched `conversation-trace`
`PostToolUse` entry. The merge preserves `claude-model-state`, the bash-policy
hook, `diff-trace`, and `conversation-trace`, keeps user-owned Claude hooks, and
stays idempotent. See
[`claude-raw-hook-capture.md`](../sce/claude-raw-hook-capture.md) for the full
generated Claude settings state.

## Dependency boundary

Dependency direction remains `claude_mutation_scope -> hooks::mutation_scope
-> mutation_trace::runtime`. Production Claude-adapter code (outside
`#[cfg(test)]`) imports no mutation-trace module and names no
`RepositoryAgentTraceDb`, `WorktreeId`, or `GitSnapshotService`. Its only
mutation-stack dependency is the reused
`super::mutation_scope::run_mutation_scope_from_payload` seam, with no second
`RuntimeBoundary` path or spawned `sce` subprocess. The production resolver
uses the hooks-layer Agent Trace DB opener only for the exact
`claude_model_state` read; the seam retains strict parsing, lazy DB acquisition,
durable-completion classification, and empty-stdout semantics.

## Related context

- [Mutation-scope hook ingress: the harness-neutral transport seam](mutation-scope-hook-ingress.md)
- [Mutation-scope runtime: the harness-adapter contract](mutation-scope-runtime.md)
- [Agent Trace hooks command routing](../sce/agent-trace-hooks-command-routing.md)
