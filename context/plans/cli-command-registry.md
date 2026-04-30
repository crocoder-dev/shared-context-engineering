# Plan: cli-command-registry

## Change summary

Replace the monolithic `app.rs` command dispatch with a lightweight command registry. Each service owns its command handler (implementing `RuntimeCommand`) in a dedicated `command.rs` within its service directory. `app.rs` becomes a thin dispatcher: parse args, build `AppContext`, look up the handler in the registry, and execute.

## Success criteria

- Each CLI command (auth, config, setup, doctor, hooks, version, completion) has its `RuntimeCommand` impl moved into `services/{name}/command.rs`.
- A `CommandRegistry` struct maps command names to `Box<dyn RuntimeCommand>` constructors.
- `app.rs` is under ~200 lines and contains no command-specific logic.
- `nix flake check` passes.
- No regression in CLI behavior, help text, or error messages.

## Constraints and non-goals

- **Do not** change clap schema or argument parsing logic.
- **Do not** change command execution behavior (this plan is purely structural relocation).
- **Do not** introduce dynamic plugin loading; registry is statically populated at compile time.
- Help text generation (`command_surface.rs`, `cli_schema.rs`) stays where it is.

## Assumptions

- The `RuntimeCommand` trait signature (or the `AppContext`-aware version from `cli-observability-di`) is the stable dispatch boundary.
- Commands that currently ignore the logger will continue to ignore it; no cleanup of unused parameters in this plan.
- `context/decisions/cli-refactor-decisions.md` is the decision record for this plan; it chooses static registry population, extraction of clap parsing/error handling into `services/parse/command_runtime.rs`, and a pre-conversion task for single-file services.

## Task stack

- [x] T00: Pre-convert single-file services to directory modules (status:done)
  - Task ID: T00
  - Goal: Mechanically convert `hooks.rs`, `config.rs`, `setup.rs`, and `local_db.rs` into directory-backed modules (`hooks/mod.rs`, etc.) before command/lifecycle files are added.
  - Boundaries (in/out of scope): In - file moves, `mod.rs` re-exports, `services/mod.rs` path compatibility, no behavior changes. Out - adding `command.rs` or `lifecycle.rs`, changing public APIs.
  - Done when: The converted modules compile with equivalent public surfaces, no command behavior changes, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`
  - Completed: 2026-04-29
  - Files changed: `cli/src/services/hooks/mod.rs`, `cli/src/services/config/mod.rs`, `cli/src/services/setup/mod.rs`, `cli/src/services/local_db/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo check'` passed.
  - Notes: Mechanical directory-module conversion only; relative `include_str!` paths were updated for the deeper `mod.rs` locations.

- [x] T01: Define `CommandRegistry` and registry builder (status:done)
  - Task ID: T01
  - Goal: Create `services/command_registry.rs` with a `CommandRegistry` struct that holds a map from `&'static str` to a constructor `fn() -> Box<dyn RuntimeCommand>`. Provide a `build_default_registry()` function that populates it.
  - Boundaries (in/out of scope): In - registry struct, builder function, registration API. Out - moving command impls, wiring into `app.rs`.
  - Done when: Registry compiles, can register and retrieve a test command, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`
  - Completed: 2026-04-29
  - Files changed: `cli/src/services/command_registry.rs` (new), `cli/src/services/mod.rs`, `cli/src/app.rs`
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt all green); `nix run .#pkl-check-generated` passed; `cargo check` passed.
  - Notes: Moved `RuntimeCommand` trait and `RuntimeCommandHandle` type alias from `app.rs` private `command_runtime` module to `services/command_registry.rs` so the registry can reference them. `app.rs` now imports these from `services::command_registry`. `build_default_registry()` starts empty; commands will be registered in T02–T04.

