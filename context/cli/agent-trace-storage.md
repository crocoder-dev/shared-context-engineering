# Repository-scoped Agent Trace storage resolver

Module at `cli/src/services/agent_trace_storage/` (T04 of the `repository-scoped-agent-trace-db` plan) that resolves the active Agent Trace database for a Git repository checkout under the target invariant: one logical Git repository = one Agent Trace database at `<state_root>/sce/repos/<repository-id>/agent-trace.db`. Clones and linked worktrees of the same logical repository resolve to the same database path while keeping distinct checkout IDs.

## Public API

- `AgentTraceStorageContext { repository_root, explicit_repository_id, repository_remote }` — borrowed inputs mirroring the `agent_trace.repository_id` / `agent_trace.repository_remote` config keys; callers pass already-resolved configuration values (the module does not read config itself; T08 wires config/hooks/lifecycle call sites).
- `ResolvedAgentTraceStorage { repository_identity, checkout_id, db_path, db }` — resolved repository identity (`ResolvedRepositoryIdentity` including source provenance), the checkout ID for diagnostics (never persisted on Agent Trace rows), the repository-scoped DB path, and the open `RepositoryAgentTraceDb`.
- `resolve_agent_trace_storage(context)` — production entrypoint using the canonical state root from the default-path catalog.
- `resolve_agent_trace_storage_at_state_root(context, state_root)` — resolution core against an explicit state root; used by tests to exercise the full path without touching the real user state directory.

## Resolution flow

1. Repository identity via `repository_identity::resolve` precedence (explicit config ID → configured remote URL, default `origin`); resolution errors carry `.sce/config.json` guidance and never echo URLs. A failed identity resolution creates no state directories.
2. Checkout identity reuse via `checkout::resolve_git_dir` + `get_or_create_checkout_id` (`<git-dir>/sce/checkout-id`).
3. DB path from `default_paths::agent_trace_db_path_for_repository{,_at}`, which rejects empty or path-unsafe repository IDs (separators, `.`, `..`).
4. DB open through `agent_trace_db::repository::RepositoryAgentTraceDb` with the same fast-path-then-migrate pattern as the checkout resolver: `open_without_migrations_at` + `ensure_schema_ready_for_hooks()` + `verify_or_initialize_repository_metadata(repository_id)`, falling back to migration-running `new_at` plus the same metadata validation. Directory creation rides on `TursoDb` parent-dir `create_dir_all`; both directory creation and schema initialization are idempotent, so concurrent first-time resolution is safe.

## Legacy boundary

The resolver never selects, creates, or touches legacy checkout-scoped `<state_root>/sce/agent-trace-<checkout-id>.db` files or the legacy global `<state_root>/sce/agent-trace.db`; tests assert neither appears after resolution. The existing checkout resolver `checkout::resolve_or_create_agent_trace_db_for_checkout` remains the active runtime path until T08 replaces its call sites.

## Status

Registered in `cli/src/services/mod.rs` behind `#[allow(dead_code)]` until T08 consumes it. T05 changes the resolved DB handle to the repository-scoped adapter and validates the stored `repository_metadata.repository_id` before returning storage. Covered by in-module tests: repository separation, SSH/HTTPS clone consolidation, linked-worktree consolidation, explicit-ID override, idempotent re-resolution, missing-identity guidance, and path-segment validation (`nix build .#checks.<system>.cli-tests`).

See also: [repository-identity.md](repository-identity.md), [checkout-identity.md](checkout-identity.md), [default-path-catalog.md](default-path-catalog.md), [../sce/agent-trace-db.md](../sce/agent-trace-db.md)
