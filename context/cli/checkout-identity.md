# Checkout Identity Service

The checkout identity service lives in `cli/src/services/checkout/`.

It assigns a stable identity to a local Git checkout or linked Git worktree. Checkout identity remains per clone/worktree, but active Agent Trace persistence is now repository-scoped through `agent_trace_storage`: checkout ID is diagnostic metadata and is not stored on Agent Trace rows. The former per-checkout Agent Trace DB opener and its path helper were removed by the `retire-legacy-agent-trace-db` plan; there is no checkout-scoped DB code path.

## Current code surface

- `cli/src/services/checkout/mod.rs`
  - `resolve_git_dir(repo_root)` runs `git rev-parse --git-dir` from the supplied repository root.
  - `read_checkout_id(git_dir)` reads `<git-dir>/sce/checkout-id` and validates non-empty UUID syntax.
  - `get_or_create_checkout_id(git_dir)` reuses an existing ID or writes a new UUIDv7 checkout ID to `<git-dir>/sce/checkout-id`.

## Current integration state

The module is registered through `cli/src/services/mod.rs` and is consumed by `agent_trace_storage` during repository-scoped storage resolution.

During setup and hook runtime:

- Config resolution provides `agent_trace.repository_id` and `agent_trace.repository_remote` (default `origin`).
- `agent_trace_storage::resolve_agent_trace_storage(...)` resolves repository identity, calls `checkout::resolve_git_dir(repo_root)`, and creates/reuses `<git-dir>/sce/checkout-id` for diagnostics.
- The active DB path is `<state_root>/sce/repos/<repository-id>/agent-trace.db`.
- `RepositoryAgentTraceDb` opens through the repository fast-path-then-migrate flow and validates `repository_metadata.repository_id`.

`sce doctor` still displays checkout identity where available. `sce trace` list/status/status-all/shell UX operates only on repository-scoped DBs. Any pre-migration `<state_root>/sce/agent-trace-*.db` checkout-scoped files left on disk are never touched by SCE and are no longer inspectable through the CLI.

## Testing boundary

No unit tests are currently included for this filesystem/Git-facing service. Filesystem, Git repository, and database behaviors should be covered in integration tests rather than unit tests per `context/patterns.md`.

See also: `context/cli/agent-trace-storage.md`, `context/cli/default-path-catalog.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`.