- [x] T02: Move `HelpCommand`, `HelpTextCommand`, `VersionCommand`, `CompletionCommand` to service commands (status:done)
  - Task ID: T02
  - Goal: Extract simple command structs (`HelpCommand`, `HelpTextCommand`, `VersionCommand`, `CompletionCommand`) from `app.rs` into `services/{name}/command.rs` files. Register them in `build_default_registry()`.
  - Boundaries (in/out of scope): In - moving the four simple commands to their service directories, registering them. Out - moving complex commands (auth, config, setup, doctor, hooks).
  - Done when: The four simple commands live in service modules, registry includes them, `app.rs` no longer defines them, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`
  - Completed: 2026-04-29
  - Files changed: `cli/src/services/help/mod.rs` (new), `cli/src/services/help/command.rs` (new), `cli/src/services/version/mod.rs` (new, replaces `version.rs`), `cli/src/services/version/command.rs` (new), `cli/src/services/completion/mod.rs` (new, replaces `completion.rs`), `cli/src/services/completion/command.rs` (new), `cli/src/services/command_registry.rs`, `cli/src/services/mod.rs`, `cli/src/app.rs`
  - Evidence: `nix flake check` passed (cli-tests, cli-clippy, cli-fmt all green); `nix run .#pkl-check-generated` passed.
  - Notes: `HelpCommand` registered in `build_default_registry()` as a zero-arg constructor. `HelpTextCommand`, `VersionCommand`, and `CompletionCommand` are stateful (carry parsed args) and remain constructed in the parse layer (`command_runtime` in `app.rs`), but their struct definitions and `RuntimeCommand` impls now live in service-owned `command.rs` files. `make_version_command()` and `make_completion_command()` default constructors are `#[allow(dead_code)]` for future registry use. `version.rs` and `completion.rs` were converted from single-file modules to directory modules (`mod.rs` + `command.rs`).

- [x] T03: Move `AuthCommand`, `ConfigCommand` to service commands (status:done)
  - Task ID: T03
  - Goal: Extract `AuthCommand` and `ConfigCommand` from `app.rs` into `services/auth_command/command.rs` and `services/config/command.rs`. Register them.
  - Boundaries (in/out of scope): In - moving structs and `RuntimeCommand` impls, registering them. Out - changing auth/config service internals.
  - Done when: Both commands live in service modules, registry includes them, `app.rs` no longer defines them, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`
  - Completed: 2026-04-29
  - Files changed: `cli/src/services/auth_command.rs` → `cli/src/services/auth_command/mod.rs` (directory-module conversion), `cli/src/services/auth_command/command.rs` (new), `cli/src/services/config/command.rs` (new), `cli/src/services/config/mod.rs`, `cli/src/services/command_registry.rs`, `cli/src/app.rs`
  - Evidence: `cargo check` passed, `cargo clippy` passed, `cargo fmt --check` passed.
  - Notes: `auth_command.rs` was converted from a single-file module to a directory module (`auth_command/mod.rs` + `auth_command/command.rs`) following the T00 pattern. Both `AuthCommand` and `ConfigCommand` are stateful commands with `#[allow(dead_code)]` default constructors for registry use, matching the T02 pattern for `VersionCommand`/`CompletionCommand`.

- [x] T04: Move `SetupCommand`, `DoctorCommand`, `HooksCommand` to service commands (status:done)
  - Task ID: T04
  - Goal: Extract `SetupCommand`, `DoctorCommand`, and `HooksCommand` from `app.rs` into their respective `services/{name}/command.rs` files. Register them.
  - Boundaries (in/out of scope): In - moving structs and `RuntimeCommand` impls, registering them. Out - changing setup/doctor/hooks service internals.
  - Done when: All three commands live in service modules, registry includes them, `app.rs` no longer defines them, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`
  - Completed: 2026-04-29
  - Files changed: `cli/src/services/setup/command.rs` (new), `cli/src/services/doctor/command.rs` (new), `cli/src/services/hooks/command.rs` (new), `cli/src/services/setup/mod.rs`, `cli/src/services/doctor/mod.rs`, `cli/src/services/hooks/mod.rs`, `cli/src/services/command_registry.rs`, `cli/src/app.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo clippy'` passed; `nix run .#pkl-check-generated` passed.
  - Notes: `SetupCommand`, `DoctorCommand`, and `HooksCommand` now follow the T02/T03 service-owned command pattern with default constructors for registry use. The new files were staged before Nix validation so the flake's Git-filtered source could see them; no commit was created.

- [x] T05: Extract command runtime parsing and thin `app.rs` dispatcher (status:done)
  - Task ID: T05
  - Goal: Move `parse_runtime_command`, clap error handling, help rendering bridges, and `convert_*` helpers into `services/parse/command_runtime.rs`; reduce `app.rs` to startup context building, parse/resolve call, command execution, and output rendering.
  - Boundaries (in/out of scope): In - parser module extraction, `app.rs` refactor, registry lookup bridge. Out - changing clap schema or user-facing parse diagnostics.
  - Done when: `app.rs` is under ~200 lines, parsing is owned by `services/parse/command_runtime.rs`, all commands are dispatched through the registry, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'` and `nix develop -c sh -c 'cd cli && cargo clippy'`
  - Completed: 2026-04-29
  - Files changed: `cli/src/app.rs`, `cli/src/services/app_support.rs` (new), `cli/src/services/parse/mod.rs` (new), `cli/src/services/parse/command_runtime.rs` (new), `cli/src/services/command_registry.rs`, `cli/src/services/mod.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo fmt && cargo check'` passed; `nix develop -c sh -c 'cd cli && cargo clippy'` passed; `wc -l cli/src/app.rs` reports 197 lines.
  - Notes: Extracted parser/clap error/help bridge/conversion logic into `services::parse::command_runtime`; moved app output/execution helper code into `services::app_support` to keep `app.rs` below the plan threshold while preserving startup, parse, execute, and render orchestration. Registered `version` and `completion` default constructors so the default registry covers the full current command catalog.

