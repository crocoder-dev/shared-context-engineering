# Compile-Time Dependency Injection Migration

## Change summary

Migrate the Rust CLI runtime from runtime dependency injection through trait objects to compile-time dependency injection through concrete generic types and enum dispatch.

The target migration covers the currently documented runtime DI seams:

- `AppContext` dependencies currently stored as `Arc<dyn Logger>`, `Arc<dyn Telemetry>`, `Arc<dyn FsOps>`, and `Arc<dyn GitOps>`.
- Runtime command dispatch currently stored as `Box<dyn RuntimeCommand>` / `RuntimeCommandHandle`.
- Lifecycle provider aggregation currently stored as `Box<dyn ServiceLifecycle>` because it is part of the same app/setup/doctor dependency graph.

The intended end state is a statically wired CLI runtime where the production app owns concrete dependencies, command dispatch is represented by an enum over all known commands, and service/lifecycle orchestration uses concrete enums or generic capability bounds rather than object-safe trait objects.

## Success criteria

- `cli/src/app.rs` no longer stores logger, telemetry, filesystem, or git dependencies behind `Arc<dyn ...>` in `AppContext`.
- Runtime command parsing/execution no longer returns or executes `Box<dyn RuntimeCommand>`.
- The command catalog remains deterministic and continues to cover the current command set: `help`, `auth`, `config`, `setup`, `doctor`, `hooks`, `version`, and `completion`.
- Commands and lifecycle providers depend on narrow capability traits/accessors where practical rather than on the full concrete production context.
- Existing CLI behavior, stdout/stderr contracts, exit-code classification, command help rendering, setup/doctor lifecycle behavior, and observability behavior remain unchanged except for internal dispatch architecture.
- No new third-party dependencies are introduced.
- Context files that describe `AppContext`, `RuntimeCommand`, `CommandRegistry`, capability traits, and lifecycle providers are updated after code changes.
- Repository validation passes with the repo-preferred checks.

## Constraints and non-goals

- Planning only; this plan does not approve implementation.
- Do not change public CLI command names, arguments, output contracts, error-code taxonomy, or generated config behavior.
- Do not introduce plugin-style dynamic command loading as a replacement for the removed trait objects.
- Do not expand the current command set or implement deferred features such as `sce sync`.
- Do not replace unrelated trait-object uses that are not dependency-injection seams unless they are required by this migration. In particular, closure trait objects such as `dyn FnMut` inside the telemetry API are not in scope unless the compiler forces a local adjustment.
- Keep each executable task as one atomic commit unit.

## Assumptions

- The desired migration is internal architecture-only: behavior must remain stable and existing tests should continue to assert the current CLI contract.
- Because production `Logger` is not currently a cheap `Clone`, the static context design may use a borrowed runtime context/view instead of cloning dependencies when deriving repo-root-scoped contexts.
- Lifecycle provider trait objects are included because they are part of the same DI architecture and currently accept `AppContext`.

## Task stack

- [x] T01: `Introduce generic borrowed AppContext and capability accessors` (status:done)
  - Task ID: T01
  - Goal: Replace the current object-storing `AppContext` design with a generic, borrowed context shape that can reference concrete production dependencies without `Arc<dyn ...>`.
  - Boundaries (in/out of scope): In - `cli/src/app.rs` context data model, production type aliases/helpers, capability accessor traits for logger/telemetry/fs/git/repo root, repo-root-scoped context/view creation, tests directly coupled to context construction. Out - replacing command trait objects or lifecycle provider trait objects.
  - Done when: `AppContext` no longer owns `Arc<dyn Logger>`, `Arc<dyn Telemetry>`, `Arc<dyn FsOps>`, or `Arc<dyn GitOps>`; existing callers can still read logger/telemetry/fs/git/repo-root through stable accessors or capability traits; code compiles with command dispatch still temporarily using the existing runtime-command abstraction.
  - Verification notes (commands or checks): Run a focused compile/test check for the CLI slice, then prefer `nix flake check` if the change touches broad app wiring.
  - Completed: 2026-06-11
  - Files changed: `cli/src/app.rs`, `cli/src/services/app_support.rs`, `cli/src/generated_migrations.rs`, `cli/src/services/agent_trace_db/mod.rs`
  - Evidence: `nix build .#checks.x86_64-linux.cli-tests && nix build .#checks.x86_64-linux.cli-clippy && nix build .#checks.x86_64-linux.cli-fmt` passed; `nix run .#pkl-check-generated` passed. Full `nix flake check` still fails on unrelated pre-existing `config-lib-biome-format` issues in `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`.
  - Notes: `AppRuntime` now owns concrete production dependencies, while `AppContext` is a generic borrowed view with capability accessors and command/lifecycle trait-object dispatch remains for later tasks. The two non-app Rust file changes are formatter-only blank-line cleanup required by `cli-fmt`.

