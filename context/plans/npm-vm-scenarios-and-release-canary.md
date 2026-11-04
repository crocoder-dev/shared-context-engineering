# Plan: npm-vm-scenarios-and-release-canary

## Change summary

Extends PR #345 (branch `npm-nixos-integration-test`, predecessor plan `npm-nixos-vm-integration-test`, all tasks done) in two separable parts.

First, the local-only NixOS VM test `nix/tests/npm-nixos-vm.nix` gains four scenario subtests on the same VM, each with its own isolated HOME/prefix/fixture directory: skipped-postinstall recovery through `npm rebuild`, a failed reinstall (bad manifest signature, then archive checksum mismatch) that must leave the previously installed binary byte-identical and executable with no abandoned staging files, real `sce setup --bootstrap-context` operation in a throwaway Git repository, and minimal-runtime execution under a restricted PATH with no tar/curl/gzip/Bash and no FHS loader. To support this without a second HTTPS fixture, the release-fixture setup now inlined in `npm/test/pack-smoke.sh` (throwaway signing key swap, canonical archive, signed manifest, loopback TLS, `fixture-release-server.mjs` start/stop, isolated install and layout assertions) is factored into one reusable harness that pack-smoke, the VM, and the new canary all consume. The VM fixture keeps substituting a throwaway public key and is documented as hermetic development coverage, not proof of the exact release artifact.

Second, a release-security canary installs the exact, unmodified `sce-v<version>-npm.tgz` with its bundled production public key against the production-signed manifest and real native archive, asserts the bundled key equals the checked-in repository key, and checks launcher/symlink/binary modes and the reported version. It runs at three gates: in `release-sce.yml` against the assembled artifacts served over loopback HTTPS before the GitHub Release is created (B1, in a job that receives no secrets); in `publish-npm.yml` against the published GitHub Release over the normal download URL with real TLS and no installer overrides before `npm publish` (B2); and after publication against the exact registry version, comparing registry bytes to the GitHub asset (registry canary). `publish-npm.yml` becomes a reusable `workflow_call` invoked by `release-sce.yml` after release creation, keeping `workflow_dispatch` and `release: published` for manual reruns and externally created releases, with `NPM_TOKEN` scoped to the publish step only. The npm installer (`npm/lib`, `npm/bin`) is not changed.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: One release-fixture harness serves every hermetic and canary install: pack-smoke fixture and `--binary` modes behave as before, `fixture-release-server.mjs` is the only HTTPS server implementation, and no other file generates its own TLS server or duplicates the sign/serve/install steps.
  - Validate: `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs` passes; `nix build .#sce-release && nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary ./result/bin/sce` passes; `nix shell nixpkgs#ripgrep -c rg -n 'createServer|openssl req' npm nix .github` hits only the shared harness and `fixture-release-server.mjs`.
- [x] AC2: In the VM, a package installed with `--ignore-scripts` has no `runtime/sce`, its launcher exits non-zero with the missing-binary message, and `npm rebuild @crocoder-dev/sce` with the fixture server available runs postinstall to install a mode-`755` `runtime/sce` that reports `.version`; a second rebuild leaves a working, executable install.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` passes and the log shows the `skipped postinstall and npm rebuild recovery` subtest with the asserted message, mode, and version for both rebuilds.
- [x] AC3: In the VM, after a successful install, a reinstall attempt against an invalid manifest signature and then against an archive checksum mismatch each fails, leaves `runtime/sce` with the recorded SHA-256 and mode `755` and still executable, and leaves no `sce-npm-install-*`/`sce-npm-text-*` temp directories or temp siblings of `runtime/sce`; restoring the valid fixture makes the next reinstall succeed.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` log shows the `failed reinstall preserves existing binary` subtest with both failure messages, unchanged hashes, empty staging checks, and the final successful reinstall.
- [x] AC4: In the VM, as the non-root user, from a working directory outside any SCE checkout, `sce setup --bootstrap-context` in a fresh Git repository creates the `context/` baseline (directories `plans`, `handovers`, `decisions`, `tmp`; files `overview.md`, `architecture.md`, `patterns.md`, `glossary.md`, `context-map.md`, `tmp/.gitignore`); a second run succeeds, creates nothing new, and preserves a sentinel appended to an existing context file.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` log shows the `real sce setup --bootstrap-context` subtest with the created paths and the preserved sentinel.
- [x] AC5: In the VM, the installed launcher (`--help`, `version --format json`) runs under `env -i` with a PATH containing only a directory that links `node`, and `runtime/sce` runs directly under `env -i` with an empty PATH; neither PATH contains tar, curl, gzip, or bash, the binary has no ELF interpreter, and no FHS loader exists at `/lib64/ld-linux-x86-64.so.2`. The Git-dependent scenario uses its own separate PATH, and no subtest falls back to the system PATH.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` log shows the `minimal runtime dependencies` subtest assertions; inspect `nix/tests/npm-nixos-vm.nix` for the restricted environments and the absence of any fallback to the full PATH.
- [x] AC6: The VM test stays local-only, non-root, and loopback-only, and the installer is unchanged: `npm-nixos-vm` is in neither `checks.<system>` nor `ci-checks`, every scenario runs as the non-root user with a per-scenario isolated directory, and `npm/lib` and `npm/bin` are unchanged by this plan.
  - Validate: `nix eval .#checks.x86_64-linux --apply builtins.attrNames` does not list `npm-nixos-vm`; `nix build .#ci-checks --print-build-logs && ls result/` shows only `sce-release` and `release-portability-audit`; `git diff a4611a4b -- npm/lib npm/bin` is empty; inspect the VM node for `useDHCP = false`, `interfaces = { }`, `vlans = [ ]`.
- [x] AC7: The release-security canary installs the exact, unmodified tarball and the production key: its SHA-256 is identical before and after the run, the packed `package/lib/release-manifest-public-key.pem` is byte-identical to the checked-in `npm/lib/release-manifest-public-key.pem`, the production-signed manifest verifies against that key, and the installed launcher, `.bin/sce` symlink target, and `runtime/sce` modes and the reported version are asserted. Local-directory, GitHub-release, and registry modes each pass against the already published `v0.4.0-pre-alpha-v12` artifacts, and the GitHub/registry modes set none of `SCE_NPM_RELEASE_BASE_URL`, `SCE_NPM_SKIP_DOWNLOAD`, or `NODE_EXTRA_CA_CERTS`.
  - Validate: run the three canary modes against `v0.4.0-pre-alpha-v12` (commands recorded in T07) and capture exit 0 with the printed hashes, key comparison, modes, and version; `nix shell nixpkgs#ripgrep -c rg -n 'SCE_NPM_RELEASE_BASE_URL|SCE_NPM_SKIP_DOWNLOAD|NODE_EXTRA_CA_CERTS' npm/test/release-canary.sh` shows them only in the local-directory mode path.
