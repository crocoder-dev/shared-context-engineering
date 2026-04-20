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

`intersect_patches(a: &ParsedPatch, b: &ParsedPatch) -> ParsedPatch` returns a `ParsedPatch` containing only exact overlapping touched lines present in both input patches.

- **File matching**: files are matched by `new_path` (the canonical post-change path)
- **Touched-line identity**: a touched line is considered identical when `kind`, `line_number`, and `content` all match
- **Result structure**: only files with at least one overlapping touched line appear in the result; hunks with no overlapping lines are excluded; hunk metadata (`old_start`, `old_count`, `new_start`, `new_count`) is preserved from the first patch (`a`)
- **Determinism**: the same inputs always produce the same output
- **Not yet wired**: `intersect_patches` is a standalone library seam not yet wired into command dispatch or hook runtime

### Combination

`combine_patches(patches: &[ParsedPatch]) -> ParsedPatch` merges multiple `ParsedPatch` values into one deterministic result with later-input-wins semantics for duplicate/conflicting touched-line entries.

- **File matching**: files are grouped by `new_path`; file metadata (`old_path`, `kind`) is taken from the last patch that contributed to each file
- **Touched-line identity and deduplication**: touched lines are deduplicated by identity (`kind`, `line_number`, `content`); when multiple patches describe the same file and logical touched-line slot, the later input's entry is retained
- **Hunk reconstruction**: surviving lines are grouped by their hunk metadata from the last contributing patch; hunks are ordered by `old_start`; lines within each hunk are ordered by `line_number` with `Removed` before `Added` at the same position, then by `content` for full determinism
- **File ordering**: files appear in the result in the order they are first encountered across the input patches
- **Determinism**: the same inputs in the same order always produce the same output
- **Not yet wired**: `combine_patches` is a standalone library seam not yet wired into command dispatch or hook runtime

### Not yet wired

The parser, JSON load helpers, intersection, and combination operations are standalone library seams not yet wired into command dispatch or hook runtime. Public types consumed by the parser or load helpers have `#[allow(dead_code)]` removed; other module internals including `intersect_patches` and `combine_patches` retain `#[allow(dead_code)]` until runtime integration.

## See also

- [overview.md](../overview.md)
- [architecture.md](../architecture.md)
- [glossary.md](../glossary.md)