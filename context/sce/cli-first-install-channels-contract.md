# SCE CLI Install Channel Contract

This file captures the current install/distribution contract for the `sce` CLI. It began with the first-wave `Nix`/`Cargo`/`npm` channel contract from `context/plans/sce-cli-first-install-channels.md` task `T01`; the current active model also approves Flatpak as an official source-built channel through `context/plans/nix-orchestrated-flatpak.md` task `T01`.

## Canonical naming

- The canonical CLI/binary/package name for this rollout is `sce`.
- Legacy `sce-editor` references are migration debt and should be removed or explicitly mapped when a touched surface still uses the old name.
- New install assets, docs, packaging, and automation should default to `sce` naming.

## Supported channels for the current implementation stage

- Repo-flake `Nix`
- `Cargo`
- `npm`
- Flatpak application ID `dev.crocoder.sce`

`Homebrew` is deferred from the current implementation stage.

No other install channels are in scope for the current implementation stage.

## Channel ownership rules

- `flake.nix` is the first-class Nix run/install surface for this rollout.
- Nix-managed build/release entrypoints remain the required build source for existing binary release automation.
- Nix may also orchestrate Flatpak tooling, local source overrides, validation, and local builds, but the Flatpak package itself must build `sce` from source inside Flatpak.
- Repo-root `.version` is the canonical checked-in release version authority for the Nix, Cargo, npm, and release-artifact surfaces.
- GitHub Releases are the canonical publication surface for release archives and manifest/checksum assets produced for the version declared in `.version`.
- `npm` consumes release artifacts produced by Nix-managed build/release flows.
- `Cargo` is a first-class supported install path and its publish metadata should stay aligned to `.version` without workflow-side version bumping.
- npm registry publication should also consume the checked-in package version aligned to `.version` without workflow-side version bumping.
- Flatpak is source-built and must not consume `nix build .#sce`, GitHub Release archives, npm native binaries, or any other prebuilt `sce` artifact.
- The first Flatpak iteration is Flathub-ready packaging plus Nix-backed local build/check tooling and docs only; CI publishing, automatic Flathub submission, GitHub Release Flatpak assets, and release-version bumping are not part of the current scope.
- `Homebrew` can return in a later plan stage, but it is not part of current code truth.

## Flatpak source-build contract

- The Flatpak application ID is `dev.crocoder.sce`.
- The Flatpak manifest model is a Flathub-style release-source manifest plus a Nix-generated local checkout-source manifest/override for local builds from the current checkout.
- The Flatpak build must use the standard source-build pattern for Rust CLI applications, including the Freedesktop SDK Rust extension, offline Cargo dependency sources generated from `cli/Cargo.lock`, and build-time preparation of `cli/assets/generated/config/**` from checked-in `config/` inputs.
- Runtime Git access uses a host `git` bridge: install `/app/bin/git` as a wrapper that delegates to `flatpak-spawn --host git`, with the required Flatpak permission for `org.freedesktop.Flatpak`.
- Nix-provided Flatpak commands/checks are orchestration only: they may provide `flatpak-builder`, AppStream validation, Flatpak linting, wrapper scripts, and generated local manifests/overrides, but they must not bypass the Flatpak source build with a Nix-built `sce` binary.

## Implemented Flatpak packaging surface

- The canonical checked-in Flatpak manifest is `packaging/flatpak/dev.crocoder.sce.yml`.
- The manifest currently uses `org.freedesktop.Platform` / `org.freedesktop.Sdk` runtime `25.08`, appends the `org.freedesktop.Sdk.Extension.rust-stable` toolchain, and runs `cargo --offline build --locked --release --manifest-path cli/Cargo.toml --bin sce` inside Flatpak.
- The manifest prepares crate-local generated setup assets by running `scripts/prepare-cli-generated-assets.sh "$PWD"` before Cargo builds, preserving `config/` as the source of truth for generated assistant config inputs.
- Flatpak Cargo dependency sources are checked in at `packaging/flatpak/cargo-sources.json`, generated from `cli/Cargo.lock`; crates.io dependencies are represented as `.crate` archives with lockfile SHA-256 checksums, and the Turso git dependency is represented as a pinned git source plus local path patches.
- AppStream metadata lives at `packaging/flatpak/dev.crocoder.sce.metainfo.xml` and declares a console application with `<provides><binary>sce</binary></provides>`.
- The host Git bridge source lives at `packaging/flatpak/git-host-bridge`; the manifest installs it as `/app/bin/git`.
- The current runtime permissions are `--share=network`, `--filesystem=home`, `--talk-name=org.freedesktop.Flatpak`, and `--talk-name=org.freedesktop.secrets`.

## Implemented Nix-backed Flatpak tooling surface

- `packaging/flatpak/sce-flatpak.sh` owns local Flatpak orchestration for this repo: lightweight validation, generated local checkout-source manifests, and opt-in `flatpak-builder` execution.
- Linux flake apps expose that script as:
  - `nix run .#flatpak-validate` for static source-build checks, local-manifest generation checks, and `appstreamcli validate --pedantic --no-net`.
  - `nix run .#flatpak-local-manifest` for generating a temporary manifest that replaces the release git source with a Flatpak `type: dir` source pointed at the current checkout.
  - `nix run .#flatpak-build -- --help` / `nix run .#flatpak-build -- ...` for explicit local `flatpak-builder` source builds from that generated local manifest.
- `checks.<linux>.flatpak-static-validation` runs the lightweight validation path during default `nix flake check`; it does not run a full Flatpak build or require network access.
- The Linux dev shell includes `appstreamcli`, `flatpak`, and `flatpak-builder`, and its banner lists the Flatpak flake apps alongside existing repo app entrypoints.
- The checked-in release manifest remains Flathub-style and source-built; Nix orchestration only supplies tools and generated local manifests and must not provide a prebuilt `sce` binary to the Flatpak package.

## Explicitly out of scope in this phase

- AUR
- `.deb`
- `.rpm`
- AppImage
- Other Linux package-manager specific channels not listed above

## Implementation implications for later tasks

- Release assets must be named and published for `sce`.
- GitHub release packaging must consume the checked-in `.version` value instead of inventing a semver bump during workflow execution.
- Cargo and npm registry publication belong to separate downstream publish stages rather than the GitHub release-packaging job.
- Flatpak packaging tasks must preserve source-built semantics and the release-source-plus-local-override model instead of wrapping existing binary release artifacts.
- Flatpak publication automation remains explicitly deferred until a later approved plan.
- Unsupported channels in older docs should be removed or explicitly deferred rather than implied as active support.
- Later packaging tasks should implement the contract above rather than redefining channel scope per channel.
