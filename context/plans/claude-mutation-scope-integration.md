# Plan: claude-mutation-scope-integration

## Change summary

Add the first concrete mutation-scope producer for SCE: a Claude Code adapter
that translates Claude's raw tool/lifecycle hook events into the normalized
mutation-scope contract already implemented by `sce hooks mutation-scope`
(`cli/src/services/hooks/mutation_scope.rs`, documented in
`context/cli/mutation-scope-hook-ingress.md`).

Data flow:

```text
Claude raw hook event
  -> sce hooks claude-mutation-scope   (new, hidden)
  -> normalize lifecycle + identity, classify tool
  -> hooks::mutation_scope generic ingress (in-process seam)
  -> coordinate() / abandon_scope()
  -> mutation cursor
```

The fundamental mapping is **one mutation-capable Claude tool execution = one SCE
mutation `ScopeId`**. A session, prompt, main agent, or subagent is never a scope;
`session_id` / `agent_id` are only identity inputs that distinguish tool
executions. Two parallel mutation-capable tools produce two simultaneously live
scopes and may correctly yield `AiContended`.

This extends the mutation-scope stack: the generic ingress and runtime already
exist and are unchanged in contract. This change adds the harness adapter layer
the ingress explicitly deferred, plus a small crate-visible in-process seam on
`mutation_scope.rs` so the adapter reuses one mutation implementation rather than
spawning a second `sce` subprocess or constructing `RuntimeBoundary` directly.

The adapter never calls `coordinate()`, `abandon_scope()`,
`RepositoryAgentTraceDb`, `WorktreeId`, `GitSnapshotService`, the mutation store,
or protocol internals directly. It never accepts, derives, stores, or constructs
a `WorktreeId`: it passes the raw hook `cwd` as `repository_root` and the runtime
derives worktree identity itself. No mutation protocol, Quint model, SQL
migration, mutation-attribution algorithm, or Agent Trace schema change is in
scope.

## Design

These are the design decisions the task stack and acceptance criteria reference
by number. `PostToolUseFailure`, `StopFailure`, `PermissionDenied`, and
`WorktreeRemove` are documented Claude Code hook events, so their existence was
never in question. T01 froze the real, tested contract for these events against
Claude Code `2.1.258` (see T01's Verify record in the Task stack below and
`cli/src/services/hooks/claude_mutation_scope/fixtures/NOTES.md`); the
decisions whose correctness depended on one of them actually firing — D10, D13,
D15, D20, and D22 — are each marked with T01's resolved finding rather than a
pending gate. D20 carries a second, narrower open sub-finding: T01 proved only
that Claude itself keeps the `Bash` tool foregrounded for the invoked process's
own duration, never whether that process can leave a detached descendant
running past `PostToolUse`. T04 (a T01 follow-up) captures live evidence for
that specific gap; see D20.

### D1 — Scope = one independently mutation-capable Claude tool execution

A mutation scope is exactly one independently mutation-capable Claude tool
execution attempt. Not a session, not a prompt, not the main agent, not a
subagent. Two tools that can edit the worktree concurrently (e.g. a main-agent
tool and a subagent tool) are two scopes with distinct `ScopeId`s, so the
protocol can report `AiContended` when they genuinely race. Sequential tool
calls are sequential scopes.

### D2 — Tool classification

The adapter classifies `tool_name` in Rust:

- **Mutation-capable (always establishes a scope):** `Bash`, `PowerShell`,
  `Write`, `Edit`, `NotebookEdit`, `MultiEdit` (when the supported Claude
  version emits it), and any `mcp__*` tool (an arbitrary MCP tool may modify the
  local repository, so it is treated conservatively).
- **Read-only (never establishes a scope):** `Read`, `Glob`, `Grep`,
  `WebFetch`, `WebSearch`, `AskUserQuestion`.
- **`Agent`:** a delegation wrapper, not itself a mutation scope. The subagent's
  own mutation-capable tool calls fire their own hooks (carrying the subagent's
  `agent_id`) and establish their own scopes; wrapping the whole delegation in a
  parent scope would fold every child mutation into it.
- **Unknown tool names:** conservatively treated as mutation-capable. A new
  read-only Claude tool would briefly create unnecessary (harmless) scopes until
  classified; the opposite default would silently miss a new mutation-capable
  tool.

### D3 — Claude execution identity

Required for a tracked `PreToolUse`: `session_id`, `cwd`, `tool_name`,
`tool_use_id`. Optional: `agent_id` (absent = main thread, present = subagent),
`prompt_id` (diagnostics only — correctness must never depend on it),
`agent_type` (diagnostics only). The tool-execution key is
`(session_id, agent_id?, tool_use_id)`.

### D4 — ScopeId / EventId derivation

A raw `tool_use_id` can recur (a deferred execution resumed), and a terminal SCE
`ScopeId` must never be reused, so `ScopeId` cannot be a pure function of
`tool_use_id`. The adapter keeps a monotonic checkout-local `attempt_seq`; each
new execution attempt gets a fresh `attempt_seq`. `ScopeId` is a
length-prefixed, hash-free encoding:

```text
cc-tool-v1|n=<attempt_seq>|s=<byte-len>:<session_id>|a=<byte-len>:<agent_id-or-empty>|t=<byte-len>:<tool_use_id>
```

