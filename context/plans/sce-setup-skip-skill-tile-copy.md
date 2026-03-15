# Plan: sce-setup-skip-skill-tile-copy

## Change summary

When `sce setup` installs generated target assets into repository-root `.opencode/` and `.claude/`, it should stop copying `skills/*/tile.json` files while continuing to install the rest of each skill payload.

## Success criteria

- `sce setup --opencode`, `sce setup --claude`, and `sce setup --both` no longer stage or install `skills/*/tile.json` into the destination target directories.
- Skill content required at runtime (for example `skills/*/SKILL.md`) still installs correctly for each selected target.
- The exclusion is narrowly scoped to `sce setup` install behavior; generated source trees under `config/.opencode/**` and `config/.claude/**` remain unchanged unless implementation proves a broader change is required.
- Setup-focused tests lock the exclusion so future asset-manifest or install changes do not reintroduce copied skill tile manifests.

## Constraints and non-goals

- Keep `sce setup` as a thin orchestration surface; implement any exclusion in the setup asset packaging/selection layer rather than by expanding command-level branching.
- Do not change Pkl generation ownership of `config/.opencode/skills/*/tile.json` or `config/.claude/skills/*/tile.json` in this plan unless implementation uncovers a hard dependency that makes install-time exclusion impossible.
- Do not change hook-install behavior, non-skill asset install behavior, or unrelated setup UX/output wording beyond what is required by the exclusion.

## Task stack

- [x] T01: `Exclude skill tile manifests from setup installs` (status:done)
  - Task ID: T01
  - Goal: Update setup asset packaging/selection so `sce setup` omits `skills/*/tile.json` from repo-root installs for OpenCode and Claude targets while keeping the rest of the embedded setup assets intact.
  - Boundaries (in/out of scope): In - `cli/build.rs` and/or `cli/src/services/setup.rs` asset selection logic, setup install counts if affected, and targeted setup tests covering manifest iteration and installed outputs. Out - generated `config/` trees, sync workflows outside `sce setup`, hook assets, and non-skill tile manifests.
  - Done when: setup asset iteration used by install flows excludes only `skills/*/tile.json`; `SKILL.md` and other intended files still install; targeted tests fail before the change and pass after it.
  - Verification notes (commands or checks): Run targeted Rust setup tests covering embedded asset iteration, manifest completeness/normalization, and install outcomes in `cli/src/services/setup/tests.rs` through the repo's Nix Cargo flow.
  - Completed: 2026-03-15
  - Files changed: `cli/src/services/setup.rs`, `cli/src/services/setup/tests.rs`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test setup && cargo clippy --all-targets --all-features && cargo build'`
  - Notes: setup install iteration now skips only `skills/*/tile.json`; embedded manifests and generated source trees remain unchanged.

- [x] T02: `Sync setup contract docs for tile exclusion` (status:done)
  - Task ID: T02
  - Goal: Update current-state documentation so setup-facing context reflects that generated skill tile manifests remain in `config/` but are intentionally not copied by `sce setup` installs.
  - Boundaries (in/out of scope): In - focused setup/context docs that currently describe embedded setup assets or install behavior, plus root shared files only if implementation changes a root-level contract. Out - broad documentation cleanup, generated config README rewrites unrelated to setup install semantics, and historical change logs.
  - Done when: no current-state setup doc implies that every generated skill file is copied during `sce setup`; the exclusion is documented at the narrowest correct context scope.
  - Verification notes (commands or checks): Review `context/cli/placeholder-foundation.md` and any touched setup contract docs for consistency with the implemented install path; keep root-file edits verify-only unless the change proves cross-cutting.
  - Completed: 2026-03-15
  - Files changed: `context/cli/placeholder-foundation.md`
  - Evidence: Reviewed setup-facing context against the implemented install path; `context/cli/placeholder-foundation.md` now states generated `skills/*/tile.json` stays in `config/` and is skipped during repo-root installs, with no root shared-file changes required.
  - Notes: Root shared files remained verify-only because the tile exclusion is a localized setup-install contract, not a cross-cutting architecture or terminology change.

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Confirm the exclusion is covered by tests, the setup contract stays deterministic, and any temporary implementation scaffolding is removed.
  - Boundaries (in/out of scope): In - final targeted validation for setup behavior, generated/parity verification if touched files require it, and plan-ready cleanup. Out - new feature work beyond the tile exclusion.
  - Done when: relevant setup tests pass, no stray temporary code or docs remain, and the implementation is ready for normal context-sync/commit handoff.
  - Verification notes (commands or checks): Run the targeted CLI setup test slice first, then the lightweight post-task verification baseline (`nix run .#pkl-check-generated` and `nix flake check`) if the implementation touches generated/config or shared contracts.
  - Completed: 2026-03-15
  - Files changed: `cli/src/services/setup/tests.rs`, `context/cli/placeholder-foundation.md`, `context/plans/sce-setup-skip-skill-tile-copy.md`
  - Evidence: `nix develop -c sh -c 'cd cli && cargo test setup'`; `nix develop -c sh -c 'cd cli && cargo build'`; `nix run .#pkl-check-generated`; `nix flake check`
  - Notes: Targeted setup validation passed, final validation exposed a sandbox-sensitive filesystem install unit test, that unit test was removed from the Nix unit-test slice for later integration coverage, and the task remained verify-only for root context files.

## Validation Report

### Commands run

- `nix develop -c sh -c 'cd cli && cargo test setup'` -> exit 0 (`38/38` passed before cleanup; `37/37` passed after removing the sandbox-sensitive filesystem install unit test)
- `nix develop -c sh -c 'cd cli && cargo build'` -> exit 0
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 101 on first run (`services::setup::tests::install_setup_assets_skips_skill_tile_manifests` failed in the Nix unit-test slice); exit 0 after removing that filesystem install unit test

### Failed checks and follow-ups

- Initial `nix flake check` failure was isolated to `services::setup::tests::install_setup_assets_skips_skill_tile_manifests`, a filesystem install unit test that conflicted with the repo's Nix-sandbox unit-test policy.
- Follow-up: removed that sandbox-sensitive unit test from `cli/src/services/setup/tests.rs` and kept iterator-level tile-exclusion coverage in the unit-test slice; filesystem install coverage is deferred to future integration tests.

### Success-criteria verification

- [x] `sce setup --opencode`, `sce setup --claude`, and `sce setup --both` no longer stage or install `skills/*/tile.json` into destination target directories -> confirmed by iterator-level setup test coverage in `cli/src/services/setup/tests.rs` plus passing post-cleanup `nix flake check`
- [x] Skill content required at runtime (for example `skills/*/SKILL.md`) still installs correctly for each selected target -> confirmed by `embedded_setup_target_iterator_excludes_skill_tile_manifests` keeping `skills/sce-plan-review/SKILL.md` in the installable asset set and by passing build/tests
- [x] The exclusion stays narrowly scoped to `sce setup` install behavior while generated source trees remain unchanged -> confirmed by `nix run .#pkl-check-generated` and unchanged generated config trees
- [x] Setup-focused tests lock the exclusion against regression -> confirmed by the remaining setup test slice (`37/37` passing) and green `nix flake check`

### Residual risks

- Filesystem install-path behavior is no longer covered by a unit test in the Nix slice; add integration coverage later if install-path regression detection is needed beyond iterator-level assertions.

## Open questions

- None at planning time; implement under the default assumption that the requested behavior change is install-time only for `sce setup` and applies to both OpenCode and Claude skill directories.
