# Plan: harden-npm-installer

## Change summary

Harden the `@crocoder-dev/sce` npm launcher package under `npm/` so that it is reliably executable after a real `npm pack` + `npm install`, only accepts a signed release manifest whose identity matches the package (version, binary `sce`, exactly one canonical target artifact with a valid SHA-256), and extracts the native binary through a bounded, entry-validating extractor instead of unrestricted `tar -xzf`, installing it atomically with mode `0755` while preserving an existing valid install on failure.

This extends the existing architecture (npm launcher -> signed GitHub Release manifest -> SHA-256 verified archive -> native binary) rather than replacing it. The bundled public key, manifest signing flow, supported platform matrix (`darwin/arm64`, `linux/arm64`, `linux/x64`), and release archive layout (`sce-v<version>-<target>/{bin/sce,LICENSE,README.md}`) are preserved. It also adds a locally controlled packed-package smoke test to the flake check surface and to `release-npm-package`, and corrects `npm/README.md`, which currently lists GNU Linux targets while `npm/lib/platform.js` and the release pipeline use musl.

Current-state facts this plan starts from:

- `npm/bin/sce.js` and every other `npm/` file are tracked in Git as mode `100644`; `package.json` maps `bin.sce` to `./bin/sce.js`.
- `npm/lib/install.js` verifies the manifest signature, then trusts `artifact.archive` (falling back to the canonical name), requires only a truthy `checksum_sha256`, uses the first `target_triple` match, never compares `manifest.version`/`binary` to the package, extracts with `tar -xzf` into the download temp dir, then `copyFileSync` + `chmodSync` directly onto `runtime/sce` (non-atomic).
- `release-manifest` emits `{version, binary: "sce", artifacts: [...]}`; each artifact fragment carries `version`, `binary`, `archive`, `checksum_file`, `checksum_sha256`, `target_triple`, `os`, `arch`.
- `flake.nix` `npmSrc` is an explicit file list; new `npm/` source or test files must be added to it to reach `npm-bun-tests` / Biome checks.
- `release-npm-package` copies `npm/` with `cp -R` and runs `npm pack`; it performs no post-pack mode or execution check.

## Acceptance criteria

- [ ] AC1: A tarball produced by `npm pack` of `npm/` installs into an isolated prefix, and the installed `sce` bin (npm-generated link to `bin/sce.js`) is executable and successfully runs `sce --help` and `sce version` against a locally served fixture release, with no network access.
  - Validate: `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs` (new check) passes; the smoke script asserts exit 0 and expected stdout for both commands.
- [ ] AC2: The root cause of the non-executable launcher is identified, fixed at its source, and recorded in the plan task evidence; the packed tarball entry `package/bin/sce.js` carries an executable mode.
  - Validate: `git ls-files -s npm/bin/sce.js` shows the fixed mode (if Git mode is the cause) and the smoke check inspects the packed tarball entry mode.
- [ ] AC3: After signature verification and before any archive download, the installer rejects manifests whose `version` differs from the package version, whose `binary` is not `sce`, that have zero or more than one artifact for the current target, whose artifact `archive` is not `sce-v<version>-<target>.tar.gz`, or whose `checksum_sha256` is not 64 lowercase hex characters.
  - Validate: `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` — negative tests for each case assert the specific error and that the archive downloader was never called.
- [ ] AC4: Archive extraction no longer shells out to unrestricted `tar -xzf`; the installer parses the gzip+ustar stream itself with bounded sizes, rejects absolute paths, `..` traversal, symlinks, hardlinks, duplicate entries, unexpected entry types, entries outside the expected root, oversize payloads, and a missing/non-regular `bin/sce`, and writes only `bin/sce` into an isolated staging directory.
  - Validate: `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` — fixture archives for each malicious case are rejected and nothing is written outside staging; `nix shell nixpkgs#ripgrep -c rg -n '"-xzf"|spawnSync\("tar"' npm/lib` returns no match.
- [ ] AC5: Installation is atomic: the binary is staged next to `runtime/sce`, chmodded `0755`, and renamed into place; a checksum failure, extraction failure, or interruption leaves a previously installed valid `runtime/sce` byte-identical and executable, and leaves no partial `runtime/sce`.
  - Validate: `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` — interrupted-install and failure tests over a real temp `runtime/` directory.
