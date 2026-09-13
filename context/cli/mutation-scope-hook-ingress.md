# Mutation-scope hook ingress: the harness-neutral transport seam

`sce hooks mutation-scope` is the one generic CLI ingress that drives the
mutation-scope runtime (`coordinate()` / `abandon_scope()`, documented in
[`mutation-scope-runtime.md`](mutation-scope-runtime.md)). It reads a single
normalized JSON lifecycle object from STDIN, strictly parses and validates it,
translates it into one `RuntimeBoundary` or one `abandon_scope()` call, and
invokes the existing runtime with a lazy DB provider.

Built by the `mutation-scope-hook-ingress` plan
(`context/plans/mutation-scope-hook-ingress.md`). It lives in
`cli/src/services/hooks/mutation_scope.rs` and is the transport/normalization
seam every future Claude Code, Codex, OpenCode, and Pi adapter will target. It
contains **no** concrete harness mapping and **no** lifecycle-event translation —
see [Generic ingress vs harness adapter](#generic-ingress-vs-harness-adapter).

## Command routing

`sce hooks mutation-scope` routes through the normal CLI/hook command stack and
is hidden with the rest of the `hooks` surface (`HOOKS_SHOW_IN_TOP_LEVEL_HELP`
is already `false`; no new visibility flag):

```
cli_schema::HooksSubcommand::MutationScope
  -> parse::command_runtime::convert_hooks_subcommand_request
  -> services::hooks::HookSubcommand::MutationScope
  -> run_hooks_subcommand_in_repo
  -> mutation_scope::run_mutation_scope_subcommand(repository_root, logger)
```

`hook_runtime_invocation_name` reports `"mutation-scope runtime invocation"`. The
dispatch arm is unwrapped like `PreCommit` — its `Result` is *not* wrapped in an
`Ok(...)` fail-open shim the way `diff-trace` / `conversation-trace` / `codex` /
`claude-model-state` are (see [Non-fail-open error semantics](#non-fail-open-error-semantics)).

The ingress reads the invoking checkout through the same `repository_root`
(`std::env::current_dir()`) that `run_hooks_subcommand` resolves for every hook;
the runtime derives `git_dir` and `WorktreeId` from it. STDIN is read once
through the shared `super::read_hook_stdin()`.

## The normalized JSON contract

One JSON object on STDIN. A required `operation` string selects the shape;
exactly five operations are supported.

| `operation` | Other accepted keys | Maps to |
| --- | --- | --- |
| `start` | `scope_id`, `event_id`, `actor_kind` (all required, non-blank) | `RuntimeBoundary::Start` |
| `advance` | `scope_id`, `event_id`, `actor_kind` (all required, non-blank) | `RuntimeBoundary::Advance` |
| `close` | `scope_id`, `event_id`, `actor_kind` (all required, non-blank) | `RuntimeBoundary::Close` |
| `flush` | *(none)* | `RuntimeBoundary::Flush` |
| `abandon` | `scope_id` only (required, non-blank) | `abandon_scope()` |

`actor_kind` is one of exactly `claude_code`, `codex`, `opencode`, `pi`, mapped
to `ActorKind::ClaudeCode` / `Codex` / `OpenCode` / `Pi`.

The parser (`parse_mutation_scope_payload`) is strict and rejects, each with a
`Invalid mutation-scope payload from STDIN: <detail>.` diagnostic:

- an empty or whitespace-only payload;
- malformed JSON, or JSON that is not an object;
- a missing `operation`, a non-string `operation`, or an unknown operation;
- an unknown `actor_kind`;
- a missing, non-string, empty, or blank `scope_id` / `event_id`;
- any unexpected field for the operation (each operation validates its exact
  allowed key set via `reject_unexpected_keys`);
- **any `worktree_id` key**, with a dedicated diagnostic
  (`field 'worktree_id' is not accepted; worktree identity is derived from the
  invoking checkout`);
- `flush` carrying any `scope_id` / `event_id` / `actor_kind` field;
- `abandon` carrying anything but `scope_id`.

Unexpected fields and `worktree_id` are validated explicitly against each
operation's allowed key set. The hook transport remains local to
`mutation_scope.rs`; no serde representation is added to the mutation-domain
types.

## Operation mapping

`start` / `advance` / `close` build the matching `RuntimeBoundary` variant,
forwarding `ScopeId(scope_id)`, `EventId(event_id)`, and the mapped `ActorKind`
**verbatim** — no trimming, prefixing, hashing, normalization, UUID generation,
or timestamping. A `scope_id` of `"  scope-A  "` reaches the runtime as
`ScopeId("  scope-A  ")` unchanged.

`flush` builds `RuntimeBoundary::Flush`, which carries no scope, event, or actor
identity. It drives the runtime's real observed-flush behavior: against a
healthy, non-rebaseline worktree, an unscoped edit followed by `flush` advances
`cursor_tree` to the edited Git tree, advances `revision` by one, writes exactly
one `mutation_trace_events` row with `attribution = IneligibleUnscoped`, and
invents no `mutation_trace_scopes` or `mutation_trace_processed_events` row.

`abandon` calls `abandon_scope(repository_root, &ScopeId(scope_id), provider)`
directly — see [Abandonment ownership](#abandonment-ownership).

## Identity ownership

The ingress owns nothing durable.

- **The external adapter owns** `scope_id`, `event_id`, and `actor_kind`.
- **SCE owns** `worktree_id`, Git tree identities, mutation revisions, and
  attempt IDs.

### No `worktree_id`

The payload never accepts `worktree_id` (any such key is a hard rejection).
Worktree identity is derived exclusively by the mutation runtime from the
invoking checkout. The production ingress does not accept, read, derive, or
construct a `WorktreeId`, and never passes one into `coordinate()` /
`abandon_scope()`. (`#[cfg(test)]` code constructs `WorktreeId` values only to
fabricate injected `CoordinateOutcome` / `AbandonScopeOutcome` runtime results.)

### `ScopeId` / `EventId` are translated, never generated

`scope_id` and `event_id` become `ScopeId(..)` / `EventId(..)` with no
transformation, because **`EventId` equality is the runtime's existing
replay/idempotency key**. A replayed `(ScopeId, EventId)` boundary is fully
idempotent at the runtime: replaying `advance(A, e2)` leaves `revision`, the
`mutation_trace_events` count, and the `mutation_trace_processed_events` count
unchanged, with exactly one `(A, e2)` processed key. If the ingress regenerated,
prefixed, or hashed `EventId`, that idempotency would break.

The scope's durable `(worktree_id, actor_kind)` identity is registered by the
runtime on every scope-carrying boundary, not only `Start`; a mismatched
`actor_kind` for an existing scope reaches `CoordinateError::ScopeIdentityConflict`
and commits no second boundary.

## Non-fail-open error semantics

Unlike `diff-trace` / `conversation-trace`, a lost mutation-scope lifecycle
boundary can change which scope stays live and therefore alter attribution, so a
valid boundary must **never** be silently discarded. There is no
`"failed open" / exit 0` branch for a dropped or malformed boundary, and no
branch that returns success without driving the runtime for a valid payload.

Results are classified by **durable completion**, matching what the runtime
already models:

- **A malformed payload** → `Err` → `CliError` / non-zero exit.
- **An ordinary pre-completion runtime error** — any `CoordinateError` /
  `AbandonScopeError` variant other than the two below — → `Err` → `CliError` /
  non-zero exit. The message is
  `mutation-scope runtime boundary failed before durable completion: <error>` (or
  `... abandonment failed before durable completion: ...`).
- **`CoordinateError::MarkerClearAfterCommit { committed, source }`** and
  **`AbandonScopeError::MarkerClearAfterCompletion { completed, source }`** →
  **durable success**. These mean the durable mutation transition **already
  succeeded** and only the trailing external-taint marker cleanup failed. The
  ingress treats the carried outcome as the result: it logs the cleanup failure
  diagnostically (`sce.hooks.mutation_scope.marker_clear_after_durable_completion`,
  `warn`, with an `entrypoint` = `coordinate` / `abandon_scope` field), emits
  empty stdout, and exits zero. It does **not** re-run or retry the transition —
  the runtime seam is invoked exactly once. The marker stays armed, so the next
  runtime invocation recovers conservatively per existing runtime semantics.

These are the exact carried-outcome variants on the base
(`origin/mutation-trace-agent-attribution`):
`CoordinateError::MarkerClearAfterCommit { source, committed: Box<CoordinateOutcome> }`
and
`AbandonScopeError::MarkerClearAfterCompletion { source, completed: Box<AbandonScopeOutcome> }`.

The ingress does not interpret otherwise-valid runtime outcomes
(`accepted = false`, `observes = false`, duplicate processed event, no tree
change, `Abandoned` / `AlreadyTerminal` / `RecoveryRequired`) — those stay
existing runtime semantics — beyond this marker-clear-after-durable-completion
classification.

## Lazy DB provider

DB acquisition must stay **inside** the runtime's protected-worktree ordering
(`WorktreeLock` → external-taint fence → `WorktreeId` → DB). The ingress
therefore passes a `FnOnce` provider closure to `coordinate()` /
`abandon_scope()`, never an already-open handle. The closure reuses the shared
`open_agent_trace_db_for_hook_runtime` resolver (with context message
`"Failed to open Agent Trace DB for mutation-scope runtime."`) — no second
DB-opening implementation. The resolver is invoked only from within that closure,
so it can only run after the runtime has taken the lock and armed the fence.

## Empty-stdout contract

A successful mutation-scope hook execution produces **empty stdout** (zero
bytes). The ingress serializes no `CoordinateOutcome`, `AbandonScopeOutcome`,
`MutationEvent`, revision, worktree ID, or scope state. The two carried-outcome
success variants above also produce empty stdout.

## Abandonment ownership

`abandon` calls `abandon_scope()` directly and never acquires `Close` / `Flush`
snapshot semantics. Abandonment captures no Git snapshot and commits no mutation
boundary; it only transitions already-durable state (status → `Abandoned`,
`revision` + 1, `needs_rebaseline = true`). In
`start(A, e1) → unobserved filesystem edit → abandon(A)`, `cursor_tree` stays the
pre-edit tree — it does **not** become the edited Git tree — and no
`mutation_trace_events` row for the edit and no `mutation_trace_processed_events`
row from the abandon are written.

Turning `abandon` into a `RuntimeBoundary` variant, or capturing a Git snapshot
on behalf of abandonment, is a deliberate non-goal.

## Durable storage boundary

The ingress reaches durable storage only through the existing mutation runtime,
writing `mutation_trace_*` rows only. No production ingress path writes
`diff_traces`, `post_commit_patch_intersections`, or `agent_traces`, and none
touches `spec/mutation_cursor.qnt`, `protocol.rs`, the mutation-trace SQL schema,
migrations, or #259 attribution behavior. The pure mutation-domain types gain no
serde derives for this command; the hook transport enum
(`MutationScopePayload`) is local to `mutation_scope.rs`.

## Generic ingress vs harness adapter

A generic SCE ingress existing is **not** concrete harness integration existing.
Out of scope for this seam, and left as future work:

- any concrete harness mapping — Claude Code hooks, Codex hook mapping, OpenCode
  plugin, Pi extension;
- `SubagentStart` / `SubagentStop` / `PostToolUse` / tool-call translation;
- `session → ScopeId` or `tool-call → EventId` derivation;
- PID tracking, process supervisors, staleness detection, automatic scope
  abandonment;
- harness settings generation or `sce setup` integration for the new hook.

Each future adapter still owns its own `ScopeId` / `EventId` / `actor_kind`
derivation and its own stale-process detection, and targets this ingress as its
transport. See [`mutation-scope-runtime.md`](mutation-scope-runtime.md) for the
lifecycle obligations every such adapter must uphold.

## Related context

- [Mutation-scope runtime: the harness-adapter contract](mutation-scope-runtime.md)
- [Mutation-trace runtime coordinator](mutation-trace-runtime-coordinator.md)
- [Mutation-trace scope abandonment](mutation-trace-scope-abandonment.md)
- [Mutation-trace protected worktree](mutation-trace-protected-worktree.md)
- [Agent Trace hooks command routing](../sce/agent-trace-hooks-command-routing.md)
