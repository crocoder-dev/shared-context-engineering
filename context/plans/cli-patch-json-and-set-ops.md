# Plan: CLI Patch JSON and Set Operations

## Change summary

Extend the standalone patch service in `cli/src/services/patch.rs` with storage-agnostic JSON reloading helpers plus deterministic patch comparison/combination operations. The new surface should let callers reload previously serialized patch data regardless of whether the JSON came from a database or a file, compute the exact touched-line intersection of two patches, and combine multiple patches into one deterministic `ParsedPatch` where later patch inputs win on duplicate/conflicting touched-line entries.

## Success criteria

1. The patch service exposes a public JSON-loading API that reconstructs `ParsedPatch` from serialized JSON without coupling to filesystem or database access.
2. The JSON-loading surface returns actionable errors for invalid serialized patch payloads.
3. The public API surface is easy to understand and developer-friendly: naming is explicit, entrypoints are discoverable, and common usage does not require callers to understand parser internals.
4. The patch service exposes a public intersection operation that returns a `ParsedPatch` containing only exact overlapping changed lines between two input patches.
5. Exact overlap means the same touched-line identity is present in both patches, using file identity plus touched-line identity rather than broad file-only or hunk-only overlap.
6. The patch service exposes a public combine operation that merges multiple `ParsedPatch` values into one deterministic result.
7. Combine semantics are deterministic and “later patches win” when duplicate/conflicting touched-line entries target the same file and logical changed-line slot.
8. Targeted tests cover JSON reload success/failure, exact-line intersection, and multi-patch combination ordering/conflict behavior.
9. Repository validation continues to pass after the new patch-service capabilities are added.

## Constraints and non-goals

- **In scope**: storage-agnostic JSON deserialization helpers for patch-domain structs, exact touched-line intersection semantics, deterministic patch-combine semantics, internal helper types/functions needed to support those operations, and focused unit tests.
- **In scope**: API naming/docs/comments small enough to make the new surface easy to understand for future contributors.
- **Out of scope**: adding DB persistence, adding filesystem read/write APIs, wiring these operations into CLI command dispatch, hooks, or sync runtime paths.
- **Out of scope**: designing a generic patch algebra beyond the requested intersection and combine operations.
- **Non-goal**: preserving original raw patch text or header formatting.
- **Non-goal**: resolving all possible semantic conflicts between unrelated patch formats beyond the current `ParsedPatch` model.
- **Assumption**: because the caller may later store serialized patches in either a DB or file, the new load API should accept already-read serialized JSON content (and optionally bytes) rather than own DB/file IO.
- **Assumption**: “exact overlap of changed lines” should be implemented against the current patch-domain model using stable file identity plus touched-line identity (`kind`, logical line number, and content), with the result returned as another `ParsedPatch`.
- **Assumption**: “later added patches win” means combination order is significant, and when multiple inputs describe the same file/logical touched-line slot differently, the later input’s touched-line entry is retained in the merged result.

## Task stack

- [x] T01: `Add storage-agnostic patch JSON load helpers` (status:done)
  - Task ID: T01
  - Goal: Add a public helper surface in `cli/src/services/patch.rs` for reconstructing `ParsedPatch` from serialized JSON in a way that callers can reuse after reading from either a DB or file, with naming and docs that make the intended usage obvious.
  - Boundaries (in/out of scope): In — helper API shape, serde-backed deserialization, actionable error mapping, concise dev-friendly docs/comments, focused tests for valid and invalid payloads. Out — file-path helpers, DB adapters, command wiring, persistence schema work.
  - Done when: Callers can load `ParsedPatch` from serialized JSON content through a public API; malformed payloads return actionable errors; the API names/docs are self-explanatory for common usage; tests cover successful reload plus representative failure cases.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests::json'` or nearest targeted patch test selection; review API to confirm it is storage-agnostic and understandable without reading parser internals.
  - Status: done
  - Completed: 2026-04-20
  - Files changed: cli/src/services/patch.rs
  - Evidence: nix flake check passed (all checks: cli-tests, cli-clippy, cli-fmt, pkl-parity); nix run .#pkl-check-generated passed; added PatchLoadError type, load_patch_from_json(&str), load_patch_from_json_bytes(&[u8]) with doc comments; 11 new focused tests covering round-trip from string, round-trip from bytes, empty patch, single file, invalid JSON syntax, valid JSON but wrong structure, missing files field, invalid UTF-8 bytes, wrong structure from bytes, all FileChangeKind variants, all TouchedLineKind variants, and end-to-end parse→serialize→load round-trip

