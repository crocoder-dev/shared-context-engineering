# Patch Service

Standalone patch domain model and parser in `cli/src/services/patch.rs` for in-memory parsed unified-diff representation.

## Domain model

- `ParsedPatch` — top-level container holding one or more `PatchFileChange` entries
- `PatchFileChange` — per-file change with `old_path`, `new_path`, `FileChangeKind`, and hunks
- `FileChangeKind` — enum: `Added`, `Modified`, `Deleted`, `Renamed` (serialized as `snake_case`)
- `PatchHunk` — hunk with `old_start`/`old_count`/`new_start`/`new_count` and touched lines
- `TouchedLine` — a single added or removed line with `kind`, `line_number`, and `content`
- `TouchedLineKind` — enum: `Added`, `Removed` (serialized as `snake_case`)

All types derive `Clone, Debug, Deserialize, Eq, PartialEq, Serialize` and support JSON round-trip fidelity via `serde` with `snake_case` field naming.

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

### Not yet wired

The parser is a standalone library seam not yet wired into command dispatch or hook runtime. Public types consumed by the parser have `#[allow(dead_code)]` removed; parser internals retain `#[allow(dead_code)]` until runtime integration.

## See also

- [overview.md](../overview.md)
- [architecture.md](../architecture.md)
- [glossary.md](../glossary.md)