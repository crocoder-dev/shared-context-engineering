# Plan: optional-nix-install-channel-integration-tests

## Change summary

Revise the optional install-channel integration-test implementation so the public Nix entrypoint stays stable while orchestration moves out of inline `flake.nix` bash into a dedicated Rust runner under `integrations/install/`. The Rust runner should own shared harness behavior first, then own npm, Bun, and Cargo channel flows behind a deterministic channel selector contract.

## Success criteria

- `nix run .#install-channel-integration-tests -- --channel <npm|bun|cargo|all>` remains the public opt-in entrypoint and keeps the same accepted channel values.
- `flake.nix` no longer owns the install-channel integration orchestration beyond thin delegation to the Rust runner.
- A standalone Cargo crate/binary under `integrations/install/` owns the install-channel integration runner.
- Shared harness behavior is implemented in Rust, including isolated per-channel work/home/XDG/npm/Bun/Cargo state and centralized deterministic `sce version` assertions.
- npm, Bun, and Cargo channel-specific install flows run through the Rust runner without changing the external selector contract.
- The optional coverage remains outside default `nix flake check`.
- Current-state context/docs reflect the new implementation ownership and invocation model.

## Constraints and non-goals

- In scope: migrating install-channel integration orchestration/harness logic from flake-owned shell to a Rust runner, channel-path migration work, thin flake delegation updates, and focused context updates required to describe the new ownership model.
- In scope: using `integrations/install/` as the implementation location for the standalone runner that orchestrates npm, Bun, and Cargo channel tests.
- In scope: preserving the existing opt-in Nix app contract and channel selector behavior.
- In scope: porting shared harness behavior before channel-specific orchestration so later tasks can build on a stable runner seam.
- Out of scope: adding this coverage to default `nix flake check`.
- Out of scope: broader release/distribution redesign.
- Out of scope: changing the external CLI contract of the opt-in integration-test command.
- Out of scope: adding install channels beyond npm, Bun, and Cargo unless a blocker is discovered.

## Assumptions

- The Rust runner can be introduced as its own Cargo crate/binary without requiring a repo-wide workspace redesign beyond what is needed to build and invoke it deterministically.
- The existing harness behavior in `flake.nix` is the source to preserve for isolation semantics and deterministic `sce version` assertions unless implementation reality forces a narrower documented adjustment.

## Task stack

- [x] T01: `Scaffold Rust install-channel runner contract` (status:done)
  - Task ID: T01
  - Goal: Establish the standalone Cargo crate/binary under `integrations/install/` with a clear CLI contract for `--channel <npm|bun|cargo|all>`, deterministic exit behavior, and internal seams for shared harness plus per-channel runners.
  - Boundaries (in/out of scope): In - crate/binary scaffold, argument parsing contract, runner module layout, deterministic reporting/error-shape decisions, and build/invocation seams needed for later migration tasks. Out - porting shared harness behavior, flake delegation rewiring, and real npm/Bun/Cargo install flows.
  - Done when: The repo contains a buildable Rust integration-runner crate in `integrations/install/` with the agreed channel selector contract and placeholder execution flow that is ready for harness migration work.
  - Verification notes (commands or checks): Build or run the new runner directly with help/argument validation; inspect crate layout to confirm channel contract and module boundaries are explicit and do not yet bundle channel-specific install logic.
  - Completed: 2026-04-07
  - Files changed: `integrations/install/Cargo.toml`, `integrations/install/src/main.rs`, `integrations/install/src/cli.rs`, `integrations/install/src/harness.rs`, `integrations/install/src/channels/{mod.rs,npm.rs,bun.rs,cargo.rs}`
  - Evidence: targeted `cargo build`, `cargo run -- --help`, `cargo run -- --channel npm`, invalid selector validation via Nix dev shell, plus `nix run .#pkl-check-generated` and `nix flake check`

