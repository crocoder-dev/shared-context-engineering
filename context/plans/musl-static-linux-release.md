# Plan: musl-static-linux-release

## Change summary

Switch the Linux release build from `*-unknown-linux-gnu` (Nix glibc, hardcoded `/nix/store/` RPATH/interpreter references that fail the native portability audit) to `*-unknown-linux-musl` (fully static binary, no runtime libc dependency, zero `/nix/store/` references). Both x86_64 and aarch64 Linux targets move to musl. The macOS target (`aarch64-apple-darwin`) is unchanged.

The npm launcher package's platform mapping and tests are updated to match the new target triples.

## Success criteria

1. `nix build .#default` on Linux produces a fully static `sce` binary with no `/nix/store/` references in ELF metadata or strings.
2. `nix run .#native-portability-audit -- --platform linux --binary result/bin/sce` passes.
3. `nix flake check` passes on Linux (cli-tests, cli-clippy, cli-fmt, pkl-parity, all JS checks, workflow-actionlint, native-portability-audit, Flatpak checks).
4. GitHub release archive names change from `sce-v<version>-x86_64-unknown-linux-gnu.tar.gz` to `sce-v<version>-x86_64-unknown-linux-musl.tar.gz` (and `aarch64` variant).
5. `npm/lib/platform.js` maps `linux:x64` → `x86_64-unknown-linux-musl` and `linux:arm64` → `aarch64-unknown-linux-musl`.
6. `nix flake check` passes for npm checks (`npm-bun-tests`, `npm-biome-check`, `npm-biome-format`).
7. Context documentation reflects the new release matrix.

## Constraints and non-goals

**In scope:**
- `flake.nix`: Add musl targets to Rust toolchain, create musl-specific Crane build pipeline, update `detect_target_triple`, conditional Linux package selection.
- `.github/workflows/release-sce-linux.yml` and `release-sce-linux-arm.yml`: target triple references.
- `npm/lib/platform.js` and npm tests: platform-to-triple mapping and assertions.
- Context files: release artifact contract, npm distribution contract, overview, glossary.

**Out of scope:**
- macOS build changes (aarch64-apple-darwin stays unchanged).
- Cargo/crates.io publish workflow changes (musl target does not affect crate publication).
- Flatpak build changes (Flatpak is source-built inside Flatpak and is already a musl-independent exception).
- New distribution channels (no AppImage, .deb, .rpm, brew).
- CI matrix changes beyond `nix flake check` verification.

**Constraints:**
- Existing glibc-hosted checks (`cli-tests`, `cli-clippy`, `cli-fmt`) must continue to compile and run on the host toolchain without targeting musl.
- The macOS Nix build (used in PR CI on `macos-latest`) must not break from toolchain changes.

## Assumptions

1. The Rust crate's dependency tree compiles for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` without requiring C-library linkage changes. If a crate (e.g., `turso`/libsql, `keyring-core`/dbus) fails to cross-compile for musl, the task will need follow-up dependency resolution.
2. `pkgs.pkgsMusl` is not required for Rust-targeted musl builds; setting `CARGO_BUILD_TARGET` and the toolchain target is sufficient because Rust's musl targets default to static linking.
3. The `release-artifacts` flake app's audit path (`nix run .#native-portability-audit`) will pass against the musl binary without code changes because the binary has no `/nix/store/` runtime references.

---

## Task stack

- [x] T01: `Add musl toolchain targets and musl Crane build pipeline in flake.nix` (status:done)
  - Task ID: T01
  - Goal: Extend the Nix flake to build a fully static musl `sce` binary on Linux, with a separate Crane pipeline so host-targeted checks remain unaffected.
  - Boundaries (in/out of scope):
    - **In**: `rustToolchain` targets extension, creation of `craneLibMusl` with musl-aware toolchain, new `cargoDepsArgsMusl` / `cargoArtifactsMusl`, new `scePackageMusl` with `CARGO_BUILD_TARGET` conditionally set for Linux, conditional `packages.sce` selection, `detect_target_triple` updated to emit `*-linux-musl`, `release-artifacts` app audit path.
    - **Out**: Workflow file changes, npm code changes, context doc changes, any macOS build changes.
  - Done when:
    - `nix build .#default` on Linux produces `result/bin/sce` that is a static binary.
    - `nix run .#native-portability-audit -- --platform linux --binary result/bin/sce` passes.
    - `nix flake check` passes on Linux (all existing checks including cli-tests, cli-clippy, cli-fmt, Flatpak checks).
    - `detect_target_triple` returns `x86_64-unknown-linux-musl` on `Linux:x86_64` and `aarch64-unknown-linux-musl` on `Linux:aarch64`.
    - `nix run .#release-artifacts -- --version $(cat .version) --out-dir /tmp/release-test` produces archives with `-linux-musl` in the name.
  - Verification notes (commands or checks):
    - `nix build .#default --print-build-logs`
    - `file result/bin/sce` (should report "statically linked")
    - `readelf -d result/bin/sce | grep -E 'RUNPATH|RPATH'` (should be empty)
    - `strings result/bin/sce | grep '/nix/store/'` (should be empty)
    - `nix run .#native-portability-audit -- --platform linux --binary result/bin/sce`
    - `nix flake check --print-build-logs`

