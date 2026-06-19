# Plan: PR CI with Magic Nix Cache

## Change summary

Replace the (already-removed) Tessl-oriented CI with a normal PR validation
workflow that runs on every PR, uses Nix for all validation/build steps, and
uses Determinate Systems' Magic Nix Cache to speed up repeated CI runs. Also
remove the now-dangling README badge that pointed at the deleted
`publish-tiles.yml`.

## Success criteria

- A new workflow at `.github/workflows/pr-ci.yml` exists and:
  - triggers on `pull_request` and `workflow_dispatch`
  - has `permissions: contents: read`
  - runs a matrix on `ubuntu-latest` and `macos-latest` with `fail-fast: false`
  - installs Nix via `DeterminateSystems/nix-installer-action@main`
  - enables `DeterminateSystems/magic-nix-cache-action@main` immediately after
  - executes, in order: `nix flake metadata`, `nix flake check
    --print-build-logs`, `nix build .#default --print-build-logs`, then
    `nix run .#sce -- --help` and `nix run .#sce -- version`
  - job name renders as `Nix CI (ubuntu-latest)` and `Nix CI (macos-latest)`
    so they can be used as required branch protection checks
- README no longer references `publish-tiles.yml` (badge removed).
- No separate Cargo / Bun / npm / `node_modules` caches are added; the Nix
  store cache is the only cache layer.
- `nix flake check` continues to cover the existing check set
  (`cli-tests`, `cli-clippy`, `cli-fmt`, `integrations-install-tests`,
  `integrations-install-clippy`, `integrations-install-fmt`, `pkl-parity`,
  `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`,
  `config-lib-bun-tests`, `config-lib-biome-check`, `config-lib-biome-format`,
  and Linux-only `flatpak-static-validation`).

## Constraints and non-goals

- Constraints
  - Cache step (`magic-nix-cache-action`) must be placed immediately after the
    Nix installer step so all later Nix invocations benefit.
  - Use `actions/checkout@v6`, `DeterminateSystems/nix-installer-action@main`,
    and `DeterminateSystems/magic-nix-cache-action@main` exactly as specified.
  - All validation and build steps must go through Nix; do not shell out to
    `cargo`, `bun`, `npm`, `biome`, etc. directly from the workflow.
- Non-goals
  - Do not publish artifacts, tiles, crates, or npm packages from this
    workflow.
  - Do not modify branch protection rules from inside the repo; the required
    checks (`Nix CI (ubuntu-latest)`, `Nix CI (macos-latest)`) are configured
    by the repo admin out-of-band.
  - Do not touch existing release workflows (`release-sce*.yml`,
    `publish-crates.yml`, `publish-npm.yml`).
  - Do not modify `flake.nix` or any check definitions; this plan only wires
    CI to what already exists.

## Assumptions

- `publish-tiles.yml` was already deleted in commit `2e3ee7e`; only the
  README badge cleanup remains from the original "remove Tessl workflow"
  step.
- `actions/checkout@v6` is the intended pin (taken from the input request as
  authoritative).
- `nix run .#sce -- version` is a valid subcommand (verified in
  `cli/src/command_surface.rs`).

## Task stack

- [x] T01: `Add PR CI workflow with Magic Nix Cache` (status:done)
  - Task ID: T01
  - Goal: Create `.github/workflows/pr-ci.yml` running a Linux+macOS matrix
    that installs Nix, enables Magic Nix Cache, then runs
    `nix flake metadata`, `nix flake check`, `nix build .#default`, and CLI
    smoke tests via `nix run`.
  - Boundaries (in/out of scope): In — adding the new workflow file exactly
    as specified in the change request (triggers, permissions, matrix,
    steps, action pins, job name). Out — modifying any other workflow,
    modifying `flake.nix`, adding non-Nix caches, configuring branch
    protection.
  - Done when: File exists at `.github/workflows/pr-ci.yml` with the
    specified content; `yamllint` / `actionlint` (if available via Nix) is
    clean; opening a PR triggers two jobs named `Nix CI (ubuntu-latest)` and
    `Nix CI (macos-latest)`; on a fresh PR run both jobs reach the
    `Smoke-test CLI` step and `nix run .#sce -- --help` and
    `nix run .#sce -- version` exit 0.
  - Verification notes (commands or checks):
    - `cat .github/workflows/pr-ci.yml` matches spec
    - `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml`
    - locally: `nix flake metadata && nix flake check --print-build-logs &&
      nix build .#default --print-build-logs && nix run .#sce -- --help &&
      nix run .#sce -- version`
    - confirm via GitHub UI after push that both matrix jobs run and pass

- [x] T02: `Remove Tessl publish-tiles badge from README` (status:done)
  - Task ID: T02
  - Goal: Drop the dangling GitHub Actions badge in `README.md:3` that points
    at the deleted `publish-tiles.yml` workflow, so the README no longer
    references Tessl tile publishing.
  - Boundaries (in/out of scope): In — removing the single badge line in
    `README.md`. Out — broader README restructuring, adding a replacement
    "PR CI" badge (left to a follow-up if desired), edits to any other doc.
  - Done when: `grep -ri "tessl\|publish-tiles" README.md` returns nothing;
    README still renders cleanly (no orphan blank lines breaking the title
    block).
  - Verification notes (commands or checks):
    - `grep -rni "tessl\|publish-tiles" README.md` → empty
    - `grep -rni "tessl\|publish-tiles" . --include="*.md" --include="*.yml"`
      → empty
    - eyeball top of README renders correctly in a Markdown preview

