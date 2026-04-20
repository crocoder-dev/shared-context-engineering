# Plan: CLI architecture full refactor

## Change summary

Refactor the Rust CLI architecture to reduce central orchestration complexity, remove duplicated config/observability types and help metadata, replace the largest manual parsing hot spots with typed seams, and break oversized service modules into more maintainable units without changing the current CLI behavior contract.

## Success criteria

1. `cli/src/app.rs`, `cli/src/services/config.rs`, `cli/src/services/doctor.rs`, and `cli/src/services/setup.rs` no longer act as oversized multi-responsibility hubs; their responsibilities are split into smaller, named seams with clearer ownership.
2. Config/observability shared concepts (`LogLevel`, `LogFormat`, `LogFileMode`, `OtlpProtocol`, env-key ownership, endpoint/bool parsing helpers) have a single canonical implementation surface instead of duplicate definitions.
3. Command parsing, startup bootstrapping, and command execution are separated enough that adding or changing one command no longer requires editing multiple unrelated orchestration branches in one place.
4. `config` runtime parsing uses typed deserialization/model mapping for file structure rather than continuing to grow the current hand-walked `serde_json::Value` parsing surface.
5. The command/help surface has one canonical ownership path for top-level command metadata so help text and command dispatch do not drift.
6. Existing user-facing behavior remains stable unless a task explicitly calls out a safe internal-only improvement; validation proves current output/error/help contracts still pass.
7. Tokio usage is declared directly through this crate's feature set rather than depending on transitive feature enablement.
8. Context is updated to reflect the resulting architecture once implementation is complete.

## Constraints and non-goals

- In scope: internal architectural refactors, module extraction, ownership cleanup, typed parsing seams, direct dependency declaration cleanup, and behavior-preserving help/dispatch unification.
- In scope: file moves/renames inside `cli/src/` when they improve architectural clarity.
- Out of scope: changing the CLI product scope, removing existing commands, redesigning auth/setup/doctor behavior, or altering stable stdout/stderr/output contracts unless required to preserve current behavior during refactor.
- Out of scope: introducing new external libraries unless a later task explicitly justifies one and the plan is updated first.
- Out of scope: converting the CLI into a multi-crate workspace; this plan stays within the current single-crate boundary.
- Out of scope: broad stylistic cleanup not directly tied to the architectural goals above.

## Task stack

- [x] T01: `Extract shared runtime/config primitives` (status:done)
  - Task ID: T01
  - Goal: Create one canonical internal seam for shared observability/config primitives and declare direct Tokio features required by the CLI.
  - Boundaries (in/out of scope): In - shared enums/constants/helpers currently duplicated across `services/config.rs` and `services/observability.rs`, direct `tokio` feature declaration cleanup in `Cargo.toml`, and local call-site rewiring. Out - changing config precedence behavior, logging output format, or OTEL semantics.
  - Done when: duplicate primitive definitions are removed or reduced to thin adapters around one owner; `tokio` directly enables the runtime/time features the crate uses; existing config/observability callers compile against the shared seam.
  - Verification notes (commands or checks): `nix flake check`; inspect that no duplicated `LogLevel`/`LogFormat`/`LogFileMode`/`OtlpProtocol` definitions remain in active production ownership paths.
  - Completed: 2026-04-20
  - Evidence: `nix develop -c cargo build --manifest-path cli/Cargo.toml`; `nix flake check`
  - Notes: `cli/src/services/config.rs` is now the canonical owner for shared observability/config enums, env-key constants, and OTEL/bool parsing helpers consumed by `cli/src/services/observability.rs`; `tokio` now declares the `time` feature directly.

- [x] T02: `Split app startup into explicit phases` (status:done)
  - Task ID: T02
  - Goal: Refactor `cli/src/app.rs` into clear startup phases for dependency bootstrapping, runtime context construction, command parsing, command execution, and output rendering.
  - Boundaries (in/out of scope): In - extracting startup/context helpers or modules, introducing a named runtime/app context object, and shrinking `try_run_with_dependency_check`. Out - command behavior redesign, command addition/removal, or altering stable exit-code/error-code contracts.
  - Done when: `app.rs` no longer contains one oversized startup function coordinating all responsibilities inline; startup flow reads as ordered phases with isolated error boundaries; existing tests still cover the invalid-config degraded-startup path.
  - Verification notes (commands or checks): `nix flake check`; verify existing `app.rs` tests still pass and the exit-code/error rendering contract remains unchanged.
  - Completed: 2026-04-20
  - Evidence: `nix flake check`; `nix develop -c sh -c 'cd cli && cargo build'`
  - Notes: `cli/src/app.rs` now uses `StartupContext`, `AppRuntime`, and `RunOutcome` to separate dependency bootstrapping, runtime initialization, command lifecycle execution, and output rendering without changing degraded-startup logging or exit-code behavior.

- [x] T03: `Introduce command execution seam` (status:done)
  - Task ID: T03
  - Goal: Add a dedicated command execution abstraction so parse-time command conversion and run-time command handling are not centralized in one large `match`.
  - Boundaries (in/out of scope): In - introducing a command handler trait or equivalent execution seam, moving per-command conversion/execution closer to command-specific modules, and reducing `dispatch`/conversion sprawl. Out - changing the exposed command list or command semantics.
  - Done when: adding a command no longer requires expanding one monolithic dispatch match for both conversion and execution; command execution ownership is localized and coherent enough for one-command-at-a-time evolution.
  - Verification notes (commands or checks): `nix flake check`; inspect that top-level command execution no longer depends on one large central `dispatch` implementation for all commands.
  - Completed: 2026-04-20
  - Evidence: `nix develop -c sh -c 'cd cli && cargo build'`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: `cli/src/app.rs` now routes command parsing through an internal `RuntimeCommand` seam and executes boxed command objects via `execute_command_phase`, removing the prior app-level `dispatch` match while preserving command behavior.