- [x] T02: `Port shared harness isolation and assertions to Rust` (status:done)
  - Task ID: T02
  - Goal: Move the reusable harness behavior out of flake-owned bash into Rust, including per-channel temporary roots, HOME/XDG/npm/Bun/Cargo isolation, and one centralized deterministic `sce version` assertion path.
  - Boundaries (in/out of scope): In - Rust harness utilities, temp/state/environment setup, common command execution helpers, and shared version assertion logic. Out - flake delegation changes and channel-specific install-command migration beyond the minimum stubs needed to exercise the shared harness.
  - Done when: The Rust runner owns the shared isolation model and common `sce version` success assertion, and later channel tasks can call that harness instead of flake-embedded shell helpers.
  - Verification notes (commands or checks): Run the narrowest runner path that exercises only shared harness behavior; inspect that isolated roots are channel-scoped and that version assertions are centralized in Rust rather than duplicated per channel.
  - Completed: 2026-04-07
  - Files changed: `integrations/install/src/harness.rs`, `integrations/install/src/channels/{mod.rs,npm.rs,bun.rs,cargo.rs}`
  - Evidence: `nix develop -c sh -c 'cd integrations/install && cargo fmt && cargo run -- --channel npm'`, `nix run .#pkl-check-generated`, and `nix flake check`

- [x] T03: `Add flake checks for integrations/install runner` (status:done)
  - Task ID: T03
  - Goal: Add dedicated root-flake check derivations for the `integrations/install/` crate covering format, clippy, and tests so the Rust runner has first-class verification entrypoints under `nix flake check`.
  - Boundaries (in/out of scope): In - root `flake.nix` check wiring for `integrations/install` only, targeted Rust fmt/clippy/test derivations for that crate, and context/doc updates that mention the new check names. Out - changing the public install-channel app contract, flake app delegation work, channel install behavior, or broadening verification for unrelated crates.
  - Done when: `flake.nix` exposes dedicated `integrations-install-fmt`, `integrations-install-clippy`, and `integrations-install-tests` checks scoped to `integrations/install/`, those checks run successfully through `nix flake check`, and current-state docs mention the new check names where relevant.
  - Verification notes (commands or checks): Run `nix flake check`; confirm the three new check derivations appear in evaluated outputs and execute against only `integrations/install/`.
  - Completed: 2026-04-07
  - Files changed: `flake.nix`, `context/overview.md`, `context/glossary.md`, `context/sce/optional-install-channel-integration-test-entrypoint.md`
  - Evidence: targeted `nix build .#checks.<system>.integrations-install-{fmt,clippy,tests}` passed; `nix flake check` evaluated and ran `integrations-install-tests`, `integrations-install-clippy`, and `integrations-install-fmt`

- [x] T04: `Thin flake app to Rust-runner delegation` (status:done)
  - Task ID: T04
  - Goal: Replace the flake-owned inline orchestration with a thin `apps.install-channel-integration-tests` delegation path that invokes the Rust runner deterministically while preserving the public Nix entrypoint contract.
  - Boundaries (in/out of scope): In - minimal `flake.nix` wiring changes required to build/invoke the Rust runner, argument pass-through preservation, and explicit non-default execution posture. Out - deeper harness logic, channel install behavior, and any expansion of default flake checks.
  - Done when: `flake.nix` acts only as thin wiring to the Rust runner, the public `nix run .#install-channel-integration-tests -- --channel ...` command shape is unchanged, and no inline shell orchestration remains for this feature.
  - Verification notes (commands or checks): Run the Nix entrypoint with help and at least one channel selection to confirm delegation works and default `nix flake check` membership is unchanged.
  - Completed: 2026-04-07
  - Files changed: `flake.nix`, `context/sce/optional-install-channel-integration-test-entrypoint.md`
  - Evidence: `nix run .#install-channel-integration-tests -- --help`, `nix run .#install-channel-integration-tests -- --channel npm`, `nix run .#pkl-check-generated`, and `nix flake check`

- [x] T05: `Migrate npm channel orchestration into Rust runner` (status:done)
  - Task ID: T05
  - Goal: Implement the npm-specific install-and-verify path inside the Rust runner using the shared harness and existing public channel selector.
  - Boundaries (in/out of scope): In - npm install orchestration, npm-specific environment/path handling, and npm-targeted use of the shared version assertion. Out - Bun/Cargo flows and unrelated runner refactors not needed for npm support.
  - Done when: The Rust runner can execute `--channel npm` end-to-end through npm installation and deterministic `sce version` verification inside isolated npm state.
  - Verification notes (commands or checks): Run the opt-in integration entrypoint or direct runner path scoped to `npm`; confirm success/failure comes from the Rust-owned harness rather than flake shell logic.
  - Completed: 2026-04-07
  - Files changed: `integrations/install/src/channels/npm.rs`, `integrations/install/src/harness.rs`, `integrations/install/src/cli.rs`, `flake.nix`, `context/sce/optional-install-channel-integration-test-entrypoint.md`
  - Evidence: `nix develop -c sh -c 'cd integrations/install && cargo fmt && cargo build'`, `nix run .#install-channel-integration-tests -- --channel npm` (installed `sce version` + `sce doctor --format json`), `nix run .#pkl-check-generated`, and `nix flake check`