`EventId`s are derived deterministically from the `ScopeId`: `<scope-id>|start`
and `<scope-id>|close`. Replaying the same hook event for one live attempt always
yields the same `ScopeId` and `EventId` (the runtime's replay/idempotency key).
After an attempt is terminal, another `PreToolUse` for the same `tool_use_id`
gets a new `attempt_seq` and therefore a new `ScopeId`.

### D5 — Checkout-local adapter bookkeeping

The adapter keeps tiny cross-hook-process state at
`<git-dir>/sce/claude-mutation-scope-state.json` (located via
`checkout::resolve_git_dir(cwd)` — worktree-specific for linked worktrees). It
holds `version`, `next_attempt_seq`, `recovery_pending`, and an `attempts[]`
list, each attempt carrying its `attempt_seq`, `scope_id`, identity fields,
`tool_name`, and `phase` (`pending_start | active`). This state is **adapter
bookkeeping, never attribution evidence**: it is not exported, not synced, not
part of Agent Trace, not authoritative for attribution. Its only purpose is to
know which Claude-created scopes may still need a terminal action.

### D6 — Durable adapter-state persistence and a separate state lock

State writes follow the existing checkout-identity durability pattern: acquire
the adapter-state lock at `<git-dir>/sce/claude-mutation-scope-state.lock`,
serialize, write a temp file, `sync_data`, atomic rename, best-effort parent-dir
`sync_all` on Unix, release. The adapter-state lock protects bookkeeping only and
is **never held across a `hooks::mutation_scope` invocation**, so no
`adapter lock -> WorktreeLock` order can form. The mutation runtime's own
`WorktreeLock` stays entirely independent.

### D7 — PreToolUse write-ahead ordering

For a new tracked mutation-capable tool:

```text
parse event -> resolve cwd to git_dir
  -> acquire adapter-state lock -> allocate attempt_seq -> persist phase=pending_start -> release lock
  -> invoke generic ingress seam with Start
  -> reacquire adapter-state lock -> phase pending_start -> active -> release lock
  -> return empty success to Claude
```

The generic ingress receives the raw Claude `cwd` as its `repository_root`; SCE
derives the `WorktreeId`. The normalized operation is
`{"operation":"start","scope_id":<derived>,"event_id":<scope>|start,"actor_kind":"claude_code"}`.

### D8 — Mutation-capable PreToolUse is fail-closed via Claude's deny decision

A mutation-capable tool must not execute if the adapter cannot durably establish
its scope. Claude treats ordinary non-2 hook failures as non-blocking, so a
generic non-zero exit would let the tool run without its `Start` boundary.
Therefore any failure during a mutation-capable `PreToolUse` returns a Claude
`PreToolUse` denial:

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"SCE could not establish mutation attribution for this tool execution."}}
```

The detailed error is logged through SCE observability. The adapter never returns
`allow` — success emits no decision (normal Claude permission flow continues),
failure emits `deny` — so SCE cannot bypass Claude's own permission system.

### D9 — PostToolUse closes the scope

For an active tracked attempt, `PostToolUse` maps to
`{"operation":"close","scope_id":<same>,"event_id":<scope>|close,"actor_kind":"claude_code"}`.
The attempt is removed from adapter state only after durable `Close` success;
duplicate `PostToolUse` delivery after cleanup is a safe adapter-layer no-op.

### D10 — Failed-tool terminal observation — resolved by T01: PASS

A tool that failed may already have changed files, so its final observed tree
must still be captured through a terminal boundary (a `Close`), never silently
dropped. This uses the documented `PostToolUseFailure` event mapped to the same
`Close` operation as D9. **T01 finding (Claude Code `2.1.258`):** a failed
`Bash` call emitted `PostToolUseFailure` only — never `PostToolUse` — for the
same `tool_use_id`, and carried the identity fields (`session_id`, `cwd`,
`tool_name`, `tool_use_id`, optional `agent_id`) this mapping needs. **PASS** —
implement the `Close` mapping as designed; no fallback is needed.

### D11 — pending_start + terminal signal must abandon, not late-Start

If the adapter persisted `pending_start` but a terminal signal
(`PostToolUse`/`PostToolUseFailure`, or a lifecycle cleanup) arrives before the
adapter ever durably recorded `active`, it cannot prove `Start` committed. It
must **not** issue a late `Start` after the tool already ran (that would observe
the post-tool tree and could misattribute the ambiguous interval to other live
scopes). Instead it abandons the scope: `abandon` on a committed `Start`
produces normal abandonment; `abandon` on a `Start` that never committed hits the
runtime's existing `MissingScope` / `NeverSeen` recovery path, forcing
conservative recovery. Either outcome prefers lost attribution over false
attribution.

### D12 — Failed Close is retired through abandonment, not a replayed Close

If a tool finished but its `Close` fails before durable completion, the original
observation time is lost. The adapter must not retry that `Close` later (at the
next prompt or minutes on) as if it were the original observation — a later tree
must not be presented as the tree observed when the tool completed. Instead it
immediately attempts `abandon` for that scope and sets `recovery_pending = true`.
The tool's attribution may be lost; that is intentional. The two generic-ingress
carried-success variants (`MarkerClearAfterCommit` /
`MarkerClearAfterCompletion`) are durable success and do not enter this path.

### D13 — PermissionDenied cleanup — resolved by T01: PASS

When Claude signals that a tool call was denied and never executed, and the
adapter has a live attempt for it, `abandon` that scope and set
`recovery_pending = true` (abandonment requires a rebaseline before attribution
resumes). This uses the documented `PermissionDenied` event. **T01 finding
(Claude Code `2.1.258`):** an auto-mode denial fired `PermissionDenied`,
carrying `tool_use_id`, `session_id`, `cwd`, and `tool_name` — everything this
mapping needs. A denial produced by a second, independent `PreToolUse` hook
produced **no** `PermissionDenied` event at all. **PASS** — `PermissionDenied`
is confirmed as an auto-mode-denial-only signal; manual denial, deny rules, and
another parallel `PreToolUse` hook blocking the tool are therefore not covered
by this signal and rely on lifecycle cleanup (`Stop`/`UserPromptSubmit`/
`SessionEnd`) instead, exactly as originally designed.

### D14 — Stop stale-main cleanup

`Stop` is **not** a `Close` of any Claude agent scope (there is no such scope).
It is positive evidence that any still-outstanding **main-thread** tool attempt
(`session_id == Stop.session_id`, `agent_id == None`) from the just-finished turn
is no longer executing: `abandon` each and remove it after durable settlement. It
does not touch subagent-owned attempts. If a later `Stop` hook makes Claude
continue, any earlier outstanding tool execution is still stale and new work gets
new `tool_use_id`s / attempts.

### D15 — StopFailure cleanup — resolved by T01: DOC-VERIFIED / NON-LOAD-BEARING

Perform the same stale main-thread cleanup as D14 when a main turn ends in
failure. This uses the documented `StopFailure` event. **T01 finding:** no live
`StopFailure` fixture was captured — exercising it requires deliberately
failing the main turn, which T01 did not manufacture. `StopFailure` support is
kept in the adapter mapping, but correctness must not depend on it firing:
D14's `Stop`, D16's `UserPromptSubmit` fallback, and D18's `SessionEnd` remain
the load-bearing backstops for the failed-turn case.

### D16 — UserPromptSubmit interruption cleanup

A new user prompt is the fallback for a main turn the user interrupted (Claude
emits no `Stop` for an interruption). Before the new prompt is processed, the
adapter cleans up outstanding **main-thread** foreground attempts for that
session (`abandon` + remove). It does not abandon subagent-owned attempts merely
because a main-thread prompt was submitted — background subagents may legitimately
continue across main-thread turns.

### D17 — SubagentStop matching-agent cleanup

For `SubagentStop(agent_id = X)`, `abandon` any still-outstanding foreground tool
attempts owned by `(session_id = event.session_id, agent_id = X)`. Safe even if
another `SubagentStop` hook makes the subagent continue — existing tool
executions have finished or failed to execute, and any continuation uses new tool
executions. No `ScopeId` is derived from `agent_id` alone, so resuming a subagent
under the same `agent_id` is safe.

### D18 — SessionEnd cleanup

`SessionEnd` cleans up remaining non-detached tool attempts for that session,
including deferred execution attempts from the process that just ended. If such a
tool later fires `PreToolUse` again after `claude --resume`, it receives a new
`attempt_seq` and a fresh `ScopeId`; a terminal `ScopeId` is never reused.

### D19 — recovery_pending barrier and quiescent Flush

Whenever abandonment or an uncertain lifecycle sets `recovery_pending = true`,
the adapter must not start a new mutation-capable tool while it still has known
outstanding tool attempts — new mutation-capable `PreToolUse` is denied (D8
shape) until those attempts settle. When
`recovery_pending == true AND attempts.is_empty()`, the adapter runs one
`{"operation":"flush"}` through the generic ingress, giving the runtime one
worktree-level recovery/rebaseline boundary after the ambiguous executions are
gone. Only after a successful `flush` does the adapter clear `recovery_pending`;
a failed `flush` keeps it fail-closed for subsequent mutation-capable
`PreToolUse`.

### D20 — Detached background Bash/PowerShell is unsupported and denied — resolved by T01: PASS for Claude-managed backgrounding; self-detaching descendants confirmed as a separate, explicit unsupported boundary by T04

A detached shell can keep mutating the repository after `PostToolUse` returns and
can outlive a session; the generic mutation-scope contract has no process
supervisor or stable background-execution terminal signal. This PR must not
pretend `PostToolUse(background Bash)` means the execution ended. An explicit
`Bash.run_in_background = true` / `PowerShell.run_in_background = true` is denied
in `PreToolUse` (D8 shape) with:

```text
SCE mutation attribution does not yet support detached background shell execution. Run this command in the foreground.
```

This is a deliberate correctness boundary, not a Bash security policy. **T01
finding (Claude Code `2.1.258`):** a `run_in_background=false` call remained
foreground for the full command duration (`duration_ms: 4018` for a `sleep 4`)
before `PostToolUse` fired; a `run_in_background=true` call returned
immediately (`duration_ms: 8`) with a `tool_response.backgroundTaskId` stub.
**PASS, narrowly** — this proves only that Claude itself keeps the `Bash` tool
call foregrounded for as long as the *invoked* process runs; it does **not**
prove that command cannot leave a *detached descendant* process still running,
and still mutating the repository, after `PostToolUse` fires. Background
**subagents** are not excluded here: their internal mutation-capable tool calls
still fire hooks with `agent_id` and establish their own scopes.

**Explicit unsupported boundary — self-detaching descendant processes.** A
`run_in_background=false` `Bash`/`PowerShell` call can still leave a
repository-mutating descendant process running after `PostToolUse` returns,
because the *invoked shell command itself* detaches a child before exiting.
Examples: shell backgrounding (`command &`), `nohup command &`, `setsid
command`, double-fork daemonization, or a child process a script starts with
detached/session-leader semantics (e.g. Python
`subprocess.Popen(..., start_new_session=True)`). **SCE cannot currently
guarantee attribution for shell commands that leave repository-mutating
descendant processes running after `PostToolUse`.** This is not solvable by
inspecting the command string: arbitrarily nested shell, script, and
interpreter invocations can detach a descendant no static text scan can
reliably catch, and this PR does not attempt one. Correct support would
require process/process-group supervision, which is out of scope here (see
Constraints and non-goals). This boundary holds regardless of what T04's probe
observes for the specific pattern it tests, because no detection or
supervision is being added either way.

**T04 finding (Claude Code `2.1.258`), corrected 2026-09-05:** the original
capture wrote its marker under `context/tmp/`, which `context/tmp/.gitignore`
ignores wholesale — `GitSnapshotService::capture_tree()`'s
`git read-tree HEAD` / `git add -A -- .` / `git write-tree` sequence never
sees an ignored path, so that capture only proved a detached descendant
*survives* `PostToolUse`, not that its mutation is one SCE's Git snapshot
would actually observe. T04 was rerun with a non-ignored, repository-root
marker path and explicit `"run_in_background":false` present in the real
captured `tool_input` (not merely omitted and defaulted): a foreground `Bash`
call ran `setsid bash -c '... sleep 3; <write-marker> ...' &` (a shell-level
self-detaching descendant, the `setsid`/backgrounding equivalent of Python
`subprocess.Popen(..., start_new_session=True)`) and returned in
`duration_ms: 13` — the invoked shell never waited on the detached child.
`PreToolUse` fired at `2026-09-04T22:18:09.195Z`, `PostToolUse` fired at
`2026-09-04T22:18:21.678Z`, and the detached descendant's own write landed at
`2026-09-04T22:18:24.674140Z` — roughly three seconds *after* `PostToolUse`,
matching the child's own `sleep 3`. `git check-ignore` confirmed the marker
path is not ignored, and a `GIT_INDEX_FILE`-scoped temporary index (never the
real `.git/index`) reproduced `capture_tree()`'s exact sequence before and
after the child's write, yielding two different tree hashes
(`596fcceafa2ebf70a087f606d7e16645f18ee17e` vs
`b24d653632e478298b625e86c99f51f4016f9f57`) — direct proof the descendant's
mutation is one SCE's snapshot model would observe.

**The self-detaching-descendant boundary is therefore confirmed by direct,
Git-observable evidence, not merely theoretical**, for this process pattern
and this Claude Code version: a foreground Claude `Bash` tool call can return
`PostToolUse` — closing the adapter's mutation scope at the tree observed at
that moment — while a descendant it spawned remains alive and later performs
a mutation that changes SCE's observable Git tree; that later mutation falls
outside the tool's own observed scope boundary and would be misattributed
(or silently dropped) if the adapter ever treated `PostToolUse` as proof no
descendant process is still running or mutating the repository. This does
not generalize to `nohup`, double-fork, daemonization, interpreter, or other
descendant-detachment patterns T04 did not exercise — those remain unproven
— and the unsupported-boundary wording above stays in place regardless,
since this PR implements no detection or supervision either way and adds no
static shell-command inspection. See
`cli/src/services/hooks/claude_mutation_scope/fixtures/probe17-detached-child-after-post-tool-use.*`
(including the `.evidence.json` capture-metadata artifact) and the T04
addendum in `NOTES.md` for the full captured evidence.

### D21 — Raw Claude hook cwd is authoritative

The mutation runtime's repository root is the raw Claude hook payload's `cwd`,
never `$CLAUDE_PROJECT_DIR` (the generated hook script may live under
`$CLAUDE_PROJECT_DIR`, but the payload's `cwd` is the actual current worktree).
For an `isolation: worktree` subagent, its tool executions happen inside the
isolated worktree and their hook events must drive the runtime from that
worktree's `cwd`; SCE then derives the correct worktree identity. `WorktreeRemove`
cleanup (D22) uses the event's `worktree_path`, not the hook process's cwd.

### D22 — WorktreeRemove cleanup — **best-effort, non-load-bearing (resolved by T01)**

Intent: before Claude removes a worktree, retire any outstanding adapter attempts
stored under that worktree-specific Git directory (using the event's
`worktree_path`, no new mutation snapshot). As drafted this uses the documented
`WorktreeRemove` event. T01 tested this against Claude Code `2.1.258` and did
not observe `WorktreeRemove` fire for either isolated-worktree path it could
exercise in a single session (see T01's Verify record and
`cli/src/services/hooks/claude_mutation_scope/fixtures/NOTES.md`). The adapter
keeps the `WorktreeRemove` handler and registration as a **best-effort cleanup
signal** — when it does fire with a `worktree_path`, the adapter retires the
outstanding attempts stored under that worktree's Git directory immediately,
which is strictly better than waiting — but correctness must **not** depend on
it firing. `SubagentStop` (D17) and `SessionEnd` (D18) are the load-bearing
cleanup backstops that retire isolated-worktree attempts whether or not
`WorktreeRemove` ever arrives.

### D23 — Adapter depends on hooks::mutation_scope only

Dependency direction is strictly
`claude_mutation_scope -> hooks::mutation_scope -> mutation_trace::runtime`. The
Claude adapter's production code must not import or reference
`crate::services::mutation_trace::runtime`, `::protocol`, or `::store`, and must
not name `RepositoryAgentTraceDb`, `WorktreeId`, or `GitSnapshotService`. It
reaches the runtime only through the smallest crate-visible in-process seam on
`cli/src/services/hooks/mutation_scope.rs` (T05) — no second `RuntimeBoundary`
construction path and no spawned `sce` subprocess. That seam reuses the strict
generic payload parser, `RuntimeBoundary` mapping, lazy DB acquisition,
durable-completion error classification, and empty-stdout semantics already in
`mutation_scope.rs`.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: `sce hooks claude-mutation-scope` exists, is hidden from top-level
  help, and routes through the normal hook command stack
  (`HooksSubcommand::ClaudeMutationScope` -> `convert_hooks_subcommand_request`
  -> `HookSubcommand::ClaudeMutationScope` -> `run_hooks_subcommand_in_repo`).
  - Validate: `sce hooks claude-mutation-scope </dev/null` exits with the strict
    parser's error (not "unknown subcommand"); `sce --help` and `sce hooks
    --help` do not list it; routing test in `command_runtime.rs`.
- [ ] AC2: The raw Claude event parser validates required fields
  (`session_id`, `cwd`, `tool_name`, `tool_use_id` for tracked `PreToolUse`) and
  rejects malformed or wrong-type payloads without fabricating identities;
  `prompt_id` is optional and correctness never depends on it.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_mutation_scope` parser
    unit tests.
