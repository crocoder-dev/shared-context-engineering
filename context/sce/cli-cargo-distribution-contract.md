# SCE CLI Cargo Distribution Contract

This file captures the implemented Cargo distribution slice from `context/plans/sce-cli-first-install-channels.md` task `T06`.

## Package posture

- The published crate name is `shared-context-engineering`; it installs the `sce` binary.
- `cli/Cargo.toml` keeps crates.io-facing metadata enabled for publication.
- `cli/README.md` is the crate-facing install guidance source referenced by Cargo/crates.io surfaces.
- Repository Cargo builds evaluate canonical Pkl inputs directly into Cargo `OUT_DIR` and do not consume `cli/assets/generated/` or the packaging fallback.
- Before packaging, `scripts/prepare-cli-generated-assets.sh` evaluates canonical Pkl twice, rejects nondeterministic output, stages generated targets plus hooks, schemas, and migrations under ignored `cli/package-fallback/`, and writes `SHA256SUMS` for the exact payload.
- Published crates include only the packaging fallback, Rust sources, and crate metadata. In an unpacked crate, `cli/build.rs` validates the fallback inventory and copies it into the consumer's `OUT_DIR`; downstream Cargo builds do not require Pkl or parent repository paths.
- Missing or changed fallback files fail the build with guidance to recreate the package through the preparation script. When canonical repository Pkl sources exist, `build.rs` always prefers direct generation and does not silently fall back when Pkl execution fails.

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
- Git repository: `cargo install --git https://github.com/crocoder-dev/shared-context-engineering shared-context-engineering --locked`
- Local checkout: `cargo install --path cli --locked`

## Scope notes

- `cargo binstall` is not part of the current implemented Cargo distribution slice.
- Cargo remains a first-wave install channel.
- Nix-managed validation remains the required verification baseline for repo task execution even when the user-facing install path is Cargo.

## Verification baseline

- `nix run .#pkl-check-generated`
- `nix flake check`

See also: [cli-first-install-channels-contract.md](./cli-first-install-channels-contract.md), [../overview.md](../overview.md)