- [x] T03: `Validate PR CI plan completion and sync context` (status:done)
  - Task ID: T03
  - Goal: Final validation pass — confirm both prior tasks landed, the new
    workflow exercises the expected flake checks, no rogue Tessl references
    remain, and the plan file is marked complete.
  - Boundaries (in/out of scope): In — running full local Nix validation,
    confirming workflow file content, repo-wide Tessl grep, marking tasks
    complete in this plan, updating any SCE context indices required by
    the repo's plan workflow. Out — implementing new behavior, retrying
    failed checks beyond reporting them.
  - Done when: Local `nix flake check --print-build-logs` and
    `nix build .#default --print-build-logs` pass; CLI smoke tests pass;
    repo-wide grep for `tessl`/`publish-tiles` is empty; all T01–T02
    checkboxes in this file are checked with `status:done`; remaining open
    questions are resolved or moved to follow-ups.
  - Verification notes (commands or checks):
    - `nix flake metadata`
    - `nix flake check --print-build-logs`
    - `nix build .#default --print-build-logs`
    - `nix run .#sce -- --help && nix run .#sce -- version`
    - `grep -rni "tessl\|publish-tiles" . \
        --include="*.md" --include="*.yml" --include="*.toml"` → empty
    - `git status` clean (or only the intended plan-status edits staged)

## Task: T01 Add PR CI workflow with Magic Nix Cache
- **Status:** done
- **Completed:** 2026-06-19
- **Files changed:** `.github/workflows/pr-ci.yml` (new)
- **Evidence:** `nix run nixpkgs#actionlint -- .github/workflows/pr-ci.yml` clean (no diagnostics)
- **Notes:** Cache step placed immediately after Nix installer; matrix on `ubuntu-latest`/`macos-latest` with `fail-fast: false`; job name renders as `Nix CI (<os>)`. Step order matches spec: `nix flake metadata` → `nix flake check --print-build-logs` → `nix build .#default --print-build-logs` → CLI smoke tests.

## Task: T02 Remove Tessl publish-tiles badge from README
- **Status:** done
- **Completed:** 2026-06-19
- **Files changed:** `README.md` (badge line removed)
- **Evidence:** `grep -rni "tessl\|publish-tiles" README.md` → empty; repo-wide grep across `*.md`/`*.yml` finds matches only inside this plan file's self-references.
- **Notes:** Single-line deletion at former README.md:3. Title block still renders cleanly. Replacement PR CI badge intentionally deferred to a follow-up per plan open questions.

## Open questions

- None blocking. Optional follow-up (out of scope for this plan): add a new
  PR CI badge to `README.md` once the workflow has run on `main` at least
  once and a stable badge URL is available.

## Validation Report (T03)

### Commands run
- `nix flake metadata` → exit 0 (flake resolves; inputs: crane, flake-utils, nixpkgs, opencode, rust-overlay, turso).
- `nix flake check --print-build-logs` → exit 0 (`all checks passed!`; warning only about omitted incompatible systems — expected, current system is `x86_64-linux`). Includes `sce-cli-tests-test` → `92 passed; 0 failed`.
- `nix build .#default --print-build-logs` → exit 0 (only `Git tree ... is dirty` warning, expected mid-plan).
- `nix run .#sce -- --help` → exit 0 (banner printed).
- `nix run .#sce -- version` → exit 0 (`shared-context-engineering 0.2.0 (2e3ee7eb09b8)`).
- `grep -rni "tessl\|publish-tiles" . --include="*.md" --include="*.yml" --include="*.toml"` → empty outside the plan file's own self-references (acceptable; plan file retires on completion).

### Success-criteria verification
- [x] `.github/workflows/pr-ci.yml` exists with required triggers, permissions, matrix, action pins, step order, and job-name template → T01 evidence + workflow file unchanged since.
- [x] README no longer references `publish-tiles.yml` → T02 evidence; repo-wide grep clean.
- [x] No Cargo/Bun/npm/`node_modules` caches added → workflow uses only Nix store cache via `magic-nix-cache-action`.
- [x] `nix flake check` covers existing check set → `cli-tests`, `cli-clippy`, `cli-fmt`, `integrations-install-tests`, `integrations-install-clippy`, `integrations-install-fmt`, `pkl-parity`, `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`, `config-lib-bun-tests`, `config-lib-biome-check`, `config-lib-biome-format`, and Linux-only `flatpak-static-validation` all execute under one `nix flake check` invocation (verified via `all checks passed!`).
- [x] All T01–T03 checkboxes marked done with `status:done`.

### Failed checks and follow-ups
- None.

### Residual risks
- Branch protection rules for `Nix CI (ubuntu-latest)` / `Nix CI (macos-latest)` must still be wired by the repo admin out-of-band (explicitly non-goal of this plan).
- macOS matrix leg was not exercised locally (current host is Linux); first PR run on GitHub will be the canonical macOS validation.