- [x] AC8: `release-sce.yml` cannot create a GitHub Release unless the B1 canary passed on the exact assembled artifacts, and the signing private key is referenced only in the manifest-signing step of a job distinct from the canary job.
  - Validate: inspect `.github/workflows/release-sce.yml`: the release-creation job `needs` the canary job and gates on its success; `SCE_RELEASE_MANIFEST_SIGNING_KEY` appears only in one step's `env`; the canary job has no `secrets.` reference; `nix build .#checks.x86_64-linux.workflow-actionlint` passes.
- [x] AC9: npm publication runs from `release-sce.yml` through `workflow_call` only after release creation succeeds, the GitHub-release canary must pass before `npm publish`, the registry canary runs after publication against the exact version with bounded retries and fails the workflow on mismatch or unusable install, `NPM_TOKEN` is passed explicitly (not `secrets: inherit`) and exposed only to the publish step, no canary job references a secret, and `workflow_dispatch` with a release tag and `release: published` still start the same pipeline.
  - Validate: inspect `.github/workflows/publish-npm.yml` and `.github/workflows/release-sce.yml` for those job dependencies, triggers, secret references, and the retry bound; `nix build .#checks.x86_64-linux.workflow-actionlint` passes. Live execution of B2 and the registry canary inside GitHub Actions is not provable before a real release and must be reported as such.
- [x] AC10: Each new check fails when its critical assertion is deliberately violated: recovery subtest (rebuild skipped), failed-reinstall subtest (binary replaced or temp file left behind), setup subtest (sentinel overwritten), minimal-runtime subtest (tar added to the restricted PATH, or a binary with an ELF interpreter), and the canary (tarball byte change, swapped public key, wrong expected version).
  - Validate: plan task evidence records each injected fault, made only in scratch copies, with the exact failing subtest or `release-canary:` message; `git status` is clean of fault residue.
- [x] AC11: Existing JavaScript installer tests for malformed manifests, invalid archive entries, and fault permutations are retained and pass, and the VM does not re-implement those permutations.
  - Validate: `nix build .#checks.x86_64-linux.npm-bun-tests --print-build-logs` passes; `git diff a4611a4b --stat -- npm/test/*.test.js` shows no removals.
- [x] AC12: Durable docs state that the hermetic VM/pack-smoke fixtures substitute a throwaway key and repack, while only the release-security canary proves the exact artifact and production key; they describe each VM scenario, the three canary gates, the `workflow_call` trigger topology and secret scoping, manual reruns, and what still needs a real release to validate.
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

- [x] T05: `Add VM subtest for minimal runtime dependencies` (status:done)
  - Task ID: T05
  - Scope: In — `nix/tests/npm-nixos-vm.nix`: a restricted bin dir containing only `node`; run `.bin/sce --help` and `version --format json` under `env -i PATH=<that dir> HOME=…`; run `runtime/sce --help` and `version --format json` under `env -i PATH=/var/empty`; parse JSON in the Python test script; assert that `tar`, `curl`, `gzip`, and `bash` do not resolve on the restricted PATH, that `runtime/sce` has no ELF `INTERP` header (via `binutils` `readelf -l`, used only by the test driver), and that `/lib64/ld-linux-x86-64.so.2` does not exist. Out — the Git scenario's environment (T04).
  - Dependencies: T04
  - Done when: the subtest passes without any fallback to the system PATH.
  - Verify: `nix build .#npm-nixos-vm --print-build-logs`; faults in a scratch copy: (a) add `tar` to the restricted dir, (b) feed a dynamically linked binary with an interpreter as `--binary`; record each failing assertion.
  - Completed: 2026-11-04
  - Files changed: `nix/tests/npm-nixos-vm.nix`.
  - Result: New `minimalScenario = mkScenario "minimal"` plus let bindings: `minimalNodeBinDir`, `minimalHomeDir`, `absentRuntimeTools` (`tar curl gzip bash`), `fhsLoader`, `readelf` (`binutils-unwrapped`, used only by the test driver), `minimalRuntimeBinary`, and the `env -i` prefixes `minimalLauncherEnv` (`PATH=<node-bin>`) and `minimalBinaryEnv` (`PATH=/var/empty`). The new `assertToolsAbsent label path` helper fails if any of those tools resolves on the given PATH.
    - The VM node now sets `environment.stub-ld.enable = false`. The locked nixpkgs installs a stub loader at `/lib64/ld-linux-x86-64.so.2` by default, so without this the absence assertion can never hold. Earlier subtests are unaffected.
    - New subtest `minimal runtime dependencies`:
      - It prepares, serves, installs, and stops through the shared scenario helpers.
      - It builds `node-bin/` with only a `node` symlink and asserts the listing is exactly `['node']`.
      - It asserts that the four tools resolve on neither the launcher PATH nor `/var/empty`.
      - Static checks: `readelf -h` confirms an ELF, `readelf -lW` shows no `INTERP`/`program interpreter`, and `/lib64/ld-linux-x86-64.so.2` is absent.
      - It runs `.bin/sce --help` and `version --format json` under `env -i PATH=<node-bin> HOME=…`, then `runtime/sce --help` and `version --format json` under `env -i PATH=/var/empty HOME=…`, with absolute `env` and binary paths. Each JSON is parsed with `json.loads`, and `.version` is asserted. Nothing falls back to the system PATH.
  - Verify outcomes:
    - `nix build .#npm-nixos-vm --print-build-logs`: passed with KVM (`Hypervisor detected: KVM`). Every earlier subtest passed. The new subtest logs:
      - `launcher PATH entries: ['node']`
      - `launcher PATH: tar curl gzip bash do not resolve on PATH=…/minimal/node-bin` and `binary PATH: tar curl gzip bash do not resolve on PATH=/var/empty`
      - `runtime/sce: no ELF INTERP program header`, `no FHS loader at /lib64/ld-linux-x86-64.so.2`
      - `launcher: --help under env -i with only node on PATH`, `launcher: parsed version: 0.4.0-pre-alpha-v12`
      - `runtime/sce: --help under env -i with PATH=/var/empty`, `runtime/sce: parsed version: 0.4.0-pre-alpha-v12`
      - The subtest finished in 8.16s.
    - Fault (a), a scratch worktree that symlinks `tar` into `node-bin/`, failed with `Test "minimal runtime dependencies" failed with error: "launcher PATH holds ['node', 'tar'], expected only node"`.
    - Fault (a2), the same worktree change with the listing assertion removed to exercise the tool check, failed with `launcher PATH: tar resolves on PATH=/home/sce-tester/scenarios/minimal/node-bin` and a `Test "minimal runtime dependencies" failed` error.
    - Fault (b), a scratch worktree where the minimal scenario's fixture uses `--binary ${pkgs.hello}/bin/hello`, failed with `Test "minimal runtime dependencies" failed with error: "runtime/sce requests an ELF interpreter: … [Requesting program interpreter: /nix/store/…-glibc-2.42-67/lib/ld-linux-x86-64.so.2]"`.
    - All scratch worktrees were removed (`git worktree list` shows none for this repo), and the main worktree has no fault residue.
    - `nix shell nixpkgs#nixfmt -c nixfmt --check nix/tests/npm-nixos-vm.nix`: passed.
    - Observation outside T05's scope, unchanged: in the passing run, the T03 `archive checksum mismatch` `npm rebuild` printed `Downloaded sce archive checksum mismatch … received 0`. It then hung until the NixOS test driver's 900s `execute` timeout killed it (`npm rebuild exited 124`; guest clock 40s → 940s). The T03 assertion accepts any non-zero exit plus the message, so the subtest passed. The three fault runs completed that step normally (subtest 25–27s), and T03's own runs exited 1. The hang is intermittent. It points at a possible installer or fixture-server hang after a checksum mismatch, and at T03 accepting exit 124. Both are outside T05's scope and are left for a follow-up decision.
  - Context impact: local to the npm VM test. The new subtest is documented in T06 (`context/sce/cli-npm-distribution-contract.md`), and no root context file changes.
  - Context synchronization: synced