- [x] T02: `Implement exact touched-line intersection for ParsedPatch` (status:done)
  - Task ID: T02
  - Goal: Add a public patch-intersection operation that returns a `ParsedPatch` containing only exact overlapping changed lines present in both input patches, using an API shape that reads clearly at the call site.
  - Boundaries (in/out of scope): In — touched-line matching rules, file grouping for overlaps, deterministic output shaping, concise docs/comments and targeted tests for identical overlap and non-overlap cases. Out — fuzzy matching, file-only overlap reporting, non-exact semantic diff reconciliation.
  - Done when: Intersecting two patches yields only exact overlapping touched lines, non-overlapping lines are excluded, output remains deterministic, the operation naming/usage is developer-friendly, and tests prove the matching contract across same-file and no-overlap cases.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests::intersection'`; review assertions and public API call sites to confirm overlap is exact line-level identity and the surface is easy to read.
  - Status: done
  - Completed: 2026-04-20
  - Files changed: cli/src/services/patch.rs
  - Evidence: nix flake check passed (all checks: cli-tests, cli-clippy, cli-fmt, pkl-parity); nix run .#pkl-check-generated passed; added `intersect_patches(a, b)` public function with `#[allow(dead_code)]` and `Hash` derive on `TouchedLineKind`; 9 new focused tests covering identical overlap, no overlap, partial overlap, same-file different lines, multi-file partial overlap, empty patches, hunk metadata preservation, line identity requiring kind+number+content, determinism, multi-hunk same file, and file matching by new_path

- [x] T03: `Implement ordered patch combination with later-wins conflict resolution` (status:done)
  - Task ID: T03
  - Goal: Add a public combine operation that merges multiple `ParsedPatch` values into one deterministic result while preserving the requested later-input-wins rule for duplicate/conflicting touched-line entries, with an API signature that is intuitive for contributors to use correctly.
  - Boundaries (in/out of scope): In — combine API, deterministic ordering rules, dedupe/conflict resolution for same file/logical touched-line slot, concise docs/comments, targeted tests for duplicate and conflicting inputs. Out — patch normalization beyond what is needed for the current model, CLI/runtime consumers.
  - Done when: Combining multiple patches yields one deterministic `ParsedPatch`; duplicate/conflicting touched-line entries resolve to the later input; the combine API communicates ordering semantics clearly; tests cover repeated identical lines, conflicting later overrides, and multi-file combination behavior.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test patch::tests::combine'`; inspect expected outputs and public API usage shape to confirm later patch order changes the result only where intended.
  - Status: done
  - Completed: 2026-04-20
  - Files changed: cli/src/services/patch.rs
  - Evidence: nix flake check passed (all checks: cli-tests, cli-clippy, cli-fmt, pkl-parity); nix run .#pkl-check-generated passed; added `combine_patches(patches: &[ParsedPatch]) -> ParsedPatch` public function with `#[allow(dead_code)]`, `LineKey` and `HunkMeta` type aliases, `FileAcc` accumulator struct, file-order-preserving merge with later-wins deduplication by `(kind, line_number, content)` identity, hunk metadata from last contributing patch, deterministic line sorting (line_number, Removed-before-Added, content); 11 new focused tests covering empty input, single patch, identical line deduplication, conflicting later-wins, multi-file merge, file metadata from last patch, determinism, hunk metadata from last contributor, multi-hunk merge, three-patch later-wins, mixed added/removed lines, and empty-patch-with-non-empty

- [ ] T04: `Validation and cleanup` (status:todo)
  - Task ID: T04
  - Goal: Run the repo validation baseline, verify all requested capabilities, and confirm whether focused patch-service context docs need updating.
  - Boundaries (in/out of scope): In — full validation, success-criteria recheck, cleanup of temporary test scaffolding, context-sync verification for the patch service contract. Out — additional feature work.
  - Done when: `nix run .#pkl-check-generated` passes, `nix flake check` passes, success criteria are re-verified against code truth, and any required `context/` updates are identified or applied in a follow-up implementation/context-sync session.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; compare resulting code truth with `context/cli/patch-service.md`, `context/context-map.md`, and root shared files for verify-only vs important-change context sync.

## Open questions

None.
