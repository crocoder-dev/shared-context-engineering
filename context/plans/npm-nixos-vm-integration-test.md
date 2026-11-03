# Plan: npm-nixos-vm-integration-test

## Change summary

Extends the hardened npm installer work (PR #344, plan `harden-npm-installer`) with a production-representative NixOS VM integration test. `npm/test/pack-smoke.sh` gains a real-binary mode (`--binary <path>`) that wraps the Nix-built `sce-release` executable in the canonical `sce-v<version>-<target>/{bin/sce,LICENSE,README.md}` archive layout, signs a manifest with a throwaway key swapped only into the staged package copy, serves it through the existing `fixture-release-server.mjs`, and runs the real `npm pack → npm install → postinstall → signed manifest verification → archive download → checksum verification → extraction → atomic install` path before executing the npm-generated `sce` launcher directly. The existing fixture (stub binary) mode stays the default and remains in `nix flake check` as `npm-pack-smoke`.

A new `pkgs.testers.runNixOSTest` definition under `nix/tests/` boots a minimal NixOS VM, runs real-binary mode as a non-root user with an isolated HOME, npm cache, and prefix, and asserts launcher/binary modes, direct shebang execution, `sce --help`, and `sce version --format json` matching `.version`. Because NixOS has no FHS `/lib64` loader or `/usr/lib` glibc, successful execution proves the static-musl binary needs no unavailable dynamic libraries. The VM test is x86_64-linux only and local-only: it stays out of the fast `nix flake check` tier and, after hosted `ubuntu-latest` runners proved to lack `/dev/kvm` (PR CI run 41700381363), out of `ciChecks` and CI as well (T05). The installer architecture, production public key, and verification code are unchanged.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [x] AC1: Fixture mode is unchanged in behavior and still runs in the fast tier: the `npm-pack-smoke` flake check passes with the stub binary and is still a member of `checks.<system>`.
  - Validate: `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs` passes; `nix eval .#checks.x86_64-linux --apply builtins.attrNames` lists `npm-pack-smoke` and does not list `npm-nixos-vm`.
- [x] AC2: Real-binary mode installs the actual release binary through the full postinstall path and executes it: given `--binary <sce-release>/bin/sce`, the smoke packs the package, installs it with lifecycle scripts enabled and without `SCE_NPM_SKIP_DOWNLOAD`, and asserts `runtime/sce` mode `0755`, an executable packed `package/bin/sce.js`, a `.bin/sce` link resolving into the installed `@crocoder-dev/sce` package, exit 0 from `sce --help`, and `sce version --format json` `.version` equal to repo `.version`.
  - Validate: `nix build .#sce-release && nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary ./result/bin/sce` exits 0 and prints the reported version; `grep -n 'release-manifest-public-key.pem' npm/test/pack-smoke.sh` shows key substitution only in the staged copy and `git diff main -- npm/lib` is empty.
- [x] AC3: The NixOS VM test boots a clean VM and, as a non-root user with isolated HOME/npm cache/prefix and no external network, installs the real packed package and runs the npm-generated `sce` launcher directly (shebang execution, not `node sce.js`), with `sce --help` succeeding and parsed `sce version --format json` `.version` equal to `.version`.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` passes and the log shows the VM test steps, the installed modes, and the parsed version; inspect `nix/tests/npm-nixos-vm.nix` for a non-root user, no `SCE_NPM_SKIP_DOWNLOAD`, no manual copy/chmod into `runtime/`, and no network beyond loopback.
- [ ] AC4: `nix build .#ci-checks` on x86_64-linux is unchanged by this plan: it builds `sce-release` and the release portability audit and does not include `npm-nixos-vm`; other systems' `ci-checks` evaluate.
  - Validate: `nix build .#ci-checks --print-build-logs` passes on x86_64-linux and `ls -l result` shows only `sce-release` and `release-portability-audit`; `nix eval .#packages.aarch64-darwin.ci-checks.drvPath` still evaluates.
- [ ] AC5: CI does not run or require KVM: `.github/workflows/pr-ci.yml` has no KVM enablement step and no `kvm`/`nixos-test` system features, and the NixOS VM test is documented as a local-only run.
  - Validate: `grep -n -i 'kvm\|nixos-test' .github/workflows/pr-ci.yml` finds nothing; `nix build .#checks.x86_64-linux.workflow-actionlint` passes; AC8 inspection confirms the local-only wording.
- [x] AC6: The tests fail for the right reason under injected faults: a packed launcher without its executable bit fails at the launcher-mode assertion; a non-executable/invalid `--binary` fails at the `sce --help` execution step; an installer that does not produce `runtime/sce` fails at the `runtime/sce` assertion. Faults are injected only in temporary worktrees or fixtures and the original state is restored.
  - Validate: plan task evidence records each injected fault, the command run, and the exact failing `pack-smoke:` message; `git status` is clean afterward.
- [x] AC7: Existing installer negative tests still reject invalid signatures, mismatched versions, bad checksums, malicious archive entries, and leave no partial `runtime/sce` on failure.
  - Validate: `nix build .#checks.x86_64-linux.npm-bun-tests --print-build-logs` passes.
- [ ] AC8: Durable docs describe fixture vs VM testing, why the VM uses the real musl binary, what the VM guarantees, what remains macOS/ARM64-release-only, how to run it locally, and that it is local-only because hosted CI runners lack KVM.
  - Validate: inspect `context/sce/cli-npm-distribution-contract.md` and `context/sce/flake-build-performance.md` for those points and matching command names.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix build .#ci-checks --print-build-logs`

### Context sync

- `context/sce/cli-npm-distribution-contract.md` (packed-package smoke modes, NixOS VM test, verification baseline)
- `context/sce/flake-build-performance.md` (`ci-checks` membership and CI tier/KVM requirement)
- `context/context-map.md` (annotation for the npm contract entry, if its summary changes)

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `npm/test/pack-smoke.sh`, new `nix/tests/npm-nixos-vm.nix`, root `flake.nix` (VM test wiring, `ciChecks`, file sets), `.github/workflows/pr-ci.yml` (Linux `release-validation` KVM enablement), `context/sce/cli-npm-distribution-contract.md`, `context/sce/flake-build-performance.md`, `context/context-map.md`.
- **Out of scope:** changes to `npm/lib/*.js`, `npm/bin/sce.js`, the production public key, `fixture-release-server.mjs` behavior beyond what real-binary mode strictly needs, release packaging apps, macOS CI steps, aarch64 VM tests, publishing.
- **Constraints:** no Docker; no external network (all artifacts from Nix inputs or the loopback fixture server); reuse the existing signing, archive, and HTTPS server logic in `pack-smoke.sh` rather than copying it; no `SCE_NPM_SKIP_DOWNLOAD=1`; no manual copy or chmod into `runtime/`; do not weaken signature, identity, or checksum verification; `sceReleasePackage` referenced, never rebuilt with different arguments.
- **Non-goal:** a generic VM test framework, or repeating unit-level negative installer tests inside the VM.

## Assumptions

- The VM test is exposed as `packages.x86_64-linux.npm-nixos-vm` (built via `nix build .#npm-nixos-vm`) rather than `checks.x86_64-linux.npm-nixos-vm`, because every `checks.<system>` member is built by `nix flake check` and the request requires the VM to stay out of the fast tier. This mirrors the existing opt-in `mutation-trace-quint-connect` package precedent.
- `nix/tests/` does not exist yet; the plan creates it with one file, `nix/tests/npm-nixos-vm.nix`, called from `flake.nix` with `sceReleasePackage` and the npm source as arguments.
- Real-binary mode reads the expected version from repo `.version` when available (passed by the caller), falling back to the staged `package.json` version; release-npm-package already enforces `.version` = `npm/package.json` = `cli/Cargo.toml`.
- Real-binary mode copies the given binary into the archive (preserving `0755`) with fixture LICENSE/README files, since the archive reader only writes `bin/sce` and the layout, not the file contents, is the contract under test.
- The npm smoke assertions added for `runtime/sce` mode `0755` and the `.bin/sce` link target apply in both modes, so the fast tier also guards them.
- GitHub-hosted `ubuntu-latest` runners expose `/dev/kvm` after the documented udev rule, and Nix needs `system-features = nixos-test benchmark big-parallel kvm` via the installer's `extra-conf`.

## Task stack

- [x] T01: `Add real-binary mode and install-layout assertions to pack-smoke.sh` (status:done)
  - Task ID: T01
  - Scope: In — `npm/test/pack-smoke.sh`: `--binary <path>` (and an optional `--expected-version` / `--prefix` if needed by the VM), real-binary archive staging in the canonical layout, `sce --help` exit-status and `sce version --format json` `.version` assertions in real mode, `runtime/sce` mode `0755` and `.bin/sce` → installed package resolution assertions in both modes; usage text. Out — any change to `npm/lib`, `npm/bin`, the production key, or `fixture-release-server.mjs` signing/serving logic; flake wiring.
  - Dependencies: none
  - Done when: default invocation behaves as before plus the new layout assertions; `--binary` drives the same signing/server/install path with the real binary; no signing, archive, or server logic is duplicated; shellcheck clean.
  - Verify: `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs`; `nix build .#sce-release && nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary ./result/bin/sce`; `nix shell nixpkgs#shellcheck -c shellcheck npm/test/pack-smoke.sh`; in a temporary worktree inject (a) `chmod -x npm/bin/sce.js` with `--package-dir`, (b) `--binary` pointing at a non-executable/garbage file, (c) an installer that skips the rename, and record each failing message.
  - Completed: 2026-11-03
  - Files changed: `npm/test/pack-smoke.sh`
  - Result: Added `--binary <path>` (copies the given executable as `bin/sce` mode 0755 into the canonical `sce-v<version>-<target>/{bin/sce,LICENSE,README.md}` archive; all signing, manifest, pack, loopback-server, and install steps are shared with fixture mode), `--expected-version <v>` (default: staged `package.json` version), and `--prefix <dir>` (persistent install prefix for the T02 VM assertions). Both modes now assert `runtime/sce` mode `755` (via `node fs.statSync`, portable to macOS) and that `node_modules/.bin/sce` realpath equals the installed `@crocoder-dev/sce/bin/sce.js`. Real mode asserts `sce --help` exit 0 and `sce version --format json` `.version` == expected version; fixture mode keeps its exact stub-output assertions. Usage text updated. No change to `npm/lib`, `npm/bin`, the production key, or `fixture-release-server.mjs`; key substitution stays confined to `$stage_dir/lib/release-manifest-public-key.pem`.
  - Verify:
    - `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs`: passed (log shows `installed runtime/sce mode 755`, `.bin/sce resolves to …/@crocoder-dev/sce/bin/sce.js`, `OK (sce --help, sce version)`).
    - `nix build .#sce-release` + `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary <sce-release>/bin/sce`: passed, exit 0, `real binary reported version 0.4.0-pre-alpha-v12`; also passed with `--expected-version "$(cat .version)" --prefix <scratch>` (installed `runtime/sce` retained as `-rwxr-xr-x`). `--expected-version 9.9.9` fails with `pack-smoke: sce version reported '0.4.0-pre-alpha-v12'; expected '9.9.9'.`
    - `nix shell nixpkgs#shellcheck -c shellcheck npm/test/pack-smoke.sh`: passed (no findings).
    - Fault (a), scratch copy of `npm/` with `chmod -x bin/sce.js`, `--package-dir`: `pack-smoke: Packed package/bin/sce.js has mode -rw-r--r--; expected an executable mode (e.g. -rwxr-xr-x). Ensure npm/bin/sce.js is committed as Git mode 100755.`
    - Fault (b), `--binary` pointing at a non-executable truncated-ELF garbage file: `pack-smoke: sce --help exited non-zero.`
    - Fault (c), scratch copy of `npm/` with `installBinaryFile(...)` removed from `lib/install.js`, `--package-dir`: `pack-smoke: Installer did not produce …/node_modules/@crocoder-dev/sce/runtime/sce.`
    - `git status` afterward shows only `npm/test/pack-smoke.sh` and this plan changed.
  - Deviations: The first fault (b) attempt used random bytes. libuv's `execvp` falls back to `/bin/sh` on ENOEXEC, so the shell interpreted the bytes and created two empty stray files in the caller's cwd. Both were removed, and the fault was re-run from a scratch cwd with a deterministic truncated-ELF file. T02 should run garbage-binary faults from a throwaway cwd.
  - Context impact: localized — npm packed-package smoke test surface (`context/sce/cli-npm-distribution-contract.md` smoke-mode description); full documentation is owned by T04.
  - Context synchronization: synced

- [x] T02: `Add NixOS VM integration test for the real npm install` (status:done)
  - Task ID: T02
  - Scope: In — new `nix/tests/npm-nixos-vm.nix` using `pkgs.testers.runNixOSTest` (minimal node with `nodejs`, `openssl`, `gnutar`, `gzip`, `coreutils`, `jq`; non-root user; no networking beyond loopback; testScript running `pack-smoke.sh --binary ${sceReleasePackage}/bin/sce` as that user with isolated HOME/cache/prefix, then asserting modes, direct launcher execution, and parsed version); `flake.nix` exposure as `packages.x86_64-linux.npm-nixos-vm` only. Out — `ciChecks` membership and CI workflow changes (T03); aarch64/darwin.
  - Dependencies: T01
  - Done when: `nix build .#npm-nixos-vm` boots the VM, completes the real install and direct CLI execution, and passes; the attribute does not evaluate on non-x86_64-linux systems; `nix flake check` does not build it.
  - Verify: `nix build .#npm-nixos-vm --print-build-logs` (requires KVM locally); `nix eval .#checks.x86_64-linux --apply builtins.attrNames`; `nix flake check --no-build`; temporary-worktree fault: VM run with a broken `--binary` fails at the execution step.
  - Completed: 2026-11-03
  - Files changed: `nix/tests/npm-nixos-vm.nix` (new), `flake.nix`
  - Result: Added `nix/tests/npm-nixos-vm.nix`, a `pkgs.testers.runNixOSTest` with one node. The node has `bash`, `coreutils`, `gnutar`, `gzip`, `jq`, `nodejs`, and `openssl`, a non-root `sce-tester` user, no DHCP, no interfaces, and `virtualisation.vlans = [ ]` (loopback only). The testScript checks that the user's uid is not 0, then copies `npmSrc/npm` into the user's home. As that user, with HOME, TMPDIR, `npm_config_cache`, and `npm_config_userconfig` isolated, it runs `pack-smoke.sh --binary ${sceReleasePackage}/bin/sce --expected-version <.version> --prefix ~/prefix`. It then asserts: `runtime/sce` mode `755`, an executable `.bin/sce` that resolves to the installed `bin/sce.js`, a direct shebang run of `.bin/sce --help`, and `sce version --format json | jq -er .version` equal to `.version`. There is no `SCE_NPM_SKIP_DOWNLOAD` and no manual copy or chmod into `runtime/`. `flake.nix` imports the test with `pkgs`, `npmSrc`, `sceReleasePackage`, and `expectedVersion = version`, and exposes it only as `packages.x86_64-linux.npm-nixos-vm` via `optionalAttrs (system == "x86_64-linux")`. On user request, the explanatory comments added in the new file and in `flake.nix` were removed.
  - Verify:
    - `nix build .#npm-nixos-vm --print-build-logs`: passed. This host has no `/dev/kvm`, so QEMU logged `failed to initialize kvm` and ran under TCG; the test script finished in 115s. The log shows `pack-smoke: installed runtime/sce mode 755`, `.bin/sce resolves to /home/sce-tester/prefix/node_modules/@crocoder-dev/sce/bin/sce.js`, `real binary reported version 0.4.0-pre-alpha-v12`, `runtime/sce mode: 755`, `launcher mode: 755`, and `parsed version: 0.4.0-pre-alpha-v12`. pack-smoke did not use its `node` fallback: `/usr/bin/env` exists, so the shebang ran directly. Removing the comments left the drv path unchanged (`d09iqhh4…-vm-test-run-npm-nixos-vm.drv`).
    - `nix eval .#checks.x86_64-linux --apply builtins.attrNames`: lists `npm-pack-smoke` and does not list `npm-nixos-vm`. `nix eval .#packages.x86_64-darwin --apply 'p: builtins.hasAttr "npm-nixos-vm" p'` returns `false`, and the `aarch64-darwin` package names do not include it.
    - `nix flake check --no-build`: passed (`all checks passed!`).
    - Fault: a scratch copy of `npm/` and `nix/tests/` was imported with `sceReleasePackage` replaced by a derivation whose `bin/sce` is a truncated 7-byte ELF header. The VM failed at the install subtest with `pack-smoke: sce --help exited non-zero.` (test `install the packed package through postinstall with the real binary` failed). The repo worktree was not touched.
  - Deviations: The new file is registered with `git add -N` (intent-to-add) so the flake can see it. The first fault-injection attempt used `builtins.getFlake` on the working directory and stalled copying the tree; it was stopped and re-run from a minimal scratch copy.
  - Context impact: localized. Adds the opt-in `.#npm-nixos-vm` package (x86_64-linux), relevant to `context/sce/cli-npm-distribution-contract.md` and `context/sce/flake-build-performance.md`. Full documentation is owned by T04, and `ciChecks` membership by T03.
  - Context synchronization: synced

- [x] T03: `Run the NixOS VM test in ci-checks with KVM on Linux CI` (status:done)
  - Task ID: T03
  - Scope: In — `flake.nix` `ciChecks` links `npm-nixos-vm` on x86_64-linux next to `release-portability-audit`; `.github/workflows/pr-ci.yml` `release-validation` Linux leg enables `/dev/kvm` and passes `extra-conf` system features to the Nix installer, failing if KVM is missing. Out — macOS steps, `nix-ci` job, new runners unless the hosted runner is proven unable to run the test.
  - Dependencies: T02
  - Done when: `nix build .#ci-checks` on x86_64-linux builds `sce-release` once and runs the VM test; Darwin `ci-checks` evaluation unchanged; workflow passes actionlint.
  - Verify: `nix build .#ci-checks --print-build-logs`; `nix eval .#packages.aarch64-darwin.ci-checks.drvPath`; `nix build .#checks.x86_64-linux.workflow-actionlint`.
  - Completed: 2026-11-03
  - Files changed: `flake.nix`, `.github/workflows/pr-ci.yml`
  - Result: `ciChecks` now links `npm-nixos-vm` (`npmNixosVmTest`) on x86_64-linux only, using `optionalString (system == "x86_64-linux")`. The new guard is concatenated onto the existing Linux audit guard, so the build script and drv are byte-identical on other systems. In `.github/workflows/pr-ci.yml`, the `release-validation` job gets an `Enable KVM` step, gated on `matrix.os == 'ubuntu-latest'`. It installs the `99-kvm4all` udev rule, reloads and triggers it, and exits 1 with `::error::/dev/kvm is not available; the NixOS VM test requires KVM.` when `/dev/kvm` is not readable and writable. The Install Nix step passes `extra-conf: system-features = nixos-test benchmark big-parallel kvm` on ubuntu only (empty on macOS). The `nix-ci` job and the macOS steps are unchanged. On user request, the explanatory comment above `ciChecks` and the new workflow comment were removed.
  - Verify:
    - `nix build .#ci-checks --print-build-logs`: passed (exit 0). `result` contains `sce-release`, `release-portability-audit`, and `npm-nixos-vm`, and its out path equals `nix eval .#packages.x86_64-linux.ci-checks.outPath` (`9zpgkyb9…-sce-ci-checks`). The log shows the portability audit passing on `dm1zsfzx…-sce-0.4.0-pre-alpha-v12/bin/sce`, plus the VM test lines `pack-smoke: installed runtime/sce mode 755`, `real binary reported version 0.4.0-pre-alpha-v12`, `runtime/sce mode: 755`, `launcher mode: 755`, `parsed version: 0.4.0-pre-alpha-v12`, and `test script finished in 109.70s`. This host has no `/dev/kvm` (QEMU logged `failed to initialize kvm`), so the VM ran under TCG. `nix-store -qR` on the `ci-checks` drv finds a single `sce-0.4.0-pre-alpha-v12.drv` (`yc4j00d7…`), referenced by both `vm-test-run-npm-nixos-vm.drv` and `sce-release-portability-audit.drv`.
    - `nix eval .#packages.aarch64-darwin.ci-checks.drvPath`: evaluates to `zxz1djgg…-sce-ci-checks.drv`, identical to the HEAD worktree. `x86_64-darwin` (`cc5cbl22…`) and `aarch64-linux` (`9w6kpm8m…`) are also identical to HEAD.
    - `nix build .#checks.x86_64-linux.workflow-actionlint`: passed (also re-run after the comment removal).
  - Deviations: The first guard was placed on its own line. That added a blank line to the Darwin build script and changed the aarch64-darwin drv. It was folded onto the existing `optionalString` line to keep non-x86_64-linux drvs unchanged. KVM availability on hosted `ubuntu-latest` is still proven only by a PR CI run (see Open questions).
  - Context impact: localized. `ci-checks` membership on x86_64-linux and the KVM requirement of the Linux `release-validation` job affect `context/sce/flake-build-performance.md` and `context/sce/cli-npm-distribution-contract.md`. Full documentation is owned by T04.
  - Context synchronization: synced

- [x] T04: `Document fixture smoke versus NixOS VM npm integration testing` (status:done)
  - Task ID: T04
  - Scope: In — `context/sce/cli-npm-distribution-contract.md` (both smoke modes, VM guarantees, real musl rationale, macOS/ARM64 release-only coverage, local run command, KVM requirement, verification baseline); `context/sce/flake-build-performance.md` (`ci-checks` membership, KVM runner requirement); `context/context-map.md` annotation if needed. Out — README marketing changes, unrelated context edits.
  - Dependencies: T03
  - Done when: docs state every point in AC8 concisely and their command names match the flake.
  - Verify: inspect the two context files; `grep -n 'npm-nixos-vm' context/sce/*.md flake.nix`.
  - Completed: 2026-11-03
  - Files changed: `context/sce/cli-npm-distribution-contract.md`, `context/sce/flake-build-performance.md`
  - Result: The earlier T01–T03 syncs already documented both smoke modes, the VM's guarantees and musl/no-FHS rationale, `ci-checks` membership, the CI KVM enablement, and the verification baseline. This task filled the remaining AC8 gaps. The npm contract now has three new bullets in its packed-package smoke section: (1) fixture mode versus the VM, including why the VM needs the real static-musl binary (the stub cannot reveal native linkage problems); (2) the VM covers only `linux/x64`, while `linux/arm64` and `darwin/arm64` remain covered only by the release pipeline's portability audits and extracted-binary smoke; (3) the local run command with KVM/TCG behavior, and the CI path through `.#ci-checks` in `release-validation (ubuntu-latest)`: KVM enabled by a udev rule, a hard failure when KVM is missing, the `nixos-test benchmark big-parallel kvm` system features, and no VM on the macOS leg. `flake-build-performance.md` gained the standalone local build command and its non-KVM behavior. `context/context-map.md` already names `.#npm-nixos-vm` accurately and was left unchanged.
  - Verify:
    - Inspection of both context files: every AC8 point is present (fixture vs VM, real musl rationale, VM guarantees, macOS/ARM64 release-only coverage, local run command, KVM/runner requirements).
    - `grep -n 'npm-nixos-vm' context/sce/*.md flake.nix`: the docs reference `.#npm-nixos-vm`, `packages.x86_64-linux.npm-nixos-vm`, and `nix/tests/npm-nixos-vm.nix`, matching `flake.nix` (`npm-nixos-vm = npmNixosVmTest;`, the `$out/npm-nixos-vm` link in `ci-checks`, and the `./nix/tests/npm-nixos-vm.nix` import). The workflow wording matches `.github/workflows/pr-ci.yml` lines 82–93.
  - Deviations: none.
  - Context impact: localized. Documentation-only within `context/sce/`; no code, interface, or terminology change.
  - Context synchronization: synced

- [x] T05: `Drop the NixOS VM test and KVM enablement from CI; keep it local-only` (status:done)
  - Task ID: T05
  - Scope: In — `flake.nix` (remove the `npm-nixos-vm` link from `ciChecks`; keep `packages.x86_64-linux.npm-nixos-vm`), `.github/workflows/pr-ci.yml` (remove the `Enable KVM` step and the `extra-conf` system features), and the context docs that described CI coverage of the VM (`context/sce/cli-npm-distribution-contract.md`, `context/sce/flake-build-performance.md`, `context/overview.md`, `context/patterns.md`, `context/glossary.md`, `context/architecture.md`). Out — `nix/tests/npm-nixos-vm.nix`, `npm/test/pack-smoke.sh`.
  - Dependencies: T04
  - Reason: PR CI run `41700381363`, job `126141783713` (`Release validation (ubuntu-latest)`, commit `41bdb5e9`) failed at `Enable KVM` with `/dev/kvm is not available; the NixOS VM test requires KVM.` The user decided to drop KVM from CI and run the VM test locally only.
  - Done when: `ciChecks` no longer includes the VM test; the workflow matches its pre-T03 state; docs describe the VM test as local-only; AC4/AC5/AC8 amended accordingly.
  - Verify: `nix build .#checks.x86_64-linux.workflow-actionlint`; `nix build .#ci-checks` and `ls result/`; `nix flake check`; `nix build .#npm-nixos-vm` still builds.
  - Completed: 2026-11-03
  - Files changed: `flake.nix`, `.github/workflows/pr-ci.yml`, `context/sce/cli-npm-distribution-contract.md`, `context/sce/flake-build-performance.md`, `context/overview.md`, `context/patterns.md`, `context/glossary.md`, `context/architecture.md`, this plan
  - Context synchronization: synced

## Open questions

- Resolved by T05: hosted `ubuntu-latest` runners do not provide `/dev/kvm` (PR CI run 41700381363), so the VM test is local-only. Originally: hosted runners were expected to provide KVM after a udev rule, but this is only proven by an actual PR CI run. If the run shows KVM is unavailable or the VM is too slow for the 90-minute `release-validation` budget, choosing a dedicated or larger runner is a cost decision for the maintainers rather than something T03 can settle alone.

## Validation Report

**Status:** failed  
**Date:** 2026-11-03

### Commands run

- `nix flake check` -> exit 0 (all checks passed; x86_64-linux only, other systems omitted as incompatible)
- `nix build .#ci-checks --print-build-logs` -> exit 0 (`result/` links `sce-release`, `release-portability-audit`, `npm-nixos-vm`; portability audit passed; VM test finished in 104.78s under TCG because this host has no `/dev/kvm`)
- `nix build .#checks.x86_64-linux.npm-pack-smoke .#checks.x86_64-linux.npm-bun-tests .#checks.x86_64-linux.workflow-actionlint` -> exit 0 (all three passed)
- `nix eval .#checks.x86_64-linux --apply builtins.attrNames` -> exit 0 (lists `npm-pack-smoke`; no `npm-nixos-vm`)
- `nix build .#sce-release` + `nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary <sce-release>/bin/sce` -> exit 0 (`installed runtime/sce mode 755`, `.bin/sce resolves to …/@crocoder-dev/sce/bin/sce.js`, `real binary reported version 0.4.0-pre-alpha-v12`, `OK (real binary: sce --help, sce version --format json)`; run from a scratch cwd, no stray files)
- `grep -n 'release-manifest-public-key.pem' npm/test/pack-smoke.sh` -> exit 0 (single hit, line 153, writes only `$stage_dir/lib/…`)
- `git diff 77a84b60 HEAD -- npm/lib npm/bin` -> exit 0 (empty; this plan's commits do not touch `npm/lib` or `npm/bin`)
- `nix-store -qR` on the `ci-checks` drv -> exit 0 (one `sce-0.4.0-pre-alpha-v12.drv`, `5f9rgjkl…`, in the closure of both `vm-test-run-npm-nixos-vm.drv` and `sce-release-portability-audit.drv`)
- `nix eval .#packages.aarch64-darwin.ci-checks.drvPath` -> exit 0 (evaluates to `506gxj81…-sce-ci-checks.drv`)
- `gh pr list --head npm-nixos-integration-test` / `gh run list --branch npm-nixos-integration-test` -> exit 0 (no PR and no CI runs; branch not pushed)

### Success-criteria verification

- [x] AC1: Fixture mode unchanged and in the fast tier -> `npm-pack-smoke` built and passed; it is listed in `checks.x86_64-linux` and `npm-nixos-vm` is not.
- [x] AC2: Real-binary mode installs and executes the actual release binary -> host `--binary` smoke passed with the asserted mode, link, `--help`, and version; key substitution only in the staged copy; no `npm/lib` changes from this plan (`git diff main -- npm/lib` is non-empty only because of the parent `harden-npm-installer` commits not yet on `main`).
- [x] AC3: NixOS VM test runs the real install as a non-root user and executes the launcher directly -> `ci-checks` log shows the VM test steps, `runtime/sce mode: 755`, `launcher mode: 755`, `parsed version: 0.4.0-pre-alpha-v12`; `nix/tests/npm-nixos-vm.nix` inspected: `sce-tester` with uid != 0 asserted, no `SCE_NPM_SKIP_DOWNLOAD`, no copy/chmod into `runtime/`, `useDHCP = false`, `interfaces = { }`, `vlans = [ ]`.
- [x] AC4: `ci-checks` builds `sce-release` once with VM test and audit -> passed; single shared `sce-release` drv; aarch64-darwin `ci-checks` still evaluates.
- [ ] AC5: `release-validation (ubuntu-latest)` runs the VM test with KVM and fails loudly without it -> workflow inspection passes (ubuntu-only `Enable KVM` step exits 1 with `::error::` when `/dev/kvm` is missing; ubuntu-only `extra-conf` system features) and `workflow-actionlint` passed, but the required PR CI run does not exist yet.
- [x] AC6: Injected faults fail for the right reason -> T01/T02 evidence records faults (a) launcher mode, (b) non-executable `--binary` at `sce --help` (host and VM), (c) missing `runtime/sce`, each with its exact `pack-smoke:` message; `git status` shows no fault residue (only the staged T04 context/plan edits).
- [x] AC7: Existing installer negative tests still pass -> `npm-bun-tests` built and passed (also in `nix flake check`).
- [x] AC8: Durable docs cover fixture vs VM, musl rationale, guarantees, macOS/ARM64 release-only coverage, local command, KVM/runner requirements -> all present in `context/sce/cli-npm-distribution-contract.md` (Packed-package smoke test section) and `context/sce/flake-build-performance.md`; command names match `flake.nix`.

### Failed checks and follow-ups

- AC5 PR CI run: no PR and no GitHub Actions run exist for `npm-nixos-integration-test` (branch not pushed); evidence: `gh pr list --head npm-nixos-integration-test` returned `[]`, `gh run list` returned nothing; required: commit the staged T04 changes, push the branch, open the PR, and confirm `release-validation (ubuntu-latest)` passes `Enable KVM` and builds `npm-nixos-vm` within the 90-minute budget.

### Residual risks

- KVM availability and VM runtime on hosted `ubuntu-latest` are unproven until the PR CI run (plan Open questions).
- Local VM runs used TCG emulation, not KVM.
- `linux/arm64` and `darwin/arm64` npm installs remain covered only by release-pipeline audits and extracted-binary smoke.

### Retry

After repairs, rerun:

`/validate context/plans/npm-nixos-vm-integration-test.md`
