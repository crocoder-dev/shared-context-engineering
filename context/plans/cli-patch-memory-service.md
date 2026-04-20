# Plan: CLI Patch Memory Service

## Change summary

Add a new standalone service under `cli/src/services/` that parses patch text into an in-memory Rust structure containing only touched lines from diff hunks (added/removed lines plus the minimal per-file/per-hunk metadata needed to interpret them), while ignoring non-hunk headers and unchanged context lines. The service must support the patch styles shown in `files/1/`, `files/2/`, and `files/3/`, and the in-memory representation must be cleanly serializable/deserializable so it can round-trip back into the same struct shape.

## Success criteria

1. A new standalone patch service exists in `cli/src/services/` and is not wired into command dispatch or hook runtime yet.
2. The service parses both `Index: ...` patch variants and `diff --git ...` patch variants from the provided fixture families.
3. The parsed representation drops patch headers and unchanged context lines, retaining only touched lines plus enough metadata to preserve file/hunk structure.
4. The representation is `serde`-serializable and deserializable, and round-trip tests prove `struct -> serialized form -> struct` fidelity.
5. Tests cover at least: new-file patches, modified-file patches, multi-file patch payloads, and removed-line handling.
6. Existing validation continues to pass after the service is added.

## Constraints and non-goals

- **In scope**: a standalone library seam in `cli/src/services/`, patch-domain structs, parsing logic for the observed patch formats, and serialization/deserialization support.
- **Out of scope**: wiring the service into `cli/src/app.rs`, `hooks.rs`, or any other runtime command path; parsing outer event wrapper JSON such as `session.diff.json`; recreating the original raw patch text including ignored headers/formatting.
- **Non-goal**: preserving unchanged context lines.
- **Non-goal**: introducing a CLI command or user-facing output contract for this service in this plan.
- **Assumption**: “serialize it nicely and load it back in the struct” means a stable `serde`-based structured representation that round-trips the parsed model, not byte-for-byte regeneration of the original patch text.

## Task stack

- [x] T01: `Add serde-friendly patch domain model and standalone service seam` (status:done)
  - Task ID: T01
  - Goal: Create a new patch-focused service module under `cli/src/services/` that exposes the core domain types for parsed patches, files, hunks, and touched lines, with a public API shaped for standalone library use and `serde` round-tripping.
  - Boundaries (in/out of scope): In — new module file(s), `mod.rs` export, Rust structs/enums for file change kind and touched line kind, derives needed for equality/debug/serialization, and unit tests for model serialization round-trip. Out — actual patch parsing logic, runtime integration, command wiring.
  - Done when: The new service module exists, its domain model captures file/hunk/touched-line structure without header retention, the types serialize/deserialize cleanly via `serde`, and focused tests prove round-trip fidelity.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch'`; review model/test names for standalone-library clarity.

### T01 completion

- **Status:** done
- **Completed:** 2026-04-20
- **Files changed:** `cli/src/services/patch.rs` (new), `cli/src/services/mod.rs` (modified)
- **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt all green); `nix run .#pkl-check-generated` passed; 10 round-trip unit tests covering ParsedPatch, PatchFileChange (Added/Modified/Deleted), PatchHunk, TouchedLine, FileChangeKind/TouchedLineKind enum variants, empty patch, empty hunks, and snake_case JSON field naming.
- **Notes:** `#[allow(dead_code)]` on all public types since they are not yet consumed by command dispatch or hook runtime (per T01 out-of-scope boundary). T02 will wire the parser and reference these types, removing the allow attributes.

- [x] T02: `Implement touched-line parsing for supported patch formats` (status:done)
  - Task ID: T02
  - Goal: Implement parsing from raw patch text into the new domain model, supporting the observed unified-diff families from `files/1/`, `files/2/`, and git-style `diff --git` samples while ignoring headers and unchanged context lines.
  - Boundaries (in/out of scope): In — parser entrypoint(s), hunk parsing, line classification for added/removed touched lines, file boundary detection, support for single-file and multi-file patch text. Out — parsing outer JSON event payloads, runtime integration, alternate diff syntaxes not evidenced by current examples.
  - Done when: Raw patch strings from the provided fixture families parse into deterministic file/hunk/touched-line structures; added-file and modified-file cases are covered; context lines are excluded; parser failures are actionable for malformed patch input.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests'`; fixture-backed unit tests using examples from `files/1/`, `files/2/`, and `files/3/`.

