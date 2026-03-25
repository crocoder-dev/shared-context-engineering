# SCE CLI npm Distribution Contract

This file captures the implemented npm distribution slice from `context/plans/sce-cli-first-install-channels.md` task `T04`.

## Package surface

- The npm package name is `sce`.
- The committed package source lives under `npm/`.
- `npm/package.json` exposes the `sce` binary through `bin/sce.js`.
- The npm package is a thin launcher package, not a separate build pipeline.

## Install/runtime behavior

- `npm/bin/sce.js` launches a package-local native `sce` binary from `npm/runtime/sce`.
- `npm/lib/install.js` runs during `postinstall` and downloads the matching GitHub release asset for the package version.
- The installer reads `sce-v<version>-release-manifest.json`, selects the artifact whose `target_triple` matches the current supported platform, downloads `sce-v<version>-<target-triple>.tar.gz`, verifies `checksum_sha256`, extracts `bin/sce`, and stores it in package-local runtime storage.
- Supported npm launcher platforms currently match the implemented release automation targets: `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`.
- Unsupported platforms fail with explicit guidance instead of attempting alternate channels inside the npm package.

## Release integration

- `flake.nix` exposes `nix run .#release-npm-package -- --version <semver> --out-dir <path>`.
- That app stages the checked-in `npm/` package, rewrites the package version for the requested release, runs `npm pack`, and emits:
  - `sce-v<version>-npm.tgz`
  - `sce-v<version>-npm.json`
- `.github/workflows/release-sce.yml` publishes those npm assets alongside the canonical CLI archives and release manifest outputs.

## Verification baseline

- `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`
- `nix develop -c sh -c 'cd npm && SCE_NPM_SKIP_DOWNLOAD=1 bun ./lib/install.js'`
- `nix run .#release-npm-package -- --version <semver> --out-dir <path>`

See also: [cli-first-install-channels-contract.md](./cli-first-install-channels-contract.md), [cli-release-artifact-contract.md](./cli-release-artifact-contract.md), [../overview.md](../overview.md)
