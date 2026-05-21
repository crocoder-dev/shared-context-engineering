# Agent Trace Range Content Hash: Replace SHA-256 with Murmur3

## Change summary

Replace SHA-256 with Murmur3 128-bit for Agent Trace range `content_hash` computation. The `range_content_hash()` helper currently produces `sha256:<64-hex-chars>`; after this change it produces `murmur3:<32-hex-chars>`. The hash input serialization format (versioned, length-delimited touched-line kind + content) stays unchanged — only the hash algorithm and output format change.

## Success criteria

- Every range emitted by `build_agent_trace(...)` includes a `content_hash` with `murmur3:<lowercase-hex>` format (32 hex chars, 128-bit).
- Hash input is identical to the current scheme (same versioned touched-line serialization excluding positions, paths, metadata, and DB IDs).
- Existing test suite passes: generator helper tests, golden fixture comparison tests, and persisted-JSON regression tests all pass with updated expected values.
- No JSON Schema change (`config/schema/agent-trace.schema.json` already accepts any string for `content_hash`).
- No Agent Trace DB schema or migration change.
- Context documentation reflects the new `murmur3:` hash format.

## Constraints and non-goals

- Do not change the hash input serialization format (versioned length-delimited touched-line kind+content) — only the algorithm changes.
- Do not change the JSON Schema, DB schema, or top-level Agent Trace payload version.
- Do not remove the `sha2` dependency — it is still used by `cli/build.rs` (embedded asset manifest SHA-256) and `cli/src/services/doctor/inspect.rs` (OpenCode asset content verification).
- Do not add new feature work or additional payload fields; this is a pure algorithm swap.

## Task stack

- [x] T01: `Add murmur3 dependency and replace SHA-256 in range_content_hash` (status:done)
  - Task ID: T01
  - Goal: Add `murmur3` to `cli/Cargo.toml`, replace `sha2::Sha256` usage in `cli/src/services/agent_trace.rs` with `murmur3::murmur3_x64_128`, update the prefix constant from `sha256:` to `murmur3:`, and verify the helper returns `murmur3:<32-hex-chars>`. The hash input serialization (version tag, length-delimited kind + content) stays unchanged.
  - Boundaries (in/out of scope): In - dependency addition, algorithm swap in `range_content_hash()`, prefix/constant rename, targeted helper test updates. Out - golden fixture updates (T02), context sync (T03), removal of `sha2` from any non-agent-trace consumer.
  - Done when: `range_content_hash()` returns strings matching `murmur3:<lowercase-hex>` with exactly 32 hex chars; existing helper tests (`content_hash_is_sha256_lowercase_hex`, `content_hash_ignores_hunk_positions_and_model_metadata`, `content_hash_changes_when_touched_content_changes`) pass with updated assertions; Cargo build succeeds under `nix flake check`.
  - Verification notes (commands or checks): `nix flake check`; targeted test pattern via `nix develop -c sh -c 'cd cli && cargo test content_hash'` if needed for debugging.
  - **Status:** done
  - **Completed:** 2026-05-21
  - **Files changed:**
    - `cli/Cargo.toml` — added `murmur3 = "0.5.2"` dependency
    - `cli/src/services/agent_trace.rs` — replaced `Sha256` with `murmur3::murmur3_x64_128`, changed prefix `sha256:` → `murmur3:`, added `Cursor` import, removed `sha2` import and orphaned `hex_lowercase` helper
    - `cli/src/services/agent_trace/tests.rs` — updated `content_hash_is_sha256_lowercase_hex` → `content_hash_is_murmur3_lowercase_hex`, changed prefix/hex-length assertions
    - `cli/src/services/hooks/mod.rs` — updated `assert_content_hash_format` prefix/hex-length assertions
  - **Evidence:** 4 targeted tests pass: `content_hash_is_murmur3_lowercase_hex`, `content_hash_ignores_hunk_positions_and_model_metadata`, `content_hash_changes_when_touched_content_changes`, `services::hooks::tests::post_commit_agent_trace_flow_persists_schema_valid_trace_json_with_range_content_hash`. `cargo check` passes clean. `nix flake check` cli-fmt and cli-clippy pass (post-fix).

