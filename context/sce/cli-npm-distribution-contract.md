# SCE CLI npm Distribution Contract

This file captures the implemented npm distribution slice of the `sce` CLI install-channel contract.

## Package surface

- The npm package name is `@crocoder-dev/sce`.
- The committed package source lives under `npm/`.
- `npm/README.md` documents the package as a thin launcher, lists the supported npm platform-to-target-triple matrix (musl Linux triples, matching `npm/lib/platform.js`), summarizes manifest identity validation, bounded extraction, and atomic install, and distinguishes npm `postinstall`/download/signature/manifest-identity/checksum/archive-validation/permission failures from runtime loader failures caused by bad native release artifact linkage.
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

- `npm/test/pack-smoke.sh` (built on the shared release-fixture harness below) verifies the real packed artifact end to end without network access: it stages a copy of the package (or `--package-dir <path>`, or the `package/` contents of an already packed `--tarball <path>`, whose launcher mode is asserted first), swaps a throwaway RSA manifest-signing public key into the staged copy only, builds a stub `sce` archive in the canonical `sce-v<version>-<target>/{bin/sce,LICENSE,README.md}` layout with a matching signed manifest, serves it over loopback HTTPS (self-signed certificate trusted via `NODE_EXTRA_CA_CERTS`, selected via `SCE_NPM_RELEASE_BASE_URL`), runs `npm pack`, fails unless the packed `package/bin/sce.js` entry is executable, installs into an isolated prefix with an isolated npm cache/config, asserts the installed `runtime/sce` has mode `755` and that `node_modules/.bin/sce` resolves to the installed `@crocoder-dev/sce/bin/sce.js`, then runs the installed CLI.
- Shared release-fixture harness: `npm/test/release-fixture.sh` owns every hermetic fixture step: tool checks, package staging and launcher-mode assertion, platform resolution, the throwaway signing key and loopback TLS certificate, canonical archive plus signed manifest, `npm pack`, starting and stopping `npm/test/fixture-release-server.mjs` (the only fixture HTTPS server), the isolated offline `npm install`, and the `runtime/sce` mode and `.bin/sce` target assertions. It is sourced as a Bash library of `fixture_*` functions whose failure and progress lines use the caller's `FIXTURE_LOG_PREFIX` (pack-smoke keeps its `pack-smoke:` messages). It also runs directly. `release-fixture.sh prepare <dir> [--package-dir <path> | --tarball <path>] [--binary <path>]` writes `stage/`, `pack/<tgz>`, `release/`, `keys/` (throwaway `signing.pem`, `tls-cert.pem`, `tls-key.pem`), and a shell-sourceable `fixture.env` (`FIXTURE_VERSION`, `FIXTURE_TARGET_TRIPLE`, `FIXTURE_TARBALL`, `FIXTURE_RELEASE_DIR`, `FIXTURE_TLS_CERT`, `FIXTURE_TLS_KEY`). It refuses an already prepared dir. `release-fixture.sh serve <dir> [--port-file <file>]` runs the server in the foreground against `<dir>/release` (default port file `<dir>/server-port`), so another process can install from the prepared tarball with `SCE_NPM_RELEASE_BASE_URL=https://127.0.0.1:<port>` and `NODE_EXTRA_CA_CERTS=$FIXTURE_TLS_CERT`. The server reads files per request, so consumers can tamper with and restore `release/` in place. Because the harness swaps in a throwaway key and repacks, its consumers never prove the exact release artifact or the production key.
- Two archive modes share every signing, manifest, pack, server, and install step. Fixture mode, the default, serves a stub `bin/sce` and asserts exact output from `sce --help` and `sce version`. Real-binary mode (`--binary <path>`, e.g. the Nix-built `sce-release` binary) serves the given executable as `bin/sce` and asserts that `sce --help` exits 0 and that `.version` from `sce version --format json` equals the expected version.
- `--expected-version <v>` overrides the expected version (default: the staged `package.json` version). `--prefix <dir>` installs into a caller-owned directory that is kept after the run; otherwise the prefix is temporary.
- The fixture archive is produced by real GNU `tar`, so the smoke also exercises the in-repo archive reader against the release pipeline's archive format under Node.
- Only the host platform is exercised. The production public key and manifest trust code are not modified.
- Root `flake.nix` runs the smoke in the build sandbox as `checks.<system>.npm-pack-smoke` (part of `nix flake check`; loopback HTTPS works inside the sandbox). The sandbox has no `/usr/bin/env`, so there the installed bin is run as `node <bin>` after its `-x` check. Host runs, including the `release-npm-package` gate, exec the bin directly.