- [x] T06: `Document the VM scenarios and the fixture-versus-exact-artifact distinction` (status:done)
  - Task ID: T06
  - Scope: In — `context/sce/cli-npm-distribution-contract.md` (shared harness, each VM subtest and what it proves, explicit statement that pack-smoke and the VM repack with a throwaway key and do not prove the exact release artifact or production key, verification baseline); `context/context-map.md` annotation if needed. Out — canary and workflow documentation (T10).
  - Dependencies: T05
  - Done when: the docs cover the VM half of AC12, with names matching the test file.
  - Verify: inspect the contract; `grep -n 'npm-nixos-vm\|subtest' context/sce/cli-npm-distribution-contract.md`.
  - Completed: 2026-11-04
  - Files changed: `context/sce/cli-npm-distribution-contract.md`, `context/context-map.md`.
  - Result: The VM documentation was one oversized bullet under `Packed-package smoke test`. It now lives in a new `## NixOS VM real-install test` section, and a new `## Hermetic fixtures versus the exact release artifact` section follows it.
    - The VM section covers the following:
      - Local-only status: not in `checks.<system>` or `.#ci-checks`.
      - The loopback-only node (`useDHCP = false`, no interfaces, no VLANs), the non-root `sce-tester`, and `environment.stub-ld.enable = false`.
      - Per-scenario isolation under `/home/sce-tester/scenarios/<name>`, and the shared helpers (`mkScenario`, `asScenarioUser`, `prepareScenarioFixture`, `startScenarioServer`, `stopScenarioServer`, `installScenarioPackage`, `rebuildScenarioPackage`, `asSetupUser`).
      - Every subtest in test-script order, under its exact name, with what it proves.
      - The fixture-mode-versus-VM comparison, the linux/x64-only coverage, and how to run it locally.
    - The new section has three points:
      - Every `release-fixture.sh` consumer swaps in a throwaway key, re-signs, repacks, and serves over loopback HTTPS with an ephemeral certificate. That covers `npm-pack-smoke`, host pack-smoke, the `release-npm-package` gate, and all VM subtests.
      - These consumers are hermetic development coverage only. They do not prove the exact published tarball bytes, the production public key, the production-signed manifest, or real GitHub Releases TLS and URLs.
      - Only the planned release-security canary proves those.
    - The verification baseline already listed the harness and the VM commands and is unchanged. The `context/context-map.md` annotation for the contract now names the four scenario subtests and the fixture-versus-exact-artifact statement. No content was dropped. The VM paragraph was restructured, not rewritten.
  - Verify outcomes:
    - Contract inspected: shared harness (existing bullet), every VM subtest, the throwaway-key/repack statement, and the verification baseline are present.
    - `grep -n 'npm-nixos-vm\|subtest' context/sce/cli-npm-distribution-contract.md`: hits at lines 71, 73, 86, 90, and 101.
    - Name parity: every `subtest("…")` name in `nix/tests/npm-nixos-vm.nix` (9 of 9) appears backticked in the contract. The `useDHCP`/`interfaces`/`vlans`/`stub-ld`/`sce-tester`/`scenarios/<name>` facts were checked against the test file, and the `--tarball` repack against `npm/test/release-fixture.sh`.
  - Context impact: durable context docs only (`context/sce/cli-npm-distribution-contract.md` and its `context/context-map.md` entry). No code, and no root context file content changes.
  - Context synchronization: synced