- [ ] AC3: No `Start` boundary is emitted for `SessionStart`, `UserPromptSubmit`,
  or `SubagentStart` merely because that lifecycle event occurred. Only an
  independently mutation-capable tool execution attempt establishes a scope.
  - Validate: adapter mapping unit tests; T08 Test-series assertions on
    processed-event keys.
- [ ] AC4: Duplicate delivery of the same live `PreToolUse` reuses the same
  `attempt_seq`, `ScopeId`, and `Start` `EventId`.
  - Validate: state + adapter unit tests; T08 Test4 (duplicate `Pre`/`Post`
    replay).
- [ ] AC5: A later execution attempt of the same Claude `tool_use_id`, after the
  previous attempt became terminal, receives a new `attempt_seq` and a new
  `ScopeId`; a terminal `ScopeId` is never reused.
  - Validate: T03 state unit tests (terminal attempt followed by a fresh
    `attempt_seq`/`ScopeId` allocation).
- [ ] AC6: Otherwise-identical tool IDs under main (`agent_id` absent),
  `agent_id=A`, and `agent_id=B` produce three distinct `ScopeId`s.
  - Validate: ScopeId-formatter unit tests.
- [ ] AC7: A tracked mutation-capable `PreToolUse` reaches durable
  generic-ingress `Start` before the hook returns success to Claude
  (write-ahead `pending_start` -> ingress `Start` -> `active`).
  - Validate: T06 adapter ordering unit test with injected ingress; optionally
    also T08 Test1 as production-path confirmation.
- [ ] AC8: Any failure to establish required adapter state or `Start` during a
  mutation-capable `PreToolUse` returns a Claude `permissionDecision: "deny"`
  object, never a plain non-zero exit and never `allow`.
  - Validate: adapter failure-classification unit tests asserting the exact
    `hookSpecificOutput` JSON.
- [ ] AC9: `PreToolUse` -> real filesystem mutation -> `PostToolUse` produces
  exactly one eligible tool interval and one terminal (`Closed`) scope with
  attribution `AiExclusive`.
  - Validate: T08 Test1 (real Git repo + real Agent Trace DB).
- [ ] AC10: `PreToolUse` -> partial filesystem mutation -> `PostToolUseFailure`
  also observes the mutation and closes the scope (`AiExclusive` + `Closed`).
  - Validate: T08 Test2.
- [ ] AC11: Two simultaneously tracked tools create two active scopes; a tree
  transition observed while both are live is attributed `AiContended`.
  - Validate: T08 Test3 and Test9 (main + subagent).
- [ ] AC12: `PreToolUse` followed by `PermissionDenied` creates no mutation event
  for the denied execution and leaves the worktree `needs_rebaseline`.
  - Validate: T08 Test5.
- [ ] AC13: A `PreToolUse` with no `PostToolUse`/`PostToolUseFailure` is retired
  by one of the positive stale signals (`Stop`, `StopFailure`,
  main-thread `UserPromptSubmit`, matching-agent `SubagentStop`, `SessionEnd`,
  `WorktreeRemove`) via `abandon_scope`.
  - Validate: T08 Test6, Test7, Test11; T06 adapter cleanup unit tests.
- [ ] AC14: `PreToolUse` -> partial change/interruption -> no `Stop` -> next
  main-thread `UserPromptSubmit` abandons the stale main attempt before another
  mutation-capable tool can start.
  - Validate: T08 Test7.
- [ ] AC15: A resumed subagent may carry the same Claude `agent_id`, but a new
  tool attempt receives a fresh tool `ScopeId`; no terminal mutation `ScopeId`
  is reused.
  - Validate: T06 adapter identity unit tests; T08 Test8.
- [ ] AC16: A hook process launched from checkout A with raw payload
  `cwd = checkout B` drives mutation state for checkout B.
  - Validate: T08 Test10 (isolated-worktree cwd) asserting the correct
    `WorktreeId`/cursor is advanced.
- [ ] AC17: Mutations from an `isolation: worktree` subagent change only that
  worktree's mutation cursor; the main checkout's cursor is unchanged.
  - Validate: T08 Test10.
- [ ] AC18: The dependency direction is exactly
  `claude_mutation_scope -> hooks::mutation_scope -> mutation_trace::runtime`.
  Production Claude-adapter code (everything in
  `cli/src/services/hooks/claude_mutation_scope/` outside `#[cfg(test)]` blocks)
  contains no `use` declaration or fully-qualified path reference naming
  `crate::services::mutation_trace::runtime`,
  `crate::services::mutation_trace::protocol`,
  `crate::services::mutation_trace::store`, `RepositoryAgentTraceDb`,
  `WorktreeId`, or `GitSnapshotService`, and its only dependency into the
  mutation stack is the single T05 seam import from
  `crate::services::hooks::mutation_scope`.
  - Validate: focused source inspection of
    `cli/src/services/hooks/claude_mutation_scope/{mod.rs,state.rs}`, excluding
    `#[cfg(test)]`-gated code, targeted at `use` declarations and qualified
    paths, e.g.
    `rg -n --type rust '^\s*use\s+crate::services::mutation_trace::(runtime|protocol|store)|::(RepositoryAgentTraceDb|WorktreeId|GitSnapshotService)\b' cli/src/services/hooks/claude_mutation_scope/`
    must return no matches outside a `#[cfg(test)]` module, and a manual check
    confirms exactly one `use` reaching `crate::services::hooks::mutation_scope`.
    This is a dependency-boundary check, not a text search for the bare words
    `coordinate` / `abandon_scope` / `WorktreeId`, which may legitimately appear
    in comments, diagnostics, or test code that fabricates outcomes.
- [ ] AC19: Claude adapter state lives only below `<git-dir>/sce/` and writes no
  Agent Trace or mutation database table directly.
  - Validate: state-module inspection; T08 Test16.
- [ ] AC20: Claude mutation-scope-only regressions leave `diff_traces`,
  `post_commit_patch_intersections`, and `agent_traces` unchanged.
  - Validate: T08 Test16 (row-count assertions before/after).
