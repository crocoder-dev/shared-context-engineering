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

- [ ] T02: `Fix line-level identity so equivalent hunks intersect correctly` (status:todo)
  - Task ID: T02
  - Goal: Adjust the patch parser/domain/intersection logic so equivalent modifications from `files/2/diff.1` through `files/2/diff.6` resolve to the same line-level change identity and intersect correctly.
  - Boundaries (in/out of scope): In — targeted changes in `cli/src/services/patch.rs`, deterministic reconstruction of the overlap result, and small doc/comment updates needed to reflect the final matching rule. Out — runtime integration, generic fuzzy diff reconciliation, unrelated patch-service refactors.
  - Done when: The regression tests from T01 pass; semantically equivalent hunks intersect to the expected touched-line result; unrelated non-overlap behavior remains covered and unchanged.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests'`; inspect resulting overlap assertions for `files/2/` plus existing non-overlap tests to confirm the fix stays exact and deterministic.

- [ ] T03: `Validation and cleanup` (status:todo)
  - Task ID: T03
  - Goal: Run the repo validation baseline, re-check the success criteria against code truth, and confirm whether any localized patch-service context needs syncing after the fix.
  - Boundaries (in/out of scope): In — full validation, cleanup of temporary regression scaffolding, success-criteria review, and context-sync verification for patch-service docs. Out — additional behavior changes.
  - Done when: `nix run .#pkl-check-generated` passes, `nix flake check` passes, the `files/2/` equivalence contract is verified against the final code, and any required context follow-up is identified.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; compare final code truth with `context/cli/patch-service.md`, `context/context-map.md`, and root shared files for verify-only vs important-change sync.

## Open questions

- None.
