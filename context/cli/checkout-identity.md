# Checkout Identity Service

The checkout identity service lives in `cli/src/services/checkout/`.

It assigns a stable identity to a local Git checkout or linked Git worktree. The setup lifecycle creates/reuses this identity and initializes the per-checkout Agent Trace database. Agent Trace hook runtime resolves persistence through this identity and still lazily initializes or upgrades a per-checkout database when setup has not run or schema metadata is incomplete.

## Current code surface

- `cli/src/services/checkout/mod.rs`
  - `resolve_git_dir(repo_root)` runs `git rev-parse --git-dir` from the supplied repository root.
  - `read_checkout_id(git_dir)` reads `<git-dir>/sce/checkout-id` and validates non-empty UUID syntax.
  - `get_or_create_checkout_id(git_dir)` reuses an existing ID or writes a new UUIDv7 checkout ID to `<git-dir>/sce/checkout-id`.
  - `resolve_or_create_agent_trace_db_for_checkout(repo_root)` gets or creates checkout identity, resolves `<state_root>/sce/agent-trace-{checkout_id}.db`, fast-opens an existing ready DB, and falls back to migration-running initialization when the DB is absent or schema metadata is incomplete.

## Current integration state

The module is registered through `cli/src/services/mod.rs` and is called by `AgentTraceDbLifecycle::setup()` during `sce setup` after the setup command has derived a repository-root-scoped context. Hook runtime also calls it before Agent Trace DB reads/writes.

During setup:

- `checkout::resolve_git_dir(repo_root)` resolves the checkout metadata directory from Git truth.
- `checkout::get_or_create_checkout_id(git_dir)` creates or reuses `<git-dir>/sce/checkout-id`.
- `default_paths::agent_trace_db_path_for_checkout(checkout_id)` computes `<state_root>/sce/agent-trace-{checkout_id}.db`.
- `AgentTraceDb::open_at(path)` opens or creates the per-checkout DB and applies all embedded migrations before setup completes.
- Setup output includes the checkout ID and initialized Agent Trace database path.

During hook runtime:

- `checkout::resolve_git_dir(repo_root)` and `checkout::get_or_create_checkout_id(git_dir)` make hooks self-sufficient when `sce setup` has not run yet.
- `default_paths::agent_trace_db_path_for_checkout(checkout_id)` computes `<state_root>/sce/agent-trace-{checkout_id}.db`.
- `AgentTraceDb::open_for_hooks_without_migrations_at(path)` is tried first; `ensure_schema_ready_for_hooks()` decides whether the schema is current.
- Missing or incomplete schema falls back to `AgentTraceDb::open_at(path)`, which runs migrations through the shared Turso adapter.

The global `agent-trace.db` path remains only as a lifecycle fallback when no checkout context or checkout ID is available. `sce doctor` displays the current checkout ID and per-checkout Agent Trace DB status when a checkout ID exists, and `sce trace db list` discovers checkouts by scanning `<state_root>/sce/agent-trace-*.db` files on disk, sorted by mtime descending.

## Testing boundary

No unit tests are currently included for this filesystem/Git-facing service. Filesystem, Git repository, and database behaviors should be covered in integration tests rather than unit tests per `context/patterns.md`.

See also: `context/cli/default-path-catalog.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`.