- [x] T04: `Unify top-level help and command catalog ownership` (status:done)
  - Task ID: T04
  - Goal: Eliminate the parallel help/catalog ownership split between `cli_schema.rs` and `command_surface.rs` so top-level command metadata is defined once.
  - Boundaries (in/out of scope): In - refactoring top-level help generation and command metadata ownership, preserving current hidden-vs-visible top-level help behavior, and keeping stable human-facing help content. Out - changing command names, removing hidden-command behavior, or redesigning clap subcommand help output.
  - Done when: one canonical source controls the top-level command catalog/help visibility contract, and the code no longer requires manually keeping separate command lists in sync for dispatch classification vs help rendering.
  - Verification notes (commands or checks): `nix flake check`; manually compare `sce --help` expectations via existing tests or help assertions to confirm no user-visible drift.
  - Completed: 2026-04-20
  - Evidence: `nix run .#pkl-check-generated`; `nix flake check`; `nix develop -c sh -c 'cd cli && cargo build'`
  - Notes: `cli/src/cli_schema.rs` now owns the canonical top-level command metadata (top-level purpose text plus help visibility), while `cli/src/command_surface.rs` consumes that catalog for help rendering and known-command classification; inline help assertions now verify visible-command output and hidden-command exclusion.

- [ ] T05: `Replace config file hand-parsing with typed deserialization` (status:todo)
  - Task ID: T05
  - Goal: Refactor `services/config.rs` so config-file structure is parsed through typed `serde` models and focused validation/mapping stages instead of the current large manual `Value` walk.
  - Boundaries (in/out of scope): In - typed file DTOs, mapping/validation helpers, preserving schema validation and precedence behavior, and reducing repetitive per-key manual parsing. Out - changing config file shape, precedence order, warning/error wording beyond unavoidable internal refactor fallout, or removing schema validation.
  - Done when: file parsing is driven by typed deserialization for top-level/nested config structures, repetitive manual object-key traversal is substantially reduced, and existing config command/startup behavior stays aligned with the current schema contract.
  - Verification notes (commands or checks): `nix flake check`; verify existing config-focused tests pass and invalid/valid config scenarios still produce the same contract outcomes.

- [ ] T06: `Decompose doctor into focused diagnostics units` (status:todo)
  - Task ID: T06
  - Goal: Split `services/doctor.rs` into smaller diagnostics/reporting units with fewer sprawling enums and lower orchestration density while preserving the doctor contract.
  - Boundaries (in/out of scope): In - extracting doctor domain types, inspection helpers, rendering helpers, and fix execution seams into focused modules/files. Out - changing the doctor JSON/text contract, problem taxonomy semantics, or fix behavior.
  - Done when: `doctor` ownership is divided into coherent submodules (for example diagnosis, rendering, and fix execution), the main file is materially smaller, and the doctor contract remains stable.
  - Verification notes (commands or checks): `nix flake check`; confirm doctor text/json output tests or equivalent assertions still pass without contract drift.

- [ ] T07: `Tighten setup and path/service support seams` (status:todo)
  - Task ID: T07
  - Goal: Reduce incidental architecture debt in `setup.rs`, `default_paths.rs`, and related support modules so path ownership and setup flows remain maintainable after the broader refactor.
  - Boundaries (in/out of scope): In - removing module-level `allow(dead_code)` where feasible, extracting focused helpers/modules from oversized setup/path code, and preserving staging/install/recovery behavior. Out - changing setup feature scope, hook-install semantics, or path contracts.
  - Done when: setup/path support code has clearer internal ownership boundaries, dead-code suppression is removed or narrowed to justified local cases, and support modules are easier to navigate without semantic changes.
  - Verification notes (commands or checks): `nix flake check`; inspect that setup/hook/path-related tests and compile-time asset flows still behave as before.

- [ ] T08: `Sync current-state architecture context` (status:todo)
  - Task ID: T08
  - Goal: Update `context/` so durable architecture and CLI context files reflect the new module ownership and command/runtime structure after the refactor tasks land.
  - Boundaries (in/out of scope): In - root/context updates required by the final architecture, relevant CLI domain context updates, and any needed decision record if ownership boundaries materially change. Out - leaving historical implementation notes in core context files or treating plan text as durable history.
  - Done when: current-state context files accurately describe the refactored CLI architecture and canonical ownership seams introduced by T01-T07.
  - Verification notes (commands or checks): verify context files match code truth; ensure durable files describe resulting architecture rather than implementation progress.

- [ ] T09: `Run full validation and cleanup` (status:todo)
  - Task ID: T09
  - Goal: Perform final validation, remove temporary scaffolding, and confirm the refactor preserves the repository contract.
  - Boundaries (in/out of scope): In - full repo validation, cleanup of temporary compatibility shims that are no longer needed, and final review for contract drift. Out - adding new features or opportunistic follow-up refactors.
  - Done when: the full required validation passes, no temporary scaffolding remains unintentionally, and the plan is ready to close with context aligned.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; verify no temporary TODO/shim paths remain unless intentionally documented.

## Open questions

- None at planning time. The plan assumes behavior-preserving internal refactors unless a later task reveals a contract-level change that must be approved before implementation.
