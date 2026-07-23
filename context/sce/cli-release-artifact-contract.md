# SCE CLI Release Artifact Contract

This file captures the current shared release artifact foundation plus the approved release-authority constraints that later release-topology tasks must satisfy.

## Canonical artifact set

- `nix run .#release-artifacts -- --version <semver> --out-dir <path>` builds the current-platform packaged CLI release assets, prepares the copied `bin/sce` binary, and audits it before archive creation.
- `nix run .#native-portability-audit -- --binary <path> [--platform auto|linux|macos]` audits a native `sce` binary for forbidden `/nix/store/` runtime references and reports offending references; macOS uses dynamic library install-name inspection, while Linux combines ELF dynamic metadata with focused binary-string scanning for shared-object references.
- The per-platform archive name is `sce-v<version>-<target-triple>.tar.gz`.
- The matching per-platform checksum file is `sce-v<version>-<target-triple>.tar.gz.sha256`.
- The matching per-platform metadata fragment is `sce-v<version>-<target-triple>.json`.
- `nix run .#release-manifest -- --version <semver> --artifacts-dir <path> --out-dir <path>` merges per-platform fragments into:
  - `sce-v<version>-release-manifest.json`
  - `sce-v<version>-release-manifest.json.sig`
  - `sce-v<version>-SHA256SUMS`
- `nix run .#release-manifest` signs the merged release manifest with a non-repo private key supplied via `SCE_RELEASE_MANIFEST_SIGNING_KEY` or `--signing-key-file <path>`.

## Implemented Flatpak source-manifest artifact set

- Flatpak GitHub Release assets are approved as source-manifest packaging metadata for the source-built `dev.crocoder.sce` Flatpak channel.
- `nix run .#release-flatpak-package -- --version <semver> --out-dir <path>` is the Linux root-flake app that emits the Flatpak asset set from checked-in `packaging/flatpak/` source.
- The approved Flatpak asset names are:
  - `sce-v<version>-flatpak-manifest.tar.gz`
  - `sce-v<version>-flatpak-manifest.tar.gz.sha256`
  - `sce-v<version>-flatpak.json`
- The Flatpak tarball contains a deterministic top-level `sce-v<version>-flatpak-manifest/` directory with `dev.crocoder.sce.yml`, `dev.crocoder.sce.metainfo.xml`, `cargo-sources.json`, and `git-host-bridge`.
- The staged packaged manifest pins the release git source to the release commit without mutating the checked-in Flatpak manifest.
- The metadata JSON describes `asset_type`, `app_id`, `version`, `release_commit`, `manifest_name`, `package_file`, `checksum_file`, `checksum_sha256`, `packaged_support_files`, and `packaged_files`.
- Flatpak source-manifest assets are not native binary archives and are not included in the signed native `sce-v<version>-release-manifest.json` consumed by npm.
- The Flatpak source-manifest asset set does not publish a prebuilt `sce` binary, `.flatpak` bundle, OSTree repository, AppImage, `.deb`, `.rpm`, AUR package, Homebrew asset, or Flathub submission. Source-built `.flatpak` bundle assets are published in a separate approved asset set (see below).

## Implemented Flatpak bundle artifact set

- Flatpak GitHub Release assets now also include source-built `.flatpak` bundle assets alongside source-manifest packaging metadata.
- The bundle is built from source inside Flatpak using `flatpak-builder` + `flatpak build-bundle`, not from a Nix-built or pre-compiled binary.
- Approved bundle asset names per architecture:
  - `sce-v<version>-x86_64.flatpak` + `.sha256` + `.json`
  - `sce-v<version>-aarch64.flatpak` + `.sha256` + `.json`
