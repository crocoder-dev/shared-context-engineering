# Patch Service

Standalone patch domain model and parser in `cli/src/services/patch.rs` for in-memory parsed unified-diff representation.

## Domain model

- `ParsedPatch` — top-level container holding one or more `PatchFileChange` entries
- `PatchFileChange` — per-file change with `old_path`, `new_path`, `FileChangeKind`, and hunks
- `FileChangeKind` — enum: `Added`, `Modified`, `Deleted`, `Renamed` (serialized as `snake_case`)
- `PatchHunk` — hunk with `old_start`/`old_count`/`new_start`/`new_count` and touched lines
- `TouchedLine` — a single added or removed line with `kind`, `line_number`, `content`, and optional `session_id`
- `TouchedLineKind` — enum: `Added`, `Removed` (serialized as `snake_case`)

All types derive `Clone, Debug, Deserialize, Eq, PartialEq, Serialize` and support JSON round-trip fidelity via `serde` with `snake_case` field naming. `TouchedLineKind` additionally derives `Hash` to support set-based intersection operations.

## Parser

`parse_patch(input: &str, session_id: Option<&str>) -> Result<ParsedPatch, ParseError>` converts raw unified-diff text into `ParsedPatch` structs and propagates `session_id` onto each produced touched line (`Some` when provided, `None` otherwise).

### Supported formats

- `Index:` (SVN-style) patches with `===` separators and `---`/`+++` path headers
- `diff --git` (git-style) patches with `a/`/`b/` path prefixes and metadata lines

### Parser behavior

- Detects file boundaries from `Index:` or `diff --git` headers
- Extracts `old_path`/`new_path` from `---`/`+++` lines, stripping `a/`/`b/` prefixes and handling `/dev/null`
- Determines `FileChangeKind` from `new file mode`/`deleted file mode`/`rename` metadata or path analysis
- Parses `@@ -old_start[,old_count] +new_start[,new_count] @@` hunk headers (count defaults to 1 when omitted)
- Classifies `+` lines as `Added`, `-` lines as `Removed`, skips space-prefixed context lines
- Tracks line numbers: new-file line numbers for added lines, old-file line numbers for removed lines
- Skips `\ No newline at end of file` markers
- Returns `ParseError` with actionable messages for malformed input

## JSON load helpers

Storage-agnostic helpers for reconstructing `ParsedPatch` from serialized JSON content:

- `load_patch_from_json(input: &str) -> Result<ParsedPatch, PatchLoadError>` — loads a `ParsedPatch` from a JSON string; callers who have already read JSON from a database or file can pass the string directly

Both functions wrap `serde_json::from_str`/`serde_json::from_slice` and map serde errors to actionable `PatchLoadError` messages. `PatchLoadError` carries a `message` field describing why the JSON payload could not be reconstructed into a valid `ParsedPatch`.

## Set operations

### Direct matching vs. causal mutation lineage

The former `intersect_patches` / `combine_patches` set operations were removed: their historical `(kind, content)` fallback was too permissive for attribution and they had no remaining runtime caller. Attribution-grade direct matching lives in `cli/src/services/agent_trace/evidence.rs`, keeps each `diff_traces` row separate, requires an exact `(file, kind, line_number, content)` match, and fails closed when candidates for one line come from different sessions. It reuses `paths_refer_to_same_file` for absolute-vs-relative file identity.

Mutation-history attribution does **not** match historical patches against the committed patch at all. `cli/src/services/mutation_trace/lineage.rs` applies each reconstructed transition's hunks structurally to a per-line provenance vector and propagates provenance forward; `attribution.rs` only supplies `patch_for_locations` / `patch_for_locations_with_provenance` (rebuild a target-shaped patch from selected line locations). File identity for mutation lineage is **exact logical repository path** equality only — the lineage keys tracked files by that path (`new_path` when non-empty, otherwise `old_path`), so a reconstructed transition touching a path that is not byte-equal to a committed target path contributes no provenance. There is no normalized-suffix or basename fallback in the mutation matcher; suffix equivalence stays confined to the direct-evidence matchers above. A committed line is AI only if the line an AI event introduced survives every later transition into the committed tree. See [mutation-trace-agent-attribution.md](mutation-trace-agent-attribution.md).

