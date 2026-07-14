# Plan: Simplify and Optimize flake.nix

## Change summary

Simplify `flake.nix` while improving local and CI build performance. The work
must prioritize faster Rust/package builds, faster `nix flake check`, and faster
PR CI without removing existing checks or package behavior unless a task uncovers
a specific candidate removal and stops for user approval before making it.

## Success criteria

- `flake.nix` is easier to maintain: repeated check/build patterns are factored
  into small local helpers or moved to narrowly scoped Nix modules where that
  reduces noise without changing public outputs.
- `nix flake check` still passes and covers the same public check names unless
  the user explicitly approves a removal in a later session.
- Local validation shows `nix flake check` is faster after the optimization work
  than the captured baseline for comparable warm/cold-cache conditions.
- CI remains Nix-based and runs faster by avoiding avoidable duplicate work,
  over-broad inputs, or needless rebuild invalidation.
- Release-facing outputs (`packages.default`, `packages.sce`, release apps,
  Flatpak helpers, npm release helpers) preserve behavior unless explicitly
  approved otherwise.

## Constraints and non-goals

- Constraints
  - Use Nix as the validation entrypoint; prefer `nix flake check` for final
    verification.
  - Preserve public flake output names and check names by default.
  - Keep stdout/stderr contracts of existing helper apps stable.
  - Keep changes atomic: each task should be landable as one coherent commit.
  - If implementation reveals that meaningful speedups require removing a
    check, package, release helper, platform, or CI step, stop and ask before
    removing it.
- Non-goals
  - Do not rewrite the Rust CLI, npm package, Flatpak packaging, or release
    workflows except where needed to consume simplified flake outputs.
  - Do not add non-Nix dependency caches to CI.
  - Do not change release artifact portability guarantees.
  - Do not pursue risky dependency upgrades as part of this plan.

## Assumptions

- "Faster" means measured wall-clock improvement for comparable local runs and
  expected CI critical-path reduction, not just aesthetic refactoring.
- Behavior preservation is preferred over maximum speed; removals or semantic
  changes need explicit user confirmation during implementation.
- The current untracked `.pi/extensions/` worktree entry is unrelated and should
  not be touched by this plan.

## Task stack

- [x] T01: `Add flake performance baseline notes` (status:done)
  - Task ID: T01
  - Goal: Capture a repeatable baseline for the current `flake.nix` check/build
    performance before changing behavior.
  - Boundaries (in/out of scope): In — record current output graph, check names,
    CI command sequence, and representative timings for `nix flake check` and
    `nix build .#default` in the plan or a small context note. Out — changing
    `flake.nix`, removing checks, or modifying CI.
  - Done when: Baseline evidence documents the command(s), host/platform,
    cache condition as best known, elapsed time, and current public check list;
    any existing unrelated worktree changes are noted but untouched.
  - Verification notes (commands or checks): `nix flake show`; `time nix flake
    check --print-build-logs`; optionally `time nix build .#default
    --print-build-logs` if practical.
  - Completed: 2026-07-14
  - Files changed: `context/plans/simplify-optimize-flake-nix.md`
  - Evidence:
    - Host/platform: `Linux nixos 6.18.23`, Nix `builtins.currentSystem = x86_64-linux`.
    - Worktree before implementation: `git status --short` showed only
      `A  context/plans/simplify-optimize-flake-nix.md`; the plan file was the
      selected active artifact and no unrelated worktree changes were touched.
      The plan assumption still notes unrelated untracked `.pi/extensions/`
      work outside this task, but it was not present in this status snapshot.
    - Current `x86_64-linux` public check names from `nix flake show --all-systems`:
      `cargo-sources-parity`, `cli-clippy`, `cli-fmt`, `cli-tests`,
      `config-lib-biome-check`, `config-lib-biome-format`,
      `config-lib-bun-tests`, `flatpak-manifest-parity`,
      `flatpak-static-validation`, `native-portability-audit`,
      `npm-biome-check`, `npm-biome-format`, `npm-bun-tests`, `pkl-parity`,
      `workflow-actionlint`.
    - Current `x86_64-linux` app names include `default`, `sce`,
      `native-portability-audit`, `pkl-check-generated`, `pkl-generate`,
      `release-artifacts`, `release-manifest`, `release-npm-package`,
      `bump-version`, `sce-flatpak`, `release-flatpak-package`,
      `release-flatpak-bundle`, `flatpak-static-check`,
      `flatpak-version-parity-check`, `flatpak-local-manifest-check`,
      `regenerate-cargo-sources`, and `regenerate-flatpak-manifest`.
    - Current `x86_64-linux` package names from flake show: `default`, `sce`,
      `bun`, `turso`.
    - CI command sequence from `.github/workflows/pr-ci.yml`: ubuntu/macOS
      matrix installs Nix, enables Magic Nix Cache, runs
      `nix flake check --print-build-logs`, then
      `nix build .#default --print-build-logs`, then smoke tests
      `nix run .#sce -- --help` and `nix run .#sce -- version`.
    - Baseline command: `TIMEFORMAT=...; time nix flake check --print-build-logs`.
      Result: passed. Elapsed: `real 30.670`, `user 3.440`, `sys 0.638`.
      Cache condition: warm/partially cached; Nix reported only 4 flake checks
      needed builds during this dirty-tree run, with other checks already cached.
    - Baseline command: `TIMEFORMAT=...; time nix build .#default --print-build-logs`.
      Result: passed. Elapsed: `real 116.440`, `user 1.066`, `sys 0.301`.
      Cache condition: partially cold for the package path; Nix built
      `sce-deps-0.3.0` and `sce-0.3.0`, with the deps derivation spending about
      1m41s in build phase and final package compile about 8.14s.
  - Notes: No `flake.nix`, package behavior, check definitions, or CI workflow
    behavior were changed in T01.