- [x] T02: `Update Linux release workflow target triples` (status:done)
  - Task ID: T02
  - Goal: Change hardcoded `*-unknown-linux-gnu` references to `*-unknown-linux-musl` in both Linux release workflow files.
  - Boundaries (in/out of scope):
    - **In**: `.github/workflows/release-sce-linux.yml` (line 44: `target_triple`), `.github/workflows/release-sce-linux-arm.yml` (line 44: `target_triple`).
    - **Out**: `release-sce-macos-arm.yml`, `release-sce.yml` orchestrator, any flake.nix changes.
  - **Completed:** 2026-07-02
  - **Files changed:** `.github/workflows/release-sce-linux.yml` (line 44 `target_triple`, line 93 artifact name), `.github/workflows/release-sce-linux-arm.yml` (line 44 `target_triple`, line 93 artifact name)
  - **Evidence:** `workflow-actionlint` passed, all `-gnu` → `-musl` confirmed in both files
  - Done when:
    - `release-sce-linux.yml` references `x86_64-unknown-linux-musl`.
    - `release-sce-linux-arm.yml` references `aarch64-unknown-linux-musl`.
    - `nix flake check` `workflow-actionlint` passes.
  - Verification notes (commands or checks):
    - `nix flake check --print-build-logs` (covers `workflow-actionlint`)

- [x] T03: `Update npm platform mapping and tests for musl triples` (status:done)
  - Task ID: T03
  - Goal: Update the npm launcher's platform-to-target-triple mapping and test assertions to use musl triples.
  - Boundaries (in/out of scope):
    - **In**: `npm/lib/platform.js` (lines 9-11, 13-15), `npm/test/platform.test.js`, `npm/test/install.test.js`.
    - **Out**: npm runtime logic, npm package.json, npm README, npm publish workflow.
  - **Completed:** 2026-07-02
  - **Files changed:** `npm/lib/platform.js` (lines 10, 14: `-linux-gnu` → `-linux-musl`), `npm/test/platform.test.js` (all `-linux-gnu` → `-linux-musl`), `npm/test/install.test.js` (all `-linux-gnu` → `-linux-musl`)
  - **Evidence:** 25/25 bun tests passed, `nix flake check` passed (npm-bun-tests, npm-biome-check, npm-biome-format all green)
  - Done when:
    - `linux:arm64` maps to `aarch64-unknown-linux-musl`.
    - `linux:x64` maps to `x86_64-unknown-linux-musl`.
    - All npm bun tests pass.
  - Verification notes (commands or checks):
    - `nix flake check --print-build-logs` (covers `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`)
    - Or narrow: `nix develop -c sh -c 'cd npm && bun test'`

- [x] T04: `Update context documentation for musl release targets` (status:done)
  - Task ID: T04
  - Goal: Reflect the new musl-based Linux release target triples in all affected context files.
  - Boundaries (in/out of scope):
    - **In**: `context/sce/cli-release-artifact-contract.md` (supported release matrix, line 98-100), `context/sce/cli-npm-distribution-contract.md` (supported npm platforms, lines 21-22), `context/overview.md` (release target reference near line 50), `context/glossary.md` (add `musl static Linux release` entry).
    - **Out**: Plan files, handover files, decisions, architecture.md, patterns.md.
  - Done when:
    - Release artifact contract lists `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` as Linux targets.
    - npm distribution contract lists matching musl triples.
    - Overview references the new triples.
    - Glossary has a `musl static Linux release` entry.
  - Verification notes (commands or checks):
    - Manual review: `grep -r 'linux-gnu' context/` should return no results (except in historical/plan files).
     - `nix run .#pkl-check-generated` (no generated-output drift from context changes)
   - **Completed:** 2026-07-02
   - **Files changed:** `context/sce/cli-release-artifact-contract.md` (5 occurrences: lines 89, 93, 94, 99, 100), `context/sce/cli-npm-distribution-contract.md` (2 occurrences: lines 21, 22), `context/overview.md` (1 occurrence: line 50), `context/glossary.md` (updated `sce split platform release workflows` entry + added `musl static Linux release` entry)
   - **Evidence:** `grep -r 'linux-gnu' context/` returns hits only in plan files + new glossary entry's historical-description text; `nix run .#pkl-check-generated` passed