- [x] T06: `Migrate Bun channel orchestration into Rust runner` (status:done)
  - Task ID: T06
  - Goal: Implement the Bun-specific install-and-verify path inside the Rust runner using the shared harness and existing public channel selector.
  - Boundaries (in/out of scope): In - Bun install orchestration, Bun-specific environment/path handling, and Bun-targeted use of the shared version assertion. Out - npm/Cargo flows and broader runner redesign beyond what Bun support requires.
  - Done when: The Rust runner can execute `--channel bun` end-to-end through Bun installation and deterministic `sce version` verification inside isolated Bun state.
  - Verification notes (commands or checks): Run the opt-in integration entrypoint or direct runner path scoped to `bun`; confirm the Bun path is implemented in Rust and reuses the shared harness.
  - Completed: 2026-04-07
  - Files changed: `integrations/install/src/channels/bun.rs`, `integrations/install/src/channels/npm.rs`, `integrations/install/src/channels/mod.rs`, `integrations/install/src/cli.rs`
  - Evidence: `nix develop -c sh -c 'cd integrations/install && cargo fmt && cargo build'`, `nix run .#install-channel-integration-tests -- --channel bun`, `nix run .#pkl-check-generated`, and `nix flake check`

- [x] T07: `Migrate Cargo channel orchestration into Rust runner` (status:done)
  - Task ID: T07
  - Goal: Implement the Cargo-specific install-and-verify path inside the Rust runner using the shared harness while preserving the existing public channel selector contract.
  - Boundaries (in/out of scope): In - Cargo install orchestration, Cargo-specific environment/path handling, and Cargo-targeted use of the shared version assertion. Out - broader Cargo publication/distribution redesign or new install-channel scope.
  - Done when: The Rust runner can execute `--channel cargo` end-to-end through Cargo installation and deterministic `sce version` verification inside isolated Cargo state.
  - Verification notes (commands or checks): Run the opt-in integration entrypoint or direct runner path scoped to `cargo`; confirm the Cargo path is implemented in Rust and reuses the shared harness.
  - Completed: 2026-04-07
  - Files changed: `integrations/install/src/channels/cargo.rs`, `context/sce/optional-install-channel-integration-test-entrypoint.md`
  - Evidence: `nix develop -c sh -c 'cd integrations/install && cargo fmt && cargo build'`, `nix run .#install-channel-integration-tests -- --channel cargo` (installed `sce version`), `nix run .#pkl-check-generated`, and `nix flake check`

- [x] T08: `Sync context and run final opt-in validation` (status:done)
  - Task ID: T08
  - Goal: Update current-state context/docs to reflect Rust-owned install-channel integration orchestration, then run final validation/cleanup through the opt-in entrypoint across all supported channels.
  - Boundaries (in/out of scope): In - context sync for ownership/invocation changes, plan evidence updates, final cleanup, and explicit opt-in validation across npm, Bun, and Cargo. Out - new feature work beyond the install-channel integration migration.
  - Done when: Relevant context/docs describe the Rust runner and thin flake delegation accurately, `nix run .#install-channel-integration-tests -- --channel all` passes through the Rust implementation, and no temporary migration scaffolding remains.
  - Verification notes (commands or checks): Run the full opt-in integration command across all channels; run required repo validation for touched surfaces; confirm context files match resulting code truth and default `nix flake check` scope is unchanged.
  - Completed: 2026-04-07
  - Files changed: `context/plans/optional-nix-install-channel-integration-tests.md`, `context/overview.md`, `context/glossary.md`, `context/context-map.md`, `context/sce/optional-install-channel-integration-test-entrypoint.md`
  - Evidence: `nix run .#install-channel-integration-tests -- --channel all` (npm, bun, cargo all passed), `nix run .#pkl-check-generated`, `nix flake check`, no temporary scaffolding found in integration runner

## Open questions

- None.