- [x] T07: `Add the release-security canary script for exact release artifacts` (status:done)
  - Task ID: T07
  - Scope: In — new `npm/test/release-canary.sh` reusing the T01 harness for install/assertions. Modes: `--release-dir <dir> --tarball <tgz>` serves the real manifest, signature, and archives over loopback HTTPS (B1). `--github-release <tag>` downloads the tgz with `gh release download` or the public URL and installs it with no installer env overrides (B2). `--registry-version <version>` downloads the exact `@crocoder-dev/sce@<version>`, compares bytes and `dist.integrity` against a given GitHub tgz, and installs it, with bounded retries for registry availability. Common checks in every mode: SHA-256 unchanged before and after, bundled key byte-equal to the checked-in key, `openssl dgst -verify` of the production manifest, launcher/`.bin` symlink/`runtime/sce` modes, and reported version equal to `--expected-version`. Out — workflow wiring, any key substitution or repacking.
  - Dependencies: T06
  - Done when: all three modes pass locally against the published `v0.4.0-pre-alpha-v12` artifacts; shellcheck clean.
  - Verify: download the v12 assets into a scratch dir and run `--release-dir`; run `--github-release v0.4.0-pre-alpha-v12` and `--registry-version 0.4.0-pre-alpha-v12` from a scratch cwd; faults: a byte-modified tgz, a tgz with a swapped key (scratch copy), and a wrong `--expected-version`, each failing with a distinct `release-canary:` message; `nix shell nixpkgs#shellcheck -c shellcheck npm/test/release-canary.sh`.
  - Completed: 2026-11-04
  - Files changed: `npm/test/release-canary.sh` (new, file mode 755, untracked), `npm/test/release-fixture.sh`.
  - Result: `npm/test/release-canary.sh` sources the harness with prefix `release-canary`. It requires `--expected-version` and exactly one mode:
    - `--release-dir <dir> --tarball <tgz>` (B1): serves `<dir>` in place through `fixture_serve_start` and installs with `fixture_npm_install`. This is the only path that sets `SCE_NPM_RELEASE_BASE_URL`/`NODE_EXTRA_CA_CERTS`, and it sets them inside the harness.
    - `--github-release <tag>` (B2): the tag must equal `v<expected-version>`. It downloads `sce-v<version>-npm.tgz`, the manifest, and the `.sig` from the public `releases/download` URL through Node `fetch`, then runs `npm install` of the downloaded tgz.
    - `--registry-version <version>` (optional `--tarball`, default the GitHub asset): it retries `npm view @crocoder-dev/sce@<version> dist.integrity` plus `npm pack` of that exact version, bounded at 10 attempts × 30s. It fails unless `dist.integrity` and the packed registry bytes both equal the sha512 of the GitHub tgz. It then runs `npm install @crocoder-dev/sce@<version>` and asserts that the lockfile integrity equals `dist.integrity`.
    - Every network, install, and `sce` invocation in the remote modes runs under `env -i` with only `PATH`, an isolated `HOME`/`TMPDIR`, and the npm cache/userconfig. No installer override can therefore leak in, and none is named in the script.
    - Versions and tags must match a strict semver pattern, so a dist-tag or range is never resolved.
    - Checks in every mode:
      - The tarball SHA-256 is recorded before the run and must be unchanged after it. It must also equal the optional `--expected-sha256`.
      - The packed `package.json` name is `@crocoder-dev/sce` and its version equals the expected version.
      - The packed `package/lib/release-manifest-public-key.pem` is byte-equal (SHA-256) to the checked-in key.
      - `openssl dgst -sha256 -verify` of the manifest `.sig` against the checked-in key passes, and the manifest `.version` equals the expected version.
      - `fixture_assert_install_layout` passes (`runtime/sce` mode `755`, `.bin/sce` → `bin/sce.js`), and the installed `bin/sce.js` is executable.
      - `--help` runs, and `version --format json` `.version` equals the expected version.
    - The run ends with `release-canary: OK (<mode> mode: …, tarball sha256 …)`.
    - In the harness, `fixture_generate_tls_cert` is split out of `fixture_generate_keys`, which now calls it, so `openssl req` stays only in the harness. pack-smoke behavior is unchanged.
  - Deviations:
    - The packed launcher mode is logged, not enforced. The published v12 tarball packs `bin/sce.js` as `-rw-r--r--`, so enforcing it would fail every mode against v12. The installed launcher mode, which npm sets, is asserted instead.
    - Downloads use the public URL, not `gh`, so no token is needed.
    - `--expected-sha256` was added so that a byte-modified tarball fails with a distinct message in B1, where no published hash exists.
    - Per the user's instruction, the new code carries no comments at all, including the `# shellcheck source=` directive that `pack-smoke.sh` uses. Shellcheck therefore needs `-x -P SCRIPTDIR` to follow the sourced harness.
  - Verify outcomes:
    - v12 assets downloaded into a scratch dir (npm tgz/json, manifest, `.sig`, x86_64 musl archive, `.sha256`, `SHA256SUMS`). `--release-dir <scratch> --tarball <scratch>/sce-v0.4.0-pre-alpha-v12-npm.tgz --expected-version 0.4.0-pre-alpha-v12` from a scratch cwd: exit 0. It logged:
      - tarball sha256 `ec6bf434…edf57` before and after
      - bundled key byte-identical (sha256 `bfc3c81e…f212`)
      - manifest verified (sha256 `071b460c…e03a`)
      - packed launcher `-rw-r--r--`; installed `runtime/sce` mode `755`, `bin/sce.js` mode `755`, `.bin/sce` → `…/bin/sce.js`
      - version `0.4.0-pre-alpha-v12`, and `release-canary: OK (local mode: …)`
    - `--github-release v0.4.0-pre-alpha-v12` and `--registry-version 0.4.0-pre-alpha-v12` from a scratch cwd, with `SCE_NPM_SKIP_DOWNLOAD=1`, `SCE_NPM_RELEASE_BASE_URL=https://127.0.0.1:9`, and `NODE_EXTRA_CA_CERTS=/nonexistent` exported by the caller: both exit 0 with the same hash, key, manifest, mode, and version lines. Registry mode also logged `registry tarball byte-identical to the GitHub tarball (sha512-Dt3odIlX…UKA==)` and the matching installed integrity. The poisoned caller env had no effect.
    - Faults, all on scratch copies (removed afterwards; `git status` shows only the two task files):
      - Byte-appended tgz in B1 with `--expected-sha256 ec6bf434…`: `release-canary: Tarball sha256 855605eb… does not match --expected-sha256 ec6bf434….`
      - The same tgz in registry mode: `release-canary: Tarball … has no readable package/package.json.`
      - A validly repacked tgz with identical contents in registry mode: `release-canary: Registry dist.integrity sha512-Dt3o… does not match the GitHub tarball integrity sha512-4z6W….`
      - Swapped public key: ``release-canary: Bundled package/lib/release-manifest-public-key.pem (sha256 0d45b4ec…) is not byte-identical to the checked-in npm/lib/release-manifest-public-key.pem (sha256 bfc3c81e…).``
      - Wrong `--expected-version 0.4.0-pre-alpha-v11`: `release-canary: Packed package version '0.4.0-pre-alpha-v12' does not match --expected-version '0.4.0-pre-alpha-v11'.`
      - `--registry-version latest`: `release-canary: --registry-version 'latest' is not an exact version; dist-tags and ranges are not resolved.`
      - Each fault exits 1. No `sce-npm-release-canary.*` temp dirs remain.
    - Shellcheck:
      - `nix shell nixpkgs#shellcheck -c shellcheck -x -P SCRIPTDIR npm/test/release-canary.sh` and `... -x -P SCRIPTDIR npm/test/*.sh`: passed.
      - Without `-x -P SCRIPTDIR`, shellcheck reports SC1091 (info) and SC2034 for the sourced harness. The same applies to the planned single-file `shellcheck npm/test/release-canary.sh` form, and `pack-smoke.sh` also fails in that form.
    - Regression and grep checks:
      - `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs`: passed (`pack-smoke: OK (sce --help, sce version)`).
      - `rg -n 'createServer|openssl req' npm nix .github`: hits only `fixture-release-server.mjs` and `release-fixture.sh`.
      - `rg -n 'SCE_NPM_RELEASE_BASE_URL|SCE_NPM_SKIP_DOWNLOAD|NODE_EXTRA_CA_CERTS' npm/test/release-canary.sh`: no hits.
  - Context impact: local to npm test tooling. The canary script and its modes are a new release-verification surface. Its durable documentation, with the B1/B2/registry gates, is T10's scope (`context/sce/cli-npm-distribution-contract.md`), and no root context file changes.
  - Context synchronization: synced