- The JSON metadata describes `asset_type: flatpak-bundle`, architecture field (`x86_64` / `aarch64`), app ID `dev.crocoder.sce`, version, and SHA-256 checksum.
- Bundle assets are separate from native binary release archives, the signed native release manifest consumed by npm, and the existing Flatpak source-manifest packaging assets.
- The release-bundle command (`packaging/flatpak/sce-flatpak.sh release-bundle`) is exposed as `nix run .#release-flatpak-bundle -- --version <semver> --arch <arch> --out-dir <path>`. GitHub workflow upload for these assets is implemented (`.github/workflows/release-sce-linux.yml` / `release-sce-linux-arm.yml` build and upload; `release-sce.yml` assembles and publishes with the GitHub Release). The bash script is a thin imperative orchestrator around `flatpak-builder` + `flatpak build-bundle`; the manifest YAML it consumes is rendered from a Nix expression (`nix/flatpak/manifest.nix`) and the `cargo-sources.json` it consumes is generated from `cli/Cargo.lock` by a Nix derivation (`nix/flatpak/cargo-sources.nix`), both guarded by parity checks (`flatpak-manifest-parity`, `cargo-sources-parity`) in `nix flake check`. Static manifest validation and release-version parity validation run from dedicated Nix-built Python scripts rather than from inline bash heredocs, while local-checkout manifest validation is a Nix-wrapped Bash script used by `sce-flatpak prepare-local-manifest`.

## Archive contents

- Each archive contains a deterministic top-level directory named `sce-v<version>-<target-triple>/`.
- That directory currently includes:
  - `bin/sce`
  - `LICENSE`
  - `README.md`

## Native binary portability gate

- `release-artifacts` copies `result/bin/sce` into the staged archive tree before mutation/audit so the Nix build output remains unchanged.
- On macOS release hosts, the staged binary is inspected with `otool -L`; Nix-store `libiconv.*.dylib` install names are rewritten to `/usr/lib/...` with `install_name_tool`, and a mutated binary is ad-hoc re-signed with `codesign --force --sign -` before audit.
- Any remaining macOS `/nix/store/` install name fails packaging before tarball creation.
- On Linux release hosts, the staged binary is audited before tarball creation; forbidden `/nix/store/` references in ELF dynamic metadata or shared-object string references fail packaging before uploadable artifacts are emitted.
- After preparation/audit, `release-artifacts` smoke-checks the staged binary with `sce version --format json` before archiving.

## Determinism rules

- Release archives are built from the static-musl release output (`nix build .#sce-release`; `nix run .#release-artifacts` builds via `.#sce-release`), not the native `.#default`/`.#sce` package.
- Tarball creation uses stable file ordering plus fixed ownership and mtime metadata.
- Gzip output is emitted with deterministic headers.
- Checksum files use SHA-256 and the standard `<hash><two spaces><filename>` line format.
- Flatpak source-manifest packaging uses the same stable tar/gzip/checksum conventions while staging only release manifest/support files, not build outputs.
- Manifest signatures are detached base64-encoded RSA-SHA256 signatures over the exact emitted `sce-v<version>-release-manifest.json` bytes.

## Workflow topology

