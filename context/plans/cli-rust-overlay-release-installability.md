# Plan: cli-rust-overlay-release-installability

## 1) Change summary
Add a Rust toolchain overlay in `cli/` and wire deterministic Nix packaging so the `sce` CLI can be built as a release artifact and installed via Nix now, while preparing the crate for future crates.io installation.

## 2) Success criteria
- `cli/flake.nix` defines a Rust overlay-backed toolchain contract used for CLI build/check paths.
- Nix outputs expose a release buildable/installable CLI package (and app entrypoint) for `sce`.
- Cargo flow supports release builds and local install (`cargo install --path cli`) and documents a future crates.io publish path.
- Repository/root flake wiring remains coherent with the nested `cli` flake outputs.
- Docs and context reflect the new install/build contracts and verification commands.

## 3) Constraints and non-goals
- In scope: `cli/flake.nix`, `cli/Cargo.toml`, CLI docs, and directly impacted root flake/context references.
- In scope: deterministic release build verification for both Nix and Cargo workflows.
- Out of scope: publishing to crates.io in this task.
- Out of scope: changing CLI runtime behavior or expanding command-domain functionality.
- Non-goal: introducing non-deterministic or host-specific build/install steps as the primary path.

## 4) Task stack (T01..T06)
- [x] T01: Lock packaging and install contracts for Nix + future crates.io (status:done)
  - Task ID: T01
  - Goal: Define the exact packaging contract for current Nix installation and future crates.io installation readiness (without publishing).
  - Boundaries (in/out of scope):
    - In: output names (`packages`, `apps`), binary name expectations, Cargo metadata policy for future publish.
    - Out: implementing build wiring and docs updates.
  - Done when:
    - Contract decisions are explicit for: Nix install command, release artifact shape, and crates.io readiness posture.
    - Assumptions are documented and reflected in subsequent tasks.
  - Verification notes (commands or checks):
    - Plan consistency check against existing `cli/flake.nix`, `cli/Cargo.toml`, and root `flake.nix` wiring.
  - Contract decisions (2026-03-02):
    - Nix package contract (to implement in T03): nested CLI flake will expose `packages.sce` and set `packages.default = packages.sce` so `nix build ./cli#default` yields the `sce` release artifact.
    - Nix app contract (to implement in T03): nested CLI flake will expose `apps.sce` that runs the packaged `sce` binary with argument passthrough (`nix run ./cli#sce -- ...`).
    - Binary naming contract: install/build surfaces continue to target binary name `sce` for both Cargo and Nix outputs.
    - Root flake ergonomics contract: repository-root pass-through aliasing (for example `.#sce`) is optional and only added if needed to keep root wiring coherent.
    - Cargo readiness policy (to implement in T04): keep `publish = false` until first-publish prerequisites are complete; add/confirm crates.io-facing metadata before flipping publish posture.
    - Local Cargo install contract: treat `cargo install --path cli --locked` as the supported local install command, with full validation evidence captured in later verification tasks.
  - Evidence (2026-03-02):
    - Contract alignment reviewed against current files: `cli/flake.nix`, `cli/Cargo.toml`, and `flake.nix` (root).

- [x] T02: Add Rust overlay-backed toolchain in `cli/flake.nix` (status:done)
  - Task ID: T02
  - Goal: Introduce a Rust overlay and pin/select the Rust toolchain used by CLI checks/builds.
  - Boundaries (in/out of scope):
    - In: `inputs` and `pkgs` composition updates in `cli/flake.nix`, minimal check wiring updates needed to consume the overlay toolchain.
    - Out: unrelated dev-shell policy changes in the repository root flake.
  - Done when:
    - CLI flake imports and applies the Rust overlay deterministically.
    - Check/build derivations use the intended overlay-provided toolchain.
  - Verification notes (commands or checks):
    - `nix flake check ./cli`
    - `nix eval ./cli#checks.${builtins.currentSystem}.cli-setup-command-surface.name`
  - Implementation notes (2026-03-02):
    - Added `rust-overlay` input (`oxalica/rust-overlay`) to `cli/flake.nix` and wired it to follow `nixpkgs`.
    - Updated `pkgs` import to apply `rust-overlay.overlays.default`.
    - Introduced `rustToolchain = pkgs.rust-bin.stable.latest.default.override { extensions = [ "rustfmt" ]; }`.
    - Introduced `rustPlatform = pkgs.makeRustPlatform { cargo = rustToolchain; rustc = rustToolchain; }` and migrated CLI check derivation to `rustPlatform.buildRustPackage`.
    - Ensured check environment uses the overlay toolchain via `nativeBuildInputs = [ rustToolchain ]`.
    - Accepted deterministic lock update in `cli/flake.lock` for the new `rust-overlay` input.
  - Evidence (2026-03-02):
    - `nix flake check ./cli` passed.
    - `nix eval ./cli#checks.x86_64-linux.cli-setup-command-surface.name` returned `"sce-cli-setup-command-surface-check-0.1.0"`.

