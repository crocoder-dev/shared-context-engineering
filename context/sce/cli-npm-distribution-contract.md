# SCE CLI npm Distribution Contract

This file captures the implemented npm distribution slice of the `sce` CLI install-channel contract.

## Package surface

- The npm package name is `@crocoder-dev/sce`.
- The committed package source lives under `npm/`.
- `npm/README.md` documents the package as a thin launcher, lists the supported npm platform-to-target-triple matrix, and distinguishes npm `postinstall`/download/signature/checksum/permission failures from runtime loader failures caused by bad native release artifact linkage.
- `npm/package.json` exposes the `sce` binary through `bin/sce.js`.
- `npm/bin/sce.js` is committed as Git mode `100755`. `release-npm-package` copies `npm/` with `cp -R` and `npm pack` records filesystem modes verbatim, so the launcher's executable bit in Git is what makes the packed `package/bin/sce.js` entry executable. npm, bun, and pnpm re-chmod bin targets while linking, which masks a non-executable entry for those installers but not for consumers that execute the packed file directly.
- The npm package is a thin launcher package, not a separate build pipeline.

## Install/runtime behavior

- `npm/bin/sce.js` launches a package-local native `sce` binary from `npm/runtime/sce`.
- `npm/lib/install.js` runs during `postinstall` and downloads the release manifest plus detached signature for the package version.
- The npm package ships a built-in manifest-signing public key at `npm/lib/release-manifest-public-key.pem`; `npm/lib/install.js` uses Node built-in crypto to verify manifest signatures without relying on network-fetched trust anchors.
- The installer downloads `sce-v<version>-release-manifest.json` and detached signature `sce-v<version>-release-manifest.json.sig`, verifies the manifest before parsing/selecting artifacts, validates the signed manifest's identity (below), downloads the canonical `sce-v<version>-<target-triple>.tar.gz`, verifies `checksum_sha256`, extracts only `bin/sce` through the bounded validating reader (below), and installs it atomically into package-local runtime storage.
- Missing or invalid manifest signatures abort installation before archive download or extraction with an authenticity failure.
- Manifest identity validation (`validateReleaseManifestIdentity` in `npm/lib/platform.js`) runs after signature verification and before any temp directory or archive download. It rejects, with a specific `Invalid sce release manifest` / `Invalid sce release artifact` / `No sce release artifact found` error, a manifest that is not a JSON object, whose `version` is not exactly the npm package version, whose `binary` is not `sce`, that has zero or more than one artifact for the current `target_triple`, whose artifact `archive` is not exactly `sce-v<version>-<target-triple>.tar.gz`, or whose `checksum_sha256` is not 64 lowercase hex characters. The download name is always the canonical archive name; the installer never falls back to or trusts a different manifest-supplied archive name. Per-artifact `version`/`binary` fields are not separately checked.
- Archive extraction (`npm/lib/archive.js`) does not shell out to `tar` or depend on a host `tar`. It gunzips and parses the ustar/GNU tar stream with Node built-ins under fixed caps (`ARCHIVE_LIMITS`: 64 MiB compressed, 256 MiB uncompressed, 192 MiB `bin/sce`, 16 entries, 4 KiB long names), verifies each header checksum and ustar/GNU magic, honors GNU `L` long names and pax `path` records, and accepts exactly `sce-v<version>-<target-triple>/`, `.../bin/`, `.../bin/sce` (regular file), `.../LICENSE`, and `.../README.md`. Any other entry fails with an `Invalid sce release archive:` error: absolute paths, `..`/empty/`.` segments, backslashes, symlinks, hardlinks, devices, FIFOs or other types, entries outside the archive root, unexpected or duplicate entries, a wrong entry kind, oversize payloads, truncated or non-gzip streams, and a missing `bin/sce`. Only `bin/sce` is written, into an isolated `staging/` directory inside the per-install download temp directory, which is removed afterward.
- Installation into `npm/runtime/sce` is atomic (`installBinaryAtomically`): the staged binary is written to a temporary file next to `runtime/sce`, set to mode `0755`, fsynced, and renamed over `runtime/sce`; the temp file is removed on any failure. A checksum failure, extraction failure, or interruption before the rename leaves an existing `runtime/sce` byte-identical and executable, and a failed fresh install leaves no `runtime/sce`.
- Supported npm launcher platforms currently match the implemented release automation targets:
  - `linux/x64` -> `x86_64-unknown-linux-musl`
  - `linux/arm64` -> `aarch64-unknown-linux-musl`
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
- After packing, the helper runs `npm/test/pack-smoke.sh --tarball <produced tgz>` against the tarball it just produced. If the packed `package/bin/sce.js` is not executable or the smoke install fails, it deletes the tarball and exits non-zero before writing metadata, so `.github/workflows/release-sce.yml` (which runs the helper before creating the GitHub Release) never uploads an unverified npm package.
- `.github/workflows/release-sce.yml` publishes those npm assets alongside the canonical CLI archives, `sce-v<version>-release-manifest.json`, and `sce-v<version>-release-manifest.json.sig`.
- Native archive portability is a release-artifact prerequisite for every npm-supported target triple. Archives selected through the signed release manifest must have passed macOS/Linux native portability audit and extracted-binary `sce version --format json` smoke validation before upload.