- GitHub Releases are the canonical publication surface for `sce` release archives, checksums, metadata fragments, merged release-manifest assets, npm package assets, and approved Flatpak source-manifest assets.
- Repo-root `.version` is the canonical checked-in release version source for release tags, archive names, and packaged metadata across the release flow.
- Release packaging consumes the checked-in version directly; workflow-side semver bump generation is not part of the current contract.
- Cargo/crates.io and npm registry publication are separate downstream publish stages and are not part of the canonical GitHub release artifact assembly job.
- `.github/workflows/release-sce.yml` remains the CLI release orchestrator that assembles GitHub release assets from the reusable platform build workflows.
- The release orchestrator injects the non-repo manifest-signing private key through the `SCE_RELEASE_MANIFEST_SIGNING_KEY` secret when assembling release-level metadata.
- The release orchestrator also runs `nix run .#release-flatpak-package -- --version <resolved-version> --out-dir dist/flatpak` and uploads `dist/flatpak/*.tar.gz`, `dist/flatpak/*.sha256`, and `dist/flatpak/*.json` to the GitHub Release.
- Manual GitHub release dispatch resolves the tag from checked-in `.version` and refuses to create the tag when `.version`, `cli/Cargo.toml`, and `npm/package.json` are not already aligned. Manual dispatch also exposes a boolean pre-release checkbox that is passed only to `softprops/action-gh-release` as the GitHub Release-level `prerelease` flag; it does not change tag naming, release title/body/notes, generated release notes, artifact names, or version validation.
- Tag-triggered release execution also refuses to proceed when the pushed tag does not equal `v<.version>` or when checked-in Cargo/npm package metadata drift from `.version`.
- `nix run .#release-artifacts` fails fast when the requested `--version` disagrees with `.version`, `cli/Cargo.toml`, `npm/package.json`, the built CLI `sce version` output, the prepared binary's `sce version` output, or the native portability audit.
- `checks.<system>.native-portability-audit` exercises the native portability audit against deterministic clean/failing Linux string-scan fixtures and clean/failing macOS `otool -L` fixtures.
- `nix run .#release-flatpak-package` fails fast when the requested `--version` disagrees with `.version`, `cli/Cargo.toml`, `npm/package.json`, or Flatpak AppStream release metadata, and also fails when it cannot resolve a release commit from a git checkout.
- `nix run .#release-artifacts` also rejects host OS/architecture pairs outside the current three-target release matrix; macOS Intel (`Darwin:x86_64`) is no longer a supported current-platform packaging host.
- The release orchestrator passes the resolved checked-in version through to the platform builds, merged release-manifest assembly, npm tarball packaging, and Flatpak source-manifest packaging without mutating package versions during workflow execution.
- Platform builds are split into separate reusable workflow files:
  - `.github/workflows/release-sce-linux.yml`
  - `.github/workflows/release-sce-linux-arm.yml`
  - `.github/workflows/release-sce-macos-arm.yml`
- Each native reusable workflow validates its generated archive before native artifact upload by deriving the expected target-specific archive name from the resolved version, extracting it, asserting `bin/sce` is executable, running `sce version --format json`, and invoking `nix run .#native-portability-audit` with the lane's platform (`linux` or `macos`).
- The reusable Linux ARM workflow builds canonical `aarch64-unknown-linux-musl` artifacts on an ARM Linux runner, and the top-level release orchestrator now requires and publishes that lane alongside the other platform workflows.

## Current orchestrated release targets in automation

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `aarch64-apple-darwin`

## Current supported release matrix

- Linux x64 release artifacts are published as `x86_64-unknown-linux-musl`.
- Linux ARM release artifacts are published as `aarch64-unknown-linux-musl`.
- macOS ARM release artifacts are published as `aarch64-apple-darwin`.
- The merged release manifest and combined checksum outputs include those three current targets for each published `sce` release.

## Downstream channel implication

- The implemented npm channel consumes this artifact naming and manifest/checksum shape rather than inventing a channel-specific archive format.
- The implemented npm channel also depends on the published `sce-v<version>-release-manifest.json.sig` asset so manifest-provided checksums are only trusted after signature verification.
- Native archives listed in the signed release manifest for npm-supported targets must already be portable: no forbidden `/nix/store/` runtime references may remain after macOS sanitization or Linux/macOS audit, and each reusable native workflow must smoke-run the extracted `bin/sce` before upload. npm consumes those published archives as-is and does not patch dynamic-link metadata.
- Additional binary-distribution install channels should reuse this artifact contract unless a later decision explicitly supersedes it.
- Flatpak is the current approved exception to binary-artifact reuse: the Flatpak package for application ID `dev.crocoder.sce` is source-built inside Flatpak, uses a release-source manifest plus a Nix-generated local checkout-source manifest/override for local builds, and must not consume Nix-built, native GitHub Release binary archives, npm native, or other prebuilt `sce` artifacts.
- GitHub Release Flatpak assets include source-manifest package assets and source-built `.flatpak` bundle assets, both uploaded by the CLI release workflow; automatic Flathub submission and prebuilt (non-source-built) Flatpak binary/bundle assets remain out of scope.
