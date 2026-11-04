# Plan: npm-vm-scenarios-and-release-canary

## Change summary

Extends PR #345 (branch `npm-nixos-integration-test`, predecessor plan `npm-nixos-vm-integration-test`, all tasks done) in two separable parts.

First, the local-only NixOS VM test `nix/tests/npm-nixos-vm.nix` gains four scenario subtests on the same VM, each with its own isolated HOME/prefix/fixture directory: skipped-postinstall recovery through `npm rebuild`, a failed reinstall (bad manifest signature, then archive checksum mismatch) that must leave the previously installed binary byte-identical and executable with no abandoned staging files, real `sce setup --bootstrap-context` operation in a throwaway Git repository, and minimal-runtime execution under a restricted PATH with no tar/curl/gzip/Bash and no FHS loader. To support this without a second HTTPS fixture, the release-fixture setup now inlined in `npm/test/pack-smoke.sh` (throwaway signing key swap, canonical archive, signed manifest, loopback TLS, `fixture-release-server.mjs` start/stop, isolated install and layout assertions) is factored into one reusable harness that pack-smoke, the VM, and the new canary all consume. The VM fixture keeps substituting a throwaway public key and is documented as hermetic development coverage, not proof of the exact release artifact.

Second, a release-security canary installs the exact, unmodified `sce-v<version>-npm.tgz` with its bundled production public key against the production-signed manifest and real native archive, asserts the bundled key equals the checked-in repository key, and checks launcher/symlink/binary modes and the reported version. It runs at three gates: in `release-sce.yml` against the assembled artifacts served over loopback HTTPS before the GitHub Release is created (B1, in a job that receives no secrets); in `publish-npm.yml` against the published GitHub Release over the normal download URL with real TLS and no installer overrides before `npm publish` (B2); and after publication against the exact registry version, comparing registry bytes to the GitHub asset (registry canary). `publish-npm.yml` becomes a reusable `workflow_call` invoked by `release-sce.yml` after release creation, keeping `workflow_dispatch` and `release: published` for manual reruns and externally created releases, with `NPM_TOKEN` scoped to the publish step only. The npm installer (`npm/lib`, `npm/bin`) is not changed.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: One release-fixture harness serves every hermetic and canary install: pack-smoke fixture and `--binary` modes behave as before, `fixture-release-server.mjs` is the only HTTPS server implementation, and no other file generates its own TLS server or duplicates the sign/serve/install steps.
  - Validate: `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs` passes; `nix build .#sce-release && nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary ./result/bin/sce` passes; `nix shell nixpkgs#ripgrep -c rg -n 'createServer|openssl req' npm nix .github` hits only the shared harness and `fixture-release-server.mjs`.
- [ ] AC2: In the VM, a package installed with `--ignore-scripts` has no `runtime/sce`, its launcher exits non-zero with the missing-binary message, and `npm rebuild @crocoder-dev/sce` with the fixture server available runs postinstall to install a mode-`755` `runtime/sce` that reports `.version`; a second rebuild leaves a working, executable install.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` passes and the log shows the `skipped postinstall and npm rebuild recovery` subtest with the asserted message, mode, and version for both rebuilds.
- [ ] AC3: In the VM, after a successful install, a reinstall attempt against an invalid manifest signature and then against an archive checksum mismatch each fails, leaves `runtime/sce` with the recorded SHA-256 and mode `755` and still executable, and leaves no `sce-npm-install-*`/`sce-npm-text-*` temp directories or temp siblings of `runtime/sce`; restoring the valid fixture makes the next reinstall succeed.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` log shows the `failed reinstall preserves existing binary` subtest with both failure messages, unchanged hashes, empty staging checks, and the final successful reinstall.
- [ ] AC4: In the VM, as the non-root user, from a working directory outside any SCE checkout, `sce setup --bootstrap-context` in a fresh Git repository creates the `context/` baseline (directories `plans`, `handovers`, `decisions`, `tmp`; files `overview.md`, `architecture.md`, `patterns.md`, `glossary.md`, `context-map.md`, `tmp/.gitignore`); a second run succeeds, creates nothing new, and preserves a sentinel appended to an existing context file.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` log shows the `real sce setup --bootstrap-context` subtest with the created paths and the preserved sentinel.
- [ ] AC5: In the VM, the installed launcher (`--help`, `version --format json`) runs under `env -i` with a PATH containing only a directory that links `node`, and `runtime/sce` runs directly under `env -i` with an empty PATH; neither PATH contains tar, curl, gzip, or bash, the binary has no ELF interpreter, and no FHS loader exists at `/lib64/ld-linux-x86-64.so.2`. The Git-dependent scenario uses its own separate PATH, and no subtest falls back to the system PATH.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` log shows the `minimal runtime dependencies` subtest assertions; inspect `nix/tests/npm-nixos-vm.nix` for the restricted environments and the absence of any fallback to the full PATH.
- [ ] AC6: The VM test stays local-only, non-root, and loopback-only, and the installer is unchanged: `npm-nixos-vm` is in neither `checks.<system>` nor `ci-checks`, every scenario runs as the non-root user with a per-scenario isolated directory, and `npm/lib` and `npm/bin` are unchanged by this plan.
  - Validate: `nix eval .#checks.x86_64-linux --apply builtins.attrNames` does not list `npm-nixos-vm`; `nix build .#ci-checks --print-build-logs && ls result/` shows only `sce-release` and `release-portability-audit`; `git diff a4611a4b -- npm/lib npm/bin` is empty; inspect the VM node for `useDHCP = false`, `interfaces = { }`, `vlans = [ ]`.