- [ ] T02: `Wire production runtime through concrete dependencies` (status:todo)
  - Task ID: T02
  - Goal: Update `AppRuntime` initialization and run-lifecycle plumbing to own concrete production dependencies and pass borrowed generic contexts through startup, telemetry, parse, execute, and rendering phases.
  - Boundaries (in/out of scope): In - `initialize_runtime`, `try_run_with_dependency_check`, `run_command_lifecycle`, `parse_command_phase`, `app_support` logger plumbing as needed, tests for startup diagnostics and stream contracts. Out - changing command parsing semantics or removing command handles.
  - Done when: production startup constructs concrete `Logger`, `NoopTelemetry`, `StdFsOps`, and `ProcessGitOps` without wrapping them in trait-object `Arc`s; runtime logging/telemetry still occurs once per command dispatch; existing app-level tests pass.
  - Verification notes (commands or checks): Run relevant app/CLI tests plus `nix flake check` when practical.

- [ ] T03: `Replace RuntimeCommandHandle with a static command enum` (status:todo)
  - Task ID: T03
  - Goal: Remove boxed runtime command dispatch by introducing a `RuntimeCommand` enum (or equivalently named static dispatcher) with variants for every current command payload.
  - Boundaries (in/out of scope): In - `cli/src/services/command_registry.rs`, command enum variants, command name lookup, enum `execute` dispatch, parse-layer return type, help/missing-subcommand command construction. Out - rewriting individual command business logic beyond adapting method signatures.
  - Done when: `RuntimeCommandHandle = Box<dyn RuntimeCommand>` is removed; parse-time conversion returns the static command enum; app execution matches enum variants and delegates to the same service-owned command implementations; command-registry tests are replaced with deterministic static-catalog tests.
  - Verification notes (commands or checks): Run command parser/registry/app tests and ensure help/version/completion/config/setup/doctor/hooks/auth dispatch paths still pass existing assertions.

- [ ] T04: `Make command execution generic over required capabilities` (status:todo)
  - Task ID: T04
  - Goal: Update service-owned command execution methods to accept generic context/capability bounds instead of the full production context type wherever possible.
  - Boundaries (in/out of scope): In - command `execute` methods for help/version/completion/auth/config/setup/doctor/hooks, capability trait bounds, logger/repo-root usage, parse/app support call sites. Out - changing command request/response models or renderer output.
  - Done when: command code expresses narrow context requirements through capability traits/accessors; the static command enum can execute against any context satisfying those bounds; public CLI behavior remains unchanged.
  - Verification notes (commands or checks): Run affected command tests and any app-level stdout/stderr/exit-code tests.

- [ ] T05: `Replace lifecycle provider trait objects with static lifecycle dispatch` (status:todo)
  - Task ID: T05
  - Goal: Remove `Box<dyn ServiceLifecycle>` provider aggregation and replace it with deterministic static provider dispatch that works with generic context capabilities.
  - Boundaries (in/out of scope): In - `cli/src/services/lifecycle.rs`, provider catalog representation/order, doctor/setup provider iteration, lifecycle method signatures, lifecycle tests/context docs references. Out - changing lifecycle health taxonomy, setup outcomes, doctor report rendering, or hook install behavior.
  - Done when: lifecycle provider catalogs no longer allocate boxed trait objects; provider order remains config → local_db → auth_db → agent_trace_db → hooks when included; doctor and setup still aggregate diagnose/fix/setup outcomes exactly as before.
  - Verification notes (commands or checks): Run doctor/setup/lifecycle tests; manually inspect static provider ordering in tests; use `nix flake check` for broad validation.

- [ ] T06: `Remove obsolete runtime-DI abstractions and update context` (status:todo)
  - Task ID: T06
  - Goal: Clean up names, tests, and durable context that still describe the old runtime DI architecture after the code migration is complete.
  - Boundaries (in/out of scope): In - remove obsolete aliases/traits/comments/tests such as `RuntimeCommandHandle` and runtime-DI wording, update `context/overview.md`, `context/glossary.md`, `context/architecture.md`, `context/cli/capability-traits.md`, `context/cli/service-lifecycle.md`, and any command-surface context affected by the static dispatcher. Out - broad prose rewrites unrelated to current architecture.
  - Done when: code and context consistently describe compile-time DI/static dispatch as the current state; no stale context claims that `AppContext` stores `Arc<dyn ...>` or commands are boxed trait objects; terminology for removed runtime seams is either deleted or marked historical only where useful.
  - Verification notes (commands or checks): Search for stale `Arc<dyn Logger>`, `Arc<dyn Telemetry>`, `RuntimeCommandHandle`, `Box<dyn RuntimeCommand>`, and `Box<dyn ServiceLifecycle>` references; verify context references match code truth.

- [ ] T07: `Run final validation and cleanup` (status:todo)
  - Task ID: T07
  - Goal: Validate the completed migration, remove temporary scaffolding, and record final evidence in this plan.
  - Boundaries (in/out of scope): In - full repository validation, generated-output parity check, review of changed files for accidental behavior drift, final plan status/evidence updates. Out - new architecture changes beyond fixes required to make validation pass.
  - Done when: `nix run .#pkl-check-generated` and `nix flake check` pass; any temporary compatibility shims or dead-code allowances introduced only for the migration are removed or justified; this plan records validation evidence and remaining follow-ups if any.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; inspect final diff for unrelated changes.

## Open questions

- None blocking. If implementation uncovers additional non-DI trait-object seams, treat them as out of scope unless they block removal of the listed DI seams or require a narrowly scoped follow-up plan.
