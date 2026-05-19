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
5. Emit one `Conversation` per `post_commit_patch` hunk, one `TraceFile` per `post_commit_patch` file.

## Domain types

| Type | Purpose |
|---|---|
| `HunkContributor` | Enum: `Ai`, `Mixed`, `Unknown` |
| `Contributor` | Nested per-conversation object carrying `type: HunkContributor` and optional `model_id` omitted when absent |
| `LineRange` | New-file line span with `start_line` + `end_line` |
| `Conversation` | Per-hunk entry: nested contributor + `ranges` (currently exactly one range derived from `post_commit_patch`) |
| `TraceFile` | Per-file entry: path + conversations |
| `AgentTraceVcs` | Optional top-level VCS metadata object carrying `type` + `revision` when present |
| `AgentTrace` | Top-level payload: `version`, `id`, `timestamp`, optional `vcs`, `files` |

All types are `serde`-serializable with `snake_case` field naming. `Conversation.contributor` serializes as a nested object with a JSON field named `type`; `model_id` is present only when a concrete value exists.

## Payload shape

Current output includes top-level metadata fields with this contract:

- `version` is fixed to `"0.1"`
- `id` is generated per `build_agent_trace(...)` call as a UUIDv7 string derived from the same commit-time moment used for `timestamp`
- `timestamp` is sourced from explicit commit metadata input (`AgentTraceMetadataInput.commit_timestamp`) and must be RFC 3339
- `vcs` is emitted only when explicit commit metadata input includes `AgentTraceMetadataInput.vcs_type`
- when `vcs` is emitted, `vcs.type` is sourced from the schema-aligned enum (`git | jj | hg | svn`) and `vcs.revision` is sourced from `AgentTraceMetadataInput.commit_revision`

```json
{
  "version": "0.1",
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
          "contributor": { "type": "ai", "model_id": "model-ai" },
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
- `build_agent_trace(constructed_patch, post_commit_patch, metadata) -> Result<AgentTrace>` — full generator entrypoint that validates `metadata.commit_timestamp` as RFC 3339, uses it as top-level `timestamp`, derives a UUIDv7 `id` from that same commit-time moment, and conditionally emits `vcs` only when `metadata.vcs_type` is present (mapping `vcs.type` from metadata and `vcs.revision` from `metadata.commit_revision`).

## Test fixture contract

- Golden fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json` pin deterministic literal values for top-level `id` and `timestamp`.
- Tests still validate runtime metadata behavior explicitly (`id` parses as UUIDv7 and `timestamp` equals provided commit metadata), then normalize those runtime values to the deterministic fixture literals before payload comparison.
- Because the embedded schema currently expects `contributor.model_id` as a string when present, golden/schema checks operate on a model-id-stripped comparison view, while dedicated assertions validate contributor `model_id` mapping semantics (`ai`/`mixed` populated when provenance exists, omitted when absent).

## Relationship to existing patch service

Consumes `intersect_patches` and `ParsedPatch`/`PatchHunk`/`TouchedLine` types from `cli/src/services/patch.rs`. Does not introduce a separate patch model.

## Out of scope

Standalone CLI command surface, OpenCode plugin behavior, non-MVP payload enrichment. Post-commit hook/runtime integration and persistence are owned by [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md) and [agent-trace-db.md](agent-trace-db.md).

## See also

- [../overview.md](../overview.md)
- [../glossary.md](../glossary.md)
- [../context-map.md](../context-map.md)