- [ ] AC7: The release-security canary installs the exact, unmodified tarball and the production key: its SHA-256 is identical before and after the run, the packed `package/lib/release-manifest-public-key.pem` is byte-identical to the checked-in `npm/lib/release-manifest-public-key.pem`, the production-signed manifest verifies against that key, and the installed launcher, `.bin/sce` symlink target, and `runtime/sce` modes and the reported version are asserted. Local-directory, GitHub-release, and registry modes each pass against the already published `v0.4.0-pre-alpha-v12` artifacts, and the GitHub/registry modes set none of `SCE_NPM_RELEASE_BASE_URL`, `SCE_NPM_SKIP_DOWNLOAD`, or `NODE_EXTRA_CA_CERTS`.
  - Validate: run the three canary modes against `v0.4.0-pre-alpha-v12` (commands recorded in T07) and capture exit 0 with the printed hashes, key comparison, modes, and version; `nix shell nixpkgs#ripgrep -c rg -n 'SCE_NPM_RELEASE_BASE_URL|SCE_NPM_SKIP_DOWNLOAD|NODE_EXTRA_CA_CERTS' npm/test/release-canary.sh` shows them only in the local-directory mode path.
- [ ] AC8: `release-sce.yml` cannot create a GitHub Release unless the B1 canary passed on the exact assembled artifacts, and the signing private key is referenced only in the manifest-signing step of a job distinct from the canary job.
  - Validate: inspect `.github/workflows/release-sce.yml`: the release-creation job `needs` the canary job and gates on its success; `SCE_RELEASE_MANIFEST_SIGNING_KEY` appears only in one step's `env`; the canary job has no `secrets.` reference; `nix build .#checks.x86_64-linux.workflow-actionlint` passes.
- [ ] AC9: npm publication runs from `release-sce.yml` through `workflow_call` only after release creation succeeds, the GitHub-release canary must pass before `npm publish`, the registry canary runs after publication against the exact version with bounded retries and fails the workflow on mismatch or unusable install, `NPM_TOKEN` is passed explicitly (not `secrets: inherit`) and exposed only to the publish step, no canary job references a secret, and `workflow_dispatch` with a release tag and `release: published` still start the same pipeline.
  - Validate: inspect `.github/workflows/publish-npm.yml` and `.github/workflows/release-sce.yml` for those job dependencies, triggers, secret references, and the retry bound; `nix build .#checks.x86_64-linux.workflow-actionlint` passes. Live execution of B2 and the registry canary inside GitHub Actions is not provable before a real release and must be reported as such.
- [ ] AC10: Each new check fails when its critical assertion is deliberately violated: recovery subtest (rebuild skipped), failed-reinstall subtest (binary replaced or temp file left behind), setup subtest (sentinel overwritten), minimal-runtime subtest (tar added to the restricted PATH, or a binary with an ELF interpreter), and the canary (tarball byte change, swapped public key, wrong expected version).
  - Validate: plan task evidence records each injected fault, made only in scratch copies, with the exact failing subtest or `release-canary:` message; `git status` is clean of fault residue.
- [ ] AC11: Existing JavaScript installer tests for malformed manifests, invalid archive entries, and fault permutations are retained and pass, and the VM does not re-implement those permutations.
  - Validate: `nix build .#checks.x86_64-linux.npm-bun-tests --print-build-logs` passes; `git diff a4611a4b --stat -- npm/test/*.test.js` shows no removals.
- [ ] AC12: Durable docs state that the hermetic VM/pack-smoke fixtures substitute a throwaway key and repack, while only the release-security canary proves the exact artifact and production key; they describe each VM scenario, the three canary gates, the `workflow_call` trigger topology and secret scoping, manual reruns, and what still needs a real release to validate.
  - Validate: inspect `context/sce/cli-npm-distribution-contract.md`, `context/sce/cli-release-artifact-contract.md`, and `context/sce/cli-first-install-channels-contract.md` for those points with command and job names matching the workflows and flake.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix build .#npm-nixos-vm --print-build-logs`
- `nix build .#ci-checks --print-build-logs`
- `nix build .#checks.x86_64-linux.npm-bun-tests --print-build-logs`
- `nix build .#checks.x86_64-linux.workflow-actionlint`

### Context sync