- [x] T06: Validate full behavior parity (status:done)
  - Task ID: T06
  - Goal: Run the full test suite and verify no behavioral regressions.
  - Boundaries (in/out of scope): In - `cargo test`, manual CLI spot-checks for help/version/doctor, relocation of existing `app.rs` startup tests to the parse/runtime module if they no longer belong in `app.rs`. Out - adding unrelated new coverage.
  - Done when: `cargo test` passes, `nix flake check` passes, and a manual `sce --help` / `sce version` spot-check looks correct.
  - Verification notes (commands or checks): `nix flake check`, `nix develop -c sh -c 'cd cli && cargo test'`, manual CLI smoke test.
  - Completed: 2026-04-29
  - Files changed: `context/plans/cli-command-registry.md`
  - Evidence: `nix flake check` passed, including `cli-tests`, `cli-clippy`, `cli-fmt`, and `pkl-parity`; `nix run .#pkl-check-generated` passed; manual smoke checks passed for `nix develop -c sh -c 'cd cli && cargo run -- --help'`, `nix develop -c sh -c 'cd cli && cargo run -- version'`, and `nix develop -c sh -c 'cd cli && cargo run -- doctor --format json'`.
  - Notes: Direct `nix develop -c sh -c 'cd cli && cargo test'` was blocked by the configured SCE bash-tool policy `use-nix-flake-check-over-cargo-test`; the required full-suite evidence is therefore the repo-approved `nix flake check`, whose `cli-tests` derivation covers the CLI Rust tests. No startup-test relocation was needed.

## Validation Report

### Commands run

- `nix flake check` -> exit 0; key output: `all checks passed!` after evaluating/building `cli-tests`, `cli-clippy`, `cli-fmt`, `pkl-parity`, and repository JS/integration check derivations.
- `nix run .#pkl-check-generated` -> exit 0; key output: `Generated outputs are up to date.`
- `nix develop -c sh -c 'cd cli && cargo run -- --help'` -> exit 0; key output: top-level help rendered the SCE banner, usage sections, examples, and visible command list (`help`, `config`, `setup`, `doctor`, `version`, `completion`).
- `nix develop -c sh -c 'cd cli && cargo run -- version'` -> exit 0; key output: `shared-context-engineering 0.2.0 (8ce594ddd16e)`.
- `nix develop -c sh -c 'cd cli && cargo run -- doctor --format json'` -> exit 0; key output: JSON payload returned `status: ok`, `command: doctor`, hook records, and deterministic environment-specific `readiness: not_ready` due repo-root OpenCode plugin content drift outside this task.
- `nix develop -c sh -c 'cd cli && cargo test'` -> blocked before execution by SCE bash-tool policy `use-nix-flake-check-over-cargo-test`; follow-up evidence is the repo-approved `nix flake check` `cli-tests` derivation.

### Success-criteria verification

- [x] Each CLI command has its `RuntimeCommand` implementation moved into service-owned `command.rs` files: confirmed by code structure and context sync for help/version/completion/auth/config/setup/doctor/hooks.
- [x] `CommandRegistry` maps command names to constructors: confirmed in `cli/src/services/command_registry.rs` and full flake checks.
- [x] `app.rs` is under ~200 lines and contains no command-specific logic: T05 evidence recorded `wc -l cli/src/app.rs` at 197 lines; T06 validation smoke checks confirm behavior still routes correctly.
- [x] `nix flake check` passes: exit 0 with `all checks passed!`.
- [x] No regression in CLI behavior, help text, or error messages identified by tests and manual smoke checks.

### Failed checks and follow-ups

- Direct Cargo test execution was intentionally blocked by the repository bash policy preferring `nix flake check`; no code failure was observed. No follow-up required unless the policy is changed.

### Residual risks

- `sce doctor --format json` reported environment-specific repo-root OpenCode plugin drift; this is outside the command-registry task and did not prevent the doctor command from executing successfully.

## Open questions

None — assumptions recorded above.
