# Minimal agent-trace generator seam

Rust library seam at `cli/src/services/agent_trace.rs` that produces the minimal agent-trace JSON shape from patch data and is consumed by the active post-commit hook flow before AgentTraceDb persistence.

## Contract

Given a `constructed_patch` (AI candidate) and a `post_commit_patch` (canonical source of truth):

1. Compute `intersection_patch = intersect_patches(constructed_patch, post_commit_patch)` — the touched-line overlap.
2. Compare `intersection_patch` hunks against `post_commit_patch` hunks slot-by-slot (matched by `old_start`).
3. Classify each `post_commit_patch` hunk:
   - **`ai`** — `intersection_patch` hunk exists with identical touched lines (same count, kind, `line_number`, content, order).
   - **`mixed`** — `intersection_patch` hunk exists at the same slot but content differs.
   - **`unknown`** — no `intersection_patch` hunk at the same `old_start` slot.
4. Map `Conversation.contributor.model_id` from the matched `intersection_patch` hunk when contributor type is `ai` or `mixed`; omit `model_id` when provenance is missing (`None`).
5. Emit one `Conversation` per `post_commit_patch` hunk, one `TraceFile` per `post_commit_patch` file, and one range per hunk with a deterministic `content_hash` computed from that hunk's touched-line kind/content.

## Domain types

| Type                    | Purpose                                                                                                      |
| ----------------------- | ------------------------------------------------------------------------------------------------------------ |
| `HunkContributor`       | Enum: `Ai`, `Mixed`, `Unknown`                                                                               |
| `Contributor`           | Nested per-conversation object carrying `type: HunkContributor` and optional `model_id` omitted when absent  |
| `ConversationRelated`   | Schema-aligned related-link entry shape (`type` as free-form string + `url`) for optional `conversation.related` |
| `LineRange`             | New-file line span with `start_line` + `end_line` + `content_hash`                                           |
| `Conversation`          | Per-hunk entry: nested contributor + `ranges` (currently exactly one range derived from `post_commit_patch`) + optional `related` omitted when `None` |
| `TraceFile`             | Per-file entry: path + conversations                                                                         |
| `AgentTraceVcs`         | Optional top-level VCS metadata object carrying `type` + `revision` when present                             |
| `AgentTraceTool`        | Optional top-level tool metadata object carrying optional `name` + optional `version`                        |
| `AgentTraceMetadata`    | Top-level implementation metadata object carrying SCE-owned metadata                                         |
| `AgentTraceSceMetadata` | Nested `metadata.sce` object carrying the compiled SCE CLI package `version`                                 |
| `AgentTrace`            | Top-level payload: `version`, `id`, `timestamp`, optional `vcs`, optional `tool`, `metadata`, `files`        |

All types are `serde`-serializable with `snake_case` field naming. `Conversation.contributor` serializes as a nested object with a JSON field named `type`; `model_id` is present only when a concrete value exists. `Conversation.related` is optional and omitted when `None` (`skip_serializing_if = "Option::is_none"`). `ConversationRelated` remains schema-aligned domain-model support and is not yet populated by runtime builder logic.

## Payload shape

Current output includes top-level metadata fields with this contract:

- `version` is fixed to `"0.1.0"` and remains the Agent Trace payload/schema version
- `id` is generated per `build_agent_trace(...)` call as a UUIDv7 string derived from the same commit-time moment used for `timestamp`
- `timestamp` is sourced from explicit commit metadata input (`AgentTraceMetadataInput.commit_timestamp`) and must be RFC 3339
- `vcs` is emitted only when explicit commit metadata input includes `AgentTraceMetadataInput.vcs_type`
- when `vcs` is emitted, `vcs.type` is sourced from the schema-aligned enum (`git | jj | hg | svn`) and `vcs.revision` is sourced from `AgentTraceMetadataInput.commit_revision`
- `tool` is omitted when `intersection_patch.files` is empty (no AI content overlapped with the post-commit patch) or when both `AgentTraceMetadataInput.tool_name` and `AgentTraceMetadataInput.tool_version` are `None`; when `intersection_patch.files` is non-empty and either metadata value is present, builder construction sets `AgentTrace.tool` and it serializes as `{ "name"?: string, "version"?: string }` with each nested field omitted when absent
- `metadata.sce.version` is always emitted and is sourced from `env!("CARGO_PKG_VERSION")`, the compiled `sce` CLI package version; it is implementation metadata and does not change top-level Agent Trace `version` semantics

```json
{
  "version": "0.1.0",
  "id": "01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1",
  "timestamp": "2026-04-23T10:20:30Z",
  "vcs": {
    "type": "git",
    "revision": "a0b1c2d3e4f5a6b7c8d9e0f11223344556677889"
  },
  "metadata": {
    "sce": {
      "version": "0.2.0"
    }
  },
  "files": [
    {
      "path": "src/example.ts",
      "conversations": [
        {
          "contributor": { "type": "ai", "model_id": "model-ai" },
          "ranges": [
            {
              "start_line": 10,
              "end_line": 14,
              "content_hash": "murmur3:a1b2c3d4"
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
- `range_content_hash(hunk) -> String` — internal helper that computes the serialized range-level `murmur3:<lowercase-hex>` content fingerprint from `PatchHunk.lines` using versioned, length-delimited touched-line serialization in patch order. The hash input includes touched-line kind and content, and excludes hunk positions, line numbers, file paths, trace metadata, contributor/model metadata, VCS metadata, tool metadata, and database IDs.
- `build_agent_trace(constructed_patch, post_commit_patch, metadata) -> Result<AgentTrace>` — full generator entrypoint that validates `metadata.commit_timestamp` as RFC 3339, uses it as top-level `timestamp`, derives a UUIDv7 `id` from that same commit-time moment, conditionally emits `vcs` only when `metadata.vcs_type` is present (mapping `vcs.type` from metadata and `vcs.revision` from `metadata.commit_revision`), carries optional tool metadata inputs (`metadata.tool_name`, `metadata.tool_version`) for top-level `tool` mapping, and always emits `metadata.sce.version` from the compiled package version. When `intersection_patch.files` is empty, `tool` is always `None` regardless of metadata values.

## Test fixture contract

- Golden fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json` pin deterministic literal values for top-level `id`, `timestamp`, optional `vcs`, `metadata.sce.version`, range-level `content_hash`, and expected file/conversation shapes.
- Tests validate golden fixtures and built payloads against the embedded schema, assert core runtime metadata directly (`version`, `timestamp`, optional `vcs`, and `metadata.sce.version`), and compare `vcs`, `metadata`, and `files` against fixture truth.

## Relationship to existing patch service

Consumes `intersect_patches` and `ParsedPatch`/`PatchHunk`/`TouchedLine` types from `cli/src/services/patch.rs`. Does not introduce a separate patch model.

## Out of scope

Standalone CLI command surface, OpenCode plugin behavior, non-MVP payload enrichment. Post-commit hook/runtime integration and persistence are owned by [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md) and [agent-trace-db.md](agent-trace-db.md).

## See also

- [../overview.md](../overview.md)
- [../glossary.md](../glossary.md)
- [../context-map.md](../context-map.md)