- [x] T08: `Gate GitHub Release creation on the B1 canary in release-sce.yml` (status:done)
  - Task ID: T08
  - Scope: In — `.github/workflows/release-sce.yml`: split `release` into `assemble` (signing secret only in the manifest step; uploads `dist/` with a SHA-pinned upload action), `release-canary` (no secrets, `contents: read`; downloads `dist/`, runs `release-canary.sh --release-dir` via Nix, outputs the tarball SHA-256), and `release` (needs both; re-checks the SHA-256 and creates the GitHub Release from the downloaded `dist/`). Out — the npm publish call (T09) and native build workflows.
  - Dependencies: T07
  - Done when: no path creates a release without a successful canary, and the secret appears in exactly one step.
  - Verify: `nix build .#checks.x86_64-linux.workflow-actionlint`; `grep -n 'SCE_RELEASE_MANIFEST_SIGNING_KEY\|secrets\.' .github/workflows/release-sce.yml`; inspect the job `needs`/`if` chain.
  - Completed: 2026-11-04
  - Files changed: `.github/workflows/release-sce.yml`.
  - Result: The former `release` job is now three jobs:
    - `assemble` (`contents: read`): the same needs/`if` as before and the same checkout, Nix, artifact download, manifest, npm, and Flatpak steps. `SCE_RELEASE_MANIFEST_SIGNING_KEY` appears only in the `Assemble CLI release manifest` step's `env`. It uploads `dist/` as the `sce-assembled-dist` workflow artifact with the already pinned `actions/upload-artifact@043fb46d… # v7.0.1` (`if-no-files-found: error`).
    - `release-canary` (`contents: read`, no `secrets.` reference): needs `resolve-release` and `assemble`, with `if` requiring both to succeed. It hardens the runner, checks out the tag, installs Nix, downloads `sce-assembled-dist` into `dist/`, and records the SHA-256 of `dist/npm/sce-v<version>-npm.tgz`. It then runs `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/release-canary.sh --release-dir dist/cli --tarball <tgz> --expected-version <version> --expected-sha256 <recorded>`. Only after the canary exits 0 does it write the `npm_tarball_sha256` job output.
    - `release` (`contents: write`): needs `resolve-release`, `assemble`, and `release-canary`. Its `if` requires all three to be `success` and a non-empty `npm_tarball_sha256`. It downloads `sce-assembled-dist`, fails unless the tarball's `sha256sum` equals the canary output, then runs the unchanged `softprops/action-gh-release` step with the existing `files` globs. It no longer checks out the repository or installs Nix, because it only consumes `dist/`.
    - The workflow-level permission dropped from `contents: write` to `contents: read`. `create-tag` (tag push) and `release` (release creation) get `contents: write` at job level. The called build workflows already declare `contents: read`.
  - Deviation: the artifact is named `sce-assembled-dist`, not `sce-release-dist`, so it cannot match `assemble`'s own `sce-release-*` download pattern on a job re-run.
  - Verify outcomes:
    - `nix build .#checks.x86_64-linux.workflow-actionlint --print-build-logs`: passed (exit 0). `nix shell nixpkgs#actionlint nixpkgs#shellcheck -c actionlint .github/workflows/release-sce.yml`: passed with no findings.
    - `grep -n 'SCE_RELEASE_MANIFEST_SIGNING_KEY\|secrets\.' .github/workflows/release-sce.yml`: one hit, line 238, the `Assemble CLI release manifest` step `env` in `assemble`. `release-canary` and `release` hold no secret reference.
    - The `needs`/`if` chain was inspected. `release` is reachable only through `release-canary.result == 'success'` with a non-empty SHA output. The only release-creation step runs after the SHA re-check, and no other job creates a release.
    - Local behavior with the published v12 assets laid out as `dist/cli` (manifest, `.sig`, x86_64 musl archive, `.sha256`, `SHA256SUMS`) and `dist/npm/sce-v0.4.0-pre-alpha-v12-npm.tgz`, with the step scripts extracted from the workflow by `yq`:
      - Canary step: exit 0, `release-canary: OK (local mode: @crocoder-dev/sce@0.4.0-pre-alpha-v12, tarball sha256 ec6bf434…edf57)`, and `GITHUB_OUTPUT` holds `npm_tarball_sha256=ec6bf434…edf57`.
      - Release verify step with that output: exit 0, `npm tarball sha256 matches the canary-verified bytes`. With a wrong SHA: exit 1, `npm tarball sha256 ec6bf434… does not match canary-verified sha256 0000…`.
      - Fault, a byte-appended tarball in the scratch copy: the canary step exits 1 with `release-canary: Tarball … has no readable package/package.json.` and writes no output, so `release` would be skipped.
    - The scratch directory was removed, and `git status` shows only the workflow change.
    - Live execution inside GitHub Actions needs a real release run and is not proven here.
  - Context impact: release workflow topology. The `assemble`/`release-canary`/`release` split and the secret scoping are documented in T10 (`context/sce/cli-release-artifact-contract.md`, `context/sce/cli-npm-distribution-contract.md`). No root context file content is expected to change.
  - Context synchronization: synced