- [ ] AC6: `nix run .#release-npm-package -- --version <.version> --out-dir <dir>` fails if the packed tarball's `bin/sce.js` is not executable or the packed-package smoke install fails.
  - Validate: run the app on a clean tree (passes); run it with `npm/bin/sce.js` temporarily set to `0644` in a scratch copy (fails with a permission diagnostic). Inspection of `.github/workflows/release-sce.yml` confirms it invokes this app before upload.
- [ ] AC7: `npm/README.md` lists `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`, and `context/sce/cli-npm-distribution-contract.md` describes the manifest identity checks, safe extraction, atomic install, and the packed-package smoke verification.
  - Validate: `nix shell nixpkgs#ripgrep -c rg -n 'linux-gnu' npm/README.md` returns no match; inspect the contract file.

### Full validation

- `nix flake check`
- `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`
- `nix run .#release-npm-package -- --version "$(cat .version)" --out-dir <scratch-dir>`

### Context sync

- `context/sce/cli-npm-distribution-contract.md` (install flow, validation rules, verification baseline)
- `context/sce/cli-release-artifact-contract.md` (only if `release-npm-package` validation behavior is described there)
- `context/glossary.md` (`npm JS flake checks` entry gains the packed-package smoke check)
- `context/overview.md` (flake check list mention of the npm slice, if it enumerates checks)

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `npm/bin/sce.js` (mode/shebang only), `npm/lib/install.js`, `npm/lib/platform.js`, new `npm/lib/` helper module(s) for archive reading, `npm/test/**`, a packed-package smoke script under `npm/test/` or `scripts/`, `npm/README.md`, `flake.nix` (`npmSrc`, new check, `release-npm-package`), `.github/workflows/release-sce.yml` only if it needs to surface the new validation, `context/sce/cli-npm-distribution-contract.md` and related context entries.
- **Out of scope:** changing the native build pipeline, release archive layout, manifest schema or signing key, supported platform matrix, publish workflow topology, or any npm/crates/GitHub publication.
- **Constraints:** no new npm runtime dependencies (use Node built-ins `zlib`, `fs`, `crypto`); Node `>=18` compatibility; Biome formatting/linting; all tooling via Nix; tests must not require network access; do not push or publish.
- **Non-goal:** a general-purpose tar implementation — the reader supports only the ustar/GNU subset the release pipeline emits (regular files and directories, optional pax/GNU long-name headers if GNU tar emits them) and rejects everything else.

## Assumptions

- The safe extractor is a small in-repo ustar reader over `zlib.gunzipSync`/streamed gunzip with explicit size caps (compressed archive, total uncompressed bytes, entry count, `bin/sce` size); no `tar` npm dependency is justified for this subset.
- Allowed archive entries are exactly the archive root directory, `bin/`, `bin/sce` (regular file), `LICENSE`, and `README.md` under `sce-v<version>-<target>/`; any other entry is rejected rather than ignored, matching what `release-artifacts` packs.
- The packed-package smoke test uses a locally controlled fixture: a stub executable `sce` (shell script answering `--help` and `version`) packed in the canonical archive layout, a test-generated signing key swapped into the staged fixture copy of the package (the production `release-manifest-public-key.pem` and trust code are unchanged), and a loopback HTTPS server with a generated self-signed certificate trusted via `NODE_EXTRA_CA_CERTS`, pointed at by the existing `SCE_NPM_RELEASE_BASE_URL` override. Real native-binary execution on every platform is not claimed; only the host platform running the check is exercised.
- The root cause of the permission failure is determined by reproduction in T01 (candidates: Git mode `100644` on `bin/sce.js` carried through `cp -R` + `npm pack`, and how npm/bin-links/other package managers treat bin targets); the fix targets whichever is reproduced.
- Manifest version comparison is exact string equality with `npm/package.json` `version`.

## Task stack

