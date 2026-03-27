# Plan: sce-npm-signed-release-manifest-verification

## Change summary

Harden the npm installer trust chain for `sce` by replacing the current trust in the live release manifest checksum with signature-anchored verification. Current code truth in `npm/lib/install.js` still downloads `sce-v<version>-release-manifest.json`, selects the platform artifact, and trusts `artifact.checksum_sha256` directly from that downloaded manifest, so the finding remains valid and requires a fix. The approved direction for this plan is option **B**: verify the release manifest with a built-in public key before using any manifest-provided checksum metadata.

## Success criteria

- `npm/lib/install.js` no longer treats `artifact.checksum_sha256` from an unsigned live manifest as trusted input.
- The npm installer verifies a release-manifest signature with a built-in public key before `selectReleaseArtifact(...)` output is used for checksum enforcement.
- Manifest verification failure blocks installation with a clear integrity/authenticity error before archive extraction.
- Release packaging/publishing in scope emits the signature material required by the npm installer so published releases remain installable.
- Tests cover valid-signature, invalid-signature, and unsigned/missing-signature failure paths for the npm installer trust flow.
- Current-state npm distribution context reflects the signed-manifest trust contract after implementation.

## Constraints and non-goals

- In scope: the npm launcher package under `npm/`, the minimal release-manifest/signature production path required to support signed-manifest verification, targeted tests, and context/doc sync for the new trust model.
- In scope: verifying the reported finding against current code before fixing it; current code truth has already confirmed the finding is real.
- In scope: using a built-in public key embedded in the shipped npm package as the trust anchor for manifest verification.
- In scope: generating a signing keypair outside the repository, bundling only the public key in the npm package, and using the private key only during trusted release signing.
- In scope: one-task/one-atomic-commit slicing only.
- Out of scope: switching to bundled package-local checksums instead of manifest signatures.
- Out of scope: broader release supply-chain redesign beyond the minimal signature generation, publication, and verification needed for the npm installer.
- Out of scope: new install channels, unrelated npm UX changes, or archive-signing schemes that replace checksum verification entirely.

## Assumptions

- The signing model uses an asymmetric keypair: a private signing key plus a distributable public verification key.
- The private key is never committed to the repository and is never shipped in the npm package.
- The public key is committed/shipped with the npm package so `npm/lib/install.js` can verify release-manifest signatures offline.
- The release process signs `sce-v<version>-release-manifest.json` and publishes a deterministic manifest-signature artifact next to the manifest.
- The installer continues to use `artifact.checksum_sha256` for archive integrity, but only after the manifest itself is authenticated by signature verification.
- Algorithm choice, exact signature file naming, and release-secret storage mechanism may be finalized during implementation unless the implementer identifies a blocking interoperability issue.

## Task stack

- [x] T01: `Add signed-manifest verification primitives to the npm installer` (status:done)
  - Task ID: T01
  - Goal: Introduce the built-in public key, manifest-signature loading, and cryptographic verification helpers needed for the npm installer to authenticate release manifests before trusting their contents.
  - Boundaries (in/out of scope): In - `npm/lib/install.js` support code, any new npm-package-shipped public-key/signature helper files, and targeted unit tests for verification primitives. Out - release pipeline signature production, private-key handling beyond documented assumptions, installer control-flow rewiring beyond the verification seam, or broad package refactors.
  - Done when: The npm package contains the built-in public verification key, installer code can verify a detached or paired manifest signature using built-in crypto facilities, and tests prove valid signatures pass while tampered manifest/signature inputs fail.
  - Verification notes (commands or checks): Run the narrow npm test coverage for the new verifier paths; confirm the public key is packaged with the npm tarball, the private key is not repo/package material, and verification does not depend on network-fetched trust material.
  - Completed: 2026-03-27
  - Files changed: `npm/lib/install.js`, `npm/lib/release-manifest-public-key.pem`, `npm/test/install.test.js`
  - Evidence: `bun test ./test/install.test.js` (4/4 passing); `npm pack --dry-run --json` confirms `lib/release-manifest-public-key.pem` is included in the npm tarball
  - Notes: Added bundled public-key loading plus detached signature verification primitives; installer enforcement remains planned for T02.

- [x] T02: `Require verified manifests before artifact checksum trust` (status:done)
  - Task ID: T02
  - Goal: Rewire `npm/lib/install.js` so `downloadJson(getManifestUrl(version))`, `selectReleaseArtifact(...)`, and `artifact.checksum_sha256` are only used after successful manifest-signature verification.
  - Boundaries (in/out of scope): In - installer control flow, integrity/authenticity error handling, and installer-focused tests for signed/unsigned/invalid manifest behavior. Out - changing supported platform selection rules, archive extraction semantics, or release asset generation.
  - Done when: Installation aborts before archive download or extraction when manifest verification fails; successful installs continue to checksum-verify the archive using manifest data only after signature verification; regression tests cover the real control flow.
  - Verification notes (commands or checks): Run npm installer tests covering valid signed manifest, bad signature, missing signature, and checksum mismatch behavior; perform the existing skip-download smoke path if still applicable after refactor.
  - Completed: 2026-03-27
  - Files changed: `npm/lib/install.js`, `npm/test/install.test.js`
  - Evidence: `bun test ./test/install.test.js` (11/11 passing); `bun test ./test/*.test.js` (17/17 passing)
  - Notes: Installer now downloads `sce-v<version>-release-manifest.json` plus deterministic detached signature `sce-v<version>-release-manifest.json.sig`, verifies the manifest before artifact selection/checksum trust, and aborts before archive download when signature validation fails or the signature asset is missing.

