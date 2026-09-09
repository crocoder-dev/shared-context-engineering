# Codex mutation-scope integration

The Codex mutation-scope adapter is SCE's second concrete harness producer. It
lives in `cli/src/services/hooks/codex_mutation_scope/` behind the hidden
`sce hooks codex-mutation-scope` command. Its only production dependency inside
the mutation stack is the in-process
`hooks::mutation_scope::run_mutation_scope_from_payload` seam; it does not call
runtime, protocol, or database modules directly.

The adapter was tested against codex-cli **0.153.4** (upstream `openai/codex`
tag `rust-v0.153.4`, commit `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`); raw
lifecycle evidence is in `fixtures/NOTES.md` and
[`codex_mutation_scope/fixtures`](../../cli/src/services/hooks/codex_mutation_scope/fixtures/).

## Scope model and coverage

The attribution unit is one independently mutation-capable **tracked** Codex
tool execution, never a session, turn, or delegated agent. Each tracked
execution receives one fresh `ScopeId`, even when session or tool identifiers
repeat.

| Codex tool class | Tool names in v1 | Mutation-scope behavior |
| --- | --- | --- |
| `TrackedMutation` | `Bash`, `apply_patch` | one attempt, one scope, write-ahead `Start`, terminal `Close` |
| `Delegation` | `collaborationspawn_agent`, `collaborationwait_agent` | no scope for the delegation tool; tracked tools of the delegated agent get their own scopes |
| `Untracked` | `mcp__*`, and every unknown/future tool name | neutral pass-through; no scope, `Start`, terminal bookkeeping, or recovery state |

`Untracked` means outside attribution coverage, not read-only. MCP and unknown
tools remain usable and may mutate the checkout, but their mutations are not
individually attributed; the adapter does not deny them, emit an explicit allow,
or claim to have observed them.

### Why MCP is outside v1 coverage

T01's live probes against codex-cli 0.153.4 established that a mutation-capable
MCP tool can write a Git-visible file, return `CallToolResult.is_error == true`,
and emit no terminal hook; a failed call can be followed directly by another
MCP `PreToolUse` with no cleanup signal; and two mutation-capable MCP calls can
genuinely overlap when parallel calls are enabled by server configuration or
the tool's `readOnlyHint` annotation.

That is D10a Case C if MCP were modeled as a scope: the adapter could not
distinguish a failed-and-dead scope from a still-running one. The v1 resolution
is therefore a deliberate coverage boundary: MCP and unknown tools create no
adapter attempt, so they cannot strand a zombie mutation scope or produce
MCP-derived `AiContended`. First-class MCP attribution using a richer,
overlap-tolerant lifecycle mechanism is deferred to a separate, explicitly
justified change.

The runtime describes exclusivity only among known tracked scopes:
`AiExclusive(scope)` means exactly one was live, not that it authored every
filesystem mutation. An MCP call, human editor, or other untracked actor may
mutate the same worktree; tracked Codex plus MCP may still yield `AiExclusive`
for the tracked scope.

## Raw event mapping

The parser accepts the probed forms `PreToolUse`, `PostToolUse`, `Stop`,
`Interrupt`, `SubagentStop`, and `SessionEnd`, with non-blank identity fields.
Tool identity is `(session_id, agent_id?, tool_use_id)`; `turn_id` is lane
metadata, not part of the execution key.

| Raw event | Tracked adapter action |
| --- | --- |
| `PreToolUse` for `Bash` or `apply_patch` | classify, run the Bash policy preflight when applicable, admit an attempt, persist `pending_start`, invoke ingress `start`, mark it `active`, then return neutral continue |
| `PostToolUse` for `Bash` or `apply_patch` | find the matching attempt and invoke ingress `close`; remove adapter state only after the close succeeds |
| `Stop` | abandon outstanding main-agent attempts in the session; it does not sweep delegated-agent attempts |
| `Interrupt` | abandon all outstanding attempts in the session |
| `SubagentStop` | abandon attempts for the matching session and `agent_id` |
| `SessionEnd` | abandon all outstanding attempts in the session; this is the load-bearing cleanup backstop |