- [x] T03: Expose Nix release package and runnable app for `sce` (status:done)
  - Task ID: T03
  - Goal: Add package/app outputs that produce and run a release build of the CLI via Nix.
  - Boundaries (in/out of scope):
    - In: `packages.<name>`/`packages.default` and `apps.<name>` wiring in `cli/flake.nix`; root flake pass-through only if required for repository-level ergonomics.
    - Out: non-CLI package outputs.
  - Done when:
    - `nix build` can produce the CLI binary from flake outputs.
    - `nix run` executes the packaged `sce` binary from flake outputs.
  - Verification notes (commands or checks):
    - `nix build ./cli#default`
    - `nix run ./cli#sce -- --help`
    - If root aliasing is added: `nix build .#sce` and `nix run .#sce -- --help`
  - Implementation notes (2026-03-02):
    - Added a release package derivation in `cli/flake.nix` as `scePackage` via `rustPlatform.buildRustPackage` with `sourceRoot = "source/cli"` and the existing `cli/Cargo.lock`.
    - Exposed package outputs as `packages.sce` and `packages.default = packages.sce` in the nested CLI flake.
    - Exposed runnable app output as `apps.sce` targeting `${scePackage}/bin/sce`.
    - Kept root flake aliasing unchanged (optional per contract) because nested flake outputs satisfy this task's install/run contract.
  - Evidence (2026-03-02):
    - `nix build ./cli#default` passed and produced the `sce` package derivation.
    - `nix run ./cli#sce -- --help` passed and printed CLI help from the packaged binary.
    - `nix flake check ./cli` passed after wiring new `packages`/`apps` outputs (with a non-blocking app `meta` warning).

- [x] T04: Prepare Cargo release/install path for local and future crates.io (status:done)
  - Task ID: T04
  - Goal: Ensure crate metadata and release-build guidance support immediate local install and future crates.io publishing.
  - Boundaries (in/out of scope):
    - In: `cli/Cargo.toml` package metadata needed for publish readiness, `cli/README.md` install guidance updates.
    - Out: actual crates.io publish execution, ownership transfer, token management, or release automation.
  - Done when:
    - Local install command is documented and validated: `cargo install --path cli`.
    - Future crates.io path is explicit (including what must change before first publish).
  - Verification notes (commands or checks):
    - `cargo build --manifest-path cli/Cargo.toml --release`
    - `cargo install --path cli --locked`
  - Implementation notes (2026-03-02):
    - Added crates.io-facing package metadata to `cli/Cargo.toml` (`description`, `license`, `repository`, `homepage`, `documentation`, `readme`, `keywords`, `categories`) while keeping `publish = false` per readiness policy.
    - Updated `cli/README.md` with explicit install/release guidance for Cargo (`cargo install --path cli --locked`, `cargo build --release`) and nested flake release surfaces (`nix build ./cli#default`, `nix run ./cli#sce -- --help`).
    - Documented the future crates.io path as readiness-only: keep publish disabled now; before first publish, flip publish posture, verify metadata accuracy, and run `cargo publish --dry-run`.
  - Evidence (2026-03-02):
    - `cargo check --manifest-path cli/Cargo.toml` passed (non-blocking pre-existing dead-code warnings in `cli/src/services/setup.rs`).
    - `cargo build --manifest-path cli/Cargo.toml --release` passed.
    - `cargo install --path cli --locked` passed and installed `sce` to cargo bin.

- [ ] T05: Sync context to current install/build contracts (status:todo)
  - Task ID: T05
  - Goal: Update context records so future sessions understand the new Nix/Cargo install surfaces and release-build flow.
  - Boundaries (in/out of scope):
    - In: directly affected `context/` files (likely `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`).
    - Out: speculative roadmap text not supported by code/docs changes.
  - Done when:
    - Context accurately states how CLI release build/install works with Nix and Cargo.
    - Crates.io status is represented as future-ready (not yet published) unless implementation scope changes.
  - Verification notes (commands or checks):
    - Context/code consistency spot-check between flake/Cargo/docs and updated context entries.

- [ ] T06: Validation and cleanup (status:todo)
  - Task ID: T06
  - Goal: Run complete verification for the new build/install pathways and leave the workspace clean of temporary artifacts.
  - Boundaries (in/out of scope):
    - In: final command checks, evidence capture in plan updates, cleanup of temporary outputs created during verification.
    - Out: adding new features or changing scope after validation.
  - Done when:
    - Nix check/build/run and Cargo release/install checks pass.
    - Success criteria have explicit verification evidence.
    - Temporary verification artifacts are removed.
  - Verification notes (commands or checks):
    - `nix flake check`
    - `nix flake check ./cli`
    - `nix build ./cli#default`
    - `nix run ./cli#sce -- --help`
    - `cargo build --manifest-path cli/Cargo.toml --release`
    - `cargo install --path cli --locked`

## 5) Open questions
- None.

## Assumptions
- "creates.io" is interpreted as "crates.io".
- This plan prepares crates.io readiness but does not publish in this change.
