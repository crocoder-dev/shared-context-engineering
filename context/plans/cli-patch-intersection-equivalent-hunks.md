# Plan: CLI Patch Intersection Equivalent Hunks

## Change summary

Update the standalone patch service in `cli/src/services/patch.rs` so patch intersection treats the equivalent modifications in `files/2/diff.1` through `files/2/diff.6` as the same change even when their hunk headers and surrounding context windows differ. The work should pin the regression with fixture-backed coverage, then refine the parsed-patch comparison/intersection logic so equivalence is derived from per-line change identity rather than broader hunk-shape differences.

## Success criteria

1. Parsing and intersection behavior is explicitly covered for the fixture family in `files/2/`, and the tests prove those six diffs represent the same logical change.
2. `intersect_patches` returns the expected overlap for semantically equivalent hunks even when the source diffs use different hunk header ranges or different amounts of surrounding context.
3. Any parser or domain-model changes needed to support that behavior remain scoped to the standalone patch service in `cli/src/services/patch.rs`.
4. Existing exact-match behavior for unrelated files or genuinely different touched lines remains intact.
5. Repository validation continues to pass after the fix.

## Constraints and non-goals

- **In scope**: fixture-backed regression coverage for `files/2/`, targeted parser/model/intersection changes needed to make equivalent hunks intersect correctly, and concise docs/comments clarifying the matching contract if code truth changes.
- **In scope**: small internal refactors inside the patch service if they are required to express per-line change identity clearly and deterministically.
- **Out of scope**: wiring the patch service into CLI command dispatch, hooks, sync flows, or external storage.
- **Out of scope**: fuzzy patch similarity matching beyond the equivalence demonstrated by `files/2/`.
- **Non-goal**: redesigning the full patch domain beyond what is necessary for correct line-level intersection.
- **Assumption**: the intended fix is that all `files/2/diff.x` fixtures should produce the same intersection result because they encode the same two removed lines and two added lines for the same file, despite differing hunk metadata/context windows.
- **Assumption**: if current parsed data is insufficient to prove that equivalence, the implementation may refine the in-memory representation or intersection key so long as the change stays local to the patch service and remains deterministic.

## Task stack

- [x] T01: `Pin equivalent-hunk intersection regression with fixture-backed tests` (status:done)
  - Task ID: T01
  - Goal: Add focused tests that parse the `files/2/diff.x` fixtures and demonstrate the current incorrect intersection/equivalence behavior in a deterministic, reviewable way.
  - Boundaries (in/out of scope): In — regression tests for parse/intersection behavior using `files/2/`, explicit assertions about expected shared logical changes, and any minimal test helpers needed inside `patch.rs` tests. Out — implementation changes to fix the bug, unrelated parser cleanup.
  - Done when: The test suite contains targeted coverage showing that the six `files/2/` fixtures represent the same logical change and exposing the current mismatch in intersection or parsed line identity.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests'`; review assertions to confirm they express the `files/2/` equivalence contract rather than broad fuzzy matching.
  - Completed: 2026-04-20
  - Files changed: `cli/src/services/patch.rs`
  - Evidence: added fixture-backed tests proving `diff.1`..`diff.6` share the same touched-line signature; regression coverage shows `diff.1` vs `diff.2` currently fails exact intersection because file identity differs; `nix flake check` passed.
  - Context sync: verify-only expected; localized patch-service behavior unchanged outside test coverage.

- [x] T02: `Fix line-level identity so equivalent hunks intersect correctly` (status:done)
  - Task ID: T02
  - Goal: Adjust the patch parser/domain/intersection logic so equivalent modifications from `files/2/diff.1` through `files/2/diff.6` resolve to the same line-level change identity and intersect correctly.
  - Boundaries (in/out of scope): In — targeted changes in `cli/src/services/patch.rs`, deterministic reconstruction of the overlap result, and small doc/comment updates needed to reflect the final matching rule. Out — runtime integration, generic fuzzy diff reconciliation, unrelated patch-service refactors.
  - Done when: The regression tests from T01 pass; semantically equivalent hunks intersect to the expected touched-line result; unrelated non-overlap behavior remains covered and unchanged.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests'`; inspect resulting overlap assertions for `files/2/` plus existing non-overlap tests to confirm the fix stays exact and deterministic.
  - Completed: 2026-04-20
  - Files changed: `cli/src/services/patch.rs`, `context/cli/patch-service.md`, `context/context-map.md`, `context/glossary.md`
  - Evidence: `intersect_patches` now matches absolute-vs-relative post-change paths by normalized suffix segments, fixture-backed equivalent-hunk regression now returns full overlap for `diff.1` vs `diff.2`, added boundary tests for suffix-only path equivalence, and `nix flake check` passed.
  - Context sync: localized patch-service contract updated in `context/cli/patch-service.md`; discoverability/term references refreshed in `context/context-map.md` and `context/glossary.md`.