- [ ] AC21: Explicit background `Bash`/`PowerShell`
  (`run_in_background = true`) is denied in `PreToolUse` with the documented
  reason and creates no mutation scope.
  - Validate: T06 adapter classification unit test; T08 Test15.
- [ ] AC22: Generated Claude settings still include and correctly merge
  `claude-model-state`, the bash policy hook, `diff-trace`, and
  `conversation-trace` alongside the new mutation adapter; user-owned Claude
  hooks are preserved; repeated `sce setup` is idempotent.
  - Validate: `config_merge.rs` tests; `nix run .#pkl-check-generated`.
- [ ] AC23: The diff against the `#261` base
  (`origin/mutation-scope-ingress`) is empty for `spec/mutation_cursor.qnt`,
  `cli/src/services/mutation_trace/protocol.rs`,
  `cli/migrations/agent-trace-repository/`, and
  `config/schema/agent-trace.schema.json`.
  - Validate: `git diff origin/mutation-scope-ingress -- <those paths>` is empty.
- [ ] AC24: Durable context clearly separates generic mutation-scope ingress,
  the Claude mutation adapter, and the mutation runtime, and records tool-attempt
  scope semantics, identity derivation, cleanup signals, worktree-cwd ownership,
  fail-closed `PreToolUse`, and the background-shell limitation.
  - Validate: inspection of `context/cli/claude-mutation-scope-integration.md`
    and the updated cross-reference files.
- [ ] AC25: A foreground Bash/PowerShell tool call (`run_in_background = false`)
  that starts a detached, self-backgrounding descendant process which mutates
  the repository after `PostToolUse` returns is not silently attributed as if
  the mutation happened inside that tool's own observed scope; the adapter
  documents this as an explicit unsupported boundary (D20) rather than
  fabricating detection or supervision.
  - Validate: T04's captured fixture + `NOTES.md` finding; D20 in the Design
    section carries the reconciled wording; T08 Test17 (documented
    unsupported-case regression).

### Full validation

- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_mutation_scope`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
- `nix develop -c ./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings`
- `nix develop -c ./scripts/run-cli-cargo.sh fmt --manifest-path cli/Cargo.toml -- --check`
- `nix run .#pkl-check-generated`
- `nix flake check`
- `git diff origin/mutation-scope-ingress -- spec/mutation_cursor.qnt cli/src/services/mutation_trace/protocol.rs cli/migrations/agent-trace-repository/ config/schema/agent-trace.schema.json` must be empty.

Final branch comparison is against `mutation-scope-ingress`, not `main`, while
the PR remains stacked on #261.

### Context sync

- New: `context/cli/claude-mutation-scope-integration.md` (owns the adapter
  domain — see AC24 list).
- Update: `context/cli/mutation-scope-runtime.md` (a concrete adapter now exists),
  `context/cli/mutation-scope-hook-ingress.md` (an in-process crate seam and a
  first adapter consumer now exist),
  `context/sce/agent-trace-hooks-command-routing.md` (new `claude-mutation-scope`
  route), `context/sce/claude-raw-hook-capture.md` (current Claude
  hook-routing/generated-settings state gains the new registrations),
  `context/context-map.md`, `context/overview.md`, `context/architecture.md`.
- `context/sce/generated-opencode-plugin-registration.md` is **not** a target for
  this plan — it owns OpenCode plugin registration, not Claude generated
  settings. `context/sce/claude-raw-hook-capture.md` is the update target unless
  T07/T09 implementation proves a new dedicated Claude settings domain file is
  required, in which case that new file becomes the owner and this list is
  updated then.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/cli_schema.rs`, `cli/src/services/parse/command_runtime.rs`,
  `cli/src/services/hooks/mod.rs`, `cli/src/services/hooks/mutation_scope.rs`
  (add one crate-visible in-process seam only),
  `cli/src/services/hooks/claude_mutation_scope/mod.rs` (new),
  `cli/src/services/hooks/claude_mutation_scope/state.rs` (new),
  `config/pkl/renderers/claude-content.pkl`,
  `cli/src/services/setup/config_merge.rs` (and focused doctor/setup test files
  if the generated-fragment comparison does not already cover the new
  registrations), and the context files listed under Context sync.
- **Out of scope:** Codex/OpenCode/Pi adapters, a generic adapter-framework
  extraction, a background-process supervisor / PID tracking / process-group
  tracking / cross-process detached Bash attribution, shell-command parsing or
  deny-listing to detect backgrounding/detachment patterns, protocol or Quint
  changes, Agent Trace schema changes, any new mutation-attribution algorithm,
  `#259` attribution code.
- **Constraints:** the adapter depends only on `hooks::mutation_scope`, never on
  `mutation_trace::runtime` directly (`claude_mutation_scope -> mutation_scope ->
  mutation_trace::runtime`); it may call `checkout::resolve_git_dir(cwd)` but not
  `read_checkout_id` / `get_or_create_checkout_id` /
  `resolve_checkout_id_for_repo` and must not construct a `WorktreeId`; the
  adapter-state lock is never held across a `hooks::mutation_scope` invocation
  (no `adapter lock -> WorktreeLock` order); latest deps pinned exactly, node24
  for any new JS work per `context/plans/feedback_deps.md` (no new deps expected
  here); ScopeId uses length-prefixed tuple encoding, no hashing / no crypto
  dependency.
- **Non-goal:** treating `PostToolUse(background Bash)` as a completed execution;
  treating a foreground (`run_in_background = false`) `PostToolUse` as proof
  that every descendant process the tool call spawned has also terminated or
  stopped mutating the repository; turning `abandon` into a `RuntimeBoundary`;
  deriving any `ScopeId` from `agent_id` alone; a long-lived Claude "session"
  or "agent" scope.

## Assumptions

- Task numbering here is `T01..T09`; the original change request's `T00..T07`
  mapped to `T01..T08` in order, and a later change request inserted a T01
  follow-up (detached-descendant lifecycle probe) as `T04`, shifting the
  original `T04..T08` to `T05..T09`. Only not-yet-completed tasks were
  renumbered; `T01`, `T02`, and `T03`, already complete when this insertion
  happened, keep their original IDs and recorded evidence unchanged.
- The crate-visible seam added to `mutation_scope.rs` is the existing private
  `run_mutation_scope_from_payload(repository_root, stdin_payload, logger)` made
  `pub(crate)` (or a thin `pub(crate)` wrapper), reused verbatim; no second
  `RuntimeBoundary` construction path and no `sce` subprocess. Rests on
  `context/cli/mutation-scope-hook-ingress.md` D23 and the current
  `mutation_scope.rs` structure.
- Adapter state path is `<git-dir>/sce/claude-mutation-scope-state.json` with lock
  `<git-dir>/sce/claude-mutation-scope-state.lock`, following the
  `checkout::persist_checkout_id_inner` durability pattern
  (`context/cli/checkout-identity.md`, `context/cli/mutation-trace-external-taint.md`).
- Generated Claude mutation-scope hook registrations carry no `matcher` (the
  adapter classifies tools in Rust per D2), consistent with the existing
  unmatched `conversation-trace` `PostToolUse` entry.

## Task stack