### T02 completion

- **Status:** done
- **Completed:** 2026-04-20
- **Files changed:** `cli/src/services/patch.rs` (modified)
- **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt all green); `nix run .#pkl-check-generated` passed; 17 parser unit tests covering Index-style new-file patches (absolute and relative paths), git-style new-file patches, Index-style modified-file patches with removed lines, git-style modified-file patches, multi-file Index-style patches, Index-style new-file relative-path patches, Index-style modified-file patches with added lines, empty input, error cases (missing file header, invalid hunk header, missing closing `@@`), git-style `/dev/null` old/new path handling, deleted-file detection, `\ No newline at end of file` skipping, multiple hunks in a single file, line-number tracking through mixed context/removed/added, hunk-header count defaulting to 1, git-style renamed-file detection, and parse-then-serialize round-trip fidelity.
- **Notes:** Removed `#[allow(dead_code)]` from domain model types (ParsedPatch, PatchFileChange, FileChangeKind, PatchHunk, TouchedLine, TouchedLineKind) since they are now consumed by the parser. Added `#[allow(dead_code)]` to parser internals (parse_patch, FileBuilder, DiffPaths, determine_file_kind, parse_git_diff_header, parse_diff_path, parse_hunk_header_and_body, parse_range_part, ParseError) since they are not yet wired into command dispatch (per T02 out-of-scope boundary). Parser supports both `Index:` (SVN-style) and `diff --git` (git-style) unified-diff formats, correctly handles `/dev/null` paths for new/deleted files, `a/`/`b/` prefix stripping, trailing tab characters in `---`/`+++` lines, multi-file patches, `\ No newline at end of file` markers, and hunk headers with or without explicit counts.

- [x] T03: `Harden coverage for multi-file and deletion-oriented cases` (status:done)
  - Task ID: T03
  - Goal: Close the acceptance gaps around multi-file payloads and deletion semantics by adding targeted tests and any minimal parser/model refinements required for removed-line and deleted-file-style behavior.
  - Boundaries (in/out of scope): In — tests using `files/3/diff.1` and similar multi-file fixtures, explicit coverage for removed lines from `files/2/**`, and a small synthetic fixture if needed to cover deleted-file-style input absent from repo samples. Out — new runtime consumers, JSON wrapper parsing, broad parser refactors unrelated to the accepted formats.
  - Done when: The parser has explicit passing coverage for multi-file payloads, removed-line capture, and any required deleted-file-style case; any refinements remain scoped to supporting those acceptance cases only.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch'`; confirm fixture-backed assertions for `files/2/` removed lines and `files/3/` multi-file parsing.

### T03 completion

- **Status:** done
- **Completed:** 2026-04-20
- **Files changed:** `cli/src/services/patch.rs` (modified — added 5 new test cases)
- **Evidence:** `nix flake check` passed (cli-tests, cli-clippy, cli-fmt all green); `nix run .#pkl-check-generated` passed; 5 new test cases added covering: git-style multi-file patch (modified + new file), Index-style deleted file (`+++ /dev/null`), multi-file Index-style patch with deleted file, hunk with only removed lines (no additions), and git-style multi-hunk multi-file patch. No parser logic changes were needed — the existing parser handled all acceptance cases correctly.
- **Notes:** All T03 acceptance gaps are closed. The parser already handled Index-style deleted files, multi-file git-style patches, and removal-only hunks correctly; the new tests confirm this explicitly.

- [ ] T04: `Validation and cleanup` (status:todo)
  - Task ID: T04
  - Goal: Run the repo validation baseline, verify all success criteria, and confirm whether the change requires focused context updates or only a verify-only root context pass.
  - Boundaries (in/out of scope): In — full validation, cleanup of any temporary parser scaffolding/tests, and context-sync verification for the new service seam. Out — new feature work.
  - Done when: `nix run .#pkl-check-generated` passes, `nix flake check` passes, success criteria are re-checked against the implemented parser/service, and `context/` is either updated accurately or explicitly verified as unchanged where appropriate.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; review implemented service against `context/architecture.md` / focused CLI context needs.

## Open questions

None.