- [x] T02: `Narrow cheap check inputs to reduce invalidation` (status:done)
  - Task ID: T02
  - Goal: Make cheap checks such as Rust formatting and script/static checks use
    the smallest necessary source sets so unrelated config/release asset changes
    do not trigger avoidable rebuilds.
  - Boundaries (in/out of scope): In — adjust filesets/src arguments for checks
    that do not need the full packaged workspace; keep check names and behavior
    the same. Out — changing test semantics, skipping checks, or changing
    release package source contents.
  - Done when: Targeted checks still pass, public check names remain stable, and
    at least one over-broad source dependency is removed from a check derivation.
  - Verification notes (commands or checks): `nix flake check --print-build-logs`;
    inspect `nix flake show` check names before/after.
  - Completed: 2026-07-14
  - Files changed: `flake.nix`, `context/plans/simplify-optimize-flake-nix.md`
  - Evidence:
    - `cli-fmt` now uses the existing `cargoDepsArgs` source rooted at `cli/`
      instead of `commonCargoArgs`, removing generated config/assets and other
      package-only workspace inputs from the Rust formatting check.
    - `pkl-parity` now copies a narrow `pklParitySrc` containing Pkl authoring
      inputs, the generated output paths it compares, and the three plugin
      source files read by `generate.pkl`, instead of copying the entire repo.
    - Public `x86_64-linux` check names from `nix flake show --all-systems --json` remain:
      `cargo-sources-parity`, `cli-clippy`, `cli-fmt`, `cli-tests`,
      `config-lib-biome-check`, `config-lib-biome-format`,
      `config-lib-bun-tests`, `flatpak-manifest-parity`,
      `flatpak-static-validation`, `native-portability-audit`,
      `npm-biome-check`, `npm-biome-format`, `npm-bun-tests`, `pkl-parity`,
      `workflow-actionlint`.
    - `nix build .#checks.x86_64-linux.cli-fmt --print-build-logs` passed.
    - `nix build .#checks.x86_64-linux.pkl-parity --print-build-logs` passed.
    - `nix flake check --print-build-logs` passed on `x86_64-linux`.
  - Notes: No public check names, test semantics, release package source
    contents, or CI workflow behavior were changed in T02.