## Registry publication topology

- npm registry publication is a separate downstream publish stage from GitHub release packaging.
- `.github/workflows/publish-npm.yml` is the dedicated npm publish workflow.
- It triggers from a published GitHub release or manual dispatch.
- It validates parity across repo-root `.version`, checked-in `npm/package.json`, and the target release tag before attempting publication.
- It downloads the canonical `sce-v<version>-npm.tgz` asset from the corresponding GitHub release rather than rebuilding or mutating package metadata during publish.
- It verifies the downloaded tarball still declares package name `@crocoder-dev/sce` and version `<.version>` before `npm publish`.
- For GitHub prerelease events, manual dispatches marked `prerelease: true`, or checked-in semver prerelease versions containing `-`, the workflow publishes with npm dist-tag `next` so the package does not become `latest`.
- When a publish run is explicitly marked prerelease, the workflow requires `.version` to be a semver prerelease version and fails before publishing if it is not.
- Real publication requires `NPM_TOKEN`; manual dispatch can remain on a dry-run path via `npm publish --dry-run`.
- The npm publish workflow publishes the already-versioned checked-in `npm/` package and does not invent or bump a release version during workflow execution.
- The npm installer continues to trust GitHub Releases as the canonical host for signed manifest and native binary artifacts.
- Native binary portability hygiene is owned by the shared release artifact surface before archive publication, including macOS `libiconv` install-name sanitization, Linux/macOS forbidden `/nix/store/` audits, and reusable-workflow extracted-binary smoke validation; npm continues to consume the signed release manifest and checksum-verified native archive without adding a separate npm-native build or dylib repair path.

## Packed-package smoke test

- `npm/test/pack-smoke.sh` (with helper `npm/test/fixture-release-server.mjs`) verifies the real packed artifact end to end without network access: it stages a copy of the package (or `--package-dir <path>`, or the `package/` contents of an already packed `--tarball <path>`, whose launcher mode is asserted first), swaps a throwaway RSA manifest-signing public key into the staged copy only, builds a stub `sce` archive in the canonical `sce-v<version>-<target>/{bin/sce,LICENSE,README.md}` layout with a matching signed manifest, serves it over loopback HTTPS (self-signed certificate trusted via `NODE_EXTRA_CA_CERTS`, selected via `SCE_NPM_RELEASE_BASE_URL`), runs `npm pack`, fails unless the packed `package/bin/sce.js` entry is executable, installs into an isolated `--prefix` with an isolated npm cache/config, and asserts exact output from `sce --help` and `sce version`.
- The fixture archive is produced by real GNU `tar`, so the smoke also exercises the in-repo archive reader against the release pipeline's archive format under Node.
- Only the host platform is exercised, with a stub binary; the production public key and manifest trust code are not modified.
- Root `flake.nix` runs the smoke in the build sandbox as `checks.<system>.npm-pack-smoke` (part of `nix flake check`; loopback HTTPS works inside the sandbox). The sandbox has no `/usr/bin/env`, so there the installed bin is run as `node <bin>` after its `-x` check. Host runs, including the `release-npm-package` gate, exec the bin directly.

## Verification baseline

- `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`
- `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh`
- `nix build .#checks.<system>.npm-pack-smoke --print-build-logs`
- `nix develop -c sh -c 'cd npm && SCE_NPM_SKIP_DOWNLOAD=1 bun ./lib/install.js'`
- `nix run .#release-manifest -- --version <semver> --artifacts-dir <path> --out-dir <path> --signing-key-file <path>`
- `nix run .#release-npm-package -- --version <semver> --out-dir <path>`

See also: [cli-first-install-channels-contract.md](./cli-first-install-channels-contract.md), [cli-release-artifact-contract.md](./cli-release-artifact-contract.md), [../overview.md](../overview.md)
