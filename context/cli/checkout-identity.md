# Checkout Identity Service

The checkout identity service lives in `cli/src/services/checkout/`.

It assigns a stable identity to a local Git checkout or linked Git worktree. The setup lifecycle creates/reuses this identity, initializes the per-checkout Agent Trace database, and records the database path. Agent Trace hook runtime resolves persistence through this identity and still lazily initializes or upgrades a per-checkout database when setup has not run or schema metadata is incomplete.

## Registry resilience

The registry includes two resilience safeguards against concurrent access and corruption:

- **Empty-file recovery:** `read_registry()` treats empty or whitespace-only registry files as corrupt. It removes the file (best-effort), prints a `[WARN]` to stderr, and returns `CheckoutRegistry::default()`. This prevents `serde_json` parse errors when a previous write was interrupted, leaving a zero-byte file.
- **PID-unique temp filenames:** `write_registry()` appends the process ID to the temp filename (`checkout-registry.json.tmp.{pid}`) so concurrent `sce hooks` processes never collide on the same temp file. After a successful atomic `rename(2)`, stale `.tmp.*` files left by crashed processes are cleaned up (best-effort, matching `checkout-registry.json.tmp.*` prefix, errors ignored).

These changes are implemented in `cli/src/services/checkout/registry.rs` in the `read_registry()` and `write_registry()` functions. No caller changes were needed.

## Current code surface

- `cli/src/services/checkout/mod.rs`
  - `resolve_git_dir(repo_root)` runs `git rev-parse --git-dir` from the supplied repository root.
  - `read_checkout_id(git_dir)` reads `<git-dir>/sce/checkout-id` and validates non-empty UUID syntax.
  - `get_or_create_checkout_id(git_dir)` reuses an existing ID or writes a new UUIDv7 checkout ID to `<git-dir>/sce/checkout-id`.
  - `resolve_checkout_id_for_repo(repo_root)` combines Git directory resolution with get-or-create checkout ID behavior.
  - `resolve_or_create_agent_trace_db_for_current_checkout()` resolves from `std::env::current_dir()` and returns `(AgentTraceDb, checkout_id)`.
  - `resolve_or_create_agent_trace_db_for_checkout(repo_root)` gets or creates checkout identity, registers it, resolves `<state_root>/sce/agent-trace-{checkout_id}.db`, fast-opens an existing ready DB, and falls back to migration-running initialization when the DB is absent or schema metadata is incomplete.
- `cli/src/services/checkout/registry.rs`
  - `CheckoutRecord` serializes `checkout_id`, `path`, `last_seen`, optional `remote_url`, and optional `database_path`.
  - `CheckoutRegistry` serializes the registry as `{ "checkouts": [...] }`.
  - Registry persistence uses `<state_root>/sce/checkout-registry.json` and atomic write-through-rename.
  - Registry operations include register, update last seen, list, and remove.

## Current integration state

The module is registered through `cli/src/services/mod.rs` and is called by `AgentTraceDbLifecycle::setup()` during `sce setup` after the setup command has derived a repository-root-scoped context. Hook runtime also calls it before Agent Trace DB reads/writes.

During setup:

- `checkout::resolve_git_dir(repo_root)` resolves the checkout metadata directory from Git truth.
- `checkout::get_or_create_checkout_id(git_dir)` creates or reuses `<git-dir>/sce/checkout-id`.
- `checkout::registry::register_checkout(...)` first writes or updates the central registry record for the checkout identity.
- `default_paths::agent_trace_db_path_for_checkout(checkout_id)` computes `<state_root>/sce/agent-trace-{checkout_id}.db`.
- `AgentTraceDb::open_at(path)` opens or creates the per-checkout DB and applies all embedded migrations before setup completes.
- `checkout::registry::register_checkout(...)` updates the central registry record with `database_path` after DB initialization succeeds.
- Setup output includes the checkout ID and initialized Agent Trace database path.

During hook runtime:

- `checkout::resolve_git_dir(repo_root)` and `checkout::get_or_create_checkout_id(git_dir)` make hooks self-sufficient when `sce setup` has not run yet.
- `default_paths::agent_trace_db_path_for_checkout(checkout_id)` computes `<state_root>/sce/agent-trace-{checkout_id}.db`.
- `AgentTraceDb::open_for_hooks_without_migrations_at(path)` is tried first; `ensure_schema_ready_for_hooks()` decides whether the schema is current.
- Missing or incomplete schema falls back to `AgentTraceDb::open_at(path)`, which runs migrations through the shared Turso adapter.
- Successful DB resolution updates the registry record with `database_path`.

The global `agent-trace.db` path remains only as a lifecycle fallback when no checkout context or checkout ID is available. `sce doctor` displays the current checkout ID and per-checkout Agent Trace DB status when a checkout ID exists, and `sce doctor dbs` lists registry records sorted by `last_seen` descending.

## Testing boundary

No unit tests are currently included for this filesystem/Git-facing service. Filesystem, Git repository, and database behaviors should be covered in integration tests rather than unit tests per `context/patterns.md`.

See also: `context/cli/default-path-catalog.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`.
