# Plan: Build embedded asset buffer hashing

## Change summary
- Verify the current `cli/build.rs` embedded-asset emission path and, if it still hashes file paths while emitting bytes via `include_bytes!`, refactor to read each asset into an in-memory buffer once and use that buffer for both SHA-256 computation and emitted byte literals.

## Success criteria
- `generate_embedded_asset_manifest` reads each asset file into a byte buffer once per file and uses that buffer to compute SHA-256 (no path-based hashing).
- The emitted `EmbeddedAsset` entries use a byte array literal derived from the same in-memory buffer; `include_bytes!` is removed from the manifest emission.
- `format_sha256_literal` still formats the computed digest, and `escape_for_rust_string` is applied only to the path string.
- No changes to asset selection, relative path normalization, or `EmbeddedAsset` interface beyond the bytes emission format.

## Constraints and non-goals
- Do not change which assets are collected or how paths are normalized.
- Do not alter the hash algorithm or the formatting produced by `format_sha256_literal`.
- Do not introduce runtime file reads in the CLI; this is build-time-only manifest generation.
- Only apply the refactor if the current code still uses `include_bytes!` with path-based hashing at the targeted location.

## Task stack
- [x] T01: Refactor embedded asset emission to use an in-memory buffer (status:done)
  - Task ID: T01
  - Goal: Read each asset into a buffer once, compute SHA-256 from that buffer, and emit the bytes field as a literal derived from the buffer (no `include_bytes!`).
  - Boundaries (in/out of scope): In scope: `cli/build.rs` changes to `generate_embedded_asset_manifest`, `compute_sha256` signature/usage, and any new helper for formatting byte literals. Out of scope: changes to asset discovery, `EmbeddedAsset` shape, or any runtime CLI code under `cli/src`.
  - Done when: The loop reads file bytes once, `compute_sha256` accepts a byte slice (or equivalent) and is called with the buffer, `include_bytes!` is removed from the emitted `EmbeddedAsset` entries, and `escape_for_rust_string` is only used for the path string.
  - Verification notes (commands or checks): Review the updated `cli/build.rs` emission block to confirm the hash and byte literal derive from the same buffer and no `include_bytes!` remains.

- [x] T02: Validation and context sync (status:done)
  - Task ID: T02
  - Goal: Run required checks and confirm context remains accurate for this localized build-script change.
  - Boundaries (in/out of scope): In scope: `nix run .#pkl-check-generated`, `nix flake check`, and a verify-only context sync pass. Out of scope: additional refactors or unrelated documentation edits.
  - Done when: Validation commands are executed (or explicitly noted if skipped), and context files are confirmed up to date with no edits required.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`.

## Open questions
- None.

## Task: T01 Refactor embedded asset emission to use an in-memory buffer
- **Status:** done
- **Completed:** 2026-04-07
- **Files changed:** cli/build.rs
- **Evidence:** `nix run .#pkl-check-generated`; `nix flake check`
- **Notes:** Embedded asset bytes are emitted from the same in-memory buffer used for SHA-256; `include_bytes!` removed from emitted manifest entries.

## Task: T02 Validation and context sync
- **Status:** done
- **Completed:** 2026-04-07
- **Files changed:** None
- **Evidence:** `nix run .#pkl-check-generated`; `nix flake check`
- **Notes:** Verify-only context sync completed; root context files reviewed with no changes required.
