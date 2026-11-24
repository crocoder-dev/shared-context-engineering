# Decision: Claude mutation scopes are routed to their PreToolUse checkout

Date: 2026-11-24
Status: Accepted
Plan: `context/plans/claude-mutation-scope-stable-repository-routing.md`
Task: `T01, T02, T03`

## Context

The Claude mutation-scope adapter resolved its repository from the hook `cwd`
on every hook event, independently. A live Claude Code `2.1.309` probe (plan
T01) showed that `cwd`, and the Git directory resolved from it, can change
within one `tool_use_id`. For example, `PreToolUse` reported repository A while
`PostToolUse` reported nested repository B after `cd nestedB`. The terminal
event then looked for the attempt in B, found nothing, and left the scope
`Active` in A.

The same probe showed that `CLAUDE_PROJECT_DIR` and its Git directory stayed the
same in every hook event, including subagent and terminal events. Every
generated SCE Claude hook command already depends on `CLAUDE_PROJECT_DIR`.

## Decision

Every tracked Claude mutation-capable tool execution is bound to the checkout
selected at its `PreToolUse`, through a durable per-invocation route:

- A route is keyed by the full attempt key (`session_id`, `agent_id`,
  `tool_use_id`) and stores the original checkout path and Git directory.
- Routes live at `<git-dir of CLAUDE_PROJECT_DIR>/sce/claude-mutation-scope-routes.json`
  under their own lock. The anchor only locates the table and never identifies
  a mutation target.
- `PreToolUse` persists the route after the recovery barrier and before attempt
  allocation and `Start`.
- `PostToolUse`, `PostToolUseFailure`, and `PermissionDenied` reach the original
  checkout through the route.
- `Stop`, `StopFailure`, `UserPromptSubmit`, `SubagentStop`, and `SessionEnd`
  act only on that session's known routes, grouped by recorded target.
- `WorktreeRemove` keeps its event-provided worktree identity and retires
  matching routes.
- A tracked `PreToolUse` is denied (fail closed) when `CLAUDE_PROJECT_DIR` is
  unset or does not resolve to a Git directory, or when the route cannot be
  persisted or conflicts with a live route.
- Terminal events never fall back to the event `cwd` for routed attempts. Only
  legacy attempts, whose adapter-state `routed` marker is absent or `false`, may
  still be found through the event `cwd`.
- A route is removed only after its attempt is gone from the original
  checkout's adapter state.

## Rationale

The route captures the only repository identity that is known to be correct:
the checkout at `PreToolUse`. Storing it outside the target checkout lets a
terminal event with a different `cwd` find it. `CLAUDE_PROJECT_DIR` was the only
Git-derived value that stayed stable across `cwd` changes, subagents, and
terminal events, and SCE hooks already depend on it. Persisting before `Start`
keeps admission fail closed: no tracked tool runs without a recoverable route.

## Alternatives considered

- **Store the start path only in checkout-local adapter state** — a terminal
  event with a different `cwd` cannot find that file.
- **Use `transcript_path` as the anchor** — it is a per-user Claude-internal
  location, not an SCE-owned path.
- **Scan repositories for a matching attempt** — unbounded, and it can match an
  unrelated checkout.
- **Fall back to the event `cwd` when no route is found** — operates on the wrong
  repository for routed attempts.

## Compatibility and risks

- Claude sessions without a `CLAUDE_PROJECT_DIR` that resolves to a Git
  directory can no longer run tracked mutation-capable tools. `sce setup`
  installs hooks at a repository root, so supported installs resolve it.
- Attempts created before routing are found only through the event `cwd`. If
  such an attempt's `cwd` changed before completion, the original defect
  remains for that attempt.
- Each tracked hook does one more locked file transaction on the anchor's Git
  directory.
- A `PreToolUse` reusing a `tool_use_id` between a terminal event's attempt check
  and its route removal could leave that attempt unrouted. Claude does not
  reuse a live `tool_use_id` concurrently.

## Guardrails

- The anchor is a lookup key only. The target is always the recorded checkout.
- A route is never evidence that a scope started.
- A live route is never retargeted.
- The routing and checkout-state locks are never held together or across a
  seam call.
- The mutation-trace protocol, generic ingress, worktree ID semantics,
  migrations, and `spec/mutation_cursor.qnt` are unchanged by this decision.

## Consequences

- A scope started in repository A closes in A when Claude later reports a
  nested repository or a directory outside every repository as `cwd`.
- Concurrent tools in different repositories, and main-agent and subagent
  tools sharing a `tool_use_id`, close only their own attempts.
- A failed abandon keeps the route, so a later cleanup event can retry it.

## Follow-up

- Live rerun and measured hook overhead in plan task T04.

## References

- Plan: [`claude-mutation-scope-stable-repository-routing`](../plans/claude-mutation-scope-stable-repository-routing.md)
- Task: `T01`, `T02`, `T03`
- Current-state context: [`claude-mutation-scope-integration.md`](../cli/claude-mutation-scope-integration.md)
- Current-state context: [`claude-mutation-scope-routes.md`](../cli/claude-mutation-scope-routes.md)
- Evidence: [`lifecycle_tests.rs`](../../cli/src/services/hooks/claude_mutation_scope/lifecycle_tests.rs)
- Related decision: [`Claude background Bash runs natively and untracked`](2026-10-02-claude-background-bash-untracked.md)