## NixOS VM real-install test

- `nix/tests/npm-nixos-vm.nix` (`pkgs.testers.runNixOSTest`, test name `npm-nixos-vm`, exposed as `packages.x86_64-linux.npm-nixos-vm`) is local-only. It is neither a `checks.<system>` member (so not part of `nix flake check`) nor a `.#ci-checks` member. It boots one minimal NixOS VM with loopback-only networking (`useDHCP = false`, no interfaces, no VLANs, so no external network) and runs every scenario as the non-root user `sce-tester`. The package comes from the repo `npm/` sources, the binary is the Nix-built static-musl `${sceReleasePackage}/bin/sce`, and the expected version is the repo `.version`.
- The VM node sets `environment.stub-ld.enable = false`, so it has no FHS loader (not even the NixOS stub at `/lib64/ld-linux-x86-64.so.2`) and no `/usr/lib` glibc. A passing run shows that the static-musl binary needs no unavailable dynamic libraries.
- Scenario isolation: each scenario subtest gets its own directory under `/home/sce-tester/scenarios/<name>` from `mkScenario` (HOME, TMPDIR, npm cache and userconfig, prefix, fixture dir, server port/PID/log files). Each scenario runs `release-fixture.sh prepare --package-dir ~/npm --binary <sce>` as the test user, starts a detached `release-fixture.sh serve`, and stops it when done. The scenarios share the Nix test helpers `mkScenario`, `asScenarioUser`, `prepareScenarioFixture`, `startScenarioServer`, `stopScenarioServer`, `installScenarioPackage`, and `rebuildScenarioPackage`, and the setup scenario adds `asSetupUser` for its restricted environment. The installer runs for real against real files. No installer function is mocked.
- Subtests, in test-script order:
  - `test user is non-root`: the test user's uid is not `0`.
  - `stage npm package source`: copies the repo `npm/` sources into the test user's HOME.
  - `install the packed package through postinstall with the real binary`: runs `pack-smoke.sh --binary <sce> --expected-version <.version> --prefix <dir>` in the VM.
  - `installed modes`: `runtime/sce` has mode `755`, and `.bin/sce` is executable and resolves to the installed `bin/sce.js`.
  - `direct launcher execution`: runs `.bin/sce --help` through its shebang and checks the `jq`-parsed `.version` from `sce version --format json`.
  - `skipped postinstall and npm rebuild recovery` (scenario `rebuild`): installs the packed tarball with `--ignore-scripts`. It asserts that `runtime/sce` is absent and that the launcher exits non-zero with `The native sce binary is not installed.`. Then, twice (`first rebuild`, `second rebuild`), it runs `npm rebuild @crocoder-dev/sce --prefix <dir> --foreground-scripts` against the fixture server and asserts `runtime/sce` mode `755`, `--help`, and the parsed `.version`. This proves that a skipped postinstall is recoverable through npm alone and that repeated rebuilds keep a working install.
  - `failed reinstall preserves existing binary` (scenario `reinstall`): installs with postinstall and records the `runtime/sce` SHA-256. It then tampers with the served `release/` in place and reruns postinstall with `npm rebuild`. First it swaps the manifest `.sig` for a well-formed signature over other bytes, and the rebuild must fail with `Release manifest authenticity check failed`. Next it restores the signature and appends bytes to the archive, and the rebuild must fail with `Downloaded sce archive checksum mismatch`. Finally it restores the archive, and the rebuild must succeed. After the initial install, after each failure, and after the restored reinstall, it asserts the recorded hash, mode `755`, a working launcher that reports `.version`, no `sce-npm-install-*`/`sce-npm-text-*` dirs in the scenario TMPDIR, and only `sce` in `runtime/`. This proves end to end that a rejected download leaves the previous binary byte-identical and leaves no staging residue. The unit-level malformed-manifest and archive permutations stay in `npm/test/*.test.js` and are not repeated here.
  - `real sce setup --bootstrap-context` (scenario `setup`): installs with postinstall, then runs the installed launcher as the test user with a PATH that holds only the `nodejs`, `git`, and `coreutils` bin dirs. Under that PATH, `tar`/`curl`/`gzip`/`bash`/`jq` must not resolve. HOME, the XDG dirs, and TMPDIR are isolated, and `GIT_CONFIG_NOSYSTEM=1` is set. It creates a fresh Git repository under the scenario directory, with a placeholder `origin` remote to satisfy the setup remote preflight. From the repo root it runs `sce setup --bootstrap-context` and asserts the exact `context/` tree: `plans`, `handovers`, `decisions`, `tmp`, `overview.md`, `architecture.md`, `patterns.md`, `glossary.md`, `context-map.md`, and `tmp/.gitignore` (`*` / `!.gitignore`). It then appends a sentinel to `context/overview.md` and reruns from a nested subdirectory. It asserts an unchanged tree, a byte-identical `overview.md`, and the preserved sentinel. Finally it asserts that no ancestor of the repo is a Git checkout or SCE source tree, and that no bootstrapped file references `~/npm` or `/nix/store`. `--repo` is not exercised, because the CLI rejects it alongside `--bootstrap-context`.
  - `minimal runtime dependencies` (scenario `minimal`): installs with postinstall, then builds a bin dir that holds only a `node` symlink. It asserts four things. `tar`, `curl`, `gzip`, and `bash` resolve neither on that dir nor on `/var/empty`. `readelf -lW` (run only by the test driver) shows no `INTERP` header for `runtime/sce`. `/lib64/ld-linux-x86-64.so.2` is absent. It then runs `.bin/sce --help` and `version --format json` under `env -i PATH=<node-only dir> HOME=<scenario dir>`, and runs `runtime/sce` directly under `env -i PATH=/var/empty`. It parses `.version` from each JSON in the test script, with no fallback to the system PATH. This proves that the launcher needs only `node` and that the installed binary needs nothing at all on PATH.
