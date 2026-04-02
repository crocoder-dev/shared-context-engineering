# SCE CLI Release Artifact Contract

This file captures the current shared release artifact foundation plus the approved release-authority constraints that later release-topology tasks must satisfy.

## Canonical artifact set

- `nix run .#release-artifacts -- --version <semver> --out-dir <path>` builds the current-platform packaged CLI release assets.
- The per-platform archive name is `sce-v<version>-<target-triple>.tar.gz`.
- The matching per-platform checksum file is `sce-v<version>-<target-triple>.tar.gz.sha256`.
- The matching per-platform metadata fragment is `sce-v<version>-<target-triple>.json`.
- `nix run .#release-manifest -- --version <semver> --artifacts-dir <path> --out-dir <path>` merges per-platform fragments into:
  - `sce-v<version>-release-manifest.json`
  - `sce-v<version>-release-manifest.json.sig`
  - `sce-v<version>-SHA256SUMS`
- `nix run .#release-manifest` signs the merged release manifest with a non-repo private key supplied via `SCE_RELEASE_MANIFEST_SIGNING_KEY` or `--signing-key-file <path>`.

## Archive contents

- Each archive contains a deterministic top-level directory named `sce-v<version>-<target-triple>/`.
- That directory currently includes:
  - `bin/sce`
  - `LICENSE`
  - `README.md`

## Determinism rules

- Release archives are built from the root flake package output (`nix build .#default`).
- Tarball creation uses stable file ordering plus fixed ownership and mtime metadata.
- Gzip output is emitted with deterministic headers.
- Checksum files use SHA-256 and the standard `<hash><two spaces><filename>` line format.
- Manifest signatures are detached base64-encoded RSA-SHA256 signatures over the exact emitted `sce-v<version>-release-manifest.json` bytes.

## Workflow topology

- GitHub Releases are the canonical publication surface for `sce` release archives, checksums, metadata fragments, and merged release-manifest assets.
- Repo-root `.version` is the canonical checked-in release version source for release tags, archive names, and packaged metadata across the release flow.
- Release packaging consumes the checked-in version directly; workflow-side semver bump generation is not part of the current contract.
- Cargo/crates.io and npm registry publication are separate downstream publish stages and are not part of the canonical GitHub release artifact assembly job.
- `.github/workflows/release-sce.yml` remains the CLI release orchestrator that assembles GitHub release assets from the reusable platform build workflows.
- The release orchestrator injects the non-repo manifest-signing private key through the `SCE_RELEASE_MANIFEST_SIGNING_KEY` secret when assembling release-level metadata.
- Manual GitHub release dispatch resolves the tag from checked-in `.version` and refuses to create the tag when `.version`, `cli/Cargo.toml`, and `npm/package.json` are not already aligned.
- Tag-triggered release execution also refuses to proceed when the pushed tag does not equal `v<.version>` or when checked-in Cargo/npm package metadata drift from `.version`.
- `nix run .#release-artifacts` fails fast when the requested `--version` disagrees with `.version`, `cli/Cargo.toml`, `npm/package.json`, or the built CLI `sce version` output.
- `nix run .#release-artifacts` also rejects host OS/architecture pairs outside the current three-target release matrix; macOS Intel (`Darwin:x86_64`) is no longer a supported current-platform packaging host.
- The release orchestrator passes the resolved checked-in version through to the platform builds, merged release-manifest assembly, and npm tarball packaging without mutating package versions during workflow execution.
- Platform builds are split into separate reusable workflow files:
  - `.github/workflows/release-sce-linux.yml`
  - `.github/workflows/release-sce-linux-arm.yml`
  - `.github/workflows/release-sce-macos-arm.yml`
- The reusable Linux ARM workflow builds canonical `aarch64-unknown-linux-gnu` artifacts on an ARM Linux runner, and the top-level release orchestrator now requires and publishes that lane alongside the other platform workflows.
- `.github/workflows/release-agents.yml` remains Tessl/agent-file release automation and is not the CLI release workflow.

## Current orchestrated release targets in automation

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `aarch64-apple-darwin`

## Current supported release matrix

- Linux x64 release artifacts are published as `x86_64-unknown-linux-gnu`.
- Linux ARM release artifacts are published as `aarch64-unknown-linux-gnu`.
- macOS ARM release artifacts are published as `aarch64-apple-darwin`.
- The merged release manifest and combined checksum outputs include those three current targets for each published `sce` release.

## Downstream channel implication

- The implemented npm channel consumes this artifact naming and manifest/checksum shape rather than inventing a channel-specific archive format.
- The implemented npm channel also depends on the published `sce-v<version>-release-manifest.json.sig` asset so manifest-provided checksums are only trusted after signature verification.
- Any future additional install channel should reuse this artifact contract unless a later decision explicitly supersedes it.
