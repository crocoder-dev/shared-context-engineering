# Plan: fix-install-release-findings

## Change summary

Fix the install/deployment findings in the current `sce` release pipeline by making `.version` the canonical release version source, aligning Cargo and npm package versions to that source during release packaging, splitting GitHub release creation from registry deployment, and repairing user-facing docs/context so Nix, Cargo, and npm describe the real current-state delivery flow.

The target operating model for this plan is:

- `.version` is the release authority.
- GitHub release automation builds and publishes the canonical release artifacts for the version in `.version`.
- Cargo/crates.io deployment is a separate workflow that publishes the already-versioned crate without bumping version numbers.
- npm deployment is a separate workflow that publishes the already-versioned npm package without bumping version numbers.

## Success criteria

- The release pipeline no longer derives a new version by auto-incrementing tags independently of checked-in package metadata.
- The built CLI reports the same version as `.version`, and release artifact names, GitHub release tags, Cargo package metadata, and npm package metadata all agree on that same version.
- GitHub release packaging/publishing is separated from crates.io and npm registry deployment.
- A Cargo publish workflow exists that publishes the checked-in `cli/` crate for the current version without performing any version bump logic.
- An npm publish workflow exists that publishes the checked-in `npm/` package for the current version without performing any version bump logic.
- User-facing install/deployment docs and durable context describe the actual current-state repo slug, release topology, and versioning rules.

## Constraints and non-goals

- In scope: version-source-of-truth fixes, GitHub release workflow changes, crates.io publish automation, npm publish automation, and install/release doc-context repairs required by those changes.
- In scope: making `.version` the canonical release version source and enforcing parity for `cli/Cargo.toml` and `npm/package.json` in release/deploy flows.
- In scope: workflow gating so registry publishing consumes an already-prepared release version rather than inventing a new one.
- In scope: repo-slug/documentation corrections where install guidance still points at the old `crocoder-dev/sce` location.
- Out of scope: adding new install channels beyond repo-flake Nix, Cargo, and npm.
- Out of scope: replacing GitHub Releases as the canonical binary artifact host for the npm installer.
- Out of scope: changing the manifest-signing trust model beyond what is needed to keep release/deploy flows coherent.
- Out of scope: Homebrew or other package-manager integration.
- Every executable task must remain one coherent commit unit.

## Task stack

- [x] T01: `Codify the release/version authority contract` (status:done)
  - Task ID: T01
  - Goal: Update durable install/release context so `.version` is the canonical release version source, GitHub Releases are the canonical artifact publication surface, and crates.io/npm deployment are separate non-bumping publish stages.
  - Boundaries (in/out of scope): In - focused release/install context files, root context files if needed, and any decision/context wording required to remove the old auto-bump implication and old repo-slug guidance. Out - workflow/code implementation.
  - Done when: Durable context unambiguously states that `.version` is authoritative, release packaging consumes that version, Cargo/npm publish workflows do not bump versions, and install docs/context use the current repository slug and channel topology.
  - Verification notes (commands or checks): Manual parity review across `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `README.md`, and `cli/README.md`.
  - Implementation notes: Updated durable install/release context and user-facing install docs to codify repo-root `.version` as the release authority, GitHub Releases as the canonical artifact publication surface, and Cargo/npm publication as separate non-bumping downstream stages; removed touched stale `github:crocoder-dev/sce` install references.

- [x] T02: `Make GitHub release packaging consume .version deterministically` (status:done)
  - Task ID: T02
  - Goal: Refactor the GitHub release workflow and supporting packaging paths so release tags, archive names, packaged binary version metadata, Cargo package version, and npm package version all resolve from `.version` instead of workflow-side version auto-increment logic.
  - Boundaries (in/out of scope): In - `.github/workflows/release-sce*.yml`, `flake.nix` release apps, and any packaging/version-parity enforcement needed in `cli/Cargo.toml`, `.version`, and `npm/package.json`. Out - crates.io publish and npm registry publish steps themselves.
  - Done when: GitHub release packaging refuses mismatched version metadata, the release tag/version comes from `.version`, and produced CLI/npm artifacts all align to that same version without workflow-side semver bump generation.
  - Verification notes (commands or checks): `nix run .#release-artifacts -- --version <semver> --out-dir <tmp>`; `nix run .#release-manifest -- --version <semver> --artifacts-dir <tmp> --out-dir <tmp2> --signing-key-file <path>`; targeted workflow/config review for removal of independent bump logic; verify `sce version` build metadata contract remains aligned to the packaged release version.
  - Implementation notes: Removed workflow-side semver bump logic from `.github/workflows/release-sce.yml`, made manual release tagging consume checked-in `.version`, added fail-fast version parity checks across `.version`, `cli/Cargo.toml`, and `npm/package.json`, and changed release packaging to refuse mismatched metadata instead of rewriting the npm package version at pack time.