- Fixture mode versus the VM: fixture mode proves the packed package, the signing/manifest/checksum/archive flow, and the launcher wiring on the host, cheaply on every `nix flake check`. Its stub cannot reveal native linkage problems. The VM installs the actual static-musl `x86_64-unknown-linux-musl` release binary that the archive pipeline ships. It also proves that binary runs through the npm launcher on a system with no FHS libraries, along with the recovery, failed-reinstall, real-setup, and minimal-runtime scenarios above.
- The VM covers only `linux/x64`. `linux/arm64` and `darwin/arm64` npm installs are not executed by any npm-level test. Their native archives are covered only by the release pipeline's portability audits and its extracted-binary `sce version --format json` smoke before upload.
- Running the VM test locally: `nix build .#npm-nixos-vm --print-build-logs` on x86_64-linux. With `/dev/kvm` it runs accelerated. Without it, QEMU falls back to TCG emulation, which is slower but still passes. CI does not run it, because hosted `ubuntu-latest` runners do not expose `/dev/kvm`. Run it manually before release-affecting npm or musl-build changes.

## Hermetic fixtures versus the exact release artifact

- Every consumer of `npm/test/release-fixture.sh` substitutes a throwaway RSA manifest-signing key. That covers `npm-pack-smoke`, host `pack-smoke.sh` runs, the `release-npm-package` post-pack gate, and every `npm-nixos-vm` subtest. Each consumer writes the throwaway public key into a staged package copy, re-signs a freshly built manifest with the throwaway private key, and repacks (or, for `--tarball`, repacks the extracted contents). They serve that repacked tarball and a rebuilt archive over loopback HTTPS with an ephemeral self-signed certificate, selected through `SCE_NPM_RELEASE_BASE_URL` and `NODE_EXTRA_CA_CERTS`.
- These checks are therefore hermetic development coverage of the package layout, installer behavior, and native binary runtime. They do not prove the exact published `sce-v<version>-npm.tgz` bytes, the bundled production public key `npm/lib/release-manifest-public-key.pem`, the production-signed release manifest, or real GitHub Releases TLS and download URLs. A passing VM or pack-smoke run is not release evidence for those properties.
- Proving the exact artifact and production key is the job of the release-security canary (below), which installs the unmodified tarball with its bundled key. It is not part of the fixture harness.