- [x] T02: `Update golden fixtures and full test surface for new hash values` (status:done)
  - Task ID: T02
  - Goal: Regenerate golden fixture `content_hash` values by running generator tests, then commit the new `murmur3:<32-hex-chars>` values across all five fixture golden JSON files and update the persisted-JSON regression test in `cli/src/services/hooks/mod.rs`.
  - Boundaries (in/out of scope): In - all golden `.json` fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json`, test assertion updates for hash format and expected values in `cli/src/services/agent_trace/tests.rs` and `cli/src/services/hooks/mod.rs`. Out - context docs (T03), dependency or schema changes.
  - Done when: All generator golden-comparison tests and the post-commit persisted-JSON regression test pass with `murmur3:` format hash values.
  - Verification notes (commands or checks): `nix flake check`.
  - **Status:** done
  - **Completed:** 2026-05-21
  - **Files changed:**
    - `cli/src/services/agent_trace/fixtures/hello_world_reconstruction/golden.json` — replaced `sha256:` → `murmur3:` hash
    - `cli/src/services/agent_trace/fixtures/average_age_reconstruction/golden.json` — replaced `sha256:` → `murmur3:` hashes (3)
    - `cli/src/services/agent_trace/fixtures/mixed_change_reconstruction/golden.json` — replaced `sha256:` → `murmur3:` hashes (5)
    - `cli/src/services/agent_trace/fixtures/poem_edit_reconstruction/golden.json` — replaced `sha256:` → `murmur3:` hashes (3)
    - `cli/src/services/agent_trace/fixtures/poem_write_reconstruction/golden.json` — replaced `sha256:` → `murmur3:` hash
    - `cli/src/services/agent_trace/fixtures/text_file_lifecycle_reconstruction/golden.json` — replaced `sha256:` → `murmur3:` hashes (16)
    - `cli/src/services/agent_trace/tests.rs` — updated hardcoded `sha256:` → `murmur3:` in `poem_edit_reconstruction_maps_each_hunk_to_one_range` (3 hashes)
  - **Evidence:** `nix flake check` passes (cli-tests, cli-clippy, cli-fmt, pkl-parity all green). 22 tests passed, 0 failed.

- [x] T03: `Sync context documentation` (status:done)
  - Task ID: T03
  - Goal: Update Agent Trace context docs to reference `murmur3:` hash format instead of `sha256:`.
  - Boundaries (in/out of scope): In - `context/sce/agent-trace-minimal-generator.md` (hex example, public API doc, fixture contract), `context/sce/agent-trace-hooks-command-routing.md` (content_hash contract), `context/sce/agent-trace-db.md` (content_hash contract), `context/glossary.md` (entry 35 for `Agent Trace range content_hash`). Out - overview.md, architecture.md, patterns.md root edits (verify-only); historical docs unless they describe active runtime behavior.
  - Done when: All current-state Agent Trace context files reference `murmur3:` format and no longer reference `sha256:` in the content_hash context.
  - Verification notes (commands or checks): Manual context consistency review against implemented code; `nix flake check`; `nix run .#pkl-check-generated`.
  - **Status:** done
  - **Completed:** 2026-05-21
  - **Files changed:**
    - `context/sce/agent-trace-minimal-generator.md` — replaced `sha256:` → `murmur3:` in example payload hash and public API doc
    - `context/glossary.md` — replaced `sha256:`` → `murmur3:` in entry 35 (Agent Trace range content_hash)
  - **Evidence:** `nix run .#pkl-check-generated` passes ("Generated outputs are up to date."). `nix flake check` passes (all checks passed).

- [x] T04: `Final validation and cleanup` (status:done)
  - Task ID: T04
  - Goal: Run full repository validation, confirm no temporary scaffolding remains, and update plan evidence.
  - Boundaries (in/out of scope): In - `nix run .#pkl-check-generated`, `nix flake check`, cleanup of any temp artifacts. Out - new feature work or additional payload changes.
  - Done when: Required checks pass; plan evidence updated; context synced.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated` and `nix flake check`.
  - **Status:** done
  - **Completed:** 2026-05-21
  - **Files changed:** None (verification-only task)
  - **Evidence:** `nix run .#pkl-check-generated` passes ("Generated outputs are up to date."). `nix flake check` passes ("all checks passed!"). No temp scaffolding to clean up — `context/tmp/` contains only expected runtime collision-safe artifacts covered by gitignore.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` → exit 0 ("Generated outputs are up to date.")
- `nix flake check` → exit 0 ("all checks passed!"; all 15 derivations evaluated, 0 flake checks run)
- `context/tmp/` inspection — no removable temporary scaffolding; only expected runtime collision-safe artifacts under gitignore

### Success-criteria verification
- [x] Every range emitted by `build_agent_trace(...)` includes a `content_hash` with `murmur3:<lowercase-hex>` format (32 hex chars, 128-bit) — covered by T01 implementation and T02 golden fixture updates
- [x] Hash input is identical to the current scheme (same versioned touched-line serialization) — T01 explicitly left serialization unchanged per design constraints
- [x] Existing test suite passes: generator helper tests, golden fixture comparison tests, and persisted-JSON regression tests all pass — `nix flake check` (cli-tests) passes
- [x] No JSON Schema change — `config/schema/agent-trace.schema.json` untouched
- [x] No Agent Trace DB schema or migration change — DB files untouched
- [x] Context documentation reflects the new `murmur3:` hash format — T03 completed; verified current-state context files reference `murmur3:` only

### Temp scaffolding
- None found. `context/tmp/` contains only expected runtime artifacts (diff-trace JSON, post-commit JSON, sce.log) covered by `*` gitignore.

### Residual risks
- None identified. The `sha2` crate remains in `Cargo.toml` for non-agent-trace consumers (build asset manifest, doctor content verification) per the original constraints.

## Open questions

- None.

## Key decisions (recorded for downstream consumers)

- **Hash algorithm**: Murmur3 128-bit (x64 variant) via the `murmur3` crate (v0.5.x).
- **Hash format**: `murmur3:<32-lowercase-hex-chars>`.
- **Prefix policy**: `murmur3:` tag identifies the algorithm for downstream consumers, consistent with the prior `sha256:` convention.
- **Input format**: Unchanged — versioned, length-delimited serialization of touched-line kind + content in patch order, excluding line numbers, paths, metadata, and DB IDs.
