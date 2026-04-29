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

- [ ] T01: Define `CommandRegistry` and registry builder (status:todo)
  - Task ID: T01
  - Goal: Create `services/command_registry.rs` with a `CommandRegistry` struct that holds a map from `&'static str` to a constructor `fn() -> Box<dyn RuntimeCommand>`. Provide a `build_default_registry()` function that populates it.
  - Boundaries (in/out of scope): In - registry struct, builder function, registration API. Out - moving command impls, wiring into `app.rs`.
  - Done when: Registry compiles, can register and retrieve a test command, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T02: Move `HelpCommand`, `HelpTextCommand`, `VersionCommand`, `CompletionCommand` to service commands (status:todo)
  - Task ID: T02
  - Goal: Extract simple command structs (`HelpCommand`, `HelpTextCommand`, `VersionCommand`, `CompletionCommand`) from `app.rs` into `services/{name}/command.rs` files. Register them in `build_default_registry()`.
  - Boundaries (in/out of scope): In - moving the four simple commands to their service directories, registering them. Out - moving complex commands (auth, config, setup, doctor, hooks).
  - Done when: The four simple commands live in service modules, registry includes them, `app.rs` no longer defines them, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T03: Move `AuthCommand`, `ConfigCommand` to service commands (status:todo)
  - Task ID: T03
  - Goal: Extract `AuthCommand` and `ConfigCommand` from `app.rs` into `services/auth/command.rs` and `services/config/command.rs`. Register them.
  - Boundaries (in/out of scope): In - moving structs and `RuntimeCommand` impls, registering them. Out - changing auth/config service internals.
  - Done when: Both commands live in service modules, registry includes them, `app.rs` no longer defines them, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T04: Move `SetupCommand`, `DoctorCommand`, `HooksCommand` to service commands (status:todo)
  - Task ID: T04
  - Goal: Extract `SetupCommand`, `DoctorCommand`, and `HooksCommand` from `app.rs` into their respective `services/{name}/command.rs` files. Register them.
  - Boundaries (in/out of scope): In - moving structs and `RuntimeCommand` impls, registering them. Out - changing setup/doctor/hooks service internals.
  - Done when: All three commands live in service modules, registry includes them, `app.rs` no longer defines them, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'`

- [ ] T05: Extract command runtime parsing and thin `app.rs` dispatcher (status:todo)
  - Task ID: T05
  - Goal: Move `parse_runtime_command`, clap error handling, help rendering bridges, and `convert_*` helpers into `services/parse/command_runtime.rs`; reduce `app.rs` to startup context building, parse/resolve call, command execution, and output rendering.
  - Boundaries (in/out of scope): In - parser module extraction, `app.rs` refactor, registry lookup bridge. Out - changing clap schema or user-facing parse diagnostics.
  - Done when: `app.rs` is under ~200 lines, parsing is owned by `services/parse/command_runtime.rs`, all commands are dispatched through the registry, and `cargo check` passes.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo check'` and `nix develop -c sh -c 'cd cli && cargo clippy'`

- [ ] T06: Validate full behavior parity (status:todo)
  - Task ID: T06
  - Goal: Run the full test suite and verify no behavioral regressions.
  - Boundaries (in/out of scope): In - `cargo test`, manual CLI spot-checks for help/version/doctor, relocation of existing `app.rs` startup tests to the parse/runtime module if they no longer belong in `app.rs`. Out - adding unrelated new coverage.
  - Done when: `cargo test` passes, `nix flake check` passes, and a manual `sce --help` / `sce version` spot-check looks correct.
  - Verification notes (commands or checks): `nix flake check`, `nix develop -c sh -c 'cd cli && cargo test'`, manual CLI smoke test.

## Open questions

None — assumptions recorded above.