- [x] T05: `Validation and cleanup` (status:done)
  - Task ID: T05
  - Goal: Run full verification, confirm the musl binary is portable, and sync context.
  - Boundaries (in/out of scope):
    - **In**: `nix flake check`, `nix run .#pkl-check-generated`, binary portability verification, context sync.
    - **Out**: Application code changes, workflow changes beyond what T01-T04 cover.
  - **Completed:** 2026-07-02
  - **Files changed:** `context/plans/musl-static-linux-release.md` (T05 status updated)
  - **Evidence:**
    - `nix run .#pkl-check-generated` — "Generated outputs are up to date." ✅
    - `grep -rn 'linux-gnu' context/sce/ context/cli/ context/overview.md context/architecture.md context/patterns.md` — no stale references ✅
    - `grep -rn 'linux-gnu' .github/workflows/release-sce-linux*.yml` — no stale references ✅
    - `nix build .#default` — build succeeded, `file result/bin/sce` reports "static-pie linked" ✅
    - `readelf -d result/bin/sce | grep RUNPATH/RPATH` — none ✅
    - `nix run .#native-portability-audit -- --platform linux --binary result/bin/sce` — "passed" ✅
    - `nix flake check` — all 15 checks passed (cli-tests, cli-clippy, cli-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format, workflow-actionlint, native-portability-audit, flatpak-static-validation, cargo-sources-parity, flatpak-manifest-parity) ✅
  - Done when:
    - `nix flake check` passes clean.
    - `nix run .#pkl-check-generated` passes.
    - `nix build .#default && nix run .#native-portability-audit -- --platform linux --binary result/bin/sce` passes.
    - `nix flake check` `workflow-actionlint` passes (validates workflow YAML syntax).
    - No stale `linux-gnu` references remain in active context files or workflow code.
  - Verification notes (commands or checks):
    - `nix flake check --print-build-logs`
    - `nix run .#pkl-check-generated`
    - `nix build .#default --print-build-logs`
    - `nix run .#native-portability-audit -- --platform linux --binary result/bin/sce`
    - Full grep for remaining `linux-gnu` in active paths (not historical plans/handovers)

---

---

## Validation Report

### Commands run

| Command | Exit code | Key output |
|---|---|---|
| `nix run .#pkl-check-generated` | 0 | "Generated outputs are up to date." |
| `grep -rn 'linux-gnu' context/sce/ context/cli/ context/overview.md context/architecture.md context/patterns.md` | 0 | No hits in active context files |
| `grep -rn 'linux-gnu' .github/workflows/release-sce-linux*.yml` | 0 | No hits in workflow files |
| `nix build .#default` | 0 | `result/bin/sce` produced |
| `file result/bin/sce` | 0 | "ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, not stripped" |
| `readelf -d result/bin/sce \| grep RUNPATH/RPATH` | 1 | No RUNPATH/RPATH entries |
| `nix run .#native-portability-audit -- --platform linux --binary result/bin/sce` | 0 | "Native binary portability audit passed" |
| `nix flake check` | 0 | All 15 checks passed |

### Checks passed (15/15)

- ✅ `cli-tests`
- ✅ `cli-clippy`
- ✅ `cli-fmt`
- ✅ `pkl-parity`
- ✅ `npm-bun-tests`
- ✅ `npm-biome-check`
- ✅ `npm-biome-format`
- ✅ `config-lib-bun-tests`
- ✅ `config-lib-biome-check`
- ✅ `config-lib-biome-format`
- ✅ `workflow-actionlint`
- ✅ `native-portability-audit`
- ✅ `flatpak-static-validation`
- ✅ `cargo-sources-parity`
- ✅ `flatpak-manifest-parity`

### Success-criteria verification

- [x] **SC1:** `nix build .#default` on Linux produces a fully static `sce` binary with no `/nix/store/` references → confirmed via `file` (static-pie linked), `readelf` (no RUNPATH/RPATH), and native portability audit (passed)
- [x] **SC2:** `nix run .#native-portability-audit -- --platform linux --binary result/bin/sce` passes → confirmed ("Native binary portability audit passed")
- [x] **SC3:** `nix flake check` passes on Linux (all checks) → confirmed (15/15 passed)
- [x] **SC4:** GitHub release archive names change to `*-linux-musl` → confirmed via grep of workflow files (no `linux-gnu` references)
- [x] **SC5:** `npm/lib/platform.js` maps to musl triples → confirmed via context doc check and passing `npm-bun-tests`
- [x] **SC6:** `nix flake check` passes for npm checks → confirmed (`npm-bun-tests`, `npm-biome-check`, `npm-biome-format` all passed)
- [x] **SC7:** Context documentation reflects the new release matrix → confirmed (overview, glossary, release-artifact-contract, npm-distribution-contract all reference musl triples)

### Context sync

- Verify-only pass: all root context files are aligned with musl release target triples. No edits required.

### Residual risks

- None identified. All checks pass, the binary is fully static, and no stale `linux-gnu` references remain.
- The glossary entry for `musl static Linux release` intentionally includes one historical `linux-gnu` reference in its description text — this is expected documentation, not a stale reference.
- Warning: Git tree is dirty with uncommitted plan and context changes; these should be committed before further work.

---

## Open questions

None. All critical details resolved during clarification gate.