Terminal hooks are positive lifecycle evidence; cleanup is never inferred from
inactivity, `ActorKind`, or a state file. `PostToolUse` for an untracked tool is
ignored, and delegation/untracked `PreToolUse` returns empty, Codex-neutral
stdout without resolving Git, acquiring state, or calling the seam.

T01 proved built-in serial execution in the `(session_id, turn_id)` lane. Before
a successor, the adapter sweeps a different tracked predecessor in that lane
when an arbitrary sibling hook may have denied it after SCE's `Start`; other
sessions/turns are not swept, and MCP is not part of this rule.

## Identity and ingress contract

Each new attempt gets a checkout-local monotonic sequence and length-prefixed ID:

```text
cx-tool-v1|n=<attempt_seq>|s=<len>:<session_id>|a=<len>:<agent_id>|t=<len>:<tool_use_id>
```

The agent ID is empty for the main agent; event IDs are `<scope>|start` and
`<scope>|close`. Live duplicates reuse attempt, scope, and event IDs; a later
execution never reuses a terminal scope, even if Codex reuses `tool_use_id`.

The raw hook `cwd` is passed as `repository_root`; the runtime derives Git
directory, checkout identity, snapshots, and revisions. The adapter never
constructs `worktree_id`, and sends `actor_kind: "codex"` through the existing
seam without spawning `sce`.

A tracked `Start` also carries scope provenance (the `cx_`-prefixed session and
normalized `model`); see [mutation-scope provenance](mutation-scope-provenance.md).

## Write-ahead admission and failure posture

Tracked `PreToolUse` follows this ordering:

```text
boundary lock
  -> state lock -> allocate sequence -> persist pending_start
  -> generic ingress Start(scope, <scope>|start, codex, provenance)
  -> state lock -> mark active
  -> empty stdout / Codex continue
```

