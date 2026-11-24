# Claude mutation-scope route store

The Claude mutation-scope adapter has a durable route store in
`cli/src/services/hooks/claude_mutation_scope/routes.rs`. A route records the
checkout where one tracked Claude tool invocation started. Terminal hook events
can use the route to reach that checkout after the hook `cwd` has changed, for
example after `cd` into a nested Git repository.

Current state: the store exists and is tested, but `lifecycle.rs` does not call
it yet. Until the lifecycle uses it, the rule in
[Raw cwd is authoritative](claude-mutation-scope-integration.md#raw-cwd-is-authoritative)
still describes runtime behavior. The
`claude-mutation-scope-stable-repository-routing` plan owns the integration.

## Route anchor

The route anchor is the Git directory of `CLAUDE_PROJECT_DIR`, resolved with
`git rev-parse --absolute-git-dir`. A live Claude Code probe showed that this
value stays the same across `cwd` changes, subagent events, and terminal events.
The event `cwd` and its Git directory do not stay the same.

The anchor only locates the route table. It never identifies a mutation target.
`resolve_route_anchor` returns a typed `RouteAnchorError`:

- `ProjectDirUnset`: `CLAUDE_PROJECT_DIR` is missing or blank.
- `Unresolvable`: `CLAUDE_PROJECT_DIR` does not resolve to a Git directory.

The store never falls back to the event `cwd` when the anchor is unavailable.

## Route table

The route table is `<anchor-git-dir>/sce/claude-mutation-scope-routes.json`. It
is separate from the checkout-local `claude-mutation-scope-state.json`. Its shape
is a versioned `{version, routes[]}`. Each route has the full attempt key
(`session_id`, optional `agent_id`, `tool_use_id`) and a target
(`checkout_path`, `git_dir`). The route table is adapter bookkeeping. It is not
attribution evidence, and a route never proves that a scope started.

Writes hold `<anchor-git-dir>/sce/claude-mutation-scope-routes.lock` and use the
adapter-state durability pattern: temp file, `sync_data`, atomic rename, and a
best-effort directory sync on Unix. A malformed or wrong-version table is
rejected and never overwritten.

## Operations

| Operation | Behavior |
| --- | --- |
| `persist_route` | `Inserted` for a new key. `AlreadyRouted` for a replay with the same target. `Conflict { existing }` for a different target; the live route is not changed. |
| `load_route` | Returns the target for the exact attempt key, or none. |
| `routes_for_session` | Returns every route of one `session_id`. |
| `remove_route` | `Removed` only while the route still has the expected target. `TargetMismatch` leaves a route for a reused `tool_use_id` intact. `Absent` makes a duplicate removal safe. |

Main-agent and subagent keys with the same `tool_use_id` are distinct routes.
The route lock and the checkout-state lock are separate files.

## Related context

- [Claude mutation-scope integration](claude-mutation-scope-integration.md)