- [x] T01: `Freeze the real Claude lifecycle contract` (status:done)
  - Task ID: T01
  - Scope: In — capture raw hook fixtures from the Claude Code version SCE
    chooses to support for every probe below and commit them under
    `cli/src/services/hooks/claude_mutation_scope/fixtures/` (one file per probe,
    named for the probe), the durable fixture path owned by the Claude adapter's
    own tests; each fixture records, or is accompanied by a note recording, the
    tested Claude Code version. `PostToolUseFailure`, `StopFailure`,
    `PermissionDenied`, and `WorktreeRemove` are documented Claude Code hook
    events — this task is not verifying whether they exist, it is verifying
    whether the chosen version actually fires each one, with the payload and
    lifecycle semantics D10/D13/D15/D22 assume, for the specific probes those
    decisions rely on. Record the tested Claude Code version, whether generated
    settings accept every required event, and any minimum compatible version;
    update this plan's Open questions / task notes if findings contradict the
    design. `context/tmp/` remains scratch-only and is not used for these
    committed fixtures. Later parser/adapter tests (T02+) consume these fixtures
    where useful. Out — any production code, any Rust module, any settings
    change; adding `PostToolBatch` handling to the design or acceptance criteria
    (see probe 16 below).
  - Dependencies: none
  - Done when: real fixtures exist, captured live against Claude Code `2.1.258`,
    for every probe correctness actually depends on, and each of D10, D13, D15,
    D20, D22 carries an explicit disposition (not merely pass/needs-revision —
    `PASS`, `ACCEPTED BEST-EFFORT`, or `DOC-VERIFIED / NON-LOAD-BEARING` are all
    valid closing dispositions provided the reasoning is recorded):
    - (1) `Write` success, (2) `Bash` success, (3) `Bash` writes then exits
      non-zero, (4) two parallel mutation tools, (6) another `PreToolUse` hook
      denies the tool, (7) auto-mode `PermissionDenied`, (10) subagent tool call
      with `agent_id`, (11) `SubagentStop` then resumed same `agent_id`,
      (12) `isolation: worktree` tool `cwd`, (14) explicit
      `run_in_background=true` Bash, (15) `run_in_background=false`
      long-running Bash (the hard gate) — all captured as real fixtures.
    - (16, optional) `PostToolBatch` — captured incidentally as research
      evidence; not required and not consumed by any design decision or
      acceptance criterion.
    - (5) manual permission denial — **waived, non-blocking**: this session's
      Claude Code instance runs with `permission_mode: "auto"`, so no
      human-interactive deny path exists to probe from inside an automated
      session. Probe 6 (another `PreToolUse` hook denies) already establishes,
      for the structurally adjacent non-auto-classifier denial path, that
      `PermissionDenied` does not fire — consistent with D13's own documented
      caveat that manual denial is covered by lifecycle cleanup, not by the
      `PermissionDenied` signal. No fixture required to close this probe.
    - (8) user interrupt before `Stop` — **accepted via documented interrupt
      semantics plus a captured forced-stop analog**: a literal main-thread
      `Ctrl+C` cannot be self-triggered inside an automated turn. A subagent's
      in-flight tool call was instead forcibly killed (`TaskStop`) and produced
      no terminal signal at all (no `PostToolUse`, no `PostToolUseFailure`, no
      `SubagentStop`) — real, captured evidence (see
      `probe08-forced-stop-analog-no-terminal-signal.pre_tool_use.json`)
      supporting the design's existing posture that cleanup cannot rely on a
      single terminal event and must fall back to `SessionEnd`.
    - (9) next main-thread `UserPromptSubmit` after interruption — **accepted
      via the documented `UserPromptSubmit` lifecycle/schema**: no probe-
      specific post-interrupt payload shape is required by D16: every other
      captured event in this fixture set already confirms `UserPromptSubmit`'s
      identity fields (`session_id`, `cwd`) are standard across this Claude
      Code version's hook payloads, and D16's cleanup trigger is the event's
      occurrence, not a special field.
    - (13) `WorktreeRemove` payload — **recorded as attempted but not
      observed**, twice: an isolated-worktree subagent that wrote a file kept
      its worktree on disk (changed worktrees are not auto-cleaned) and an
      isolated-worktree subagent that made no tool calls left no worktree to
      remove. `WorktreeRemove` did not fire in either case within this session.
      D22 is accepted as best-effort rather than requiring a further artificial
      capture attempt; see the D22 disposition below.
  - Verify: fixtures committed under
    `cli/src/services/hooks/claude_mutation_scope/fixtures/` (27 raw payload
    files plus `NOTES.md`) and referenced from this plan. Actual dispositions
    recorded:
    - **D10 PASS** — a real `PostToolUseFailure` fixture exists
      (`probe03-bash-partial-write-then-nonzero-exit.post_tool_use_failure.json`);
      the failed `Bash` call emitted `PostToolUseFailure`, never `PostToolUse`,
      for the same `tool_use_id`; the required identity fields (`session_id`,
      `cwd`, `tool_name`, `tool_use_id`) are present.
    - **D13 PASS** — a real `PermissionDenied` fixture exists for the
      auto-mode-classifier denial path
      (`probe07-auto-mode-permission-denied.permission_denied.json`); a real
      `PreToolUse`-hook denial (`probe06-*`) produced no `PermissionDenied`
      event, matching D13's documented caveat exactly.
    - **D20 PASS** — `run_in_background=false`
      (`probe15-run-in-background-false-hard-gate.*`) blocked in the foreground
      for the full command duration (`duration_ms: 4018` for a `sleep 4`)
      before `PostToolUse` fired; `run_in_background=true`
      (`probe14-run-in-background-true.*`) returned immediately
      (`duration_ms: 8`) with a `tool_response.backgroundTaskId` stub. D20 as
      written is sound; the hard gate is satisfied.
    - **D22 ACCEPTED BEST-EFFORT** — `WorktreeRemove` was not observed because
      neither tested isolated-worktree path actually reached removal (an
      unremoved changed worktree, and an agent that never materialized one).
      `WorktreeRemove` is not made load-bearing; `SubagentStop` (D17) and
      `SessionEnd` (D18) remain the correctness backstops for retiring
      isolated-worktree attempts.
    - **D15 DOC-VERIFIED / NON-LOAD-BEARING** — no live `StopFailure` fixture
      was captured (unreachable without deliberately failing the main turn,
      which this task will not manufacture). `StopFailure` support is kept in
      the adapter mapping, but correctness must not depend on it firing;
      `Stop`, `UserPromptSubmit`, and `SessionEnd` remain the recovery
      backstops per D14/D16/D18.
  - Completed: 2026-09-04
  - Files changed:
    - `cli/src/services/hooks/claude_mutation_scope/fixtures/NOTES.md` (new)
    - `cli/src/services/hooks/claude_mutation_scope/fixtures/probe{01,02,03,04,06,07,08,10,11,12,14,15,16}-*.json`
      (new — 27 raw Claude Code hook-event payloads captured live against
      Claude Code `2.1.258`; see `NOTES.md` for the full manifest and per-probe
      disposition)
    - `context/plans/claude-mutation-scope-integration.md` (this reconciliation)
  - Result: Captured real Claude Code `2.1.258` hook-event fixtures for every
    probe correctness depends on, including the D20 hard gate (PASS) and the
    D10/D13 conditionals (both PASS). D22 (`WorktreeRemove`) and D15
    (`StopFailure`) could not be positively observed within an automated
    session and are closed as accepted-best-effort / doc-verified-non-load-
    bearing rather than forced to a false pass. Probes 5, 8, and 9 are waived
    or accepted on documented semantics plus adjacent captured evidence rather
    than requiring further live capture. No production code, settings, schema,
    or other context files were touched; `.claude/settings.json` was
    temporarily modified during capture (with explicit approval) and fully
    reverted before this task closed.
  - Context impact: None beyond this plan. No Rust, Pkl, generated-settings,
    schema, migration, Quint, or `context/cli|sce` file was changed. T09 will
    draw on these findings (the fixture manifest, `NOTES.md`, and the
    dispositions recorded here) when it authors
    `context/cli/claude-mutation-scope-integration.md`.
  - Context synchronization: synced — this was a research/evidence-gathering
    task; its durable output is the committed fixture files under
    `cli/src/services/hooks/claude_mutation_scope/fixtures/`, `NOTES.md`, and
    this reconciled plan record. No code or domain context file changed, so no
    cross-file context synchronization was required.

- [x] T02: `Raw event model, tool classification, and identity` (status:done)
  - Task ID: T02
  - Scope: In — `cli/src/services/hooks/claude_mutation_scope/mod.rs`: raw event
    parser, supported hook-event enum, tool classifier (known
    mutation-capable: `Bash`, `PowerShell`, `Write`, `Edit`, `NotebookEdit`,
    `MultiEdit` when emitted, `mcp__*`; known read-only: `Read`, `Glob`, `Grep`,
    `WebFetch`, `WebSearch`, `AskUserQuestion`; `Agent` = not a scope; unknown =
    potentially mutation-capable), the explicit-background-shell classifier
    `is_explicit_background_shell` (`tool_name` in `{Bash, PowerShell}` AND
    `run_in_background == true`; model/classify only — D20's `PreToolUse`
    denial is T06's), owner identity (`agent_id` absent = main, present =
    subagent), attempt-key type `(session_id, agent_id?, tool_use_id)`, the
    length-prefixed `cc-tool-v1|n=..|s=..|a=..|t=..` `ScopeId` formatter, and
    the `<scope-id>|start` / `<scope-id>|close` `EventId` formatter. Out — any
    durable state, any runtime/ingress call, any CLI wiring.
  - Dependencies: T01
  - Done when: the module compiles behind the existing hooks module tree; unit
    tests prove AC2, AC4 (formatter determinism), AC5 (formatter is a function of
    `attempt_seq`), AC6, AC21 (classification of explicit background shell), and
    the read-only / delegation / unknown classification table.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_mutation_scope`; `clippy` clean.
  - Completed: 2026-09-04
  - Files changed:
    - `cli/src/services/hooks/claude_mutation_scope/mod.rs` (new)
    - `cli/src/services/hooks/mod.rs` (add `pub mod claude_mutation_scope;`
      registration, alphabetically ordered)
  - Result: Implemented the raw Claude hook-event model: `ClaudeHookEvent` (all
    ten tracked/lifecycle variants plus the no-scope `SessionStart`/
    `SubagentStart` units), `ClaudeToolIdentity`/`ClaudeToolExecution`/
    `ClaudeSessionIdentity`/`ClaudeAgentIdentity`/`ClaudeWorktreeRemove`,
    `AttemptKey` and `ClaudeToolIdentity::attempt_key()`/`is_subagent()`, the
    D2 `classify_tool` classifier, the D20/AC21
    `is_explicit_background_shell(tool_name, run_in_background)` classifier
    (`true` only for `tool_name` in `{Bash, PowerShell}` with
    `run_in_background == true`; a pure model function — no denial behavior,
    which stays T06's), and the D4 `format_claude_scope_id` /
    `claude_scope_start_event_id` / `claude_scope_close_event_id` formatters.
    The strict parser (`parse_claude_hook_event`) follows the existing
    `mutation_scope.rs`/`hooks/mod.rs` validation-helper style
    (`required_field`/`required_non_blank_str`/`optional_non_blank_str` +
    local `validation_error`), validates required identity fields per event,
    and rejects malformed/wrong-type payloads without fabricating identities.
    The module is marked `#![allow(dead_code)]` (matching the
    `services::capabilities` staged-implementation convention) since nothing
    calls it until T06 wires a CLI command. No durable state, ingress call, or
    CLI wiring was added, matching the task's Out-of-scope boundary.

    PR #263 follow-up: the original AC21 tests only exercised the
    `run_in_background` field parser against the default `Write` tool, never
    proving the classification condition on the two tools D20 actually
    targets. Added `is_explicit_background_shell` plus five dedicated
    classification tests (`Bash+true` and `Bash+false` parsed from the real
    committed T01 fixtures `probe14-run-in-background-true.pre_tool_use.json`
    and `probe15-run-in-background-false-hard-gate.pre_tool_use.json` via
    `include_str!`; `PowerShell+true`, `PowerShell+false`, and `Write+true`
    as direct unit calls), so AC21's classification is now actually proven
    rather than only its field-parsing prerequisite.
  - Verify: `services::hooks::claude_mutation_scope` — 42 passed, 0 failed
    (37 + 5 new `is_explicit_background_shell` classification tests);
    `clippy --all-targets -- -D warnings` — clean; `fmt -- --check` — clean;
    AC18 dependency-boundary grep
    (`rg -n --type rust '^\s*use\s+crate::services::mutation_trace::(runtime|protocol|store)|::(RepositoryAgentTraceDb|WorktreeId|GitSnapshotService)\b' cli/src/services/hooks/claude_mutation_scope/`)
    — no matches.
  - Context impact: None beyond this plan. This task adds only an internal,
    not-yet-wired data-model module and one module registration; no CLI
    surface, settings, schema, or documented behavior changed yet, so no
    `context/cli|sce` file needed an update for this task. T09 will document
    the shipped adapter (including this model) once T06 wires it in.
  - Context synchronization: synced — root pass confirmed
    `context/{overview,architecture,glossary,patterns,context-map}.md` contain
    no mention of `claude_mutation_scope`/this task and are unaffected;
    `context/cli/mutation-scope-runtime.md` and
    `context/cli/mutation-scope-hook-ingress.md` already correctly state that
    no concrete harness adapter is wired yet, which T02 leaves true (the new
    module is `#[allow(dead_code)]` and has no caller). No feature, public
    interface, or observable behavior was introduced. No decision qualified
    for an ADR. Documentation of this model is intentionally deferred to T09
    per the plan's own task boundary.