- [x] T03: `Deduplicate repeated flake check helpers` (status:done)
  - Task ID: T03
  - Goal: Simplify repeated `runCommand` and formatter/test check definitions
    into small reusable helpers without changing public outputs.
  - Boundaries (in/out of scope): In — local helper functions or small Nix module
    extraction for repeated Bun/Biome/npm/config check patterns. Out — combining
    or deleting public checks, changing tool versions, or changing commands each
    check runs.
  - Done when: `flake.nix` has less duplicated check boilerplate, the existing
    check attr names still exist, and the generated derivations run the same
    underlying commands.
  - Verification notes (commands or checks): `nix flake check --print-build-logs`;
    `nix flake show` to confirm check attr names.
  - Completed: 2026-07-14
  - Files changed: `flake.nix`, `context/plans/simplify-optimize-flake-nix.md`
  - Evidence:
    - Added local `mkCopiedSourceCheck`, `mkBunCheck`, and `mkBiomeCheck`
      helpers to factor repeated source-copy, chmod, workdir, Bun test, and
      Biome check boilerplate while preserving each check's public attr name.
    - Existing config-lib and npm checks still run the same underlying commands:
      `bun test`, `bun test ./test/*.test.js`,
      `biome check --formatter-enabled=false .`, and
      `biome check --linter-enabled=false .` in their previous workdirs.
    - Public `x86_64-linux` check names from `nix flake show --all-systems --json` remain:
      `cargo-sources-parity`, `cli-clippy`, `cli-fmt`, `cli-tests`,
      `config-lib-biome-check`, `config-lib-biome-format`,
      `config-lib-bun-tests`, `flatpak-manifest-parity`,
      `flatpak-static-validation`, `native-portability-audit`,
      `npm-biome-check`, `npm-biome-format`, `npm-bun-tests`, `pkl-parity`,
      `workflow-actionlint`.
    - Targeted JS/config-lib check build passed:
      `nix build .#checks.x86_64-linux.config-lib-bun-tests
      .#checks.x86_64-linux.config-lib-biome-check
      .#checks.x86_64-linux.config-lib-biome-format
      .#checks.x86_64-linux.npm-bun-tests
      .#checks.x86_64-linux.npm-biome-check
      .#checks.x86_64-linux.npm-biome-format --print-build-logs`.
    - `nix flake check --print-build-logs` passed on `x86_64-linux`.
  - Notes: No public check names, tool versions, check commands, CI workflow
    behavior, or release/package outputs were changed in T03.

- [x] T04: `Reduce Rust build duplication where behavior is preserved` (status:done)
  - Task ID: T04
  - Goal: Improve Rust build/check reuse across `cli-tests`, `cli-clippy`, and
    packages by sharing compatible Crane args/artifacts and avoiding redundant
    setup only where it does not change generated assets or release behavior.
  - Boundaries (in/out of scope): In — behavior-preserving Crane argument
    cleanup, shared helpers, and source partitioning between check-only and
    package/release builds. Out — changing default package target, removing musl
    release builds, dropping generated config assets, or disabling clippy/test
    coverage without explicit approval.
  - Done when: Rust check/package definitions are simpler, compatible derivation
    inputs are shared intentionally, and `nix flake check` plus `nix build
    .#default` pass.
  - Verification notes (commands or checks): `nix flake check --print-build-logs`;
    `nix build .#default --print-build-logs`; compare timings to T01 baseline.
  - Completed: 2026-07-14
  - Files changed: `flake.nix`, `context/plans/simplify-optimize-flake-nix.md`,
    `context/architecture.md`, `context/patterns.md`
  - Evidence:
    - Added `generatedConfigFileset` and narrowed the Rust package/check
      `workspaceSrc` from all of `config/` to the generated config trees and
      schema files actually copied into `cli/assets/generated/config/` during
      Crane package/check builds.
    - Added shared `cargoBaseArgs` for host Rust Crane derivations so dependency,
      package, test, clippy, and fmt definitions intentionally reuse common
      Cargo metadata, lockfile, strict-deps, check, and toolchain settings.
    - Kept host `cargoArtifacts` and Linux musl `cargoArtifactsMusl` separate;
      no default package target, musl release behavior, generated assets, or
      clippy/test coverage was changed.
    - Public `x86_64-linux` check names from `nix flake show --all-systems --json` remain:
      `cargo-sources-parity`, `cli-clippy`, `cli-fmt`, `cli-tests`,
      `config-lib-biome-check`, `config-lib-biome-format`,
      `config-lib-bun-tests`, `flatpak-manifest-parity`,
      `flatpak-static-validation`, `native-portability-audit`,
      `npm-biome-check`, `npm-biome-format`, `npm-bun-tests`, `pkl-parity`,
      `workflow-actionlint`.
    - `nix flake check --print-build-logs` passed on `x86_64-linux`; elapsed
      `real 27.479`, `user 3.476`, `sys 0.702`, compared to T01 warm baseline
      `real 30.670`.
    - `nix build .#default --print-build-logs` passed; elapsed `real 14.660`,
      `user 1.062`, `sys 0.255`, compared to T01 partially-cold baseline
      `real 116.440` (this T04 run reused the musl dependency artifact and only
      rebuilt the final package derivation).
    - Required context sync classified this as an important root flake contract
      update; `context/architecture.md` and `context/patterns.md` now document
      narrowed Rust package/check source ownership. `nix run .#pkl-check-generated`
      passed with generated outputs up to date.
  - Notes: Optimization is behavior-preserving source narrowing and Crane-arg
    cleanup only; no public outputs, release helpers, CI workflow behavior, or
    validation coverage were removed.