## Release-security canary

- `npm/test/release-canary.sh` installs the exact, unmodified `sce-v<version>-npm.tgz` with its bundled production key. It never re-signs, repacks, or substitutes a key. It sources `release-fixture.sh` with the `release-canary` log prefix for the tool check, the loopback server, the offline fixture install, and the layout assertions. It requires `--expected-version` and an exact version: dist-tags and ranges are rejected, never resolved.
- Modes (exactly one):
  - `--release-dir <dir> --tarball <tgz>`: serves the real manifest, `.sig`, and archives from `<dir>` over loopback HTTPS with an ephemeral certificate. It is the only mode that sets `SCE_NPM_RELEASE_BASE_URL`/`NODE_EXTRA_CA_CERTS`, and it sets them through the harness install.
  - `--github-release v<version>`: downloads the tgz, manifest, and `.sig` from the public `releases/download` URL and installs with no installer overrides.
  - `--registry-version <version>` (optional `--tarball`, default the GitHub asset): with bounded retries (10 × 30s), it requires the registry `dist.integrity` and the `npm pack` bytes of that exact version to equal the GitHub tgz. It then installs `@crocoder-dev/sce@<version>` from the registry and checks the lockfile integrity.
  - Both remote modes run every download, install, and `sce` call under `env -i` with only `PATH`, an isolated `HOME`/`TMPDIR`, and the npm cache/userconfig, so caller-set installer overrides cannot leak in.
- Every mode asserts the following:
  - The tarball SHA-256 is unchanged by the run and equals the optional `--expected-sha256`.
  - The packed name and version are correct.
  - The packed `package/lib/release-manifest-public-key.pem` is byte-identical to the checked-in `npm/lib/release-manifest-public-key.pem`.
  - `openssl dgst -verify` of the production manifest against that key passes, and the manifest version matches.
  - `runtime/sce` has mode `755`, `.bin/sce` resolves to `bin/sce.js`, and the installed launcher is executable.
  - `sce --help` runs, and `version --format json` reports the expected version.
  - The packed launcher mode is logged, not enforced, because published tarballs up to `v0.4.0-pre-alpha-v12` pack `bin/sce.js` as `0644`.
- Failures print distinct `release-canary:` messages and exit 1. Success ends with `release-canary: OK (<mode> mode: …)`.
- The script is not yet wired into any workflow. Its release-gate usage is not established here.

## Verification baseline

- `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`
- `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh`
- `nix build .#sce-release && nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary ./result/bin/sce`
- `nix build .#checks.<system>.npm-pack-smoke --print-build-logs`
- `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/release-fixture.sh prepare <dir>` then `... release-fixture.sh serve <dir>` (reusable hermetic fixture for other consumers)
- `nix build .#npm-nixos-vm --print-build-logs` (x86_64-linux; uses KVM when `/dev/kvm` exists, otherwise slower QEMU emulation)
- `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/release-canary.sh --expected-version <v> (--release-dir <dist/cli> --tarball <tgz> | --github-release v<v> | --registry-version <v>)` (exact-artifact canary; the remote modes need network)
- `nix develop -c sh -c 'cd npm && SCE_NPM_SKIP_DOWNLOAD=1 bun ./lib/install.js'`
- `nix run .#release-manifest -- --version <semver> --artifacts-dir <path> --out-dir <path> --signing-key-file <path>`
- `nix run .#release-npm-package -- --version <semver> --out-dir <path>`

See also: [cli-first-install-channels-contract.md](./cli-first-install-channels-contract.md), [cli-release-artifact-contract.md](./cli-release-artifact-contract.md), [../overview.md](../overview.md)