- `context/sce/cli-npm-distribution-contract.md` (shared fixture harness, VM scenarios, fixture-vs-exact-artifact distinction, release-security canary, registry publication topology, verification baseline)
- `context/sce/cli-release-artifact-contract.md` (B1 canary gate and job split in the release orchestrator)
- `context/sce/cli-first-install-channels-contract.md` (release-to-npm workflow topology)
- `context/context-map.md` (annotations for the entries above, if their summaries change)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `npm/test/pack-smoke.sh`, a new shared harness under `npm/test/`, `npm/test/fixture-release-server.mjs` only if the harness strictly needs it, new `npm/test/release-canary.sh`, `nix/tests/npm-nixos-vm.nix`, `flake.nix` (VM test inputs and file sets only), `.github/workflows/release-sce.yml`, `.github/workflows/publish-npm.yml`, the context files listed under Context sync.
- **Out of scope:** `npm/lib/*.js`, `npm/bin/sce.js`, the production public key, release packaging apps (`release-npm-package`, `release-manifest`, `release-artifacts`), `pr-ci.yml`, native build workflows, aarch64 or darwin VM tests, putting the VM into `nix flake check` or `ci-checks`, crates.io/Flatpak publishing.
- **Constraints:** commit on `npm-nixos-integration-test` (the current `more-integration` checkout is at the same commit, `a4611a4b`); VM test tasks (T01–T06) and release-security workflow tasks (T07–T10) land as separate commits; VM scenarios use the real installed package and real filesystem operations, never mocked installer functions, with no external network; production signing secrets never enter Nix derivations, fixtures, logs, or artifacts; B2 and registry canaries set none of `SCE_NPM_RELEASE_BASE_URL`, `SCE_NPM_SKIP_DOWNLOAD`, `NODE_EXTRA_CA_CERTS`; registry canaries never resolve a dist-tag; all registry retries are bounded; workflow actions stay SHA-pinned with `harden-runner` as in the existing jobs; shell scripts follow `#!/usr/bin/env bash` + `set -euo pipefail` and are shellcheck-clean.
- **Non-goal:** repeating the unit-level malformed-manifest/archive permutations in the VM, a generic VM or HTTPS test framework, changing installer behavior or messages, or claiming live gate success before a real release runs.

## Assumptions

- PR #345 is the open PR whose head is `npm-nixos-integration-test` (confirmed with `gh pr view 345`). This is a new plan because every task in `npm-nixos-vm-integration-test` is done and this scope is distinct. Its stale validation failure (AC5 expected KVM in CI, which T05 superseded) is not repaired here.
- The harness is a sourced Bash library plus a small `prepare`/`serve` entrypoint. `prepare` writes a fixture directory (staged package with the throwaway key, packed tgz, release dir, TLS cert and key). `serve` starts `fixture-release-server.mjs` against that directory. The server reads files on each request, so VM scenarios can tamper with `release/` in place and restore from saved copies.
- Reinstall attempts in the VM use `npm rebuild @crocoder-dev/sce --prefix <dir> --foreground-scripts`, which reruns postinstall in place. A failed `npm install` of a tarball may let npm roll back or remove the package directory, which would hide whether the installer preserved the old binary.
- The missing-binary assertion matches the launcher's current text, `The native sce binary is not installed.`. The launcher is not edited (see Open questions).
- "Expected repository key" means byte equality with `npm/lib/release-manifest-public-key.pem` at the checked-out release ref, plus an independent `openssl dgst -verify` of the production-signed manifest with that key.
- The canary's local-directory mode (B1) serves the real `dist/cli` artifacts over loopback HTTPS with the shared harness's ephemeral self-signed certificate. It therefore needs `SCE_NPM_RELEASE_BASE_URL` and `NODE_EXTRA_CA_CERTS`, which the request permits only for B1.
- `release-sce.yml` splits its current `release` job into three jobs. `assemble` is the only job holding the signing secret, and only in its signing step; it uploads `dist/` as a workflow artifact. `release-canary` receives no secrets. `release` needs `release-canary`, re-checks the canary-recorded tarball SHA-256, and creates the GitHub Release.
- The registry stores the published tarball verbatim, so `dist.integrity` (sha512) and the bytes from `npm pack @crocoder-dev/sce@<version>` must equal the GitHub Release asset. A mismatch fails the workflow.
- For reruns, the publish step checks the registry first. When the exact version already exists with matching integrity, it skips `npm publish` and proceeds to the registry canary. When it exists with different integrity, the workflow fails.
- `release: published` stays as a trigger. Releases created with `GITHUB_TOKEN` do not fire it, so it starts publication only for externally created releases, and the existing per-tag concurrency group serializes any overlap.

## Task stack

