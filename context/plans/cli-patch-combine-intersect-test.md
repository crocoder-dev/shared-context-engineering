# Plan: CLI Patch Combine-Intersect Integration Test

## Change summary

Add an integration-style test to `cli/src/services/patch.rs` that proves the `combine_patches` → `intersect_patches` pipeline produces the correct result when fed real incremental patch data. The test extracts only the `diff` field from each `message.part.updated` JSON file in `hunk-files/`, fixes the file paths in those diffs to match the post-commit format (relative paths like `hunks/fib.ts` instead of absolute paths like `/home/ssv/Projects/crocoder/shared-context-engineering/hunks/fib.ts`), inlines those diffs as string constants, combines them with `combine_patches`, parses the `post-commit` `head_patch_from_git` diff, intersects the combined result with the post-commit patch, and asserts the intersection equals the post-commit patch.

## Success criteria

1. A new `#[cfg(test)] mod tests` block (or appended tests) in `cli/src/services/patch.rs` contains a test that parses all seven `message.part.updated` diffs (with corrected file paths), combines them with `combine_patches`, parses the `post-commit` `head_patch_from_git` diff, intersects the combined result with the post-commit patch, and asserts the intersection equals the post-commit patch.
2. Only the `diff` strings from `metadata.files[].diff` are inlined — no JSON envelope content, no `before`/`after`/`additions`/`deletions` fields, no whole-file content.
3. File paths in the inlined incremental diffs are corrected to use relative paths matching the post-commit format (e.g., `hunks/fib.ts`, `hunks/optimized.ts`) so that `intersect_patches` can match files across the combined and post-commit patches.
4. The test compiles and passes under `nix flake check` (which runs `cli-tests`).
5. Existing tests and behavior in `patch.rs` remain unchanged.
6. Repository validation (`nix run .#pkl-check-generated` and `nix flake check`) continues to pass.

## Constraints and non-goals

- **In scope**: One new test function (or a small test module) in `cli/src/services/patch.rs` with inline diff-string constants that exercises `parse_patch`, `combine_patches`, and `intersect_patches` end-to-end.
- **In scope**: Minimal test helper constants/functions for readability if needed.
- **In scope**: Correcting file paths in the inlined diff strings so `intersect_patches` can match files across the combined and post-commit patches by `new_path`.
- **Out of scope**: Wiring the patch service into CLI command dispatch, hooks, or runtime.
- **Out of scope**: Changes to `combine_patches` or `intersect_patches` logic.
- **Out of scope**: Adding the `hunk-files/` directory to the repo or referencing it at test runtime.
- **Out of scope**: Inlining the full JSON envelope content from `hunk-files/` — only the `diff` strings matter.
- **Non-goal**: Testing JSON parsing of the `hunk-files/` envelope format.
- **Assumption**: The `message.part.updated` files' `metadata.files[].diff` fields contain valid unified-diff text parseable by `parse_patch` once file paths are corrected.
- **Assumption**: The `post-commit` file's `input.head_patch_from_git` field contains valid unified-diff text parseable by `parse_patch` as-is (it already uses relative `a/`/`b/` paths).
- **Assumption**: Combining all seven incremental patches and intersecting with the post-commit patch should yield the post-commit patch itself (i.e., all post-commit touched lines are present in the combined incremental patches).

## Path correction detail

The `message.part.updated` diffs use `Index:` format with absolute paths like:
```
Index: /home/ssv/Projects/crocoder/shared-context-engineering/hunks/unoptimized.ts
```
and `---`/`+++` lines with the same absolute paths.

The `post-commit` diff uses `diff --git` format with relative `a/`/`b/` paths like:
```
diff --git a/hunks/fib.ts b/hunks/fib.ts
```

For `intersect_patches` to match files across the combined and post-commit patches, the incremental diffs must use file paths that resolve to the same `new_path` after parsing. The test inlines corrected diff strings where:
- `Index:` lines use relative paths (e.g., `hunks/unoptimized.ts` instead of the absolute path)
- `---`/`+++` lines use relative paths (e.g., `hunks/unoptimized.ts` instead of the absolute path)

This ensures `parse_patch` produces `new_path` values like `hunks/unoptimized.ts` that `intersect_patches` can match against the post-commit patch's `new_path` values like `hunks/fib.ts` and `hunks/optimized.ts`.

## Task stack

- [x] T01: `Add combine-intersect integration test with inline corrected-diff fixtures` (status:done)
  - Task ID: T01
  - Goal: Add a test in `cli/src/services/patch.rs` that proves `combine_patches` of all seven `message.part.updated` diffs (with corrected file paths), intersected with the `post-commit` diff via `intersect_patches`, equals the post-commit patch. Only the `diff` strings are inlined as constants, with file paths corrected to match the post-commit format.
  - Boundaries (in/out of scope): In — new `#[cfg(test)] mod tests` block or appended tests in `patch.rs`, inline string constants for the seven incremental diffs (path-corrected) and the post-commit diff, the test function itself, and any minimal helpers. Out — changes to production code in `patch.rs`, runtime file I/O, changes to `combine_patches` or `intersect_patches` logic, adding `hunk-files/` to the repo, inlining full JSON envelope content.
  - Done when: The test compiles and passes; `nix flake check` passes; the test asserts `intersect_patches(&combined, &post_commit) == post_commit`; all diff data is inline in the test module with corrected file paths; no JSON envelope content is inlined.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests::combine_intersect_matches_post_commit'`; `nix flake check`.

- [ ] T02: `Validation and cleanup` (status:todo)
  - Task ID: T02
  - Goal: Run the full repo validation baseline, verify success criteria, and confirm context sync.
  - Boundaries (in/out of scope): In — `nix run .#pkl-check-generated`, `nix flake check`, success-criteria review, context-sync verification. Out — additional behavior changes.
  - Done when: `nix run .#pkl-check-generated` passes, `nix flake check` passes, success criteria are verified against code truth, and any required context follow-up is identified.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions

- None.