- [x] T01: `Fix launcher executable permission and add packed-package smoke test` (status:done)
  - Task ID: T01
  - Scope: In — reproduce the non-executable launcher via real `npm pack` + `npm install --prefix <tmp>`; identify and fix the root cause (e.g. Git mode `100755` for `npm/bin/sce.js`, packaging step); add a smoke script that packs `npm/`, builds the local fixture release (stub binary archive, signed manifest with a test key swapped into the staged package copy, loopback HTTPS server), installs into an isolated prefix with `SCE_NPM_RELEASE_BASE_URL`, asserts the packed `package/bin/sce.js` mode, and runs `sce --help` and `sce version`. Out — manifest validation and extractor changes (T02/T03), flake/release wiring (T04).
  - Dependencies: none
  - Done when: the smoke script fails on the pre-fix tree for the reproduced reason and passes after the fix; root cause is recorded in task evidence.
  - Verify: `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash <smoke-script>` before and after the fix; `git ls-files -s npm/bin/sce.js`; `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`.
  - Completed: 2026-11-03
  - Files changed: `npm/bin/sce.js` (mode only, `100644` -> `100755`, staged in the index; content unchanged), `npm/test/pack-smoke.sh` (new, mode `0755`), `npm/test/fixture-release-server.mjs` (new).
  - Root cause: `npm/bin/sce.js` was committed as Git mode `100644`; `release-npm-package` copies `npm/` with `cp -R` (preserving the non-executable filesystem mode) and `npm pack` records it verbatim, so the published tarball entry `package/bin/sce.js` was `-rw-r--r--`. Reproduction (npm 11.13.0 / Node 24.16.0): `tar -tvzf` of the packed tarball showed `-rw-r--r-- package/bin/sce.js`. npm 11 (`--prefix` and `-g`), bun 1.3, and pnpm all chmod bin targets while linking, so those installers mask the defect; any consumer that executes the packed `bin/sce.js` without bin-link repair gets a permission failure. Fixed at the source by committing the launcher as `100755`.
  - Result: Added `npm/test/pack-smoke.sh`, which stages `npm/`, swaps a throwaway RSA-2048 manifest key into the staged copy only, builds a stub `sce` archive in the canonical `sce-v<version>-<target>/{bin/sce,LICENSE,README.md}` layout with a signed manifest (`openssl dgst -sha256 -sign`, base64), serves it from `npm/test/fixture-release-server.mjs` (loopback HTTPS, self-signed cert trusted via `NODE_EXTRA_CA_CERTS`), runs `npm pack`, asserts the packed `package/bin/sce.js` mode is executable, installs with `npm install --prefix <tmp> --offline` using `SCE_NPM_RELEASE_BASE_URL` and an isolated npm cache/config, then asserts exact stdout for `sce --help` and `sce version`. Accepts `--package-dir <path>` for later reuse (T04). The production public key and trust code are unchanged.
  - Verify outcomes:
    - Pre-fix smoke: `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh` -> exit 1, `Packed package/bin/sce.js has mode -rw-r--r--; expected an executable mode` (reproduced reason).
    - Post-fix smoke: same command -> exit 0, `packed package/bin/sce.js mode -rwxr-xr-x`, `pack-smoke: OK (sce --help, sce version)`; temp work dir removed on exit.
    - `git ls-files -s npm/bin/sce.js` -> `100755 13ef5b35... npm/bin/sce.js`.
    - Bun tests: `nix shell nixpkgs#bun -c bun test ./test/install.test.js ./test/platform.test.js` (from `npm/`) -> 25 pass, 0 fail. The `nix develop -c sh -c '... bun test ...'` form was blocked by the repo bash policy `use-nix-for-bun`, so the nixpkgs Bun form was used.
    - `nix develop -c biome check npm/bin npm/lib npm/test` -> no errors (one pre-existing warning: unused `getRepositorySlug` in `npm/lib/install.js`, untouched).
    - `nix shell nixpkgs#shellcheck -c shellcheck npm/test/pack-smoke.sh` -> clean.
  - Deviations: the new smoke files are not yet in `flake.nix` `npmSrc`, so flake Biome/test checks do not see them until T04 wires them in (as planned).
  - Context impact: localized to the npm launcher packaging/verification surface; affects `context/sce/cli-npm-distribution-contract.md` (launcher mode requirement, new local smoke script). No flake check or glossary changes yet (T04/T05).
  - Context synchronization: synced

