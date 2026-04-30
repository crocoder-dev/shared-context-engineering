# CLI Capability Traits

The CLI exposes broad dependency-injection capability traits in `cli/src/services/capabilities.rs` and wires their production implementations into `AppContext` in `cli/src/app.rs`.

## Current contract

- `FsOps: Send + Sync` defines broad filesystem operations: `read_file`, `write_file`, `metadata`, and `exists`.
- `StdFsOps` is the production filesystem implementation and wraps `std::fs` / `Path::exists` with `anyhow` context on fallible operations.
- `GitOps: Send + Sync` defines broad git process capabilities: `run_command`, `resolve_repository_root`, `resolve_hooks_directory`, and `is_available`.
- `ProcessGitOps` is the production git implementation and shells out to `git`, returning stdout as UTF-8 text with command/directory context on failure.
- `resolve_repository_root` uses `git rev-parse --show-toplevel`.
- `resolve_hooks_directory` uses `git rev-parse --git-path hooks` and resolves relative hook paths against the provided repository root.
- `AppContext` holds `Arc<dyn FsOps>` and `Arc<dyn GitOps>` alongside observability dependencies and is passed to `RuntimeCommand::execute` (trait defined in `cli/src/services/command_registry.rs`). It exposes narrow `fs()` / `git()` accessors and `with_repo_root(...)` for deriving a repository-scoped context while preserving the runtime capability objects.
- Test-only `UnimplementedFsOps` and `UnimplementedGitOps` stubs are available under `capabilities::test_stubs` for tests that need to satisfy trait bounds before providing focused fakes.

## Boundary

Existing service internals do not consume these traits yet. Doctor/setup/hooks/config internals still use their current local filesystem and git seams until later lifecycle/AppContext tasks migrate call sites.

See also: [overview](../overview.md), [architecture](../architecture.md), [context map](../context-map.md)