### Codex apply_patch boundary

Codex `PostToolUse(apply_patch)` evidence enters this service only after the
Codex-specific outer wrapper normalization, canonical parsing, and event-cwd
path resolution have produced safe repository-relative paths. Its normalizer
emits only provable Add/Update touched lines as `Index:`-form text, assigning
bounded deterministic synthetic line identities from `tool_use_id`; those
identities are evidence keys, not physical source line numbers. Delete File,
pure rename, and Bash filesystem mutations produce no line-level evidence.

Attribution-grade direct matching requires exact line numbers, and synthetic
Codex positions (offsets of at least 2^31 per event) never equal real
post-commit positions. Codex `apply_patch` rows therefore cannot attribute or
falsely attribute a committed line; Codex edits are attributed through Codex
mutation scopes and causal lineage. The Codex handler still persists through
the existing `diff_traces` row shape; no Codex-specific Agent Trace builder,
pending state, or schema migration is introduced.

### Runtime wiring status

| Operation | Wired into | Notes |
|-----------|-----------|-------|
| `parse_patch` | Hook runtime, Agent Trace DB recent-row parsing, tests | Consumed by `post-commit` capture flow and `recent_diff_trace_patches` to parse stored raw `diff_traces.patch` text |
| `load_patch_from_json` | Storage-agnostic JSON reconstruction callers | Reconstructs serialized `ParsedPatch` JSON when callers already have JSON payloads; not used for raw `diff_traces.patch` text in `recent_diff_trace_patches` |

For `payload_type="structured"` database rows, the repository adapter enriches the derived `ParsedPatch` before direct-evidence matching: every hunk receives the persisted row `model_id`, and every touched line receives the persisted tool-prefixed canonical row `session_id`. The raw unprefixed session carried inside the stored Claude payload is not reconstructed as touched-line provenance.

Public types consumed by the parser or load helpers have `#[allow(dead_code)]` removed; other module internals that are not yet consumed outside the crate retain `#[allow(dead_code)]`.

## Reconstruction fixture suites

Reconstruction scenarios (`average_age`, `hello_world`, `mixed_change`, `poem_edit`, `poem_write`, `text_file_lifecycle`, `file_rename`, `line_and_file_deletion`) live under `cli/src/services/agent_trace/fixtures/` and are asserted end-to-end through `build_agent_trace_from_evidence` golden Agent Trace payloads in `cli/src/services/agent_trace/tests.rs`.

- `diff_creation/` is the checked-in Claude diff-render golden fixture suite used by Rust derivation coverage, located at `cli/src/services/structured_patch/fixtures/`. Each active golden scenario consumes:
  - `claude-post-tool-use.json` containing a sanitized synthetic Claude `PostToolUse` payload shaped for `derive_claude_structured_patch(...)`
  - `expected.patch` containing the expected unified diff output
  - No `input.json` sidecar is part of the active fixture contract for this suite.
  - Covered scenarios: Write structured-update hunk derivation, empty/no-newline/multiline Write content-create fallback flows, and single-hunk/multi-hunk/additions-only/deletions-only Edit flows
  - Rust golden tests in `cli/src/services/structured_patch/tests.rs` discover and validate all scenarios at test time, failing on missing or extra fixtures.
- The current tmp-hunks scenario is materialized as `text_file_lifecycle_reconstruction/` with:
  - `incremental_01.patch` .. `incremental_26.patch` reconstructed from `tmp_hunks/*-message.part.json` in lexical filename order
  - `post_commit.patch` reconstructed from `tmp_hunks/*-post-commit.json` `input.head_patch_from_git`
- Incremental fixture patch headers are normalized to relative repo paths for parser/file matching compatibility.

## See also

- [structured-patch-service.md](structured-patch-service.md)
- [overview.md](../overview.md)
- [architecture.md](../architecture.md)
- [glossary.md](../glossary.md)