- [x] T02: `Validate signed manifest identity before archive download` (status:done)
  - Task ID: T02
  - Scope: In — after `parseVerifiedReleaseManifest`, validate `version` equals package version, `binary === "sce"`, exactly one artifact for the target triple, `archive` equals `getArchiveName(version, target)`, `checksum_sha256` matches `/^[0-9a-f]{64}$/`; drop the `artifact.archive ??` fallback trust; update `selectReleaseArtifact` and existing fixtures (`abc123` checksum) accordingly; negative tests for version mismatch, wrong binary, missing/duplicate target, invalid archive name, invalid digest, each asserting no archive download. Out — extraction and install mechanics.
  - Dependencies: T01
  - Done when: every inconsistency is rejected with a specific error before `downloadArchive` is invoked, valid manifests still install, and the bundled key/signature flow is unchanged.
  - Verify: `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`; `nix develop -c biome check npm`.
  - Completed: 2026-11-03
  - Files changed: `npm/lib/platform.js`, `npm/lib/install.js`, `npm/test/install.test.js`, `npm/test/platform.test.js`.
  - Result: Added `validateReleaseManifestIdentity(manifest, { version, targetTriple })` to `npm/lib/platform.js`; it requires a JSON object manifest, exact `version` equality with the package version, `binary === "sce"`, exactly one artifact for the target (via `selectReleaseArtifact`, which now rejects duplicates with `found N artifacts for target <triple>; expected exactly one.`), `archive === getArchiveName(version, target)`, and `checksum_sha256` matching `/^[0-9a-f]{64}$/`, returning `{ artifact, archiveName, expectedChecksum }`. `installBinaryWithDependencies` calls it immediately after `loadReleaseManifest`, before creating the temp dir or downloading; the `artifact.archive ??` fallback and the truthy-only checksum check were removed, so the downloaded archive name is always canonical. Signature verification, the bundled public key, and `parseVerifiedReleaseManifest` are unchanged. Test fixtures now use full canonical manifests (`binary`, `archive`, 64-hex checksum) instead of `abc123`; new negative tests cover version mismatch, wrong/missing binary, missing target, duplicate target, non-canonical/missing archive, and short/uppercase/non-hex/missing checksum, each asserting zero archive downloads; a positive test asserts the canonical archive URL is requested.
  - Verify outcomes:
    - `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'` -> 38 pass, 0 fail.
    - `nix develop -c biome check npm` -> no errors; one pre-existing warning (unused `getRepositorySlug` in `npm/lib/install.js`, untouched).
    - Additional regression: `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh` -> `pack-smoke: OK (sce --help, sce version)` (the T01 fixture manifest already carries canonical identity fields).
  - Assumptions: per-artifact `version`/`binary` fields are not separately validated (AC3 covers top-level identity; the canonical archive name already encodes version and target). Version comparison is exact string equality.
  - Context impact: localized to the npm installer trust flow; affects `context/sce/cli-npm-distribution-contract.md` (manifest identity validation rules and new error surface). No architecture, glossary, or flake check changes.
  - Context synchronization: synced

