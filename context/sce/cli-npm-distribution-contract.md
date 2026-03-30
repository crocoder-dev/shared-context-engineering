# SCE CLI npm Distribution Contract

This file captures the implemented npm distribution slice from `context/plans/sce-cli-first-install-channels.md` task `T04`.

## Package surface

- The npm package name is `sce`.
- The committed package source lives under `npm/`.
- `npm/package.json` exposes the `sce` binary through `bin/sce.js`.
- The npm package is a thin launcher package, not a separate build pipeline.

## Install/runtime behavior

- `npm/bin/sce.js` launches a package-local native `sce` binary from `npm/runtime/sce`.
- `npm/lib/install.js` runs during `postinstall` and downloads the release manifest plus detached signature for the package version.
- The npm package ships a built-in manifest-signing public key at `npm/lib/release-manifest-public-key.pem`; `npm/lib/install.js` uses Node built-in crypto to verify manifest signatures without relying on network-fetched trust anchors.
- The installer downloads `sce-v<version>-release-manifest.json` and detached signature `sce-v<version>-release-manifest.json.sig`, verifies the manifest before parsing/selecting artifacts, then selects the artifact whose `target_triple` matches the current supported platform, downloads `sce-v<version>-<target-triple>.tar.gz`, verifies `checksum_sha256`, extracts `bin/sce`, and stores it in package-local runtime storage.
- Missing or invalid manifest signatures abort installation before archive download or extraction with an authenticity failure.
- Supported npm launcher platforms currently match the implemented release automation targets:
  - `linux/x64` -> `x86_64-unknown-linux-gnu`
  - `linux/arm64` -> `aarch64-unknown-linux-gnu`
  - `darwin/x64` -> `x86_64-apple-darwin`
  - `darwin/arm64` -> `aarch64-apple-darwin`
- Unsupported platforms fail with explicit guidance instead of attempting alternate channels inside the npm package.

## Release integration

- Repo-root `.version` is the canonical release version authority; checked-in `npm/package.json` metadata should match it before release packaging or registry publication proceeds.
- `flake.nix` exposes `nix run .#release-npm-package -- --version <semver> --out-dir <path>`.
- `flake.nix` also exposes `nix run .#release-manifest -- --version <semver> --artifacts-dir <path> --out-dir <path> [--signing-key-file <path>]`, which emits the merged release manifest, detached signature, and combined `SHA256SUMS`.
- `release-manifest` signing uses a non-repo private key from `SCE_RELEASE_MANIFEST_SIGNING_KEY` or `--signing-key-file`; the npm package ships only the matching public key.
- The current `release-npm-package` helper stages the checked-in `npm/` package, requires the requested version to match checked-in `.version` and `npm/package.json`, runs `npm pack` without rewriting package metadata, and emits:
  - `sce-v<version>-npm.tgz`
  - `sce-v<version>-npm.json`
- The helper also refuses to proceed when checked-in `.version`, `cli/Cargo.toml`, and `npm/package.json` are not already aligned.
- `.github/workflows/release-sce.yml` publishes those npm assets alongside the canonical CLI archives, `sce-v<version>-release-manifest.json`, and `sce-v<version>-release-manifest.json.sig`.

## Registry publication topology

- npm registry publication is a separate downstream publish stage from GitHub release packaging.
- `.github/workflows/publish-npm.yml` is the dedicated npm publish workflow.
- It triggers from a published GitHub release or manual dispatch.
- It validates parity across repo-root `.version`, checked-in `npm/package.json`, and the target release tag before attempting publication.
- It downloads the canonical `sce-v<version>-npm.tgz` asset from the corresponding GitHub release rather than rebuilding or mutating package metadata during publish.
- It verifies the downloaded tarball still declares package name `sce` and version `<.version>` before `npm publish`.
- Real publication requires `NPM_TOKEN`; manual dispatch can remain on a dry-run path via `npm publish --dry-run`.
- The npm publish workflow publishes the already-versioned checked-in `npm/` package and does not invent or bump a release version during workflow execution.
- The npm installer continues to trust GitHub Releases as the canonical host for signed manifest and native binary artifacts.

## Verification baseline

- `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`
- `nix develop -c sh -c 'cd npm && SCE_NPM_SKIP_DOWNLOAD=1 bun ./lib/install.js'`
- `nix run .#release-manifest -- --version <semver> --artifacts-dir <path> --out-dir <path> --signing-key-file <path>`
- `nix run .#release-npm-package -- --version <semver> --out-dir <path>`

See also: [cli-first-install-channels-contract.md](./cli-first-install-channels-contract.md), [cli-release-artifact-contract.md](./cli-release-artifact-contract.md), [../overview.md](../overview.md)