- [x] T03: `Emit and publish release-manifest signatures for npm installs` (status:done)
  - Task ID: T03
  - Goal: Add the minimal release packaging/release automation changes required to generate, stage, and publish the manifest signature artifact consumed by the npm installer.
  - Boundaries (in/out of scope): In - release-manifest generation outputs, signature generation with the private release key, release artifact naming/wiring, npm package packaging updates if needed, and targeted release-path tests or fixture updates. Out - unrelated workflow cleanup, key-rotation policy expansion, or broader artifact-signing rollout for all release assets.
  - Done when: The release process produces the manifest signature asset in a stable location/name, published releases contain both the manifest and its signature, signing uses a non-repo private key, and the npm installer can locate the signature using deterministic conventions.
  - Verification notes (commands or checks): Validate the release-output contract for manifest plus signature artifacts and run the narrowest packaging/release checks that prove the npm tarball and release outputs stay aligned.
  - Completed: 2026-03-27
  - Files changed: `flake.nix`, `.github/workflows/release-sce.yml`, `scripts/lib/release-manifest-signing.mjs`, `scripts/sign-release-manifest.mjs`, `npm/test/install.test.js`, `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`
  - Evidence: `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` (19/19 passing); `nix run .#release-manifest -- --version 0.1.0 --artifacts-dir <tmp> --out-dir <tmp>/out --signing-key-file <tmp>/release-private-key.pem` (emits `sce-v0.1.0-release-manifest.json.sig`); `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: `release-manifest` now signs `sce-v<version>-release-manifest.json` into deterministic detached artifact `sce-v<version>-release-manifest.json.sig`; GitHub release publishing uploads that signature asset and the signing private key stays external via env/file input only.

- [x] T04: `Sync npm trust-chain context and validate end-to-end behavior` (status:done)
  - Task ID: T04
  - Goal: Update current-state context for the npm distribution trust model and run final validation/cleanup for the signed-manifest implementation.
  - Boundaries (in/out of scope): In - focused context updates (`context/sce/cli-npm-distribution-contract.md` and root context only if the trust model changes cross-cutting wording), final validation evidence, and cleanup of stale checksum-trust wording. Out - new feature work or additional security hardening beyond this plan.
  - Done when: Durable context describes the npm installer as trusting `checksum_sha256` only from a signature-verified manifest, validation evidence is captured, and no stale current-state docs still describe the old live-manifest trust path.
  - Verification notes (commands or checks): Run the repo baseline validation plus targeted npm/release checks required by prior tasks; manually verify current-state context matches the implemented signed-manifest trust flow.
  - Completed: 2026-03-27
  - Files changed: `context/plans/sce-npm-signed-release-manifest-verification.md`
  - Evidence: `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` (19/19 passing); `nix develop -c sh -c 'cd npm && SCE_NPM_SKIP_DOWNLOAD=1 bun ./lib/install.js'` (skip-download smoke passes); `nix run .#release-npm-package -- --version 0.1.0 --out-dir <tmp>` (emits `sce-v0.1.0-npm.tgz` and `sce-v0.1.0-npm.json`); `nix run .#release-manifest -- --version 0.1.0 --artifacts-dir <tmp> --out-dir <tmp>/out --signing-key-file <tmp>/release-private-key.pem`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Verify-only context sync: `context/sce/cli-npm-distribution-contract.md` plus root shared context already matched the implemented signed-manifest trust flow, so no durable context edits were required beyond recording final validation evidence.

## Open questions

- None. The user selected option **B** as the required trust anchor strategy.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` -> exit 0 (19/19 tests passing)
- `nix develop -c sh -c 'cd npm && SCE_NPM_SKIP_DOWNLOAD=1 bun ./lib/install.js'` -> exit 0 (`SCE_NPM_SKIP_DOWNLOAD=1` smoke path confirmed)
- `nix run .#release-npm-package -- --version 0.1.0 --out-dir <tmp>` -> exit 0 (emits `sce-v0.1.0-npm.tgz` and `sce-v0.1.0-npm.json`)
- `nix run .#release-manifest -- --version 0.1.0 --artifacts-dir <tmp> --out-dir <tmp>/out --signing-key-file <tmp>/release-private-key.pem` -> exit 0 (signed manifest generation path exercised with non-repo private key)
- `nix run .#pkl-check-generated` -> exit 0 (generated outputs up to date)
- `nix flake check` -> exit 0 (flake outputs/checks evaluated successfully)

### Failed checks and follow-ups

- None.

### Success-criteria verification

- [x] `npm/lib/install.js` no longer treats `artifact.checksum_sha256` from an unsigned live manifest as trusted input -> previously implemented in T02; preserved by passing npm installer test suite and current durable context.
- [x] The npm installer verifies a release-manifest signature with a built-in public key before `selectReleaseArtifact(...)` output is used for checksum enforcement -> represented in `context/sce/cli-npm-distribution-contract.md`, `context/architecture.md`, `context/overview.md`, and `context/glossary.md`; verified during context sync.
- [x] Manifest verification failure blocks installation with a clear integrity/authenticity error before archive extraction -> covered by prior task tests and retained in passing `npm/test/*.test.js` suite.
- [x] Release packaging/publishing in scope emits the signature material required by the npm installer so published releases remain installable -> exercised by `nix run .#release-manifest ...` and reflected in `context/sce/cli-release-artifact-contract.md` and `context/sce/cli-npm-distribution-contract.md`.
- [x] Tests cover valid-signature, invalid-signature, and unsigned/missing-signature failure paths for the npm installer trust flow -> confirmed by passing `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` suite.
- [x] Current-state npm distribution context reflects the signed-manifest trust contract after implementation -> verify-only context sync confirmed no stale docs remained; no additional durable context edits were required.

### Residual risks

- None identified within the approved plan scope.