- [x] T01: `Factor release-fixture setup out of pack-smoke.sh into a shared harness` (status:done)
  - Task ID: T01
  - Scope: In — new shared harness under `npm/test/` (TLS cert generation, `fixture-release-server.mjs` start/wait/stop, throwaway-key staging, canonical archive + signed manifest build, isolated npm install, layout/mode/version assertions, `prepare`/`serve` entrypoint); `npm/test/pack-smoke.sh` rewritten as a consumer with unchanged CLI flags and `pack-smoke:` messages; `flake.nix` file sets if the new file needs inclusion. Out — new VM scenarios, canary script, installer code, server behavior changes.
  - Dependencies: none
  - Done when: pack-smoke fixture and `--binary` modes produce the same assertions and messages as before; the existing VM test still passes; the harness's `prepare` output can be served and installed by a separate process; shellcheck clean.
  - Verify: `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs`; host `--binary` smoke with `./result/bin/sce`; `nix build .#npm-nixos-vm --print-build-logs`; `nix shell nixpkgs#shellcheck -c shellcheck npm/test/*.sh`; re-run predecessor faults (non-executable launcher, garbage `--binary`) from a scratch cwd and confirm identical messages.
  - Completed: 2026-11-04
  - Files changed: `npm/test/release-fixture.sh` (new, Git mode 100755, added intent-to-add so the flake sees it), `npm/test/pack-smoke.sh`, `flake.nix` (`npmSrc` file set gains `./npm/test/release-fixture.sh`).
  - Result: `npm/test/release-fixture.sh` is a sourced Bash library (`fixture_require_tools`, `fixture_assert_launcher_mode`, `fixture_stage_package`, `fixture_resolve_platform`, `fixture_generate_keys`, `fixture_build_release`, `fixture_pack`, `fixture_serve_start`/`fixture_serve_stop`, `fixture_npm_install`, `fixture_assert_install_layout`; messages use the caller-set `FIXTURE_LOG_PREFIX`) and a CLI entrypoint. `prepare <dir> [--package-dir|--tarball] [--binary]` writes `stage/`, `pack/<tgz>`, `release/`, `keys/{signing,tls-cert,tls-key}.pem`, and a shell-sourceable `fixture.env` (`FIXTURE_VERSION`, `FIXTURE_TARGET_TRIPLE`, `FIXTURE_TARBALL`, `FIXTURE_RELEASE_DIR`, `FIXTURE_TLS_CERT`, `FIXTURE_TLS_KEY`), and refuses to run on an already prepared dir. `serve <dir> [--port-file f]` execs `fixture-release-server.mjs` in the foreground (default port file `<dir>/server-port`). `pack-smoke.sh` sources the harness with prefix `pack-smoke`, and its flags, usage text (plus one note that the fixture uses a throwaway key), step order, and messages are unchanged. `fixture-release-server.mjs` is unchanged. Deviation from the gate: `prepare` takes no `--expected-version`, because version assertions stay with the consumer.
  - Verify outcomes:
    - `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs`: passed (`pack-smoke: OK (sce --help, sce version)`).
    - Host `--binary` smoke with a Nix-built `sce-release` (`nix build .#sce-release` into a scratch out-link, then `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary <out>/bin/sce`): passed (`real binary reported version 0.4.0-pre-alpha-v12`, `OK (real binary: ...)`).
    - `nix build .#npm-nixos-vm --print-build-logs`: passed on this host under TCG (no `/dev/kvm`). Every existing subtest finished, and the log shows the unchanged `pack-smoke:` lines and `parsed version: 0.4.0-pre-alpha-v12`.
    - `nix shell nixpkgs#shellcheck -c shellcheck npm/test/*.sh` and with `-x`: passed, no findings.
    - Predecessor faults from a scratch cwd, with output diffed against the pre-refactor script after normalizing temp paths: identical. Non-executable launcher: `pack-smoke: Packed package/bin/sce.js has mode -rw-r--r--; expected an executable mode ...`, exit 1. Garbage `--binary`: `pack-smoke: sce --help exited non-zero.`, exit 1. Clean fixture run: identical, exit 0.
    - Separate-process reuse: `release-fixture.sh prepare <dir>`, then `release-fixture.sh serve <dir> --port-file f` in a background process, then sourcing `fixture.env` and running `npm install --offline` of `$FIXTURE_TARBALL` against it from another process. Passed: `sce fixture help`, `sce 0.4.0-pre-alpha-v12`, `runtime/sce` mode `755`. Re-running `prepare` on the same dir fails with `Fixture directory ... is already prepared.`
  - Context impact: local to npm test tooling. The harness is a new reusable test surface. Durable docs for it are T06's scope (`context/sce/cli-npm-distribution-contract.md`), and no root context file changes.
  - Context synchronization: synced