- [x] T05: `Trim PR CI duplicate work without dropping validation` (status:done)
  - Task ID: T05
  - Goal: Make PR CI faster while preserving Nix-based validation and the same
    effective coverage.
  - Boundaries (in/out of scope): In — reorder, group, or parameterize existing
    CI Nix commands to reduce duplicate evaluation/build work and expose timing
    evidence. Out — removing the default-package build, platform matrix entries,
    or smoke tests unless implementation first obtains explicit user approval.
  - Done when: `.github/workflows/pr-ci.yml` still validates via Nix, keeps the
    intended coverage, and should have a shorter critical path or clearer timing
    data than before.
  - Verification notes (commands or checks): `nix run nixpkgs#actionlint --
    .github/workflows/pr-ci.yml`; local `nix flake check --print-build-logs`;
    compare GitHub Actions timings after the next PR run.
  - Completed: 2026-07-14
  - Files changed: `.github/workflows/pr-ci.yml`, `context/overview.md`,
    `context/patterns.md`, `context/plans/simplify-optimize-flake-nix.md`
  - Evidence:
    - PR CI still runs on the same `ubuntu-latest` + `macos-latest` matrix and
      still uses Nix for validation.
    - `nix flake check --print-build-logs` remains the first validation command;
      it is now wrapped with shell `time` so GitHub logs expose elapsed timing.
    - The default package build remains present and now writes the default
      package result to `result` with `nix build .#default --out-link result
      --print-build-logs`, also wrapped with shell `time` for CI timing data.
    - CLI smoke coverage is preserved by executing the already-built binary at
      `./result/bin/sce --help` and `./result/bin/sce version`, avoiding the two
      extra `nix run .#sce` evaluations/build-plan resolutions from the prior
      workflow while smoke-testing the same packaged `sce` binary produced by
      the default package build.
    - `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` passed.
    - `nix flake check --print-build-logs` passed on `x86_64-linux`; elapsed
      `real 6.865`, `user 3.493`, `sys 0.685` with cached checks except the
      workflow actionlint derivation rebuilt for the changed workflow.
    - Local build/smoke command passed:
      `nix build .#default --out-link result --print-build-logs &&
      ./result/bin/sce --help && ./result/bin/sce version`; elapsed build time
      `real 1.664`, `user 1.018`, `sys 0.272` with cached package output.
    - Required context sync classified this as an important CI validation
      contract update; `context/overview.md` and `context/patterns.md` now
      document the timed flake-check/package-build gates plus already-built
      binary smoke-test flow. `nix run .#pkl-check-generated` passed.
  - Notes: No checks, package builds, matrix entries, or smoke-test commands were
    removed; the smoke tests now consume the already-built default package output
    instead of invoking separate `nix run` commands.

