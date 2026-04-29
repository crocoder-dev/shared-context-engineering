# Plan: cli-observability-di

## Change summary

Extract concrete observability types (`Logger`, `TelemetryRuntime`) into trait-based interfaces so that services consume `dyn Logger` and `dyn Telemetry` instead of concrete structs. Introduce an `AppContext` dependency-injection container that carries observability plus shared filesystem and git capability traits. This is a pure refactoring: no CLI behavior changes, no new commands.

## Success criteria

- `Logger` and `Telemetry` are traits in `services/observability/traits.rs`.
- Concrete `Logger` and `TelemetryRuntime` implement those traits unchanged.
- `AppContext` exists as a struct holding `Arc<dyn Logger>`, `Arc<dyn Telemetry>`, `Arc<dyn FsOps>`, and `Arc<dyn GitOps>`.
- All service `execute` methods accept `&AppContext` (or `&dyn Logger` as an intermediate step) instead of `&services::observability::Logger`.
- `nix flake check` passes with zero new warnings.
- Existing tests compile and pass without behavior changes.

## Constraints and non-goals

- **No behavioral changes** to log output, telemetry export, or CLI exit codes.
- Filesystem and git abstractions are limited to broad capability traits (`FsOps`, `GitOps`) and concrete production implementations; migrating service internals to consume them is covered in `cli-service-lifecycle`.
- **Do not** introduce async or change the `anyhow::Result` return types.
- **Do not** modify `cli_schema.rs`, `command_surface.rs`, or help text.
- Keep the `AppContext` minimal; resist adding unrelated concerns.

## Assumptions

- The existing `Logger` API surface (`info`, `debug`, `warn`, `error`, `log_classified_error`) is stable enough to freeze as a trait.
- `TelemetryRuntime::with_default_subscriber` signature is acceptable as the trait boundary.
- `context/decisions/cli-refactor-decisions.md` is the decision record for this plan; it chooses a capabilities-style `AppContext` with broad `FsOps` and `GitOps` traits.

## Task stack

- [ ] T01: Extract `Logger` trait and `NoopLogger` test impl (status:todo)
  - Task ID: T01
  - Goal: Move the `Logger` public API into a `Logger` trait in `services/observability/traits.rs` and provide a `NoopLogger` for tests.
  - Boundaries (in/out of scope): In - trait definition, `impl Logger for services::observability::Logger`, `NoopLogger`. Out - changing any call sites, telemetry abstraction.
  - Done when: `services::observability::traits::Logger` compiles, concrete `Logger` implements it, `NoopLogger` exists, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T02: Extract `Telemetry` trait (status:todo)
  - Task ID: T02
  - Goal: Move `TelemetryRuntime::with_default_subscriber` into a `Telemetry` trait and implement it for `TelemetryRuntime`.
  - Boundaries (in/out of scope): In - trait definition, `impl Telemetry for TelemetryRuntime`. Out - wiring traits into `AppContext` or call sites.
  - Done when: `services::observability::traits::Telemetry` compiles, concrete `TelemetryRuntime` implements it, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T03: Define filesystem and git capability traits (status:todo)
  - Task ID: T03
  - Goal: Add broad `FsOps` and `GitOps` traits plus production implementations that wrap `std::fs` and `git` process execution.
  - Boundaries (in/out of scope): In - trait definitions, production impls, basic test stubs. Out - migrating doctor/setup/hooks/config internals to consume these traits.
  - Done when: `FsOps` and `GitOps` compile, production implementations are available to `AppContext`, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T04: Introduce `AppContext` and wire into command dispatch (status:todo)
  - Task ID: T04
  - Goal: Create `AppContext` holding `Arc<dyn Logger>`, `Arc<dyn Telemetry>`, `Arc<dyn FsOps>`, and `Arc<dyn GitOps>`. Update `AppRuntime` to use it. Update `RuntimeCommand::execute` signature to accept `&AppContext`.
  - Boundaries (in/out of scope): In - `AppContext` struct, `AppRuntime` refactor, `RuntimeCommand` signature change, updating all command `execute` impls to accept `&AppContext`. Out - changing what commands do with the context beyond logger/telemetry access.
  - Done when: All `RuntimeCommand` impls compile with `&AppContext`, `app.rs` builds the context once and passes it through, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T05: Update tests for trait-based observability and capabilities (status:todo)
  - Task ID: T05
  - Goal: Replace any test code that directly constructs `Logger` or `TelemetryRuntime` with `NoopLogger` or minimal trait impls where appropriate. Ensure all existing tests still pass.
  - Boundaries (in/out of scope): In - test files under `cli/src/` that touch observability or `AppContext`; minimal test stubs for `FsOps`/`GitOps`. Out - broad lifecycle migration tests.
  - Done when: `cargo test` passes with zero failures, `cargo clippy` is clean.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test'` and `nix develop -c sh -c 'cd cli && cargo clippy'`

- [ ] T06: Validate and sync context (status:todo)
  - Task ID: T06
  - Goal: Run full repo validation, update `context/cli/cli-command-surface.md` or relevant context files to document the new trait boundaries.
  - Boundaries (in/out of scope): In - `nix flake check`, context sync for architecture/observability contracts. Out - modifying behavior docs.
  - Done when: `nix flake check` passes, context files reflect the new `Logger`/`Telemetry` trait boundaries.
  - Verification notes (commands or checks): `nix flake check`

## Open questions

None — assumptions recorded above.