- [x] T02: `Add VM subtest for skipped postinstall and npm rebuild recovery` (status:done)
  - Task ID: T02
  - Scope: In — `nix/tests/npm-nixos-vm.nix`: per-scenario directory, harness `prepare` + background `serve` as the test user, `npm install --ignore-scripts` of the packed tgz, assert absent `runtime/sce` and the launcher's non-zero exit with the missing-binary message, `npm rebuild @crocoder-dev/sce` with fixture env, assert mode `755`, parsed version, then a second rebuild and re-assert. Out — failure-injection scenarios, setup scenario, restricted PATH.
  - Dependencies: T01
  - Done when: the subtest passes in the VM and its log shows each assertion.
  - Verify: `nix build .#npm-nixos-vm --print-build-logs`; fault in a scratch copy: skip the rebuild step and record the failing assertion.
  - Completed: 2026-11-04
  - Files changed: `nix/tests/npm-nixos-vm.nix`.
  - Result: New let bindings for the scenario paths under `/home/sce-tester/scenarios/rebuild` and the missing-binary message, plus an `asScenarioUser` wrapper over `asUser` that isolates HOME, TMPDIR, npm cache, and npm userconfig per scenario. New subtest `skipped postinstall and npm rebuild recovery` runs as the test user. It runs `release-fixture.sh prepare` with `--package-dir ~/npm --binary <sceReleasePackage>/bin/sce` and starts `release-fixture.sh serve` detached through `setsid`, recording its PID and waiting for the port file. Next, `npm install --offline --ignore-scripts` of `$FIXTURE_TARBALL`. It asserts `runtime/sce` is absent and the launcher exits non-zero with `The native sce binary is not installed.`. Two passes of the `rebuild_and_assert` helper follow, each running `npm rebuild @crocoder-dev/sce --prefix … --offline --foreground-scripts` with `SCE_NPM_RELEASE_BASE_URL`/`NODE_EXTRA_CA_CERTS`, then asserting `runtime/sce` mode `755`, `--help`, and the parsed `version --format json` `.version`. The server is killed in a `finally`. Deviation: the shell snippets avoid an explicit `exit`, because in the `su -` login shell it sources `/etc/bash_logout`, which fails under `set -u` (`__ETC_BASHLOGOUT_SOURCED: unbound variable`) and turned `exit 0` into exit 1 on the first run.
  - Verify outcomes:
    - `nix build .#npm-nixos-vm --print-build-logs`: passed under TCG (no `/dev/kvm`). Every earlier subtest is unchanged. The new subtest log shows `ignore-scripts install: runtime/sce is absent` and ``launcher without binary exited 1: The native sce binary is not installed. Try reinstalling the package or run `npm rebuild sce` to retry the download.``. For each rebuild it shows postinstall `node ./lib/install.js` running and `rebuilt dependencies successfully`, then `first rebuild: runtime/sce mode: 755` and `first rebuild: parsed version: 0.4.0-pre-alpha-v12`. The same lines appear for `second rebuild`.
    - Fault (scratch `git worktree` at HEAD plus the new test file, with `npm rebuild` replaced by `true`): build failed with `Test "skipped postinstall and npm rebuild recovery" failed with error: "command \`stat -c %a /home/sce-tester/scenarios/rebuild/prefix/node_modules/@crocoder-dev/sce/runtime/sce\` failed (exit code 1)"` (`stat: cannot statx … No such file or directory`). The worktree was then removed and the main worktree has no fault residue.
    - `nix shell nixpkgs#nixfmt -c nixfmt --check nix/tests/npm-nixos-vm.nix`: passed.
  - Context impact: local to the npm VM test. The new subtest is documented as part of T06 (`context/sce/cli-npm-distribution-contract.md`), and no root context file changes.
  - Context synchronization: synced

