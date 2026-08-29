# Checkout Identity Service

The checkout identity service lives in `cli/src/services/checkout/`.

It assigns a stable identity to a local Git checkout or linked Git worktree. Checkout identity remains per clone/worktree, but active Agent Trace persistence is now repository-scoped through `agent_trace_storage`: checkout ID is diagnostic metadata and is not stored on Agent Trace rows. The former per-checkout Agent Trace DB opener and its path helper were removed by the `retire-legacy-agent-trace-db` plan; there is no checkout-scoped DB code path.

## Current code surface

- `cli/src/services/checkout/mod.rs`
  - `resolve_git_dir(repo_root)` runs `git rev-parse --git-dir` from the supplied repository root.
  - `read_checkout_id(git_dir)` reads `<git-dir>/sce/checkout-id` and validates non-empty UUID syntax. This lock-free fast path is unchanged for the already-created case.
  - `get_or_create_checkout_id(git_dir)` reuses an existing ID via `read_checkout_id`, or, on the slow path, acquires a dedicated `<git-dir>/sce/checkout-id.lock` (blocking `std::fs::File::lock()`, no timeout), re-checks for an existing ID under the lock, and otherwise generates a new UUIDv7 and persists it crash-safely: written to a unique `checkout-id.tmp-<id>` file (`OpenOptions::create_new(true)`), synced with `File::sync_data()`, then moved into the canonical `checkout-id` path with an atomic `std::fs::rename` (plus a best-effort `#[cfg(unix)]` parent-directory sync). This guarantees every concurrent caller — the coordinator, `agent_trace_storage`, or any other caller of this function — converges on exactly one checkout ID, and that the canonical path is never observable as partially written, even across a process crash mid-write. The lock is scoped only to identity creation; it is distinct from the separate mutation-cursor runtime lock at `<git-dir>/sce/mutation-cursor.lock` (a different, larger critical section owned by an in-progress runtime coordinator, not yet part of this service). Orphaned `checkout-id.tmp-*` files left by an interrupted write are harmless and are not cleaned up by this function.

## Current integration state

The module is registered through `cli/src/services/mod.rs` and is consumed by `agent_trace_storage` during repository-scoped storage resolution.

During setup and hook runtime:

- Config resolution provides `agent_trace.repository_id` and `agent_trace.repository_remote` (default `origin`).
- `agent_trace_storage::resolve_agent_trace_storage(...)` resolves repository identity, calls `checkout::resolve_git_dir(repo_root)`, and creates/reuses `<git-dir>/sce/checkout-id` for diagnostics.
- The active DB path is `<state_root>/sce/repos/<repository-id>/agent-trace.db`.
- `RepositoryAgentTraceDb` opens through the repository fast-path-then-migrate flow and validates `repository_metadata.repository_id`.

`sce doctor` still displays checkout identity where available. The former Agent Trace list/status/status-all/shell UX is no longer available; `sce sync` operates on the current repository-scoped DB. Any pre-migration `<state_root>/sce/agent-trace-*.db` checkout-scoped files left on disk are never touched by SCE and are no longer inspectable through the CLI.

## Testing boundary

`get_or_create_checkout_id` and its identity-creation lock have an inline `#[cfg(test)]` module in `cli/src/services/checkout/mod.rs` covering concurrent first-time convergence on one ID, the fast path never touching the lock, a completed rename leaving a complete ID, a simulated crash before rename leaving the canonical path absent, and an orphaned temp file not blocking a later call — each test uses a unique temp directory rather than a shared fixture, following the same filesystem-touching inline-unit-test precedent already used in `cli/src/services/mutation_trace/store.rs`. `resolve_git_dir` and Git-subprocess behavior remain uncovered by unit tests; that and other database behaviors should still be covered in integration tests rather than unit tests per `context/patterns.md`.

See also: `context/cli/agent-trace-storage.md`, `context/cli/default-path-catalog.md`, `context/sce/agent-trace-db.md`, `context/sce/agent-trace-hooks-command-routing.md`.