- [x] T03: `Replace tar extraction with bounded validating extractor and atomic install` (status:done)
  - Task ID: T03
  - Scope: In — new `npm/lib/` archive module reading gzip+ustar with size/entry caps and the allowed-entry rules; extract only `bin/sce` into a fresh staging dir; stage into `runtime/` with a temp name, `chmod 0755`, `fsync`, and `rename` over `runtime/sce`; clean up staging on any failure; tests with generated fixture archives for absolute path, `..` traversal, symlink, hardlink, duplicate entry, unexpected type (e.g. FIFO/char device), entry outside root, oversize payload, missing or non-regular `bin/sce`, checksum failure, and an interrupted install (failure injected between staging and rename) preserving a prior valid `runtime/sce`. Out — manifest identity rules (T02), flake/release wiring.
  - Dependencies: T02
  - Done when: `install.js` no longer spawns `tar`; all negative fixtures are rejected without writing outside staging; a prior valid install survives every failure path; the T01 smoke script still passes.
  - Verify: `nix develop -c sh -c 'cd npm && bun test ./test/*.test.js'`; `nix shell nixpkgs#ripgrep -c rg -n 'spawnSync' npm/lib`; rerun the T01 smoke script.
  - Completed: 2026-11-03
  - Files changed: `npm/lib/archive.js` (new), `npm/lib/install.js`, `npm/test/archive.test.js` (new), `npm/test/tar-fixture.js` (new), `npm/test/install.test.js`, `flake.nix` (`npmSrc` only).
  - Result: Added `npm/lib/archive.js` with `readReleaseBinaryFromArchive`, `extractReleaseBinary`, `installBinaryAtomically`, and `ARCHIVE_LIMITS` (64 MiB compressed, 256 MiB uncompressed via `gunzipSync` `maxOutputLength`, 192 MiB `bin/sce`, 16 entries, 4 KiB long names). The reader parses ustar/GNU headers itself (header checksum and magic verified, GNU `L` long names and pax `x` `path` supported, base-256 numeric fields and pax `size`/`linkpath` rejected) and accepts only `<root>/`, `<root>/bin/`, `<root>/bin/sce` (regular), `<root>/LICENSE`, `<root>/README.md`; it rejects absolute paths, `..`/empty/`.` segments, backslashes, symlinks, hardlinks, devices, FIFOs and other types, entries outside the root, unexpected or duplicate entries, wrong entry kinds, oversize payloads, truncated streams, and a missing `bin/sce`. Only `bin/sce` is written, to `<download-temp>/staging/sce` (`wx`, `0600`). `installBinaryAtomically` writes a `.sce.<pid>.<ts>.tmp` file in `runtime/`, `fchmod 0755`, `fsync`, renames it over `runtime/sce`, fsyncs the directory best-effort, and removes the temp file on any failure. `installBinaryWithDependencies` no longer spawns `tar`; its `fileExists`/`createRuntimeDir`/`copyBinary`/`chmodBinary` injection points were replaced by `extractArchiveFn(archivePath, { archiveRoot, stagingDir })`, `runtimeDir`, and `installBinaryFile`. Tests build fixture archives with an in-repo ustar/GNU writer (`npm/test/tar-fixture.js`) and cover every malicious case, a real GNU tar archive, prefix-split and long-name headers, and checksum, extraction, and interrupted-rename failures over a real temp `runtime/` that preserve a prior `0755` binary byte-identical with no leftover temp files.
  - Verify outcomes:
    - The planned `nix develop -c sh -c 'cd npm && <bun> test ./test/*.test.js'` form was blocked by repo bash policy `use-nix-for-bun`; ran the nixpkgs Bun form over `./test/archive.test.js ./test/install.test.js ./test/platform.test.js` (from `npm/`) -> 69 pass, 0 fail.
    - `nix build .#checks.x86_64-linux.npm-bun-tests .#checks.x86_64-linux.npm-biome-check .#checks.x86_64-linux.npm-biome-format` -> exit 0 (69 pass in the sandbox, including the GNU tar compatibility test).
    - `nix shell nixpkgs#ripgrep -c rg -n 'spawnSync|"-xzf"|spawnSync\("tar"' npm/lib` -> no match (exit 1).
    - T01 smoke: `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh` -> `pack-smoke: OK (sce --help, sce version)` (a real GNU tar archive extracted by the new reader under Node).
    - `nix develop -c biome check npm` -> no errors; one pre-existing warning (unused `getRepositorySlug`, untouched).
  - Deviations: `flake.nix` `npmSrc` gained `npm/lib/archive.js`, `npm/test/archive.test.js`, and `npm/test/tar-fixture.js` now rather than in T04, because `install.js` imports the new module and the flake npm checks would otherwise break; the new files were registered with `git add -N` so the flake sees them. The T01 smoke files remain unwired until T04. Per user instruction during implementation, the new code carries no comments.
  - Context impact: localized to the npm installer extraction/install surface; affects `context/sce/cli-npm-distribution-contract.md` (safe extraction rules, size limits, atomic install, removed host `tar` runtime requirement). No architecture or glossary changes.
  - Context synchronization: synced