- [x] T03: `Add VM subtest proving a failed reinstall preserves the existing binary` (status:done)
  - Task ID: T03
  - Scope: In — `nix/tests/npm-nixos-vm.nix`: successful install, record `sha256sum` and mode of `runtime/sce`; replace the manifest `.sig` with an invalid signature, `npm rebuild` must fail with the authenticity error; restore it, then corrupt the served archive so its checksum mismatches, `npm rebuild` must fail with the checksum error; after each failure assert unchanged hash, mode `755`, successful execution, and no `sce-npm-install-*`/`sce-npm-text-*` directories in the user's TMPDIR or temp siblings in `runtime/`; restore the valid fixture and assert a successful reinstall. Out — the JS unit-test permutations (malformed JSON, identity fields, archive entry types).
  - Dependencies: T02
  - Done when: the subtest passes and its log shows both failure messages and the unchanged-binary checks.
  - Verify: `nix build .#npm-nixos-vm --print-build-logs`; faults in a scratch copy: (a) overwrite `runtime/sce` between attempts, (b) leave a temp file beside `runtime/sce`; record each failing assertion.
  - Completed: 2026-11-04
  - Files changed: `nix/tests/npm-nixos-vm.nix`.
  - Result: The per-scenario paths are now built by one `mkScenario name` attrset (`dir`, `prefixDir`, `fixtureDir`, `tmpDir`, `portFile`, `pidFile`, `serverLog`, `installedDir`, `launcher`). The T02 fixture shell steps moved into Nix helpers `prepareScenarioFixture`, `startScenarioServer`, `stopScenarioServer`, and `rebuildScenarioPackage`. The rebuild subtest uses those helpers and behaves as before. New subtest `failed reinstall preserves existing binary` runs under `~/scenarios/reinstall`:
    - It prepares and serves its own fixture, saves pristine copies of `release/` under `saved-release/`, and runs `npm install --foreground-scripts` with postinstall against the fixture server. It records the `runtime/sce` SHA-256.
    - It replaces the manifest `.sig` with a well-formed signature, made with the fixture key, over different bytes. `npm rebuild` must then exit non-zero with `Release manifest authenticity check failed`.
    - It restores the `.sig` and appends bytes to the served archive. `npm rebuild` must then exit non-zero with `Downloaded sce archive checksum mismatch`.
    - It restores the archive and requires a successful `npm rebuild`.
    - `assert_binary_preserved` runs after the initial install, after each failure, and after the restored reinstall. It asserts the recorded SHA-256, mode `755`, `--help`, and the parsed `version --format json` `.version`. It also asserts that the scenario TMPDIR has no `sce-npm-install-*`/`sce-npm-text-*` entries and that `runtime/` holds only `sce`.
    - Installer code and the fixture server are unchanged.
  - Verify outcomes:
    - `nix build .#npm-nixos-vm --print-build-logs`: passed under TCG (no `/dev/kvm`). Every earlier subtest is unchanged, and the rebuild subtest still logs `first rebuild`/`second rebuild` mode `755` and version `0.4.0-pre-alpha-v12`. The new subtest logs the following:
      - `Failed to install sce via npm: Release manifest authenticity check failed: signature verification did not succeed.` and `invalid manifest signature: npm rebuild exited 1`.
      - `Failed to install sce via npm: Downloaded sce archive checksum mismatch for sce-v0.4.0-pre-alpha-v12-x86_64-unknown-linux-musl.tar.gz: expected 0d1aecdc…, received f262e7f9…` and `archive checksum mismatch: npm rebuild exited 1`.
      - For `initial install`, `after invalid manifest signature`, `after archive checksum mismatch`, and `restored fixture reinstall`: sha256 `ba3565750ad5030412d8ca238ffb555367ecd6a9ae91f004399c444a6cdff484`, mode `755`, version `0.4.0-pre-alpha-v12`, `installer temp dirs in TMPDIR: none`, and `runtime/ entries: ['sce']`.
    - Fault (a), in a scratch `git worktree` at HEAD plus the new test file, appended bytes to `runtime/sce` after the signature failure. The build failed with `Test "failed reinstall preserves existing binary" failed with error: "after invalid manifest signature: runtime/sce sha256 d86b3109…, expected ba356575…"`.
    - Fault (b), in a separate scratch worktree, touched `runtime/.sce.4242.1.tmp` after the checksum failure. The build failed with `Test "failed reinstall preserves existing binary" failed with error: "after archive checksum mismatch: runtime/ holds ['sce', '.sce.4242.1.tmp'], expected only sce"`.
    - Rerun with KVM after `/dev/kvm` became available: `nix build .#npm-nixos-vm --rebuild --print-build-logs` passed, and the guest logged `Hypervisor detected: KVM`. Every subtest passed, and the test script finished in 40.81s, against 49.79s under TCG.
    - Both scratch worktrees were removed (`git worktree list` shows none), and the main worktree has no fault residue.
    - `nix shell nixpkgs#nixfmt -c nixfmt --check nix/tests/npm-nixos-vm.nix`: passed.
  - Context impact: local to the npm VM test. The new subtest and the shared scenario helpers are documented as part of T06 (`context/sce/cli-npm-distribution-contract.md`), and no root context file changes.
  - Context synchronization: synced

