# Optional install-channel integration-test entrypoint

The repository exposes an explicit opt-in flake app for install-channel integration coverage:

- `nix run .#install-channel-integration-tests -- --channel <npm|bun|cargo|all>`

## Current contract

- The public entrypoint remains `apps.install-channel-integration-tests` in `flake.nix`.
- It stays intentionally excluded from `checks.<system>` and therefore does not run during default `nix flake check`.
- The accepted channel selector contract remains `npm`, `bun`, `cargo`, or `all`.

## Current implementation split

- `flake.nix` owns only the public opt-in app surface plus thin delegation to the standalone Rust runner.
- `integrations/install/` contains the standalone Rust runner binary, `install-channel-integration-tests`, with the same `--channel <npm|bun|cargo|all>` selector contract.
- The flake app exports `SCE_INSTALL_CHANNEL_SCE_BIN` pointing at the packaged `sce` binary, adds Node/npm to PATH, and then execs the Rust runner so the public Nix entrypoint stays stable while orchestration stays out of inline flake shell code.
- The Rust runner now owns the shared harness behavior: channel-scoped temporary roots, isolated `HOME`/`XDG_*`/npm/Bun/Cargo state, executable resolution inside the isolated PATH, and centralized deterministic command assertions for the installed `sce` binary.
- The npm channel now stages a local `@crocoder-dev/sce@.version` package fixture with the packaged `sce` binary preloaded into `runtime/`, installs that tarball into isolated npm state with download skipping enabled, and then reuses the shared Rust execution path to run both `sce version` and `sce doctor --format json` against the installed npm launcher path; the current `doctor` check only requires successful completion, not output inspection.
- The Bun channel now reuses the same staged local npm-package fixture shape as npm, performs a real isolated `bun add --global <tarball>` install with download skipping enabled, and then reuses the shared Rust execution path to run deterministic `sce version` against the installed Bun launcher path.
- The Cargo channel now performs a real isolated `cargo install --path cli --locked` install from the repository root, reuses the shared Rust execution path to run deterministic `sce version` against the installed Cargo binary, and completes the existing binary install-channel coverage for npm, Bun, and Cargo.

## Current execution posture

- The Rust runner already has dedicated default-flake checks: `integrations-install-fmt`, `integrations-install-clippy`, and `integrations-install-tests`.
- The opt-in app remains outside default `nix flake check`.
- Real npm, Bun, and Cargo install orchestration now run through the Rust runner behind the unchanged selector contract for the existing binary install channels. Flatpak validation/build orchestration is implemented as a separate Nix-owned source-build path (umbrella `sce-flatpak <subcommand>` for `validate` / `prepare-local-manifest`, `release-flatpak-package`, `release-flatpak-bundle`, and the `flatpak-static-validation` flake check) rather than part of this app's current selector contract.

See also: [../overview.md](../overview.md), [../architecture.md](../architecture.md), [../patterns.md](../patterns.md)
