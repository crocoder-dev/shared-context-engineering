# Minimal agent-trace generator seam

Library-only Rust seam at `cli/src/services/agent_trace.rs` that produces the minimal agent-trace JSON shape from patch data.

## Contract

Given a `constructed_patch` (AI candidate) and a `post_commit_patch` (canonical source of truth):

1. Compute `intersection_patch = intersect_patches(constructed_patch, post_commit_patch)` — the touched-line overlap.
2. Compare `intersection_patch` hunks against `post_commit_patch` hunks slot-by-slot (matched by `old_start`).
3. Classify each `post_commit_patch` hunk:
   - **`ai`** — `intersection_patch` hunk exists with identical touched lines (same count, kind, `line_number`, content, order).
   - **`mixed`** — `intersection_patch` hunk exists at the same slot but content differs.
   - **`unknown`** — no `intersection_patch` hunk at the same `old_start` slot.
4. Emit one `Conversation` per `post_commit_patch` hunk, one `TraceFile` per `post_commit_patch` file.

## Domain types

| Type | Purpose |
|---|---|
| `HunkContributor` | Enum: `Ai`, `Mixed`, `Unknown` |
| `Contributor` | Nested per-conversation object carrying `type: HunkContributor` |
| `LineRange` | New-file line span with `start_line` + `end_line` |
| `Conversation` | Per-hunk entry: nested contributor + `ranges` (currently exactly one range derived from `post_commit_patch`) |
| `TraceFile` | Per-file entry: path + conversations |
| `AgentTraceVcs` | Top-level VCS metadata object carrying `type` + `revision` |
| `AgentTrace` | Top-level payload: `version`, `id`, `timestamp`, `vcs`, `files` |

All types are `serde`-serializable with `snake_case` field naming. `Conversation.contributor` serializes as a nested object with a JSON field named `type`.

## Payload shape

Current output includes top-level metadata fields with this contract:

- `version` is fixed to `"0.1.0"` and follows strict numeric `x.y.z`
- `id` is generated per `build_agent_trace(...)` call as a UUIDv7 string derived from the same commit-time moment used for `timestamp`
- `timestamp` is sourced from explicit commit metadata input (`AgentTraceMetadataInput.commit_timestamp`) and must be RFC 3339
- `vcs.type` is fixed to `"git"`
- `vcs.revision` is sourced from explicit commit metadata input (`AgentTraceMetadataInput.commit_revision`)

```json
{
  "version": "0.1.0",
  "id": "01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1",
  "timestamp": "2026-04-23T10:20:30Z",
  "vcs": {
    "type": "git",
    "revision": "a0b1c2d3e4f5a6b7c8d9e0f11223344556677889"
  },
  "files": [
    {
      "path": "src/example.ts",
      "conversations": [
        {
          "contributor": { "type": "ai" },
          "ranges": [
            {
              "start_line": 10,
              "end_line": 14
            }
          ]
        }
      ]
    }
  ]
}
```

## Public API

- `classify_hunk(post_commit_hunk, intersection_hunks) -> HunkContributor` — classify a single `post_commit_patch` hunk against `intersection_patch` hunks.
- `build_agent_trace(constructed_patch, post_commit_patch, metadata) -> Result<AgentTrace>` — full generator entrypoint that validates `metadata.commit_timestamp` as RFC 3339, uses it as top-level `timestamp`, derives a UUIDv7 `id` from that same commit-time moment, sets `vcs.type = "git"`, and maps `metadata.commit_revision` to `vcs.revision`.

## Test fixture contract

- Golden fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json` pin deterministic literal values for top-level `id` and `timestamp`.
- Tests still validate runtime metadata behavior explicitly (`id` parses as UUIDv7 and `timestamp` equals provided commit metadata), then normalize those runtime values to the deterministic fixture literals before whole-payload golden comparison.

## Relationship to existing patch service

Consumes `intersect_patches` and `ParsedPatch`/`PatchHunk`/`TouchedLine` types from `cli/src/services/patch.rs`. Does not introduce a separate patch model.

## Out of scope

CLI command surface, hook/runtime integration (including post-commit wiring), persistence, OpenCode plugin behavior, non-MVP payload enrichment.

## See also

- [../overview.md](../overview.md)
- [../glossary.md](../glossary.md)
- [../context-map.md](../context-map.md)