- [x] T04: `Add VM subtest running sce setup --bootstrap-context in an isolated Git repository` (status:done)
  - Task ID: T04
  - Scope: In — `nix/tests/npm-nixos-vm.nix`: add `git` to the node; as the test user with a dedicated Git-enabled PATH (node, git, coreutils) and isolated HOME/XDG dirs, `git init` a temp repo, run the installed `sce setup --bootstrap-context` from that repo and once with `--repo <path>` from an unrelated cwd, assert the baseline directories and files, append a sentinel to `context/overview.md`, rerun, and assert success, an identical file list, and the preserved sentinel. Out — target installs (`--claude` etc.), hooks, auth.
  - Dependencies: T03
  - Done when: the subtest passes, and no SCE source path appears in the cwd or the repo.
  - Verify: `nix build .#npm-nixos-vm --print-build-logs`; fault in a scratch copy: overwrite the sentinel before the second run and record the failing assertion.
  - Completed: 2026-11-04
  - Files changed: `nix/tests/npm-nixos-vm.nix`.
  - Result: New `setupScenario = mkScenario "setup"` plus let bindings for the repo dir, a nested cwd (`repo/nested/cwd`), the sentinel, the expected baseline dirs/files, and `setupToolsPath = lib.makeBinPath [ nodejs git coreutils ]`. New helpers:
    - `installScenarioPackage`: npm install with postinstall against the scenario fixture server. The reinstall subtest now uses it in place of its inline copy, with the same behavior.
    - `asSetupUser`: wraps `asScenarioUser` and exports `PATH` to only `setupToolsPath`, isolated `XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME` under the scenario HOME, and `GIT_CONFIG_NOSYSTEM=1`.
    - The new subtest `real sce setup --bootstrap-context` prepares and serves its own fixture, installs the package, and stops the server. Under the setup PATH it asserts that `tar`/`curl`/`gzip`/`bash`/`jq` do not resolve. It then runs `git init` on `~/scenarios/setup/repo` and adds a placeholder `origin` remote, because setup requires the configured repository remote.
    - First run: `sce setup --bootstrap-context` from the repo root. It asserts the four dirs and six files, an exact sorted `context/` tree, and the `tmp/.gitignore` contents.
    - It appends the sentinel to `context/overview.md`. Second run: from the nested cwd, it asserts the same tree, a byte-identical `overview.md`, and the preserved sentinel.
    - Finally it asserts that the nested cwd resolves to the repo, the repo holds only `.git`, `context`, and `nested`, no ancestor of the repo is a Git checkout or holds `cli/Cargo.toml`, and no bootstrapped file mentions `~/npm` or `/nix/store`.
  - Deviation: the planned `--repo <path>` run was replaced by a run from a nested subdirectory of the repo. The CLI rejects `--bootstrap-context` combined with `--repo` (`Option '--bootstrap-context' must be used alone`, `cli/src/services/setup/mod.rs`), and the CLI is out of scope. Per the user's mid-task instruction, the new code carries no explanatory comments. `git` was not added to `environment.systemPackages`: the scenario reaches it only through its dedicated PATH.
  - Verify outcomes:
    - `nix build .#npm-nixos-vm --print-build-logs`: passed. Every earlier subtest still passes. The new subtest logs `first run cwd: …/scenarios/setup/repo`, `Context baseline ensured.`, `first run: context tree: ['context/architecture.md', 'context/context-map.md', 'context/decisions', 'context/glossary.md', 'context/handovers', 'context/overview.md', 'context/patterns.md', 'context/plans', 'context/tmp', 'context/tmp/.gitignore']`, then `second run cwd: …/repo/nested/cwd`, `Context baseline ensured.`, `second run: context tree unchanged`, `second run: sentinel preserved: 'sce-tester sentinel: keep this line'`, `repo entries: ['context', '.git', 'nested']`, and `no SCE source path in the cwd ancestry or the bootstrapped context`.
    - Fault (a scratch `git worktree` at HEAD plus the new test file, overwriting `context/overview.md` with `# Overview\n` before the second run): the build failed with `Test "real sce setup --bootstrap-context" failed with error: "sentinel lost from context/overview.md: '# Overview\n'"`. The worktree was removed, and the main worktree has no fault residue.
    - `nix shell nixpkgs#nixfmt -c nixfmt --check nix/tests/npm-nixos-vm.nix`: passed.
  - Context impact: local to the npm VM test. The new subtest is documented in T06 (`context/sce/cli-npm-distribution-contract.md`), and no root context file changes.
  - Context synchronization: synced

- [ ] T05: `Add VM subtest for minimal runtime dependencies` (status:todo)
  - Task ID: T05
  - Scope: In — `nix/tests/npm-nixos-vm.nix`: a restricted bin dir containing only `node`; run `.bin/sce --help` and `version --format json` under `env -i PATH=<that dir> HOME=…`; run `runtime/sce --help` and `version --format json` under `env -i PATH=/var/empty`; parse JSON in the Python test script; assert that `tar`, `curl`, `gzip`, and `bash` do not resolve on the restricted PATH, that `runtime/sce` has no ELF `INTERP` header (via `binutils` `readelf -l`, used only by the test driver), and that `/lib64/ld-linux-x86-64.so.2` does not exist. Out — the Git scenario's environment (T04).
  - Dependencies: T04
  - Done when: the subtest passes without any fallback to the system PATH.
  - Verify: `nix build .#npm-nixos-vm --print-build-logs`; faults in a scratch copy: (a) add `tar` to the restricted dir, (b) feed a dynamically linked binary with an interpreter as `--binary`; record each failing assertion.
  - Context synchronization: pending

- [ ] T06: `Document the VM scenarios and the fixture-versus-exact-artifact distinction` (status:todo)
  - Task ID: T06
  - Scope: In — `context/sce/cli-npm-distribution-contract.md` (shared harness, each VM subtest and what it proves, explicit statement that pack-smoke and the VM repack with a throwaway key and do not prove the exact release artifact or production key, verification baseline); `context/context-map.md` annotation if needed. Out — canary and workflow documentation (T10).
  - Dependencies: T05
  - Done when: the docs cover the VM half of AC12, with names matching the test file.
  - Verify: inspect the contract; `grep -n 'npm-nixos-vm\|subtest' context/sce/cli-npm-distribution-contract.md`.
  - Context synchronization: pending