- [x] T03: `Validation and cleanup` (status:done)
  - Task ID: T03
  - Goal: Run the repo validation baseline, re-check the success criteria against code truth, and confirm whether any localized patch-service context needs syncing after the fix.
  - Boundaries (in/out of scope): In — full validation, cleanup of temporary regression scaffolding, success-criteria review, and context-sync verification for patch-service docs. Out — additional behavior changes.
  - Done when: `nix run .#pkl-check-generated` passes, `nix flake check` passes, the `files/2/` equivalence contract is verified against the final code, and any required context follow-up is identified.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; compare final code truth with `context/cli/patch-service.md`, `context/context-map.md`, and root shared files for verify-only vs important-change sync.
  - Completed: 2026-04-20
  - Files changed: `context/plans/cli-patch-intersection-equivalent-hunks.md`
  - Evidence: see Validation Report below.
  - Context sync: verify-only; root shared files (`overview.md`, `architecture.md`, `glossary.md`, `patterns.md`) confirmed aligned with code truth; domain file `context/cli/patch-service.md` already updated in T02; no root edits required.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` → exit 0 ("Generated outputs are up to date.")
- `nix flake check` → exit 0 (all checks passed: cli-tests, cli-clippy, cli-fmt, integrations-install-tests, integrations-install-clippy, integrations-install-fmt, pkl-parity, npm-bun-tests, npm-biome-check, npm-biome-format, config-lib-bun-tests, config-lib-biome-check, config-lib-biome-format)

### Success-criteria verification
- [x] SC1: Parsing and intersection behavior is explicitly covered for the `files/2/` fixture family, and tests prove those six diffs represent the same logical change — confirmed via inline fixture tests in `cli/src/services/patch.rs` (`parse_index_style_modified_file_with_removed_lines`, `parse_git_style_modified_file`, `parse_index_style_modified_with_full_context`, plus intersection tests `equivalent_hunks_intersect_across_absolute_and_relative_paths`, `equivalent_hunks_intersect_across_different_context_windows`, `path_identity_matches_absolute_path_suffixes_only_on_segment_boundaries`).
- [x] SC2: `intersect_patches` returns the expected overlap for semantically equivalent hunks even when source diffs use different hunk header ranges or different amounts of surrounding context — confirmed; `intersect_patches` uses `paths_refer_to_same_file` for suffix-based path equivalence and touched-line identity (`kind` + `line_number` + `content`) for line matching, independent of hunk metadata.
- [x] SC3: Parser/domain/intersection changes remain scoped to `cli/src/services/patch.rs` — confirmed; no changes outside the patch service module.
- [x] SC4: Existing exact-match behavior for unrelated files or genuinely different touched lines remains intact — confirmed via existing non-overlap tests (`intersect_patches_returns_empty_for_non_overlapping_patches`, `intersect_patches_excludes_files_with_no_overlapping_lines`, `intersect_patches_preserves_only_overlapping_lines_in_multi_file_patches`).
- [x] SC5: Repository validation continues to pass — confirmed; `nix run .#pkl-check-generated` and `nix flake check` both exit 0.

### Context verification
- `context/cli/patch-service.md`: aligned with code truth (intersection section covers suffix-path equivalence and equivalent-hunk behavior).
- `context/context-map.md`: patch-service entry references current intersection/combination behavior.
- `context/glossary.md`: `intersect_patches` entry covers path-suffix equivalence at the public API level; private helpers are implementation details that don't need separate glossary entries.
- `context/overview.md`, `context/architecture.md`, `context/patterns.md`: verify-only; no root-level behavior, architecture, or terminology changes from this plan.

### Temporary scaffolding
- Untracked development artifacts (`files/`, `poem.md`, `poem-2.md`) identified but left in place per user preference; they do not affect validation or runtime behavior.

### Residual risks
- None identified.

## Open questions

- None.