- [x] T09: `Make publish-npm.yml a reusable workflow with pre- and post-publish canaries` (status:done)
  - Task ID: T09
  - Scope: In — `.github/workflows/publish-npm.yml`: add a `workflow_call` trigger with `release_tag`/`dry_run`/`prerelease` inputs and an explicit `NPM_TOKEN` secret, keeping `workflow_dispatch` and `release: published`. Jobs: `resolve` (parity and tag checks), `github-release-canary` (no secrets; downloads the asset and runs `release-canary.sh --github-release`), `publish` (needs the canary; `NPM_TOKEN` only in the publish step's env; skips when the exact version already exists with matching integrity and fails on differing integrity; supports dry-run), and `registry-canary` (no secrets; `--registry-version` with bounded retries; skipped on dry-run). `.github/workflows/release-sce.yml`: a `publish-npm` job that `needs: release` and `uses: ./.github/workflows/publish-npm.yml`, passing the tag, the prerelease flag, `dry_run: false`, and only `NPM_TOKEN`. Out — crates.io publishing, npm provenance changes.
  - Dependencies: T08
  - Done when: the trigger and dependency chain meets AC9, and actionlint passes.
  - Verify: `nix build .#checks.x86_64-linux.workflow-actionlint`; `grep -n 'NPM_TOKEN\|secrets' .github/workflows/publish-npm.yml .github/workflows/release-sce.yml`; inspect for no `secrets: inherit` and no `latest` resolution in canary commands.
  - Completed: 2026-11-04
  - Files changed: `.github/workflows/publish-npm.yml`, `.github/workflows/release-sce.yml`.
  - Result:
    - `publish-npm.yml` triggers: `release: published` (kept), `workflow_dispatch` (kept; `dry_run` still defaults to `true`), and a new `workflow_call` with `release_tag` (required string), `dry_run` (boolean, default `false`), `prerelease` (boolean, default `false`), and an explicitly declared optional `NPM_TOKEN` secret. The workflow-level `contents: read` and the per-tag concurrency group are unchanged.
    - Trigger mapping: the tag is `inputs.release_tag || github.event.release.tag_name`. `DRY_RUN` is `github.event_name != 'release' && inputs.dry_run || false`, and `PRE_RELEASE` reads the release payload for `release` events or `inputs.prerelease` otherwise. Because a called workflow sees the caller's `event_name` (`push` or `workflow_dispatch` from `release-sce.yml`), a call always maps to the call inputs.
    - The single `publish` job is now four jobs, each with `harden-runner` and job-level `contents: read`:
      - `resolve`: checks out the tag and runs the existing `.version`/`npm/package.json`/tag/prerelease parity checks. It derives the `next`/`latest` dist-tag and the normalized `dry_run` value, downloads `sce-v<version>-npm.tgz` with `gh release download` (`github.token`), and verifies the packed name and version. It outputs `tag`, `version`, `npm_tag`, `dry_run`, and `npm_tarball_sha256`.
      - `github-release-canary` (needs `resolve`, no secret reference): installs Nix, checks out the tag, and runs `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/release-canary.sh --github-release <tag> --expected-version <version> --expected-sha256 <resolve sha>` (B2).
      - `publish` (needs `resolve` and `github-release-canary`; `if` requires both `success`):
        - It downloads the asset again and fails unless its SHA-256 equals the canary-verified `resolve` hash.
        - `Check registry for an existing publication` compares `npm view @crocoder-dev/sce@<version> dist.integrity` with the local `sha512-` integrity. An empty result or `E404` means unpublished. A matching value sets `already_published=true`. A differing value or any other registry error fails the job.
        - The dry-run step runs `npm publish --dry-run`.
        - The publish step is the only place `secrets.NPM_TOKEN` appears in this file, as `NODE_AUTH_TOKEN` in that step's `env`. The former separate token-check step is folded into it. It writes a temporary `--userconfig` containing the literal `${NODE_AUTH_TOKEN}` reference, so the token never touches disk. `setup-node` no longer sets `registry-url`, so no placeholder auth token is exported to other steps.
        - Both publish steps are skipped when `already_published` is `true`, which makes reruns idempotent.
      - `registry-canary` (needs `resolve` and `publish`; `if` requires both `success` and `dry_run != 'true'`; no secret reference): runs `release-canary.sh --registry-version <version> --expected-version <version> --expected-sha256 <resolve sha>`. The script's bounded 10 × 30s registry retries, its byte/`dist.integrity` comparison with the GitHub tgz, and its exact-semver guard apply.
    - `release-sce.yml` has a new `publish-npm` job: `needs: [resolve-release, release]`, an `if` requiring both `success`, `permissions: contents: read`, and `uses: ./.github/workflows/publish-npm.yml`. It passes `release_tag: needs.resolve-release.outputs.tag`, `dry_run: false`, `prerelease: github.event_name == 'workflow_dispatch' && inputs.prerelease || false` (the same expression as the release step), and `secrets: NPM_TOKEN: ${{ secrets.NPM_TOKEN }}` only. There is no `secrets: inherit`.
  - Deviations and assumptions:
    - The handoff hash is computed in `resolve` from the GitHub Release asset. It is not passed in from `release-sce.yml`'s B1 canary output, which keeps the `workflow_call` interface to the three planned inputs.
    - `github.token` is used for `gh release download` in `resolve` and `publish`. It is not a `secrets.` reference. The canary jobs use the public release URL, as in T07.
  - Verify outcomes:
    - Actionlint:
      - `nix build .#checks.x86_64-linux.workflow-actionlint --print-build-logs`: passed (exit 0).
      - `nix shell nixpkgs#actionlint nixpkgs#shellcheck -c actionlint .github/workflows/publish-npm.yml .github/workflows/release-sce.yml`: passed with no findings.
    - `grep -n 'NPM_TOKEN\|secrets' .github/workflows/publish-npm.yml .github/workflows/release-sce.yml`:
      - `publish-npm.yml`: the `workflow_call` `secrets:`/`NPM_TOKEN:` declaration (lines 23–24), `NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}` in the `Publish package to npm` step `env` (line 302), and that step's error text (line 307).
      - `release-sce.yml`: the signing key in `assemble`'s manifest step (line 238, unchanged), and the `publish-npm` job's explicit `secrets: NPM_TOKEN` (lines 417–418).
      - No canary job references a secret.
    - `grep -n 'inherit\|latest'`: no `secrets: inherit`. The only `latest` hits are the `next`/`latest` publish dist-tag selection and the input descriptions. Both canary commands pass an exact `.version`-derived semver, never a dist-tag.
    - The `needs`/`if` chain was inspected:
      - `release` → `publish-npm` (`release-sce.yml`).
      - `resolve` → `github-release-canary` → `publish` → `registry-canary` (`publish-npm.yml`).
      - `npm publish` is reachable only after a successful B2 canary. The registry canary runs only after a successful, non-dry-run `publish`, and a failure there fails the workflow.
    - Local behavior check: the `Check registry for an existing publication` step script was extracted with `yq` and run against the published `sce-v0.4.0-pre-alpha-v12-npm.tgz`:
      - Unmodified tgz at version `0.4.0-pre-alpha-v12`: exit 0 with `already_published=true` (matching `sha512-Dt3odIlX…UKA==`).
      - The same bytes under an unpublished `0.4.0-pre-alpha-v999`: exit 0 with `already_published=false`.
      - A byte-appended tgz: exit 1 with `… already published with integrity sha512-Dt3o…, which does not match the GitHub Release tarball integrity sha512-Q6fp…`.
      - The extracted publish step shows the heredoc writes the literal `//registry.npmjs.org/:_authToken=${NODE_AUTH_TOKEN}` line. The scratch directory was removed.
    - Not proven here: live `workflow_call` handoff, B2 and registry canaries inside Actions, and real `npm publish`/rerun behavior. These need a real release run.
  - Context impact: release and npm publication workflow topology (the `workflow_call` trigger, the four-job publish chain, `NPM_TOKEN` scoping, idempotent reruns). Durable documentation is T10's scope (`context/sce/cli-npm-distribution-contract.md`, `context/sce/cli-first-install-channels-contract.md`, `context/sce/cli-release-artifact-contract.md`). No root context file content is expected to change.
  - Context synchronization: synced

- [x] T10: `Document release-security canary gates and npm publication topology` (status:done)
  - Task ID: T10
  - Scope: In — `context/sce/cli-npm-distribution-contract.md` (release-security canary, the B1/B2/registry gates, updated Registry publication topology covering the `workflow_call` trigger, secret scoping, reruns, and idempotent republish), `context/sce/cli-release-artifact-contract.md` (assemble/canary/release job split), `context/sce/cli-first-install-channels-contract.md` (release-to-npm topology), and `context/context-map.md` annotations if needed. Each doc states what can only be validated by a real release. Out — VM docs (T06).
  - Dependencies: T09
  - Done when: the docs cover the release half of AC12, with job and script names matching the workflows.
  - Verify: inspect the three contracts; `grep -n 'release-canary' context/sce/*.md .github/workflows/*.yml`.
  - Completed: 2026-11-04
  - Files changed: `context/sce/cli-npm-distribution-contract.md`, `context/sce/cli-release-artifact-contract.md`, `context/sce/cli-first-install-channels-contract.md`, `context/context-map.md`.
  - Result:
    - `cli-npm-distribution-contract.md`:
      - Replaced the stale "remote modes are not yet wired" line with a "Release gates" list: B1 is the `release-canary` job in `release-sce.yml`, B2 is the `github-release-canary` job in `publish-npm.yml`, and the post-publish gate is `registry-canary`.
      - Added the rule that only B1 sets installer overrides.
      - Added the caveat that local `v0.4.0-pre-alpha-v12` runs exercise the script only, not the new installer code or the gates inside Actions.
      - Added "Manual reruns": re-running failed jobs, a `workflow_dispatch` of `publish-npm.yml` with `release_tag` and `dry_run: false` (dispatch defaults to dry-run), and idempotent republish on matching `dist.integrity` (fails on a mismatch).
      - The existing T09-synced topology bullets (`workflow_call`, `NPM_TOKEN` scoping, real-release-only proof) are kept.
    - `cli-release-artifact-contract.md`: labels `release-canary` as the B1 gate. Adds rerun guidance (reuses `sce-assembled-dist`, still requires the canary-recorded SHA-256) and a pointer to the downstream B2 and registry gates.
    - `cli-first-install-channels-contract.md`: new "Release-to-npm topology" bullet covering the job chain across both workflows, secret confinement, triggers, and real-release-only proof.
    - `context-map.md`: the npm-contract annotation now names the three gates and the `workflow_call`/`NPM_TOKEN`/rerun/idempotent-republish topology.
  - Verify outcomes:
    - `grep -n 'release-canary' context/sce/*.md .github/workflows/*.yml`: passed.
      - Doc job names `release-canary`, `github-release-canary`, and `registry-canary` match `release-sce.yml:271` and `publish-npm.yml:180`/`:321`.
      - Step names `Assemble CLI release manifest` (`release-sce.yml:236`) and `Publish package to npm` (`publish-npm.yml:298`) and the artifact `sce-assembled-dist` match the workflows.
    - Inspected the three contracts against the workflow triggers, `needs`/`if` chains, secret references, and `workflow_dispatch` `dry_run: true` default: consistent.
  - Context impact: durable docs only (`context/sce/` contracts and `context/context-map.md`); no code or root context file changes.
  - Context synchronization: synced

## Open questions

- The launcher's missing-binary message tells users to run `npm rebuild sce`, but the package is `@crocoder-dev/sce`, and `npm rebuild sce` does not rebuild it. The plan asserts the current text and leaves `npm/bin/sce.js` alone. Correcting the message is a one-line launcher change, which would also update the VM assertion. Should it be included?
- B2 inside GitHub Actions, the registry canary, the `workflow_call` handoff, and the idempotent-rerun path can only be proven by a real release run. Before that, the evidence will be actionlint, workflow inspection, and local canary runs against the already published `v0.4.0-pre-alpha-v12` artifacts. That release predates the hardened installer, so a pass exercises the canary script, not the new installer code.

## Validation Report

**Status:** validated  
**Date:** 2026-11-04

### Commands run

- `nix flake check` -> exit 0 (all `checks.x86_64-linux` derivations passed)
- `nix build .#npm-nixos-vm --print-build-logs` -> exit 0 (all nine subtests passed; test script finished in 54.86s; no `exited 124` in the log)
- `nix build .#ci-checks --print-build-logs` -> exit 0 (`result/` holds only `release-portability-audit` and `sce-release`)
- `nix build .#checks.x86_64-linux.npm-bun-tests --print-build-logs` -> exit 0
- `nix build .#checks.x86_64-linux.workflow-actionlint` -> exit 0
- `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs` -> exit 0
- `nix build .#sce-release && nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary ./result/bin/sce` -> exit 0 (`pack-smoke: OK (real binary: sce --help, sce version --format json)`, version `0.4.0-pre-alpha-v12`)
- `nix shell nixpkgs#ripgrep -c rg -n 'createServer|openssl req' npm nix .github` -> exit 0 (hits only `npm/test/fixture-release-server.mjs` and `npm/test/release-fixture.sh`)
- `nix shell nixpkgs#ripgrep -c rg -n 'SCE_NPM_RELEASE_BASE_URL|SCE_NPM_SKIP_DOWNLOAD|NODE_EXTRA_CA_CERTS' npm/test/release-canary.sh` -> exit 1 (no hits; the overrides are set only in `fixture_npm_install` in `release-fixture.sh`, which only the `--release-dir` path calls)
- `bash npm/test/release-canary.sh --release-dir <scratch v12 assets> --tarball <scratch>/sce-v0.4.0-pre-alpha-v12-npm.tgz --expected-version 0.4.0-pre-alpha-v12` -> exit 0 (`release-canary: OK (local mode …)`)
- `bash npm/test/release-canary.sh --github-release v0.4.0-pre-alpha-v12 --expected-version 0.4.0-pre-alpha-v12` -> exit 0 (`release-canary: OK (github mode …)`)
- `bash npm/test/release-canary.sh --registry-version 0.4.0-pre-alpha-v12 --expected-version 0.4.0-pre-alpha-v12` -> exit 0 (registry tarball byte-identical; `release-canary: OK (registry mode …)`)
- `nix eval .#checks.x86_64-linux --apply builtins.attrNames` -> exit 0 (`npm-nixos-vm` not listed)
- `git diff a4611a4b --stat -- npm/lib npm/bin` -> exit 0 (empty)
- `git diff a4611a4b --stat -- 'npm/test/*.test.js'` -> exit 0 (empty; no removals)

### Success-criteria verification

- [x] AC1: One shared release-fixture harness -> the `npm-pack-smoke` check and the host `--binary` smoke passed; `createServer|openssl req` hits only `fixture-release-server.mjs` and `release-fixture.sh`.
- [x] AC2: Skipped postinstall and `npm rebuild` recovery -> VM log: `launcher without binary exited 1: The native sce binary is not installed. …`; `runtime/sce` mode 755 and `parsed version: 0.4.0-pre-alpha-v12` after the first and the second rebuild.
- [x] AC3: Failed reinstall preserves the binary -> VM log: `Release manifest authenticity check failed` and `Downloaded sce archive checksum mismatch` (each `npm rebuild exited 1`); `runtime/sce` sha256 `97fb0fd6…fe5a` and mode 755 unchanged; `installer temp dirs in TMPDIR: none` and `runtime/ entries: ['sce']` after each failure; the restored-fixture reinstall succeeded.
- [x] AC4: `sce setup --bootstrap-context` -> VM log as `sce-tester` (uid 1000) in a fresh Git repository: `Context baseline ensured.`, the full context tree listed, `second run: context tree unchanged`, `sentinel preserved`.
- [x] AC5: Minimal runtime -> VM log: `launcher PATH entries: ['node']`; `tar curl gzip bash do not resolve` on the launcher PATH and on `PATH=/var/empty`; `no ELF INTERP program header`; `no FHS loader at /lib64/ld-linux-x86-64.so.2`. Inspected `nix/tests/npm-nixos-vm.nix`: `env -i` environments at lines 49–50; the setup scenario exports its own `setupToolsPath`.
- [x] AC6: Local-only, non-root, loopback-only, installer unchanged -> `npm-nixos-vm` absent from `checks.x86_64-linux`; `ci-checks` holds only `sce-release` and `release-portability-audit`; the `npm/lib` and `npm/bin` diff is empty; the VM node sets `useDHCP = false`, `interfaces = { }`, `vlans = [ ]`; scenarios run under `/home/sce-tester/scenarios/<name>`.
- [x] AC7: Exact-artifact canary -> all three modes exit 0 against `v0.4.0-pre-alpha-v12`: tarball sha256 `ec6bf434…edf57` unchanged, bundled key byte-identical (`bfc3c81e…f212`), manifest verified, `runtime/sce` and `bin/sce.js` mode 755, `.bin/sce` → `bin/sce.js`, version `0.4.0-pre-alpha-v12`; the override variables are absent from `release-canary.sh`.
- [x] AC8: B1 gate -> inspected `release-sce.yml`: `release` needs `release-canary` and gates on its success plus a non-empty `npm_tarball_sha256`; `SCE_RELEASE_MANIFEST_SIGNING_KEY` appears only in the `env` of the `Assemble CLI release manifest` step in `assemble`; `release-canary` has no `secrets.` reference; actionlint passed.
- [x] AC9: npm publication topology -> `publish-npm` in `release-sce.yml` needs `release` and calls `publish-npm.yml` through `uses:` with `NPM_TOKEN` passed explicitly (no `secrets: inherit`); `publish` needs `github-release-canary`; `registry-canary` needs `publish` and runs `--registry-version`, bounded at 10 × 30s in `release-canary.sh`; `secrets.NPM_TOKEN` appears only in the `Publish package to npm` step; the `release: published`, `workflow_call`, and `workflow_dispatch` triggers are present; actionlint passed. Live B2 and registry execution in GitHub Actions is not provable before a real release.
- [x] AC10: Fault injection -> task evidence records each fault in scratch worktrees or copies with its failing message (T02 rebuild skipped; T03 binary replaced and temp sibling left; T04 sentinel overwritten; T05 tar on the PATH and an ELF-interpreter binary; T07 tarball byte change, swapped key, wrong expected version); `git worktree list` shows no scratch worktrees, and `git status` shows no fault residue.
- [x] AC11: Existing JS installer tests -> `npm-bun-tests` passed; the `npm/test/*.test.js` diff against `a4611a4b` is empty.
- [x] AC12: Durable docs -> inspected `cli-npm-distribution-contract.md`, `cli-release-artifact-contract.md`, and `cli-first-install-channels-contract.md`: throwaway-key repack versus the production-key canary, each VM scenario, the B1/B2/registry gates, the `workflow_call` topology, `NPM_TOKEN` scoping, manual reruns, and real-release-only proof; job and step names match the workflows.

### Failed checks and follow-ups

- None.

### Residual risks

- B2 (`github-release-canary`), `registry-canary`, the `workflow_call` handoff, and the idempotent-republish path are proven only by inspection and local canary runs. They need a real release to confirm.
- The v12 canary runs exercise the canary script, not the hardened installer, because v12 predates it.
- T05 recorded an intermittent hang after the archive-checksum-mismatch `npm rebuild` (exit 124, which the T03 assertion accepts). It did not recur in this run (`npm rebuild exited 1`), but the root cause and the permissive T03 assertion remain open follow-ups.
