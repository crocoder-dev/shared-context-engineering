# Checkout Identity Service

The checkout identity service lives in `cli/src/services/checkout/`.

It assigns a stable identity to a local Git checkout or linked Git worktree. The setup lifecycle now creates/reuses this identity and registers the checkout; hook integration, per-checkout Agent Trace database resolution, and doctor reporting are deferred to later tasks in `context/plans/agent-trace-checkout-identity.md`.

## Current code surface

- `cli/src/services/checkout/mod.rs`
  - `resolve_git_dir(repo_root)` runs `git rev-parse --git-dir` from the supplied repository root.
  - `read_checkout_id(git_dir)` reads `<git-dir>/sce/checkout-id` and validates non-empty UUID syntax.
  - `get_or_create_checkout_id(git_dir)` reuses an existing ID or writes a new UUIDv7 checkout ID to `<git-dir>/sce/checkout-id`.
  - `resolve_checkout_id_for_repo(repo_root)` combines Git directory resolution with get-or-create checkout ID behavior.
- `cli/src/services/checkout/registry.rs`
  - `CheckoutRecord` serializes `checkout_id`, `path`, `last_seen`, optional `remote_url`, and optional `database_path`.
  - `CheckoutRegistry` serializes the registry as `{ "checkouts": [...] }`.
  - Registry persistence uses `<state_root>/sce/checkout-registry.json` and atomic write-through-rename.
  - Registry operations include register, update last seen, list, and remove.

## Current integration state

The module is registered through `cli/src/services/mod.rs` and is called by `AgentTraceDbLifecycle::setup()` during `sce setup` after the setup command has derived a repository-root-scoped context.

During setup:

- `checkout::resolve_git_dir(repo_root)` resolves the checkout metadata directory from Git truth.
- `checkout::get_or_create_checkout_id(git_dir)` creates or reuses `<git-dir>/sce/checkout-id`.
- `checkout::registry::register_checkout(...)` writes or updates the central registry record with `database_path: null`.
- Setup output includes the checkout ID and states that the Agent Trace database will be created on first write.

The existing global Agent Trace database path remains the active runtime path until later plan tasks switch consumers to per-checkout database resolution.

## Testing boundary

No unit tests are currently included for this filesystem/Git-facing service. Filesystem, Git repository, and database behaviors should be covered in integration tests rather than unit tests per `context/patterns.md`.

See also: `context/cli/default-path-catalog.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`.
