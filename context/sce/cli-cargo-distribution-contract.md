# SCE CLI Cargo Distribution Contract

This file captures the implemented Cargo distribution slice from `context/plans/sce-cli-first-install-channels.md` task `T06`.

## Package posture

- The published crate name is `sce`.
- `cli/Cargo.toml` keeps crates.io-facing metadata enabled for publication.
- `cli/README.md` is the crate-facing install guidance source referenced by Cargo/crates.io surfaces.
- `cli/assets/generated/` is an ephemeral crate-local mirror of generated setup/config assets prepared from canonical `config/` outputs before Cargo packaging/builds; `cli/build.rs` and `cli/src/services/config.rs` consume that mirror instead of relying on committed crate snapshots or workspace-external generated paths.

## Publish workflow

- `.github/workflows/publish-crates.yml` is the dedicated crates.io publish workflow.
- It triggers from `release.published` and from manual `workflow_dispatch`.
- It validates parity across the requested release tag (`v<version>`), repo-root `.version`, and `cli/Cargo.toml` before any publish step runs.
- It copies the checked-out repository into a temporary clean workspace, prepares the ephemeral `cli/assets/generated/` mirror there from canonical `config/` outputs, and runs Cargo packaging/publish from that clean workspace.
- Manual dispatch supports `dry_run: true` by default so maintainers can verify packaging without publishing.
- Real publication requires the `CARGO_REGISTRY_TOKEN` secret and runs `nix develop -c cargo publish --manifest-path <temp-copy>/cli/Cargo.toml --locked` from the clean temporary workspace without mutating package metadata.

## Supported Cargo install paths

- crates.io: `cargo install sce`
- Git repository: `cargo install --git https://github.com/crocoder-dev/shared-context-engineering sce --locked`
- Local checkout: `cargo install --path cli --locked`

## Scope notes

- `cargo binstall` is not part of the current implemented Cargo distribution slice.
- Cargo remains a first-wave install channel.
- Nix-managed validation remains the required verification baseline for repo task execution even when the user-facing install path is Cargo.

## Verification baseline

- `nix run .#pkl-check-generated`
- `nix flake check`

See also: [cli-first-install-channels-contract.md](./cli-first-install-channels-contract.md), [../overview.md](../overview.md)
