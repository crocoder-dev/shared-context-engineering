# SCE CLI Cargo Distribution Contract

This file captures the implemented Cargo distribution slice from `context/plans/sce-cli-first-install-channels.md` task `T06`.

## Package posture

- The published crate name is `sce`.
- `cli/Cargo.toml` keeps crates.io-facing metadata enabled for publication.
- `cli/README.md` is the crate-facing install guidance source referenced by Cargo/crates.io surfaces.

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