- [ ] T07: `Add the release-security canary script for exact release artifacts` (status:todo)
  - Task ID: T07
  - Scope: In — new `npm/test/release-canary.sh` reusing the T01 harness for install/assertions. Modes: `--release-dir <dir> --tarball <tgz>` serves the real manifest, signature, and archives over loopback HTTPS (B1). `--github-release <tag>` downloads the tgz with `gh release download` or the public URL and installs it with no installer env overrides (B2). `--registry-version <version>` downloads the exact `@crocoder-dev/sce@<version>`, compares bytes and `dist.integrity` against a given GitHub tgz, and installs it, with bounded retries for registry availability. Common checks in every mode: SHA-256 unchanged before and after, bundled key byte-equal to the checked-in key, `openssl dgst -verify` of the production manifest, launcher/`.bin` symlink/`runtime/sce` modes, and reported version equal to `--expected-version`. Out — workflow wiring, any key substitution or repacking.
  - Dependencies: T06
  - Done when: all three modes pass locally against the published `v0.4.0-pre-alpha-v12` artifacts; shellcheck clean.
  - Verify: download the v12 assets into a scratch dir and run `--release-dir`; run `--github-release v0.4.0-pre-alpha-v12` and `--registry-version 0.4.0-pre-alpha-v12` from a scratch cwd; faults: a byte-modified tgz, a tgz with a swapped key (scratch copy), and a wrong `--expected-version`, each failing with a distinct `release-canary:` message; `nix shell nixpkgs#shellcheck -c shellcheck npm/test/release-canary.sh`.
  - Context synchronization: pending

- [ ] T08: `Gate GitHub Release creation on the B1 canary in release-sce.yml` (status:todo)
  - Task ID: T08
  - Scope: In — `.github/workflows/release-sce.yml`: split `release` into `assemble` (signing secret only in the manifest step; uploads `dist/` with a SHA-pinned upload action), `release-canary` (no secrets, `contents: read`; downloads `dist/`, runs `release-canary.sh --release-dir` via Nix, outputs the tarball SHA-256), and `release` (needs both; re-checks the SHA-256 and creates the GitHub Release from the downloaded `dist/`). Out — the npm publish call (T09) and native build workflows.
  - Dependencies: T07
  - Done when: no path creates a release without a successful canary, and the secret appears in exactly one step.
  - Verify: `nix build .#checks.x86_64-linux.workflow-actionlint`; `grep -n 'SCE_RELEASE_MANIFEST_SIGNING_KEY\|secrets\.' .github/workflows/release-sce.yml`; inspect the job `needs`/`if` chain.
  - Context synchronization: pending

- [ ] T09: `Make publish-npm.yml a reusable workflow with pre- and post-publish canaries` (status:todo)
  - Task ID: T09
  - Scope: In — `.github/workflows/publish-npm.yml`: add a `workflow_call` trigger with `release_tag`/`dry_run`/`prerelease` inputs and an explicit `NPM_TOKEN` secret, keeping `workflow_dispatch` and `release: published`. Jobs: `resolve` (parity and tag checks), `github-release-canary` (no secrets; downloads the asset and runs `release-canary.sh --github-release`), `publish` (needs the canary; `NPM_TOKEN` only in the publish step's env; skips when the exact version already exists with matching integrity and fails on differing integrity; supports dry-run), and `registry-canary` (no secrets; `--registry-version` with bounded retries; skipped on dry-run). `.github/workflows/release-sce.yml`: a `publish-npm` job that `needs: release` and `uses: ./.github/workflows/publish-npm.yml`, passing the tag, the prerelease flag, `dry_run: false`, and only `NPM_TOKEN`. Out — crates.io publishing, npm provenance changes.
  - Dependencies: T08
  - Done when: the trigger and dependency chain meets AC9, and actionlint passes.
  - Verify: `nix build .#checks.x86_64-linux.workflow-actionlint`; `grep -n 'NPM_TOKEN\|secrets' .github/workflows/publish-npm.yml .github/workflows/release-sce.yml`; inspect for no `secrets: inherit` and no `latest` resolution in canary commands.
  - Context synchronization: pending

- [ ] T10: `Document release-security canary gates and npm publication topology` (status:todo)
  - Task ID: T10
  - Scope: In — `context/sce/cli-npm-distribution-contract.md` (release-security canary, the B1/B2/registry gates, updated Registry publication topology covering the `workflow_call` trigger, secret scoping, reruns, and idempotent republish), `context/sce/cli-release-artifact-contract.md` (assemble/canary/release job split), `context/sce/cli-first-install-channels-contract.md` (release-to-npm topology), and `context/context-map.md` annotations if needed. Each doc states what can only be validated by a real release. Out — VM docs (T06).
  - Dependencies: T09
  - Done when: the docs cover the release half of AC12, with job and script names matching the workflows.
  - Verify: inspect the three contracts; `grep -n 'release-canary' context/sce/*.md .github/workflows/*.yml`.
  - Context synchronization: pending

## Open questions

- The launcher's missing-binary message tells users to run `npm rebuild sce`, but the package is `@crocoder-dev/sce`, and `npm rebuild sce` does not rebuild it. The plan asserts the current text and leaves `npm/bin/sce.js` alone. Correcting the message is a one-line launcher change, which would also update the VM assertion. Should it be included?
- B2 inside GitHub Actions, the registry canary, the `workflow_call` handoff, and the idempotent-rerun path can only be proven by a real release run. Before that, the evidence will be actionlint, workflow inspection, and local canary runs against the already published `v0.4.0-pre-alpha-v12` artifacts. That release predates the hardened installer, so a pass exercises the canary script, not the new installer code.