- [x] T03: `Add crates.io publish workflow without version bumping` (status:done)
  - Task ID: T03
  - Goal: Add a dedicated Cargo publish workflow that publishes the `cli/` crate for the already-declared current version, consuming checked-in metadata instead of performing release-version mutation.
  - Boundaries (in/out of scope): In - GitHub Actions workflow(s), publish gating, cargo packaging/publish invocation, and any safety checks that ensure the published crate version matches `.version`. Out - GitHub binary release creation and npm publish automation.
  - Done when: A dedicated workflow exists for crates.io publication, it performs no version bumping, it validates version parity against `.version`, and it is documented as a separate publish stage from GitHub release packaging.
  - Verification notes (commands or checks): Workflow review for crates.io-only scope and non-bumping behavior; targeted dry-run/safety-path validation where supported by repo tooling; manual parity review of `cli/Cargo.toml`, `.version`, and Cargo publish docs.
  - Implementation notes: Added `.github/workflows/publish-crates.yml` as a crates.io-only workflow triggered by published GitHub releases or manual dispatch with an optional dry-run path, enforcing `.version`/`cli/Cargo.toml`/release-tag parity and requiring `CARGO_REGISTRY_TOKEN` for real publish runs. Retargeted CLI embed/schema includes to an ephemeral `cli/assets/generated/` mirror prepared from canonical `config/` outputs during Nix builds and Cargo publish runs so packaged crate verification and crates.io publication succeed without committing generated crate assets.

- [x] T04: `Add npm publish workflow without version bumping` (status:done)
  - Task ID: T04
  - Goal: Add a dedicated npm publish workflow that publishes the `sce` npm package for the already-declared current version, consuming the checked-in package metadata and canonical GitHub release assets without performing release-version mutation.
  - Boundaries (in/out of scope): In - GitHub Actions workflow(s), npm publish invocation, release-asset consumption assumptions, and version-parity safety checks against `.version`. Out - changing the npm installer trust model or reworking GitHub binary asset publication.
  - Done when: A dedicated workflow exists for npm publication, it performs no version bumping, it validates version parity against `.version`, and npm docs/context describe npm registry publish as a separate stage from GitHub release packaging.
  - Verification notes (commands or checks): `nix run .#release-npm-package -- --version <semver> --out-dir <tmp>`; `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`; workflow review for npm-only scope and non-bumping behavior; manual parity review of `npm/package.json`, `.version`, and npm publish docs.
  - Implementation notes: Added `.github/workflows/publish-npm.yml` as an npm-only workflow triggered by published GitHub releases or manual dispatch with an optional dry-run path, enforcing `.version`/`npm/package.json`/release-tag parity, downloading the canonical `sce-v<version>-npm.tgz` GitHub release asset, verifying its embedded package metadata, and requiring `NPM_TOKEN` only for real npm publication.

- [x] T05: `Repair install and release documentation for current flows` (status:done)
  - Task ID: T05
  - Goal: Update user-facing docs and any remaining non-context guidance so install commands, repo slug references, and deployment descriptions match the implemented release/publish topology.
  - Boundaries (in/out of scope): In - `README.md`, `cli/README.md`, `npm/README.md`, and other touched repo docs that currently imply the wrong repo slug or incomplete deployment behavior. Out - durable context-heavy policy work already covered in T01.
  - Done when: User-facing docs consistently reference `crocoder-dev/shared-context-engineering`, describe GitHub release packaging plus separate Cargo/npm publish stages correctly, and no longer imply unsupported auto-bump or single-step publish behavior.
  - Verification notes (commands or checks): Manual parity review across touched docs against the implemented workflows and package metadata.
  - Implementation notes: Updated `README.md`, `cli/README.md`, and `npm/README.md` so install guidance consistently references `crocoder-dev/shared-context-engineering`, clarifies that GitHub Releases publish canonical artifacts first, and states that Cargo/npm publication are separate non-auto-bumping downstream stages.

