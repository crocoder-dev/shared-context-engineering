# Patch Service

Standalone patch domain model and parser in `cli/src/services/patch.rs` for in-memory parsed unified-diff representation.

## Domain model

- `ParsedPatch` — top-level container holding one or more `PatchFileChange` entries
- `PatchFileChange` — per-file change with `old_path`, `new_path`, `FileChangeKind`, and hunks
- `FileChangeKind` — enum: `Added`, `Modified`, `Deleted`, `Renamed` (serialized as `snake_case`)
- `PatchHunk` — hunk with `old_start`/`old_count`/`new_start`/`new_count` and touched lines
- `TouchedLine` — a single added or removed line with `kind`, `line_number`, and `content`
- `TouchedLineKind` — enum: `Added`, `Removed` (serialized as `snake_case`)

All types derive `Clone, Debug, Deserialize, Eq, PartialEq, Serialize` and support JSON round-trip fidelity via `serde` with `snake_case` field naming. `TouchedLineKind` additionally derives `Hash` to support set-based intersection operations.

## Parser

`parse_patch(input: &str) -> Result<ParsedPatch, ParseError>` converts raw unified-diff text into `ParsedPatch` structs.

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
- `load_patch_from_json_bytes(input: &[u8]) -> Result<ParsedPatch, PatchLoadError>` — loads a `ParsedPatch` from JSON bytes; convenient when the caller has raw bytes (for example, from a database BLOB column or file read) rather than a UTF-8 string

Both functions wrap `serde_json::from_str`/`serde_json::from_slice` and map serde errors to actionable `PatchLoadError` messages. `PatchLoadError` carries a `message` field describing why the JSON payload could not be reconstructed into a valid `ParsedPatch`.

## Set operations

### Intersection

`intersect_patches(a: &ParsedPatch, b: &ParsedPatch) -> ParsedPatch` returns a `ParsedPatch` containing only the touched lines from `b` that are also represented in `a` for the same logical file.

- **File matching**: files are matched by post-change path identity — exact `new_path` equality, or absolute-vs-relative path variants whose normalized path segments share the same relative suffix
- **Touched-line matching**: matching prefers exact identity (`kind`, `line_number`, and `content`); when no exact match exists, it falls back to historical reconstruction matching by `kind` and `content` only so canonical post-commit patches can still intersect with earlier incremental diffs whose line numbers drifted
- **Result structure**: only files with at least one overlapping touched line appear in the result; hunks with no overlapping lines are excluded; hunk range metadata (`old_start`, `old_count`, `new_start`, `new_count`) is preserved from the second patch (`b`) so the result keeps the target patch shape, while hunk `model_id` provenance is inherited from the matched hunk in the first patch (`a`) when available (and remains `None` when matched constructed provenance is absent)
- **Determinism**: the same inputs always produce the same output
- **Equivalent-hunk behavior**: semantically identical hunks still intersect when they differ only in surrounding context windows, hunk header ranges, or absolute-vs-relative `Index:` path spelling, as long as their touched-line identities match exactly
- **Consumed by**: the post-commit hook runtime combines recent DB diff-trace patches and then intersects with the current commit patch (see `agent-trace-hooks-command-routing.md`). Previously listed as "not yet wired" before T04.

### Combination

`combine_patches(patches: &[ParsedPatch]) -> ParsedPatch` merges multiple `ParsedPatch` values into one deterministic result with later-input-wins semantics for duplicate/conflicting touched-line entries.

- **File matching**: files are grouped by `new_path`; file metadata (`old_path`, `kind`) is taken from the last patch that contributed to each file
- **Touched-line identity and deduplication**: touched lines are deduplicated by identity (`kind`, `line_number`, `content`); when multiple patches describe the same file and logical touched-line slot, the later input's entry is retained
- **Hunk reconstruction and provenance**: surviving lines are grouped by their hunk metadata from the last contributing patch; each reconstructed hunk preserves that winning hunk's `model_id` provenance; hunks are ordered by `old_start`; lines within each hunk are ordered by `line_number` with `Removed` before `Added` at the same position, then by `content` for full determinism
- **File ordering**: files appear in the result in the order they are first encountered across the input patches
- **Determinism**: the same inputs in the same order always produce the same output
- **Consumed by**: the post-commit hook runtime combines recent DB diff-trace patches before intersecting (see `agent-trace-hooks-command-routing.md`). Previously listed as "not yet wired" before T04.

### Runtime wiring status

| Operation | Wired into | Notes |
|-----------|-----------|-------|
| `parse_patch` | Hook runtime, Agent Trace DB recent-row parsing, tests | Consumed by `post-commit` capture flow and `recent_diff_trace_patches` to parse stored raw `diff_traces.patch` text |
| `load_patch_from_json` / `load_patch_from_json_bytes` | Storage-agnostic JSON reconstruction callers | Reconstructs serialized `ParsedPatch` JSON when callers already have JSON payloads; not used for raw `diff_traces.patch` text in `recent_diff_trace_patches` |
| `intersect_patches` | Post-commit hook runtime | Combines recent patches then intersects with current commit patch |
| `combine_patches` | Post-commit hook runtime | Combines chronological recent patches before intersection |

Public types consumed by the parser or load helpers have `#[allow(dead_code)]` removed; other module internals that are not yet consumed outside the crate retain `#[allow(dead_code)]`.

## Reconstruction fixture suites

Patch reconstruction tests use deterministic fixture suites under `cli/src/services/patch/fixtures/`.

- Existing suites remain intact (`average_age_reconstruction`, `hello_world_reconstruction`).
- The current tmp-hunks scenario is materialized as `text_file_lifecycle_reconstruction/` with:
  - `incremental_01.patch` .. `incremental_26.patch` reconstructed from `tmp_hunks/*-message.part.updated.json` in lexical filename order
  - `post_commit.patch` reconstructed from `tmp_hunks/*-post-commit.json` `input.head_patch_from_git`
- Incremental fixture patch headers are normalized to relative repo paths for parser/file matching compatibility.

## See also

- [overview.md](../overview.md)
- [architecture.md](../architecture.md)
- [glossary.md](../glossary.md)