- [x] T03: `Durable checkout-local adapter state` (status:done)
  - Task ID: T03
  - Scope: In — `cli/src/services/hooks/claude_mutation_scope/state.rs`:
    versioned JSON schema (`version`, `next_attempt_seq`, `recovery_pending`,
    `attempts[]` with `phase` in `pending_start | active`), a bounded OS lock at
    `<git-dir>/sce/claude-mutation-scope-state.lock`, atomic durable write
    (temp -> `sync_data` -> rename -> best-effort parent `sync_all` on Unix),
    and read/allocate/update-phase/remove helpers. Out — opening the Agent Trace
    DB, any mutation-runtime call, any hook-event handling.
  - Dependencies: T02
  - Done when: tests cover parallel writers, a leftover lock file, atomic
    replacement, malformed-state rejection, `attempt_seq` allocation, duplicate
    live-attempt reuse (same key -> same `attempt_seq`), and a terminal attempt
    followed by a fresh allocation. Proves AC4, AC5, AC19 (path + no DB/table
    writes).
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_mutation_scope::state`; `clippy` clean.
  - Completed: 2026-09-04
  - Files changed:
    - `cli/src/services/hooks/claude_mutation_scope/state.rs` (new)
    - `cli/src/services/hooks/claude_mutation_scope/mod.rs` (add
      `pub(crate) mod state;` registration)
  - Result: Implemented the D5/D6 adapter-state store: `AdapterState`
    (`version`, `next_attempt_seq`, `recovery_pending`, `attempts: Vec<AdapterAttempt>`)
    and `AdapterAttempt` (`attempt_seq`, `scope_id`, `session_id`, `agent_id`,
    `tool_use_id`, `tool_name`, `phase: AttemptPhase { PendingStart, Active }`),
    versioned via a rejected-not-fabricated `ADAPTER_STATE_VERSION` check. A
    self-contained `AdapterStateLock` (bounded `try_lock` polling with a
    `TimedOut` error, modeled on `mutation_trace::runtime::worktree_lock`'s
    pattern but with no dependency on that type, per D23) guards
    `<git-dir>/sce/claude-mutation-scope-state.lock`; state itself is written
    via a temp-file -> `sync_data` -> rename -> best-effort parent `sync_all`
    durability sequence matching `checkout::persist_checkout_id_inner`, through
    an injectable-hook inner function used by the interruption test. Four
    helpers cover the task's Scope: `read_state` (missing file -> default,
    malformed/wrong-version -> rejected), `allocate_attempt` (idempotent reuse
    of an existing live attempt for the same `AttemptKey`, AC4; otherwise a
    fresh monotonic `attempt_seq` and `ScopeId` via
    `format_claude_scope_id`, never reused once terminal, AC5),
    `mark_active` (`PendingStart` -> `Active`), and `remove_attempt`
    (safe no-op on an already-removed scope, matching D9's duplicate-terminal-
    delivery note). All four are lock-guarded read-modify-write operations.
    Thirteen new unit tests cover: default-on-missing-file, sequential
    `attempt_seq` allocation across distinct keys, duplicate-key reuse without
    advancing the counter, a terminal attempt followed by a fresh
    (non-reused) allocation, the `PendingStart`->`Active` transition,
    idempotent removal, malformed-JSON rejection, unsupported-version
    rejection, an injected pre-rename interruption leaving the canonical path
    untouched, a leftover lock file (no active OS lock) not blocking a new
    acquirer, concurrent writers on distinct keys converging without lost
    updates, lock contention between two acquirers, and an AC19 path-boundary
    check that every written path stays under `<git-dir>/sce/`. No Agent
    Trace DB, mutation-runtime, or hook-event-handling code was touched,
    matching the task's Out-of-scope boundary.
  - Verify: `services::hooks::claude_mutation_scope` (including the new
    `::state` module) — 55 passed, 0 failed (42 existing + 13 new state
    tests); `clippy --all-targets -- -D warnings` — clean; `fmt -- --check` —
    clean; AC18 dependency-boundary grep
    (`rg -n --type rust '^\s*use\s+crate::services::mutation_trace::(runtime|protocol|store)|::(RepositoryAgentTraceDb|WorktreeId|GitSnapshotService)\b' cli/src/services/hooks/claude_mutation_scope/`)
    — no matches.
  - Context impact: None beyond this plan. This task adds only an internal,
    not-yet-wired durable-state module (no CLI surface, settings, schema, or
    documented behavior changed), so no `context/cli|sce` file needed an
    update for this task. T09 will document the shipped adapter state (this
    module) once T06 wires it in, per the plan's existing Context sync list.
  - Context synchronization: synced — root pass confirmed
    `context/{overview,architecture,glossary,patterns,context-map}.md`
    contain no mention of `claude_mutation_scope`/this task and are
    unaffected; `context/cli/mutation-scope-hook-ingress.md` and
    `context/cli/mutation-scope-runtime.md` both already correctly state that
    no concrete harness adapter is wired yet, which T03 leaves true (the new
    state module is internal bookkeeping with no caller). No feature, public
    interface, or observable behavior was introduced. No decision qualified
    for an ADR. Documentation of this state module is intentionally deferred
    to T09 per the plan's own task boundary.

- [x] T04: `Capture the detached-descendant Bash lifecycle probe` (status:done)
  - Task ID: T04
  - Scope: In — a T01 follow-up, using the same live-capture methodology
    against the same pinned Claude Code version (`2.1.258`, or whatever
    version this plan still targets at execution time): drive one foreground
    `Bash` tool call (`run_in_background=false`) whose command starts a
    detached, self-backgrounding descendant process that writes a repository
    file after a short delay, with the invoked (parent) process exiting
    immediately — e.g. Python `subprocess.Popen([...], start_new_session=True)`
    launched from the `Bash` command, or an equivalent `setsid`/double-fork
    shell pattern. Record the `PreToolUse` timestamp, the `PostToolUse`
    timestamp, and the wall-clock time the detached child actually wrote the
    file, and determine whether `PostToolUse` fired before or after that
    write. Commit the raw fixture(s) under the existing
    `cli/src/services/hooks/claude_mutation_scope/fixtures/` directory,
    following the existing `probeNN-*` naming convention (e.g.
    `probe17-detached-child-after-post-tool-use`), and record the observation,
    the exact timestamps, and the reconciled D20 disposition in `NOTES.md`.
    Update D20 in this plan's Design section with the concrete finding (the
    exact observed ordering and the process pattern actually tested).
    Out — any production code, any Rust module or CLI wiring, any process
    supervision / PID tracking / process-group implementation, any shell-
    command parsing or deny-listing, any change to the generic mutation-scope
    protocol, Quint model, schema, or attribution algorithm, and modifying any
    existing (already-committed) T01 fixture file.
  - Dependencies: T01
  - Done when: the new fixture(s) exist and are referenced from this plan;
    `NOTES.md` records the exact `PreToolUse`/`PostToolUse`/child-write
    timestamps and the derived ordering; and D20 is updated with exactly one
    of these two dispositions, chosen by what was actually observed rather
    than assumed:
    - If the descendant's mutation is observed to land after `PostToolUse`:
      D20 stays PASS only for Claude-managed explicit background execution
      (`run_in_background=true`); the self-detaching-descendant boundary this
      task's evidence supports is recorded as confirmed, not merely
      theoretical — foreground shell execution is not claimed to be
      universally safe.
    - If `PostToolUse` is observed to wait for the descendant to exit too:
      record that exact observed behavior for this specific process pattern
      and this specific Claude Code version, without generalizing it into a
      safety guarantee for every detachment technique — `nohup`, `setsid`,
      double-fork, and daemonizing patterns this probe did not exercise
      remain unproven, and the unsupported-boundary wording in D20 stays in
      place regardless (this PR still implements no detection or
      supervision).
    Satisfies AC25 together with T08 Test17.
  - Verify: fixture(s) exist under
    `cli/src/services/hooks/claude_mutation_scope/fixtures/`; `NOTES.md`
    documents the observation and timestamps; this plan's D20 section reflects
    the reconciled finding.
  - Completed: 2026-09-04
  - Files changed:
    - `cli/src/services/hooks/claude_mutation_scope/fixtures/probe17-detached-child-after-post-tool-use.pre_tool_use.json`
      (new, then replaced with a corrected real recapture — see PR #263
      follow-up below)
    - `cli/src/services/hooks/claude_mutation_scope/fixtures/probe17-detached-child-after-post-tool-use.post_tool_use.json`
      (new, then replaced with a corrected real recapture for the same
      `tool_use_id` — see PR #263 follow-up below)
    - `cli/src/services/hooks/claude_mutation_scope/fixtures/probe17-detached-child-after-post-tool-use.evidence.json`
      (new — PR #263 follow-up — machine-readable capture metadata:
      versions, timestamps, `git check-ignore`/`git status` results, and the
      before/after tree hashes)
    - `cli/src/services/hooks/claude_mutation_scope/fixtures/NOTES.md` (update
      — new probe-17 table row plus a "T04 addendum" section recording the
      exact captured timestamps, the tested process pattern, and the
      reconciled D20 disposition; rewritten in the PR #263 follow-up to
      separate Claude lifecycle evidence from Git-observable mutation
      evidence)
    - `context/plans/claude-mutation-scope-integration.md` (this
      reconciliation — D20 updated with the concrete T04 finding, then
      corrected in the PR #263 follow-up)
  - Result: Captured a real, live Claude Code `2.1.258` fixture for the
    detached-descendant probe using T01's exact methodology (a scratch
    `PreToolUse`/`PostToolUse` dump hook temporarily registered on the `Bash`
    matcher in `.claude/settings.json`, alongside the existing SCE entries).
    Drove one foreground `Bash` call (`run_in_background` omitted, i.e.
    `false`) running `setsid bash -c '... sleep 3; <write-marker> ...' &` — a
    shell-level self-detaching descendant, the `setsid`/backgrounding
    equivalent of Python `subprocess.Popen(..., start_new_session=True)`
    named in the task. The invoked shell returned in `duration_ms: 15`
    (`tool_response.stdout` independently confirms `"parent exiting at
    2026-09-04T21:45:07.207863Z"`), never waiting on the detached child.
    Observed timestamps: `PreToolUse` `2026-09-04T21:44:58.468Z`,
    `PostToolUse` `2026-09-04T21:45:07.219Z`, descendant's own
    repository-mutating write `2026-09-04T21:45:10.221976Z` (~3.00s after
    `PostToolUse`, matching the child's `sleep 3`). **Ordering: the
    descendant's mutation landed after `PostToolUse`** — the first of the
    task's two anticipated dispositions. D20 was updated accordingly: the
    self-detaching-descendant boundary is now a confirmed, observed finding
    for this process pattern and Claude Code version, not merely a
    theoretical one, without generalizing to `nohup`/double-fork/daemonizing
    patterns this probe did not exercise. No production code, Rust module,
    CLI wiring, process supervision, or protocol/schema change was made,
    matching the task's Out-of-scope boundary; no already-committed T01
    fixture file was modified. `.claude/settings.json` was temporarily
    modified during capture (with explicit user approval, since the
    auto-mode classifier initially blocked the edit) and fully reverted
    (byte-for-byte, confirmed via diff) before this task closed.

    **PR #263 follow-up (2026-09-05):** review found the original capture's
    marker path (`context/tmp/probe17-detached-child-write.marker`) was
    wholesale-ignored by `context/tmp/.gitignore`, so
    `GitSnapshotService::capture_tree()`'s `git add -A -- .` would never see
    it — the original evidence proved only that a detached descendant
    survives `PostToolUse`, not that its mutation is Git-observable to SCE's
    own snapshot model. T04 was rerun end-to-end with: (1) a non-ignored,
    repository-root marker path
    (`probe17-detached-child-write.marker`), verified via
    `git check-ignore -v` (exit `1`, not ignored) both before the probe and
    after the child's write; (2) explicit `"run_in_background":false`
    present in the real captured `tool_input` (previously omitted, not
    hand-edited in afterward); (3) the same `setsid`-based self-detaching
    descendant pattern; and (4) a `GIT_INDEX_FILE`-scoped temporary index
    (never the real `.git/index`) that reproduced
    `capture_tree()`'s exact `git read-tree HEAD` / `git add -A -- .` /
    `git write-tree` sequence before and after the child's write, yielding
    two different tree hashes
    (`596fcceafa2ebf70a087f606d7e16645f18ee17e` vs
    `b24d653632e478298b625e86c99f51f4016f9f57` — T1 ≠ T2). New observed
    timestamps: `PreToolUse` `2026-09-04T22:18:09.195Z`, `PostToolUse`
    `2026-09-04T22:18:21.678Z` (`duration_ms: 13`), descendant's write
    `2026-09-04T22:18:24.674140Z` (~3.00s after `PostToolUse`) — the same
    `t1 < t2 < t3` ordering and disposition as before, now with direct proof
    the descendant's mutation is Git-observable. The two raw fixture files
    were replaced with these real recaptured payloads (same `tool_use_id`
    `toolu_011DiMMHcxCZr6HzXWZhWzmD` across both), a new `.evidence.json`
    artifact was added, `NOTES.md`'s T04 addendum was rewritten to separate
    Claude lifecycle evidence from Git-observable mutation evidence and the
    derived SCE attribution consequence, and D20 above was updated
    accordingly. The temporary marker file was deleted after capture and is
    not part of the committed fixture set. `.claude/settings.json` was again
    temporarily modified (with explicit user approval) and fully reverted
    (byte-for-byte, confirmed via diff) before this follow-up closed. No
    production code, Rust module, CLI wiring, protocol, Quint, or schema
    change was made; no already-committed T01 fixture file was touched.
  - Verify: fixtures
    `probe17-detached-child-after-post-tool-use.{pre_tool_use,post_tool_use,evidence}.json`
    committed and referenced from this plan and from `NOTES.md`; `NOTES.md`'s
    "T04 addendum" documents the exact observation and timestamps plus the
    Git-observability proof; this plan's D20 section reflects the corrected,
    reconciled finding — all satisfied. `git diff --stat` against the prior
    commit confirms no file outside
    `cli/src/services/hooks/claude_mutation_scope/fixtures/` and this plan
    changed.
  - Context impact: None beyond this plan. No Rust, Pkl, generated-settings,
    schema, migration, Quint, or `context/cli|sce` file was changed; only new
    fixture files, `NOTES.md`, and this plan's own D20/task record were
    touched. T09 will incorporate this finding when it authors
    `context/cli/claude-mutation-scope-integration.md` per the plan's
    existing task boundary.
  - Context synchronization: synced — root pass confirmed
    `context/{overview,architecture,glossary,patterns,context-map}.md` contain
    no mention of the detached-descendant finding and are unaffected;
    `context/cli/mutation-scope-runtime.md` and
    `context/cli/mutation-scope-hook-ingress.md` both already correctly state
    that no concrete harness adapter is wired yet, which T04 leaves true (only
    a research fixture, `NOTES.md`, and this plan's own D20 wording changed —
    no adapter, CLI, or production code exists yet for D20's confirmed finding
    to attach to). No feature, public interface, or observable behavior was
    introduced. The detached-descendant boundary D20 now confirms was already
    an established design decision in this plan before T04 ran; T04 supplied
    empirical evidence for it rather than establishing a new system-wide
    decision, so no ADR qualified. Documentation of this finding in durable
    `context/cli` files is intentionally deferred to T09 per the plan's own
    task boundary.

- [ ] T05: `Expose the in-process generic-ingress seam` (status:todo)
  - Task ID: T05
  - Scope: In — make the minimal crate-visible function on
    `cli/src/services/hooks/mutation_scope.rs` that runs a normalized JSON
    payload against `coordinate()` / `abandon_scope()` in-repo with a lazy DB
    provider (the existing `run_mutation_scope_from_payload` made `pub(crate)`,
    or a thin `pub(crate)` wrapper with the documented signature). Out — any
    behavior change to the existing `sce hooks mutation-scope` command, any new
    payload operation, any `RuntimeBoundary` construction outside
    `mutation_scope.rs`.
  - Dependencies: T01
  - Done when: the seam is callable from a sibling `hooks` module, the existing
    `mutation_scope` command path is byte-for-byte unchanged in behavior, and
    `services::hooks::mutation_scope` tests still pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::mutation_scope`; `git diff` shows only a visibility/wrapper change.
  - Context synchronization: pending

- [ ] T06: `Claude adapter driver + CLI command` (status:todo)
  - Task ID: T06
  - Scope: In — `cli_schema::HooksSubcommand::ClaudeMutationScope` (hidden),
    `convert_hooks_subcommand_request` arm,
    `services::hooks::HookSubcommand::ClaudeMutationScope`,
    `run_hooks_subcommand_in_repo` dispatch (unwrapped, non-fail-open like
    `mutation-scope`), and the adapter driver in `claude_mutation_scope/mod.rs`
    mapping each event: `PreToolUse -> Start` (write-ahead `pending_start` ->
    seam `Start` -> `active`, fail-closed Claude `deny` on any failure, explicit
    background-shell `deny`), `PostToolUse -> Close`, `PostToolUseFailure ->
    Close`, `PermissionDenied -> Abandon`, `Stop` / `StopFailure` -> main
    stale cleanup, `UserPromptSubmit` -> interrupted-main cleanup, `SubagentStop`
    -> matching-agent cleanup, `SessionEnd` -> session cleanup,
    `WorktreeRemove` -> worktree cleanup (using `worktree_path`), plus the
    uncertain-boundary abandonment rules (D11/D12) and the recovery barrier
    (D19: deny new mutation-capable `PreToolUse` while `recovery_pending` and
    outstanding attempts remain; `flush` through the seam once quiescent). Reads
    exactly one raw Claude hook JSON object from STDIN; emits empty stdout except
    the intentional `PreToolUse` decision object. Out — generated settings /
    `sce setup` wiring (T07), real Git/DB regressions (T08).
  - Dependencies: T02, T03, T05
  - Done when: focused tests with an injected generic-ingress seam cover every
    event-to-operation mapping, fail-closed `PreToolUse` (exact
    `permissionDecision: "deny"` JSON, AC8), write-ahead ordering (AC7),
    `pending_start` + terminal -> abandon (D11), close-failure -> abandon +
    `recovery_pending` (D12), and the recovery barrier (D19). AC1 routing test
    passes.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_mutation_scope`; `sce hooks claude-mutation-scope </dev/null` shows the strict-parser error; `sce hooks --help` omits it.
  - Context synchronization: pending

- [ ] T07: `Generated Claude integration, setup merge, and doctor` (status:todo)
  - Task ID: T07
  - Scope: In — `config/pkl/renderers/claude-content.pkl`: add
    `sce hooks claude-mutation-scope` registrations for `PreToolUse`,
    `PostToolUse`, `PostToolUseFailure`, `PermissionDenied`, `UserPromptSubmit`,
    `Stop`, `StopFailure`, `SubagentStop`, `SessionEnd`, `WorktreeRemove` (no
    `matcher`), leaving `claude-model-state`, bash policy, `diff-trace`, and
    `conversation-trace` unchanged; verify `config_merge.rs` still preserves
    user hooks, replaces only SCE-owned entries, adds the new event keys, and
    stays idempotent; add or adjust the focused setup/doctor tests only if the
    existing generated-fragment comparison does not already cover the new
    registrations. Out — any adapter behavior change, any non-Claude renderer.
  - Dependencies: T06
  - Done when: `nix run .#pkl-check-generated` passes with the new registrations;
    `config_merge.rs` tests prove AC22 (merge + idempotency + user-hook
    preservation); doctor recognizes a missing/stale new registration.
  - Verify: `nix run .#pkl-check-generated`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::setup::`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::doctor::` (or the specific test module the new registrations land in, if narrower).
  - Context synchronization: pending

- [ ] T08: `Real Git/DB regressions through the production path` (status:todo)
  - Task ID: T08
  - Scope: In — regressions using real temporary Git repositories and real
    repository Agent Trace DBs, driven through the production Claude-adapter ->
    generic-ingress path (no manual `mutation_trace_*` inserts): Test1 foreground
    `Write` -> `AiExclusive` + `Closed`; Test2 failed `Bash` with partial write
    -> `AiExclusive` + `Closed`; Test3 parallel mutation tools -> `AiContended`;
    Test4 duplicate `Pre`/`Post` replay -> no duplicate transition; Test5 auto
    `PermissionDenied` -> `Abandoned` + rebaseline; Test6 manual/other-hook
    denial -> `Stop` cleanup; Test7 interrupted main turn -> `UserPromptSubmit`
    cleanup; Test8 subagent tool uses a distinct scope; Test9 main + subagent
    concurrent mutation -> `AiContended`; Test10 isolated subagent worktree ->
    correct `WorktreeId`/cursor, main cursor unchanged; Test11 supplying a valid
    `WorktreeRemove` event cleans the correct outstanding worktree attempt state
    (a best-effort signal the adapter acts on when it arrives — this test does
    not claim Claude must emit `WorktreeRemove` in every cleanup case; D17/D18
    remain the load-bearing backstops for when it does not); Test12
    `pending_start` crash before
    `Start` -> conservative recovery; Test13 `Start` committed before state
    settlement -> abandonment recovery; Test14 terminal runtime success before
    state cleanup -> replay-safe; Test15 explicit background `Bash` -> denied, no
    scope; Test16 raw Agent Trace tables (`diff_traces`,
    `post_commit_patch_intersections`, `agent_traces`) unchanged; Test17 a
    foreground `Bash` (`run_in_background=false`) that starts a detached,
    self-backgrounding descendant which mutates the repository after
    `PostToolUse` returns — asserts that the scope closes (`Closed`) at the
    tool's own observed tree and that the descendant's later mutation is
    **not** captured by that scope or folded into its attribution; this is a
    documented unsupported-case regression proving the adapter does not
    silently claim correct attribution for a mutation occurring after the
    tool scope already closed, not an assertion that the adapter detects or
    supervises the descendant. Each applicable test asserts scope status,
    processed-event keys, revision, `cursor_tree`, mutation-event count,
    attribution kind, `needs_rebaseline`, and adapter state. Out — new
    production behavior; any process supervision, PID tracking, or
    detached-child detection; any test that inserts the event it means to
    prove.
  - Dependencies: T06 (and T07 for any test that installs generated settings);
    Test17 also depends on T04's captured fixture/evidence.
  - Done when: all seventeen regressions pass and collectively satisfy
    AC9–AC17, AC19, AC20, AC21, AC25.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::hooks::claude_mutation_scope`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::mutation_trace::`.
  - Context synchronization: pending

- [ ] T09: `Author the durable adapter context` (status:todo)
  - Task ID: T09
  - Scope: In — create `context/cli/claude-mutation-scope-integration.md` owning
    the tool-attempt scope model, tool classification, `ScopeId`/`EventId`
    derivation, adapter state, write-ahead `Start`, fail-closed `PreToolUse`,
    terminal `Close` and failed-tool behavior, abandonment cleanup signals, the
    recovery barrier, subagent identity, worktree-cwd ownership, the
    background-shell limitation (both Claude-managed `run_in_background=true`
    denial and the separate self-detaching-descendant unsupported boundary
    D20 records, with T04's reconciled finding), and the generic-ingress
    dependency boundary;
    update `context/cli/mutation-scope-runtime.md`,
    `context/cli/mutation-scope-hook-ingress.md`,
    `context/sce/agent-trace-hooks-command-routing.md`,
    `context/sce/claude-raw-hook-capture.md` (current Claude
    hook-routing/generated-settings state; not
    `context/sce/generated-opencode-plugin-registration.md`, which owns OpenCode
    plugin registration), `context/context-map.md`, `context/overview.md`,
    `context/architecture.md` to reference the shipped adapter and the new
    in-process seam. Out — any code change; describing behavior not actually
    shipped by T02–T08.
  - Dependencies: T02, T03, T04, T05, T06, T07, T08
  - Done when: the new file exists and the cross-references are updated; AC24
    and AC25 inspection passes; `nix flake check` (context has no generated
    check but the map/overview must stay internally consistent).
  - Verify: inspection against AC24/AC25; `grep` shows the new route documented
    in the routing file and the new file linked from `context/context-map.md`.
  - Context synchronization: pending

## Open questions

- ~~**Does the Claude Code version SCE chooses to support implement
  `PostToolUseFailure`, `StopFailure`, `PermissionDenied`, and `WorktreeRemove`
  with the payloads and lifecycle semantics D10, D13, D15, and D22 require?**~~
  **Resolved by T01** against Claude Code `2.1.258` (see T01's Verify record and
  `cli/src/services/hooks/claude_mutation_scope/fixtures/NOTES.md`):
  `PostToolUseFailure` and `PermissionDenied` fire exactly as D10/D13 assume and
  carry the required identity fields (**PASS** for both). `WorktreeRemove` was
  not observed to fire for either isolated-worktree cleanup path tested, and
  `StopFailure` could not be exercised without deliberately failing a turn.
  Neither is treated as blocking, and neither registration is dropped: T06/T07
  keep the `WorktreeRemove` handler and registration, and the adapter keeps
  `StopFailure` support, but correctness does not depend on either firing
  (D22 accepted best-effort; D15 doc-verified/non-load-bearing). `SessionEnd`
  (D18), `Stop` (D14), `UserPromptSubmit` (D16), and `SubagentStop` (D17)
  remain the load-bearing correctness backstops for all stale-attempt cleanup
  regardless of whether `WorktreeRemove`/`StopFailure` arrive — i.e. T02+
  proceeds on the full original event set (AC10, AC12, AC13, and
  Test2/Test5/Test11 all still apply, with Test11 reframed to prove
  `WorktreeRemove` cleanup when the event is supplied rather than to require
  Claude to always emit it), since `PostToolUseFailure` and `PermissionDenied`
  themselves came back `PASS`.
- ~~For a failed tool, does the chosen Claude Code version fire `PostToolUse` at
  all, only `PostToolUseFailure`, or both?~~ **Resolved by T01**: on Claude Code
  `2.1.258`, exactly one of the two fires per attempt — a failed `Bash` call
  emits only `PostToolUseFailure`, never `PostToolUse`, for the same
  `tool_use_id` (see
  `probe03-bash-partial-write-then-nonzero-exit.post_tool_use_failure.json`).
  D9/D10's mapping (both events close the scope) needs no revision.
- Is a 10-event, 16-regression first adapter the right size, or should the first
  PR land the core loop (`PreToolUse`/`PostToolUse` + `Stop`/`SessionEnd`
  cleanup, foreground `Write`/`Edit`/`Bash`, no subagent-worktree isolation) and
  leave subagent identity, `isolation: worktree`, and the full cleanup matrix to
  a stacked follow-up? The current slicing is coherent, but T06 is large and its
  correctness rests entirely on T01's findings.
