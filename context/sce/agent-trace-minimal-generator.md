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
| `AgentTrace` | Top-level payload: files |

All types are `serde`-serializable with `snake_case` field naming. `Conversation.contributor` serializes as a nested object with a JSON field named `type`.

## Payload shape

Current output remains file-only and does not include top-level `version`, `id`, or `timestamp` metadata.

```json
{
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
- `build_agent_trace(constructed_patch, post_commit_patch) -> AgentTrace` — full generator entrypoint.

## Relationship to existing patch service

Consumes `intersect_patches` and `ParsedPatch`/`PatchHunk`/`TouchedLine` types from `cli/src/services/patch.rs`. Does not introduce a separate patch model.

## Out of scope

CLI command surface, hook/runtime integration, persistence, OpenCode plugin behavior, non-MVP payload enrichment.

## See also

- [../overview.md](../overview.md)
- [../glossary.md](../glossary.md)
- [../context-map.md](../context-map.md)