- [x] T06: `Validate optimized flake and sync context` (status:done)
  - Task ID: T06
  - Goal: Run final validation, compare performance against the baseline, and
    update plan/context evidence.
  - Boundaries (in/out of scope): In — full checks, package build, CI/workflow
    lint, timing comparison, and context/plan status updates. Out — adding new
    optimizations beyond reporting follow-ups.
  - Done when: `nix flake check` passes, `nix build .#default` passes, PR CI
    workflow lint passes if touched, public flake outputs are preserved, measured
    local timing is faster than the T01 baseline or any shortfall is explained,
    and this plan records validation evidence.
  - Verification notes (commands or checks): `nix flake check --print-build-logs`;
    `nix build .#default --print-build-logs`; `nix flake show`; `nix run
    nixpkgs#actionlint -- .github/workflows/pr-ci.yml` if CI changed; `git diff
    --stat` and `git status --short`.
  - Completed: 2026-07-14
  - Files changed: `context/plans/simplify-optimize-flake-nix.md`
  - Evidence:
    - `nix flake check --print-build-logs` passed on `x86_64-linux`; elapsed
      `real 6.070`, `user 3.371`, `sys 0.640` with cached checks and zero flake
      checks rebuilt during this final validation run.
    - `nix build .#default --out-link result --print-build-logs` passed; elapsed
      `real 1.592`, `user 0.984`, `sys 0.241` with the default package output
      cached.
    - `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` passed;
      elapsed `real 0.165`, `user 0.036`, `sys 0.026`.
    - `nix run .#pkl-check-generated` passed; generated outputs are up to date;
      elapsed `real 8.094`, `user 6.195`, `sys 1.149`.
    - Public `x86_64-linux` check names from `nix flake show --all-systems --json` remain:
      `cargo-sources-parity`, `cli-clippy`, `cli-fmt`, `cli-tests`,
      `config-lib-biome-check`, `config-lib-biome-format`,
      `config-lib-bun-tests`, `flatpak-manifest-parity`,
      `flatpak-static-validation`, `native-portability-audit`,
      `npm-biome-check`, `npm-biome-format`, `npm-bun-tests`, `pkl-parity`,
      `workflow-actionlint`.
    - Public `x86_64-linux` app names from `nix flake show --all-systems --json` remain:
      `default`, `sce`, `native-portability-audit`, `pkl-check-generated`,
      `pkl-generate`, `release-artifacts`, `release-manifest`,
      `release-npm-package`, `bump-version`, `sce-flatpak`,
      `release-flatpak-package`, `release-flatpak-bundle`,
      `flatpak-static-check`, `flatpak-version-parity-check`,
      `flatpak-local-manifest-check`, `regenerate-cargo-sources`, and
      `regenerate-flatpak-manifest`.
    - Public `x86_64-linux` package names remain: `default`, `sce`, `bun`, and
      `turso`.
    - Final local timing is faster than the T01 comparable warm/partially cached
      baseline: `nix flake check` improved from `real 30.670` to `real 6.070`;
      `nix build .#default` improved from `real 116.440` to `real 1.592` in this
      final cached run. The T04 package-build comparison remains the more
      representative post-optimization rebuild evidence (`real 14.660`) because
      it rebuilt the final package derivation rather than reusing a fully cached
      result.
    - Final `git status --short` showed the expected plan/worktree changes only:
      `.github/workflows/pr-ci.yml`, `context/architecture.md`,
      `context/overview.md`, `context/patterns.md`,
      `context/plans/simplify-optimize-flake-nix.md`, and `flake.nix`.
  - Notes: T06 performed validation and evidence capture only; no new flake,
    package, check, release, or CI behavior changes were introduced. Context
    sync classified T06 itself as verify-only because T02-T05 already updated
    root context for the behavior changes; final sync verified
    `context/overview.md`, `context/architecture.md`, `context/patterns.md`,
    `context/glossary.md`, and `context/context-map.md` against code truth.

## Validation Report

### Commands run

- `nix flake check --print-build-logs` -> exit 0; all checks passed; elapsed
  `real 6.070`, `user 3.371`, `sys 0.640`.
- `nix build .#default --out-link result --print-build-logs` -> exit 0;
  default package output available at `result`; elapsed `real 1.592`,
  `user 0.984`, `sys 0.241`.
- `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` -> exit 0;
  elapsed `real 0.165`, `user 0.036`, `sys 0.026`.
- `nix run .#pkl-check-generated` -> exit 0; generated outputs are up to date;
  elapsed `real 8.094`, `user 6.195`, `sys 1.149`.
- `nix flake show --all-systems --json` -> exit 0; public `x86_64-linux`
  checks, apps, and packages remain present as recorded in T06 evidence.
- `git diff --stat --cached` / `git diff --stat HEAD` -> expected plan stack
  changes only: `.github/workflows/pr-ci.yml`, `flake.nix`,
  `context/architecture.md`, `context/overview.md`, `context/patterns.md`, and
  `context/plans/simplify-optimize-flake-nix.md`.
- `git status --short --untracked-files=all` -> expected staged/modified plan
  stack files only; T06 scratch logs are under ignored `context/tmp/`.

### Success-criteria verification

- [x] `flake.nix` is easier to maintain: T03 factored repeated JS/check
  derivations into helpers, and T04 factored shared Crane args/source sets.
- [x] `nix flake check` still passes and public check names are preserved:
  confirmed by final `nix flake check` and `nix flake show --all-systems --json`.
- [x] Local validation is faster than the T01 baseline under comparable cached
  conditions: final `nix flake check` `real 6.070` vs T01 `real 30.670`; final
  cached package build `real 1.592` vs T01 `real 116.440`, with T04's `real
  14.660` retained as the more representative package-rebuild comparison.
- [x] PR CI remains Nix-based and avoids duplicate smoke-test work: T05 changed
  smoke tests to use `./result/bin/sce` from the already-built default package.
- [x] Release-facing outputs preserve behavior: public package/app names remain
  present, including `packages.default`, `packages.sce`, release apps, Flatpak
  helpers, and npm release helpers.

### Failed checks and follow-ups

- None.

### Residual risks

- Final local package-build timing was fully cached; compare GitHub Actions
  timing after the next PR run for end-to-end CI critical-path evidence.

## Open questions

- None blocking. During execution, stop for user approval before removing or
  semantically weakening any check, package, platform, release helper, or CI
  validation step.
