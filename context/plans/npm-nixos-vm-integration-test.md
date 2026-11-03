# Plan: npm-nixos-vm-integration-test

## Change summary

Extends the hardened npm installer work (PR #344, plan `harden-npm-installer`) with a production-representative NixOS VM integration test. `npm/test/pack-smoke.sh` gains a real-binary mode (`--binary <path>`) that wraps the Nix-built `sce-release` executable in the canonical `sce-v<version>-<target>/{bin/sce,LICENSE,README.md}` archive layout, signs a manifest with a throwaway key swapped only into the staged package copy, serves it through the existing `fixture-release-server.mjs`, and runs the real `npm pack → npm install → postinstall → signed manifest verification → archive download → checksum verification → extraction → atomic install` path before executing the npm-generated `sce` launcher directly. The existing fixture (stub binary) mode stays the default and remains in `nix flake check` as `npm-pack-smoke`.

A new `pkgs.testers.runNixOSTest` definition under `nix/tests/` boots a minimal NixOS VM, runs real-binary mode as a non-root user with an isolated HOME, npm cache, and prefix, and asserts launcher/binary modes, direct shebang execution, `sce --help`, and `sce version --format json` matching `.version`. Because NixOS has no FHS `/lib64` loader or `/usr/lib` glibc, successful execution proves the static-musl binary needs no unavailable dynamic libraries. The VM test is x86_64-linux only, is added to the expensive `ciChecks` aggregate (built by `release-validation` in `.github/workflows/pr-ci.yml`), and stays out of the fast `nix flake check` tier. The installer architecture, production public key, and verification code are unchanged.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: Fixture mode is unchanged in behavior and still runs in the fast tier: the `npm-pack-smoke` flake check passes with the stub binary and is still a member of `checks.<system>`.
  - Validate: `nix build .#checks.x86_64-linux.npm-pack-smoke --print-build-logs` passes; `nix eval .#checks.x86_64-linux --apply builtins.attrNames` lists `npm-pack-smoke` and does not list `npm-nixos-vm`.
- [ ] AC2: Real-binary mode installs the actual release binary through the full postinstall path and executes it: given `--binary <sce-release>/bin/sce`, the smoke packs the package, installs it with lifecycle scripts enabled and without `SCE_NPM_SKIP_DOWNLOAD`, and asserts `runtime/sce` mode `0755`, an executable packed `package/bin/sce.js`, a `.bin/sce` link resolving into the installed `@crocoder-dev/sce` package, exit 0 from `sce --help`, and `sce version --format json` `.version` equal to repo `.version`.
  - Validate: `nix build .#sce-release && nix shell nixpkgs#nodejs nixpkgs#openssl -c bash npm/test/pack-smoke.sh --binary ./result/bin/sce` exits 0 and prints the reported version; `grep -n 'release-manifest-public-key.pem' npm/test/pack-smoke.sh` shows key substitution only in the staged copy and `git diff main -- npm/lib` is empty.
- [ ] AC3: The NixOS VM test boots a clean VM and, as a non-root user with isolated HOME/npm cache/prefix and no external network, installs the real packed package and runs the npm-generated `sce` launcher directly (shebang execution, not `node sce.js`), with `sce --help` succeeding and parsed `sce version --format json` `.version` equal to `.version`.
  - Validate: `nix build .#npm-nixos-vm --print-build-logs` passes and the log shows the VM test steps, the installed modes, and the parsed version; inspect `nix/tests/npm-nixos-vm.nix` for a non-root user, no `SCE_NPM_SKIP_DOWNLOAD`, no manual copy/chmod into `runtime/`, and no network beyond loopback.
- [ ] AC4: `nix build .#ci-checks` on x86_64-linux builds the real `sce-release` once and includes the NixOS VM test alongside the release portability audit; other systems' `ci-checks` and macOS CI behavior are unchanged.
  - Validate: `nix build .#ci-checks --print-build-logs` passes on x86_64-linux and `ls -l result` shows `sce-release`, `release-portability-audit`, and `npm-nixos-vm`; `nix derivation show .#ci-checks` (recursive) shows a single `sce-release` output path referenced by both the VM test and the audit; `nix eval .#packages.aarch64-darwin.ci-checks.drvPath` still evaluates.
- [ ] AC5: The `release-validation` CI job on `ubuntu-latest` can run the VM test with KVM acceleration and fails loudly (not skips) if KVM is unavailable.
  - Validate: inspect `.github/workflows/pr-ci.yml` for KVM enablement and `kvm`/`nixos-test` system features on the Linux leg only; `nix build .#checks.x86_64-linux.workflow-actionlint` passes; a PR CI run shows the `release-validation (ubuntu-latest)` job executing the VM test.
- [ ] AC6: The tests fail for the right reason under injected faults: a packed launcher without its executable bit fails at the launcher-mode assertion; a non-executable/invalid `--binary` fails at the `sce --help` execution step; an installer that does not produce `runtime/sce` fails at the `runtime/sce` assertion. Faults are injected only in temporary worktrees or fixtures and the original state is restored.
  - Validate: plan task evidence records each injected fault, the command run, and the exact failing `pack-smoke:` message; `git status` is clean afterward.
- [ ] AC7: Existing installer negative tests still reject invalid signatures, mismatched versions, bad checksums, malicious archive entries, and leave no partial `runtime/sce` on failure.
  - Validate: `nix build .#checks.x86_64-linux.npm-bun-tests --print-build-logs` passes.
- [ ] AC8: Durable docs describe fixture vs VM testing, why the VM uses the real musl binary, what the VM guarantees, what remains macOS/ARM64-release-only, how to run it locally, and KVM/runner requirements.
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

- [ ] T02: `Add NixOS VM integration test for the real npm install` (status:todo)
  - Task ID: T02
  - Scope: In — new `nix/tests/npm-nixos-vm.nix` using `pkgs.testers.runNixOSTest` (minimal node with `nodejs`, `openssl`, `gnutar`, `gzip`, `coreutils`, `jq`; non-root user; no networking beyond loopback; testScript running `pack-smoke.sh --binary ${sceReleasePackage}/bin/sce` as that user with isolated HOME/cache/prefix, then asserting modes, direct launcher execution, and parsed version); `flake.nix` exposure as `packages.x86_64-linux.npm-nixos-vm` only. Out — `ciChecks` membership and CI workflow changes (T03); aarch64/darwin.
  - Dependencies: T01
  - Done when: `nix build .#npm-nixos-vm` boots the VM, completes the real install and direct CLI execution, and passes; the attribute does not evaluate on non-x86_64-linux systems; `nix flake check` does not build it.
  - Verify: `nix build .#npm-nixos-vm --print-build-logs` (requires KVM locally); `nix eval .#checks.x86_64-linux --apply builtins.attrNames`; `nix flake check --no-build`; temporary-worktree fault: VM run with a broken `--binary` fails at the execution step.
  - Context synchronization: pending

- [ ] T03: `Run the NixOS VM test in ci-checks with KVM on Linux CI` (status:todo)
  - Task ID: T03
  - Scope: In — `flake.nix` `ciChecks` links `npm-nixos-vm` on x86_64-linux next to `release-portability-audit`; `.github/workflows/pr-ci.yml` `release-validation` Linux leg enables `/dev/kvm` and passes `extra-conf` system features to the Nix installer, failing if KVM is missing. Out — macOS steps, `nix-ci` job, new runners unless the hosted runner is proven unable to run the test.
  - Dependencies: T02
  - Done when: `nix build .#ci-checks` on x86_64-linux builds `sce-release` once and runs the VM test; Darwin `ci-checks` evaluation unchanged; workflow passes actionlint.
  - Verify: `nix build .#ci-checks --print-build-logs`; `nix eval .#packages.aarch64-darwin.ci-checks.drvPath`; `nix build .#checks.x86_64-linux.workflow-actionlint`.
  - Context synchronization: pending

- [ ] T04: `Document fixture smoke versus NixOS VM npm integration testing` (status:todo)
  - Task ID: T04
  - Scope: In — `context/sce/cli-npm-distribution-contract.md` (both smoke modes, VM guarantees, real musl rationale, macOS/ARM64 release-only coverage, local run command, KVM requirement, verification baseline); `context/sce/flake-build-performance.md` (`ci-checks` membership, KVM runner requirement); `context/context-map.md` annotation if needed. Out — README marketing changes, unrelated context edits.
  - Dependencies: T03
  - Done when: docs state every point in AC8 concisely and their command names match the flake.
  - Verify: inspect the two context files; `grep -n 'npm-nixos-vm' context/sce/*.md flake.nix`.
  - Context synchronization: pending

## Open questions

- Hosted `ubuntu-latest` runners are expected to provide KVM after a udev rule, but this is only proven by an actual PR CI run. If the run shows KVM is unavailable or the VM is too slow for the 90-minute `release-validation` budget, choosing a dedicated or larger runner is a cost decision for the maintainers rather than something T03 can settle alone.
