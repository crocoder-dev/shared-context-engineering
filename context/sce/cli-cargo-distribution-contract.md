# SCE CLI Cargo Distribution Contract

This file captures the implemented Cargo distribution slice from `context/plans/sce-cli-first-install-channels.md` task `T06`.

## Package posture

- The published crate name is `shared-context-engineering`; it installs the `sce` binary.
- `cli/Cargo.toml` keeps crates.io-facing metadata enabled for publication.
- `cli/README.md` is the crate-facing install guidance source referenced by Cargo/crates.io surfaces.
- Repository Cargo builds require a pre-Cargo generated-input directory through `SCE_CLI_GENERATED_INPUT_DIR`; `cli/build.rs` never invokes Pkl and does not consume `cli/assets/generated/` or the packaging fallback when canonical repository sources are present.
- The handoff contains `pkl-generated/`, `SHA256SUMS` for that exact payload, and `INPUTS.SHA256SUMS` for canonical `config/pkl` plus referenced `config/lib` inputs. `cli/build.rs` rejects missing, incomplete, modified, or stale handoffs, then copies the validated payload into `OUT_DIR/pkl-generated`.
- `scripts/run-cli-cargo.sh` is the supported repository Cargo boundary. It generates canonical Pkl twice into a fresh temporary directory, rejects nondeterminism, writes both inventories, exports the handoff only for the requested Cargo process, forwards Cargo arguments unchanged, and removes the directory after success, failure, or a handled signal. Build, targeted-test, Clippy, run, and `cargo install --path cli` workflows use this wrapper rather than direct Cargo.
- Before packaging, `scripts/prepare-cli-generated-assets.sh` evaluates canonical Pkl twice, rejects nondeterministic output, stages generated targets plus hooks, schemas, and migrations under ignored `cli/package-fallback/`, and writes `SHA256SUMS` for the exact payload.
- Published crates include only the packaging fallback, Rust sources, and crate metadata. In an unpacked crate, `cli/build.rs` validates the fallback inventory and copies it into the consumer's `OUT_DIR`; downstream Cargo builds do not require Pkl or parent repository paths.
- Missing or changed fallback files fail the build with guidance to recreate the package through the preparation script. When canonical repository Pkl sources exist, `build.rs` requires the generated-input handoff and does not silently use the packaging fallback.

## Publish workflow

- `.github/workflows/publish-crates.yml` is the dedicated crates.io publish workflow.
- It triggers from `release.published` and from manual `workflow_dispatch`.
- It validates parity across the requested release tag (`v<version>`), repo-root `.version`, and `cli/Cargo.toml` before any publish step runs.
- It copies the checked-out repository into a temporary clean workspace and runs the package-fallback preparation script from that copy's Nix dev shell before Cargo packaging or publication.
- Manual dispatch supports `dry_run: true` by default so maintainers can verify packaging without publishing.
- Manual dispatch also supports `prerelease: true`; GitHub prerelease events are treated as prerelease publish runs automatically.
- When a publish run is marked prerelease, the workflow requires `.version` to include semver prerelease metadata such as `-alpha.1`, `-beta.1`, or `-rc.1` before publishing. Crates.io has no npm-style dist-tag channel, so the semver prerelease version is the crate prerelease marker.
- Real publication requires the `CARGO_REGISTRY_TOKEN` secret and runs `nix develop -c cargo publish --manifest-path <temp-copy>/cli/Cargo.toml --locked` from the clean temporary workspace without mutating package metadata.

## Supported Cargo install paths

- crates.io: `cargo install shared-context-engineering --locked`
- Local checkout: `./scripts/run-cli-cargo.sh install --path cli --locked`

Direct `cargo install --git` is unsupported because Cargo provides no repository-owned pre-generation boundary before compiling the checkout.

## Scope notes

- `cargo binstall` is not part of the current implemented Cargo distribution slice.
- Cargo remains a first-wave install channel.
- Nix-managed validation remains the required verification baseline for repo task execution even when the user-facing install path is Cargo.

## Verification baseline

- `nix run .#pkl-check-generated`
- `nix flake check`

See also: [cli-first-install-channels-contract.md](./cli-first-install-channels-contract.md), [../overview.md](../overview.md)
