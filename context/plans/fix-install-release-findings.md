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

- [ ] T01: `Codify the release/version authority contract` (status:todo)
  - Task ID: T01
  - Goal: Update durable install/release context so `.version` is the canonical release version source, GitHub Releases are the canonical artifact publication surface, and crates.io/npm deployment are separate non-bumping publish stages.
  - Boundaries (in/out of scope): In - focused release/install context files, root context files if needed, and any decision/context wording required to remove the old auto-bump implication and old repo-slug guidance. Out - workflow/code implementation.
  - Done when: Durable context unambiguously states that `.version` is authoritative, release packaging consumes that version, Cargo/npm publish workflows do not bump versions, and install docs/context use the current repository slug and channel topology.
  - Verification notes (commands or checks): Manual parity review across `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `README.md`, and `cli/README.md`.

- [ ] T02: `Make GitHub release packaging consume .version deterministically` (status:todo)
  - Task ID: T02
  - Goal: Refactor the GitHub release workflow and supporting packaging paths so release tags, archive names, packaged binary version metadata, Cargo package version, and npm package version all resolve from `.version` instead of workflow-side version auto-increment logic.
  - Boundaries (in/out of scope): In - `.github/workflows/release-sce*.yml`, `flake.nix` release apps, and any packaging/version-parity enforcement needed in `cli/Cargo.toml`, `.version`, and `npm/package.json`. Out - crates.io publish and npm registry publish steps themselves.
  - Done when: GitHub release packaging refuses mismatched version metadata, the release tag/version comes from `.version`, and produced CLI/npm artifacts all align to that same version without workflow-side semver bump generation.
  - Verification notes (commands or checks): `nix run .#release-artifacts -- --version <semver> --out-dir <tmp>`; `nix run .#release-manifest -- --version <semver> --artifacts-dir <tmp> --out-dir <tmp2> --signing-key-file <path>`; targeted workflow/config review for removal of independent bump logic; verify `sce version` build metadata contract remains aligned to the packaged release version.

- [ ] T03: `Add crates.io publish workflow without version bumping` (status:todo)
  - Task ID: T03
  - Goal: Add a dedicated Cargo publish workflow that publishes the `cli/` crate for the already-declared current version, consuming checked-in metadata instead of performing release-version mutation.
  - Boundaries (in/out of scope): In - GitHub Actions workflow(s), publish gating, cargo packaging/publish invocation, and any safety checks that ensure the published crate version matches `.version`. Out - GitHub binary release creation and npm publish automation.
  - Done when: A dedicated workflow exists for crates.io publication, it performs no version bumping, it validates version parity against `.version`, and it is documented as a separate publish stage from GitHub release packaging.
  - Verification notes (commands or checks): Workflow review for crates.io-only scope and non-bumping behavior; targeted dry-run/safety-path validation where supported by repo tooling; manual parity review of `cli/Cargo.toml`, `.version`, and Cargo publish docs.

- [ ] T04: `Add npm publish workflow without version bumping` (status:todo)
  - Task ID: T04
  - Goal: Add a dedicated npm publish workflow that publishes the `sce` npm package for the already-declared current version, consuming the checked-in package metadata and canonical GitHub release assets without performing release-version mutation.
  - Boundaries (in/out of scope): In - GitHub Actions workflow(s), npm publish invocation, release-asset consumption assumptions, and version-parity safety checks against `.version`. Out - changing the npm installer trust model or reworking GitHub binary asset publication.
  - Done when: A dedicated workflow exists for npm publication, it performs no version bumping, it validates version parity against `.version`, and npm docs/context describe npm registry publish as a separate stage from GitHub release packaging.
  - Verification notes (commands or checks): `nix run .#release-npm-package -- --version <semver> --out-dir <tmp>`; `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`; workflow review for npm-only scope and non-bumping behavior; manual parity review of `npm/package.json`, `.version`, and npm publish docs.

- [ ] T05: `Repair install and release documentation for current flows` (status:todo)
  - Task ID: T05
  - Goal: Update user-facing docs and any remaining non-context guidance so install commands, repo slug references, and deployment descriptions match the implemented release/publish topology.
  - Boundaries (in/out of scope): In - `README.md`, `cli/README.md`, `npm/README.md`, and other touched repo docs that currently imply the wrong repo slug or incomplete deployment behavior. Out - durable context-heavy policy work already covered in T01.
  - Done when: User-facing docs consistently reference `crocoder-dev/shared-context-engineering`, describe GitHub release packaging plus separate Cargo/npm publish stages correctly, and no longer imply unsupported auto-bump or single-step publish behavior.
  - Verification notes (commands or checks): Manual parity review across touched docs against the implemented workflows and package metadata.

- [ ] T06: `Run validation and cleanup for release topology fixes` (status:todo)
  - Task ID: T06
  - Goal: Execute final validation for the version-source, GitHub release, crates.io, npm, and documentation changes; remove any stale references or mismatched assumptions left by the rollout.
  - Boundaries (in/out of scope): In - full repo validation for touched surfaces, targeted packaging/install checks, workflow parity review, and final context-sync verification. Out - new delivery features beyond the scoped findings.
  - Done when: Required validation passes, no in-scope stale versioning/deployment guidance remains, and the repo records evidence that `.version`, GitHub release packaging, Cargo publish, and npm publish are coherent end to end.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; targeted release packaging checks from T02-T04; manual parity review across workflow files, docs, and focused install/release context.

## Open questions

- None. The user confirmed that `.version` should be the release authority, GitHub release packaging should be separate from crates.io/npm deployment, and the Cargo/npm publish workflows should publish the current version without bumping it.
