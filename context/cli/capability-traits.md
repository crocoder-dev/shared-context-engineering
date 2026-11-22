# CLI Capability Traits

The CLI exposes broad capability traits in `cli/src/services/capabilities.rs` and wires their production implementations through the borrowed, compile-time-typed `AppContext` view in `cli/src/app.rs`.

## Current contract

- `FsOps: Send + Sync` defines filesystem `exists`, `create_dir_all`, `read_to_string`, and `write` operations, implemented by `StdFsOps`. Setup context baseline creation and repository-local config bootstrap/persistence now use this borrowed capability; additional filesystem paths remain to be migrated.
- `StdFsOps` is the production `FsOps` implementation.
- `GitOps: Send + Sync` defines broad git process capabilities: `run_command` and `is_available`.
- `ProcessGitOps` is the production git implementation and shells out to `git`, returning stdout as UTF-8 text with command/directory context on failure.
- `AppRuntime` owns concrete `StdFsOps` and `ProcessGitOps` production dependencies alongside concrete observability dependencies; these dependencies are borrowed by `AppContext` rather than stored behind type-erased runtime containers.
- `AppContext` is generic over logger, telemetry, filesystem, and git capability implementations and borrows them from the runtime; it is passed through directly awaited static `RuntimeCommand::execute` (enum defined in `cli/src/services/command_registry.rs`) behind narrow accessor bounds. Because it borrows from `AppRuntime`, `AppContext` is command-scoped: borrowing across directly awaited execution is safe while the owner lives, but detached work cannot retain that borrowed view. No new `Send`, `Sync`, or `'static` command bounds are required. Its inherent `logger()` helper returns the concrete generic reference (`&L`), and `HasLogger`, `HasTelemetry`, and `HasGit` use associated capability types that return `&Self::{Capability}`, and `HasFs` exposes `fn fs(&self) -> &Self::Fs`, instead of object-erased `&dyn ...` values. This is the current static-DI boundary: command and service call sites express narrow capability requirements while preserving concrete borrowed dependency types. `ContextWithRepoRoot` derives a repository-scoped context while preserving the borrowed runtime capability objects.
- Current command execution bounds are capability-oriented: central dispatch requires logger access plus repo-root scoping, context-free commands accept any context, hooks require logger access, and setup/doctor require repo-root scoping.
- Test-only `UnimplementedFsOps` and `UnimplementedGitOps` stubs are available under `capabilities::test_stubs` for tests that need to satisfy trait bounds before providing focused fakes.

## Boundary

Filesystem injection is incremental: `sce setup` context baseline and repository-local config paths use `FsOps` through `AppContext`, but setup asset installation, hooks, doctor, config resolution, and other direct filesystem operations still need migration. This is not yet an enforced all-I/O boundary. Keep OS-level access control separate from dependency injection.

See also: [overview](../overview.md), [architecture](../architecture.md), [context map](../context-map.md)
