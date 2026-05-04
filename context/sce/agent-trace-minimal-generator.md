# Minimal agent-trace generator seam

Library-focused Rust seam at `cli/src/services/agent_trace.rs` that produces the minimal agent-trace JSON shape from patch data and exposes the pure patch-intersection builder consumed by post-commit persistence wiring.

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
| `AgentTrace` | Top-level payload: `version`, `id`, `timestamp`, `files` |
| `IntersectionSourcePatch` | Raw stored diff-trace patch input: `id` provenance plus raw unified-diff string |
| `PatchIntersectionBuildResult` | Serialized `ParsedPatch` intersection JSON plus ordered source diff-trace IDs |
| `PatchIntersectionBuildError` | Deterministic errors for empty source diffs, invalid source/post-commit patch data, or serialization failure |

All types are `serde`-serializable with `snake_case` field naming. `Conversation.contributor` serializes as a nested object with a JSON field named `type`.

## Payload shape

Current output includes top-level metadata fields with this contract:

- `version` is fixed to `"0.1"`
- `id` is generated per `build_agent_trace(...)` call as a UUIDv7 string derived from the same commit-time moment used for `timestamp`
- `timestamp` is sourced from explicit commit metadata input (`AgentTraceMetadataInput.commit_timestamp`) and must be RFC 3339

```json
{
  "version": "0.1",
  "id": "01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1",
  "timestamp": "2026-04-23T10:20:30Z",
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
- `build_agent_trace(constructed_patch, post_commit_patch, metadata) -> Result<AgentTrace>` — full generator entrypoint that validates `metadata.commit_timestamp` as RFC 3339, uses it as top-level `timestamp`, and derives a UUIDv7 `id` from that same commit-time moment.
- `build_patch_intersection_json(source_patches, post_commit_patch) -> Result<PatchIntersectionBuildResult, PatchIntersectionBuildError>` — pure helper for the intersection-persistence flow. It requires at least one ordered source patch, parses each raw stored diff with `parse_patch(...)`, combines parsed sources with `combine_patches(...)`, parses the canonical post-commit patch, computes `intersect_patches(...)`, serializes the resulting `ParsedPatch` as compact JSON, and returns source diff-trace IDs in the same order used for combination.

## Patch intersection builder

- The helper has no database, git, hook-routing, plugin, or full `AgentTrace` side effects; `sce hooks post-commit` owns those orchestration concerns when it calls the helper.
- Source patch order is significant and preserved in `PatchIntersectionBuildResult.source_diff_trace_ids` for later `patch_intersections.source_diff_trace_ids` insertion.
- Empty source lists fail as `PatchIntersectionBuildError::EmptySourceDiffs`.
- Invalid stored diff patches fail as `SourcePatchParse { id, message }`, preserving the source `diff_traces.id` for diagnostics.
- Invalid post-commit patch data fails as `PostCommitPatchParse { message }`.
- The serialized `intersection_json` is a `ParsedPatch` JSON payload, not full Agent Trace JSON.

## Test fixture contract

- Golden fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json` pin expected file/conversation output for minimal Agent Trace reconstruction tests.
- Patch-intersection builder tests assert deterministic compact `ParsedPatch` JSON serialization and deterministic error variants/messages for invalid or empty source inputs.

## Relationship to existing patch service

Consumes `parse_patch`, `combine_patches`, `intersect_patches`, and `ParsedPatch`/`PatchHunk`/`TouchedLine` types from `cli/src/services/patch.rs`. Does not introduce a separate patch model.

## Out of scope

Full `AgentTrace` persistence, OpenCode plugin behavior, non-MVP payload enrichment, retry/backfill behavior, and direct side effects inside the helper itself.

## See also

- [../overview.md](../overview.md)
- [../glossary.md](../glossary.md)
- [../context-map.md](../context-map.md)