- [x] T06: `Run validation and cleanup for release topology fixes` (status:done)
  - Task ID: T06
  - Goal: Execute final validation for the version-source, GitHub release, crates.io, npm, and documentation changes; remove any stale references or mismatched assumptions left by the rollout.
  - Boundaries (in/out of scope): In - full repo validation for touched surfaces, targeted packaging/install checks, workflow parity review, and final context-sync verification. Out - new delivery features beyond the scoped findings.
  - Done when: Required validation passes, no in-scope stale versioning/deployment guidance remains, and the repo records evidence that `.version`, GitHub release packaging, Cargo publish, and npm publish are coherent end to end.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; `nix run .#release-artifacts -- --version 0.1.0 --out-dir <tmp>`; `nix run .#release-manifest -- --version 0.1.0 --artifacts-dir <tmp> --out-dir <tmp> --signing-key-file <tmp-key>`; `nix run .#release-npm-package -- --version 0.1.0 --out-dir <tmp>`; manual parity review across `.github/workflows/release-sce.yml`, `.github/workflows/publish-crates.yml`, `.github/workflows/publish-npm.yml`, `README.md`, `cli/README.md`, `npm/README.md`, and focused install/release context.
  - Implementation notes: Validation passed for generated-output parity, full flake checks, release artifact packaging, release-manifest assembly/signing with a temporary test key, and npm package release asset generation. Manual parity review found the workflows/docs/context aligned to the checked-in `.version` authority and separate GitHub Release / crates.io / npm publish topology, so no additional cleanup edits were required beyond recording validation evidence.

## Open questions

- None. The user confirmed that `.version` should be the release authority, GitHub release packaging should be separate from crates.io/npm deployment, and the Cargo/npm publish workflows should publish the current version without bumping it.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (all 10 flake checks passed on `x86_64-linux`)
- `nix run .#release-artifacts -- --version 0.1.0 --out-dir /tmp/...` -> exit 0 (`sce-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`, checksum, and metadata fragment emitted)
- `nix run .#release-manifest -- --version 0.1.0 --artifacts-dir /tmp/... --out-dir /tmp/... --signing-key-file /tmp/...` -> exit 0 (`sce-v0.1.0-release-manifest.json`, detached signature, and `SHA256SUMS` emitted)
- `nix run .#release-npm-package -- --version 0.1.0 --out-dir /tmp/...` -> exit 0 (`sce-v0.1.0-npm.tgz` and metadata JSON emitted)

### Failed checks and follow-ups

- None.

### Success-criteria verification

- [x] The release pipeline no longer derives a new version independently of checked-in package metadata -> verified by parity review of `.github/workflows/release-sce.yml`, `.github/workflows/publish-crates.yml`, `.github/workflows/publish-npm.yml`, `.version`, `cli/Cargo.toml`, and `npm/package.json`.
- [x] Built CLI, release artifacts, GitHub release tags, Cargo metadata, and npm metadata agree on `.version` -> verified by `.version = 0.1.0`, matching `cli/Cargo.toml` + `npm/package.json`, and successful `release-artifacts` / `release-manifest` / `release-npm-package` runs.
- [x] GitHub release packaging is separated from crates.io and npm registry deployment -> verified by workflow review: `.github/workflows/release-sce.yml` assembles GitHub Release assets only, while `.github/workflows/publish-crates.yml` and `.github/workflows/publish-npm.yml` are dedicated downstream publish workflows.
- [x] Cargo publish workflow exists and publishes checked-in crate version without bumping -> verified by `.github/workflows/publish-crates.yml` parity checks plus publish/dry-run flow against checked-in `.version`.
- [x] npm publish workflow exists and publishes checked-in package version without bumping -> verified by `.github/workflows/publish-npm.yml` parity checks plus canonical GitHub release asset download/publish flow.
- [x] User-facing install/deployment docs and durable context describe the actual current-state repo slug, release topology, and versioning rules -> verified by parity review of `README.md`, `cli/README.md`, `npm/README.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, `context/context-map.md`, `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, and `context/sce/cli-npm-distribution-contract.md`.

### Residual risks

- Cross-platform release automation for macOS targets is validated by workflow/config parity in this task; artifact execution was only exercised locally for the Linux target during command-based validation.