Failure to resolve checkout, acquire a lock, persist state, evaluate Bash
policy, or establish `Start` is fail-closed for tracked tools:

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"SCE could not establish mutation attribution for this tool execution."}}
```

The detailed failure is logged as
`sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed`, not exposed in the
denial reason. Delegation and untracked tools do not use this path.

The generated `PreToolUse` bootstrap also fails closed for matcher
`^(Bash|apply_patch)$` when Git-root resolution, the helper, `sce`, or adapter
cannot be reached, while preserving MCP/unknown pass-through. If `Start` may
have committed but `mark_active` failed, `pending_start` blocks a tracked
successor until positive cleanup and a quiescent flush complete.

## Recovery and durable state

Adapter bookkeeping is checkout-local:

```text
<git-dir>/sce/codex-mutation-scope-state.json
<git-dir>/sce/codex-mutation-scope-state.lock
```

Version-3 JSON stores `next_attempt_seq`, recovery generation,
`clear`/`pending`/`flushing`, and attempts containing scope, execution identity,
tool, lane turn, and `pending_start`/`active` phase. Writes stage, `sync_data`,
rename, and sync the directory where supported. The boundary lock serializes
state-to-ingress-to-state across hook processes (boundary lock then state lock);
file existence alone is never ownership.

Positive cleanup arms `recovery_pending`, abandons matching tracked scopes via
generic `abandon`, and removes settled attempts. Known attempts keep tracked
`PreToolUse` denied; once empty, one generic `flush` re-baselines and clears
recovery. Flush failure leaves the barrier armed. Untracked events never enter
or are blocked by this barrier.

The cleanup matrix is identity-scoped: `Stop` covers the session's main agent,
`Interrupt` and `SessionEnd` cover the session, and `SubagentStop` covers one
delegated agent. A linked worktree's `cwd` resolves to its own independent state
and cursor.

## Concurrency and attribution confirmation

Codex built-in tracked tools were observed serially, so no Codex-only overlap
was observed; a tracked Codex scope can currently overlap a Claude Code scope on
the same worktree. Generic runtime and `ActorKind` support future OpenCode/Pi adapters once wired.

The accepted boundary-aware rule is important: Codex `Start` is write-ahead
admission, not positive execution confirmation, because an arbitrary sibling
`PreToolUse` hook can deny after SCE's hook succeeds. While an active Codex
scope remains unconfirmed, a mutation observed at any non-confirming boundary
is `IneligibleUnscoped`, including at another harness's boundary. Only that
exact Codex scope's own proven `PostToolUse` → `Close` confirms it for the
current boundary. Then ordinary `AiExclusive`/`AiContended` rules apply if no
other unconfirmed Codex scope remains. A second unconfirmed live Codex scope
keeps the result ineligible. The complete live scope set remains in the
mutation event; no `confirmed` bit is persisted.

This conservative rule prefers false negatives to false-positive authorship
claims. It is the only accepted protocol/Quint follow-up; T07 adds no further
protocol, runtime-semantic, attribution-algorithm, SQL, or Agent Trace schema
change.

## Configuration, trust, and ownership

The hidden command is separate from the existing fail-open `sce hooks codex`
conversation/diff dispatcher. `sce setup --codex` and `--all` install both
contracts through the shared `codex_hook_config` merge service. The generated
`.codex/hooks.json` has four existing `sce hooks codex` registrations and six
mutation-scope registrations: matched `PreToolUse` and `PostToolUse` groups for
`^(Bash|apply_patch)$`, plus unmatched `Stop`, `Interrupt`, `SubagentStop`, and
`SessionEnd` groups. They are appended after existing groups so the event,
matcher, group index, and handler index used by Codex trust remain stable.

Ownership recognizes the helper path plus the exact trailing command contract
for either `sce hooks codex` or `sce hooks codex-mutation-scope`. Unrelated valid
Codex handlers and fields survive merge; malformed or Codex-invalid documents
fail before staging. Doctor reports mutation-scope rows separately from the
conversation/diff rows and keeps three dimensions distinct: structural
registration, Codex trust, and effective project-hook policy. Doctor reads
Codex trust/policy state and never writes it; `--fix` repairs only SCE-owned
structure. The event-key labels `interrupt`, `subagent_stop`, and `session_end`
are verified against the supported upstream event names.

## Existing Codex evidence remains separate

The existing `sce hooks codex` behavior is additive and unchanged: it captures
`UserPromptSubmit`/`Stop` conversation rows, applies the Bash policy, and
captures `PostToolUse(apply_patch)` diff evidence. The mutation-scope adapter
writes only through the generic mutation runtime and only to `mutation_trace_*`
tables. It does not write `diff_traces`, `post_commit_patch_intersections`,
`agent_traces`, `messages`, or `parts`, and it does not fold the complementary
apply-patch evidence pipeline into mutation-scope storage.

Codex has no Codex-managed background execution surface in the supported
`codex exec` path. A foreground Bash command can still spawn a self-detaching
descendant that writes after `PostToolUse`; the adapter does not supervise
processes, inspect process groups, poll for staleness, or treat `PostToolUse` as
proof that every descendant stopped mutating. Such writes are outside the
closed tracked interval and remain conservatively unscoped.

## Unsupported / coverage boundary

The v1 adapter intentionally leaves these cases outside individual Codex
mutation-scope attribution:

- MCP tools, including mutation-capable and parallel MCP tools;
- unknown and future Codex tool names until their lifecycle is researched;
- filesystem mutations by humans or detached descendants; and
- first-class attribution for MCP, which is future work requiring a richer
  lifecycle mechanism and a separate design/implementation change.

This boundary is tested and documented for codex-cli 0.153.4. It is not a claim
that excluded tools are read-only, harmless, or immediately detectable.

## Verification evidence

The adapter is exercised through real temporary Git repositories, real
repository Agent Trace databases, and the production entrypoint. Regressions
cover write-ahead admission, duplicate and reused identities, tracked
success/failure, cleanup signals, linked worktrees, MCP pass-through,
mutate-then-error and parallel MCP, tracked-tool plus MCP overlap, denial
recovery, crash points, both directions of the boundary-aware attribution rule,
and scope provenance for both tracked tools.

The frozen protocol/Quint/runtime baseline and the Agent Trace SQL/schema
boundary remain unchanged after the accepted D14 follow-up:
```text
git diff b72f6c2c -- spec/mutation_cursor.qnt spec/mutation_cursor.md \
  cli/src/services/mutation_trace/protocol.rs \
  cli/src/services/mutation_trace/runtime/
git diff origin/claude-mutation-scope-integration -- \
  cli/migrations/agent-trace-repository/ config/schema/agent-trace.schema.json
```