- [x] T04: `Wire packed-package smoke into flake checks and release-npm-package` (status:done)
  - Task ID: T04
  - Scope: In — add new `npm/` files to `npmSrc`; add a `npm-pack-smoke` flake check running the T01 smoke script in the sandbox (nodejs, openssl); make `release-npm-package` assert the packed tarball's `package/bin/sce.js` is executable and run the smoke against the produced tarball before reporting success; confirm `release-sce.yml` already invokes `release-npm-package` (edit only if needed). Out — publish workflow behavior changes, real network release testing.
  - Dependencies: T03
  - Done when: `nix flake check` includes and passes the new check; `release-npm-package` fails on a `0644` launcher in a scratch copy and passes on the real tree.
  - Verify: `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs`; `nix run .#release-npm-package -- --version "$(cat .version)" --out-dir <scratch>`; `nix run nixpkgs#actionlint` if a workflow is touched.
  - Completed: 2026-11-03
  - Files changed: `flake.nix`, `npm/test/pack-smoke.sh`.
  - Result: `npmSrc` now includes `npm/test/fixture-release-server.mjs` and `npm/test/pack-smoke.sh`. New flake check `npm-pack-smoke` (`mkCopiedSourceCheck` over `npmSrc`, inputs bash/coreutils/gnutar/gzip/nodejs/openssl, `HOME` set inside the build dir) runs `bash ./test/pack-smoke.sh` in the sandbox and is registered in `checks`. `pack-smoke.sh` gained `--tarball <path>`: it checks the launcher mode in that packed tarball, extracts its `package/` contents into the stage directory, and then runs the usual key-swap/repack/install/run flow. When `/usr/bin/env` is missing (Nix sandbox), the installed bin is run as `node <bin>` after the existing `-x` check; on hosts it is still exec'd directly. `release-npm-package` adds bash/gnutar/gzip/openssl to its runtime inputs, runs `bash npm/test/pack-smoke.sh --tarball "$out_dir/$package_name"` after packing, and on failure deletes the tarball and exits 1 before writing metadata. `.github/workflows/release-sce.yml` already runs `nix run .#release-npm-package` in the "Build npm release package" step, before release creation/upload, so it was not edited. Per a user instruction during implementation, the descriptive comments in `pack-smoke.sh` were removed (only the shebangs and the functional `shellcheck disable` directive remain).
  - Verify outcomes:
    - `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs` -> exit 0; `packed package/bin/sce.js mode -rwxr-xr-x`, `pack-smoke: OK (sce --help, sce version)` (loopback HTTPS works inside the sandbox, which answers the plan's first open question).
    - `nix run .#release-npm-package -- --version "$(cat .version)" --out-dir <scratch>` on the real tree -> exit 0; smoke OK; tarball and metadata written.
    - Same app run from a scratch copy of the tracked tree with `npm/bin/sce.js` set to `0644` -> exit 1, `pack-smoke: Packed package/bin/sce.js has mode -rw-r--r--; expected an executable mode ...`, tarball removed, output dir left empty.
    - `nix flake check` -> `all checks passed!`; `checks.x86_64-linux` includes `npm-pack-smoke`.
    - `nix shell nixpkgs#shellcheck -c shellcheck npm/test/pack-smoke.sh` -> clean; `nix develop -c biome check npm` -> no errors (one pre-existing `getRepositorySlug` warning, untouched).
    - actionlint not run: no workflow was touched.
  - Deviations: the sandbox has no `/usr/bin/env`, so the launcher's `#!/usr/bin/env node` shebang cannot exec there. In that case the smoke runs the installed bin through `node` and relies on the `-x` check plus the packed-mode assertion. The direct-exec path is still exercised by `release-npm-package` and by host runs.
  - Context impact: affects verification and release surfaces. `context/sce/cli-npm-distribution-contract.md` (new `npm-pack-smoke` check, `--tarball` smoke mode, `release-npm-package` post-pack gate), `context/glossary.md` `npm JS flake checks` entry, and any flake-check enumeration in `context/overview.md`; possibly `context/sce/cli-release-artifact-contract.md` if it describes `release-npm-package` validation.
  - Context synchronization: synced

- [ ] T05: `Correct npm README targets and update distribution contract` (status:todo)
  - Task ID: T05
  - Scope: In — `npm/README.md` musl target triples and a short note on manifest identity/safe-extraction failures in troubleshooting; `context/sce/cli-npm-distribution-contract.md` install-flow, validation, atomic-install, and verification-baseline updates; glossary/overview entries for the new flake check. Out — documentation site changes.
  - Dependencies: T04
  - Done when: README matches `npm/lib/platform.js`; contract describes implemented behavior and the new verification commands.
  - Verify: `nix shell nixpkgs#ripgrep -c rg -n 'linux-gnu' npm/README.md` (no match); `nix develop -c biome check npm`.
  - Context synchronization: pending

## Open questions

- Is the loopback HTTPS + self-signed certificate fixture acceptable inside the Nix build sandbox for `npm-pack-smoke`? If sandbox loopback networking proves unavailable on a CI runner, the fallback is to keep the smoke as a `nix run` app invoked by `release-npm-package` and PR CI instead of a pure flake check.
- Should the smoke additionally run the real `nix build .#default` binary inside the fixture archive on the host platform? This gives real-binary coverage for the host only, at the cost of making the check depend on the full CLI build; the plan defaults to a stub binary.
