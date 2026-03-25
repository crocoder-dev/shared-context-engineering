# SCE CLI Release Artifact Contract

This file captures the implemented shared release artifact foundation from `context/plans/sce-cli-first-install-channels.md` task `T02`.

## Canonical artifact set

- `nix run .#release-artifacts -- --version <semver> --out-dir <path>` builds the current-platform packaged CLI release assets.
- The per-platform archive name is `sce-v<version>-<target-triple>.tar.gz`.
- The matching per-platform checksum file is `sce-v<version>-<target-triple>.tar.gz.sha256`.
- The matching per-platform metadata fragment is `sce-v<version>-<target-triple>.json`.
- `nix run .#release-manifest -- --version <semver> --artifacts-dir <path> --out-dir <path>` merges per-platform fragments into:
  - `sce-v<version>-release-manifest.json`
  - `sce-v<version>-SHA256SUMS`

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

## Workflow topology

- `.github/workflows/release-sce.yml` is the CLI release orchestrator for tag pushes and manual bump/tag creation.
- Platform builds are split into separate reusable workflow files:
  - `.github/workflows/release-sce-linux.yml`
  - `.github/workflows/release-sce-macos-intel.yml`
  - `.github/workflows/release-sce-macos-arm.yml`
- `.github/workflows/release-agents.yml` remains Tessl/agent-file release automation and is not the CLI release workflow.

## Current supported release targets in automation

- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

## Downstream channel implication

- The implemented npm channel consumes this artifact naming and manifest/checksum shape rather than inventing a channel-specific archive format.
- Any future additional install channel should reuse this artifact contract unless a later decision explicitly supersedes it.
