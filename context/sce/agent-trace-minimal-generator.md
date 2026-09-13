# Minimal agent-trace generator seam

Rust library seam at `cli/src/services/agent_trace.rs` that produces the minimal agent-trace JSON shape from patch data and is consumed by the active post-commit hook flow before AgentTraceDb persistence.

## Contract

Given a `constructed_patch` (AI candidate) and a `post_commit_patch` (canonical source of truth):

1. Compute `intersection_patch = intersect_patches(constructed_patch, post_commit_patch)` — the touched-line overlap (the direct evidence).
2. Classify each `post_commit_patch` hunk from the union of the direct `intersection_patch` hunk and, when supplied, the mutation-derived AI hunk at the same `old_start` slot (see [Separated direct/mutation evidence](#separated-directmutation-evidence)):
   - **`ai`** — every touched line in the `post_commit_patch` hunk is covered by direct evidence, mutation-derived AI evidence, or both.
   - **`mixed`** — a non-empty proper subset of the hunk's touched lines is covered.
   - **`unknown`** — no touched line in the hunk is covered.
3. With no mutation evidence this is equivalent to the direct-only slot rule, since a direct-intersection hunk always holds an ordered sub-multiset of its `post_commit_patch` hunk's touched lines: `ai` when the `intersection_patch` hunk has identical touched lines (same count, kind, `line_number`, content, order), `mixed` when that hunk exists but is a proper subset, `unknown` when no `intersection_patch` hunk shares the `old_start`. The direct-only path `build_agent_trace(...)` produces exactly this classification.
4. Map `Conversation.contributor.model_id` from the matched `intersection_patch` hunk when contributor type is `ai` or `mixed`; omit `model_id` when provenance is missing (`None`). A hunk classified `ai`/`mixed` purely through mutation-derived coverage has no matched direct hunk and therefore no `model_id`.
5. For each emitted conversation, derive optional `conversation.related` entries from non-empty `session_id` values on touched lines in the matched `intersection_patch` hunk; emit related entries as `{ "type": "session", "url": "https://sce.crocoder.dev/sessions/<session_id>" }`, deduplicated by session ID with deterministic ordering, and omit `related` when no included lines provide `session_id`. Structured diff-trace reconstruction supplies the persisted canonical `cc_...` row session on every touched line, so Claude related-session URLs use canonical persisted provenance rather than the raw payload session.
6. Emit one `Conversation` per `post_commit_patch` hunk, each carrying the trace lookup `url`, one `TraceFile` per `post_commit_patch` file, and one range per hunk with a deterministic `content_hash` computed from that hunk's touched-line kind/content.

## Separated direct/mutation evidence

`build_agent_trace_from_evidence(evidence, post_commit_patch, metadata)` is the seam that classifies from two independent AI-evidence sources:

- `evidence.direct_patch` — the reconstructed direct patch, intersected against `post_commit_patch` internally exactly as before. It is the sole source of `Conversation.contributor.model_id`, `Conversation.related` session links, and the top-level `tool` object (still omitted when the direct intersection is empty).
- `evidence.mutation_ai_patch` — a target-shaped, provenance-free set of committed touched lines that causal mutation-lineage replay attributed to AI (an AI event's line that survived every later observed tree transition into the committed tree), produced by [../cli/mutation-trace-agent-attribution.md](../cli/mutation-trace-agent-attribution.md). It only widens combined AI coverage for hunk classification and contributes no model, session, tool, or tool-version metadata.

Per hunk, a `post_commit_patch` touched line is covered when it pairs one-to-one on `(kind, line_number, content)` with a line in the direct intersection hunk or the mutation-AI hunk at the same `old_start`; the covered fraction drives `ai` / `mixed` / `unknown` as in the Contract above. `line_changes` buckets follow that combined classification.

`build_agent_trace(constructed_patch, post_commit_patch, metadata)` is retained unchanged and delegates to `build_agent_trace_from_evidence` with an empty `mutation_ai_patch`. Wiring the mutation-AI patch in at post-commit is owned by [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md).

## Domain types

| Type                    | Purpose                                                                                                      |
| ----------------------- | ------------------------------------------------------------------------------------------------------------ |
| `HunkContributor`       | Enum: `Ai`, `Mixed`, `Unknown`                                                                               |
| `AgentTraceEvidence`    | Internal builder input pairing borrowed `direct_patch` (reconstructed direct patch) and `mutation_ai_patch` (target-shaped, provenance-free mutation-AI coverage) |
| `Contributor`           | Nested per-conversation object carrying `type: HunkContributor` and optional `model_id` omitted when absent  |
| `ConversationRelated`   | Schema-aligned related-link entry shape (`type` as free-form string + `url`) for optional `conversation.related` |
| `LineRange`             | New-file line span with `start_line` + `end_line` + `content_hash`                                           |
| `Conversation`          | Per-hunk entry: trace lookup `url`, nested contributor, `ranges` (currently exactly one range derived from `post_commit_patch`), and optional `related` omitted when `None` |
| `TraceFile`             | Per-file entry: path + conversations                                                                         |
| `AgentTraceVcs`         | Optional top-level VCS metadata object carrying `type` + `revision` when present                             |
| `AgentTraceTool`        | Optional top-level tool metadata object carrying optional `name` + optional `version`                        |
| `AgentTraceMetadata`    | Top-level implementation metadata object carrying SCE-owned metadata                                         |
| `AgentTraceSceMetadata` | Nested `metadata.sce` object carrying the compiled SCE CLI package `version` plus `line_changes`             |
| `LineChangeCounts`      | `{ added, removed }` `u64` touched-line counters for one hunk-classification bucket                          |
| `LineChangeAttribution` | `metadata.sce.line_changes` shape: `{ ai, mixed, unknown }`, each a `LineChangeCounts`, `#[serde(default)]`  |
| `AgentTrace`            | Top-level payload: `version`, `id`, `timestamp`, optional `vcs`, optional `tool`, `metadata`, `files`        |

All types are `serde`-serializable with `snake_case` field naming. `Conversation.url` is always serialized as `https://sce.crocoder.dev/conversations/{agent_trace.id}` for the generated top-level trace ID. `Conversation.contributor` serializes as a nested object with a JSON field named `type`; `model_id` is present only when a concrete value exists. `Conversation.related` is optional and omitted when `None` (`skip_serializing_if = "Option::is_none"`) and populated from matched intersection-line `session_id` provenance as session links.

## Payload shape

Current output includes top-level metadata fields with this contract:

- `version` is fixed to `"0.1.0"` and remains the Agent Trace payload/schema version
- `id` is generated per `build_agent_trace(...)` call as a UUIDv7 string derived from the same commit-time moment used for `timestamp`
- `timestamp` is sourced from explicit commit metadata input (`AgentTraceMetadataInput.commit_timestamp`) and must be RFC 3339
- `vcs` is emitted only when explicit commit metadata input includes `AgentTraceMetadataInput.vcs_type`
- when `vcs` is emitted, `vcs.type` is sourced from the schema-aligned enum (`git | jj | hg | svn`) and `vcs.revision` is sourced from `AgentTraceMetadataInput.commit_revision`
- `tool` is omitted when `intersection_patch.files` is empty (no AI content overlapped with the post-commit patch) or when both `AgentTraceMetadataInput.tool_name` and `AgentTraceMetadataInput.tool_version` are `None`; when `intersection_patch.files` is non-empty and either metadata value is present, builder construction sets `AgentTrace.tool` and it serializes as `{ "name"?: string, "version"?: string }` with each nested field omitted when absent
- `metadata.sce.version` is always emitted and is sourced from `env!("CARGO_PKG_VERSION")`, the compiled `sce` CLI package version; it is implementation metadata and does not change top-level Agent Trace `version` semantics
- `metadata.sce.line_changes` is always emitted (`{ ai, mixed, unknown }`, each `{ added, removed }`) and carries exact touched-line attribution counts derived from `PatchHunk.lines` on the canonical `post_commit_patch` — the same hunk-level classification already used for `Conversation.contributor.type` (no independent second classification pass for the common per-file/per-hunk path); a `mixed` hunk's *entire* touched-line count is recorded, not just the subset also present in `intersection_patch`; the deleted-`.patch` embedded-expansion branch counts only the deleted file's own literal `post_commit_patch` hunks (classified by `old_path` against the top-level `intersection_patch`, since a deleted file's `new_path` is always empty and would otherwise collide with other deleted files in the same patch), never the embedded reconstructed hunks used to synthesize that branch's `Conversation` entries; `#[serde(default)]` on `line_changes` and its parent keeps pre-existing `metadata.sce.version`-only payloads deserializing with all-zero counts
- every `Conversation.url` is the absolute URI `https://sce.crocoder.dev/conversations/{agent_trace.id}` derived from the generated top-level `AgentTrace.id`; all conversations in one payload therefore share the same URL

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
      "version": "0.2.0",
      "line_changes": {
        "ai": { "added": 5, "removed": 0 },
        "mixed": { "added": 0, "removed": 0 },
        "unknown": { "added": 0, "removed": 0 }
      }
    }
  },
  "files": [
    {
      "path": "src/example.ts",
      "conversations": [
        {
          "url": "https://sce.crocoder.dev/conversations/01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1",
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

- `classify_hunk(post_commit_hunk, intersection_hunks) -> HunkContributor` — the direct-only slot rule, retained as a primitive; the builder itself now classifies through the internal combined direct+mutation line-coverage rule.
- `range_content_hash(hunk) -> String` — internal helper that computes the serialized range-level `murmur3:<lowercase-hex>` content fingerprint from `PatchHunk.lines` using versioned, length-delimited touched-line serialization in patch order. The hash input includes touched-line kind and content, and excludes hunk positions, line numbers, file paths, trace metadata, contributor/model metadata, VCS metadata, tool metadata, and database IDs.
- `build_agent_trace(constructed_patch, post_commit_patch, metadata) -> Result<AgentTrace>` — direct-only entrypoint, retained unchanged; delegates to `build_agent_trace_from_evidence` with an empty `mutation_ai_patch`. It validates `metadata.commit_timestamp` as RFC 3339, uses it as top-level `timestamp`, derives a UUIDv7 `id` from that same commit-time moment, derives one conversation URL from that `id`, conditionally emits `vcs` only when `metadata.vcs_type` is present (mapping `vcs.type` from metadata and `vcs.revision` from `metadata.commit_revision`), carries optional tool metadata inputs (`metadata.tool_name`, `metadata.tool_version`) for top-level `tool` mapping, and always emits `metadata.sce.version` from the compiled package version. When the direct `intersection_patch.files` is empty, `tool` is always `None` regardless of metadata values.
- `build_agent_trace_from_evidence(evidence: AgentTraceEvidence, post_commit_patch, metadata) -> Result<AgentTrace>` — separated-evidence entrypoint (see [Separated direct/mutation evidence](#separated-directmutation-evidence)): identical top-level metadata behavior, but classifies each hunk from the union of direct and mutation-derived AI coverage while keeping `model_id`, `related`, and `tool` bound to the direct intersection only.

## Test fixture contract

- Golden fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json` pin deterministic literal values for top-level `id`, `timestamp`, optional `vcs`, `metadata.sce.version`, `metadata.sce.line_changes`, per-conversation `url`, range-level `content_hash`, and expected file/conversation shapes.
- Reconstruction fixtures pair `incremental_*.patch` inputs with a `post_commit.patch` and drive `build_agent_trace`. Evidence fixtures (`direct_only`, `exclusive_without_direct`, `direct_plus_mutation`, `partial_combined`, `newer_nonexclusive_blocks`, `mutation_only_no_provenance`) instead pair `direct.patch` + `mutation_ai.patch` + `post_commit.patch` and drive `build_agent_trace_from_evidence`, pinning that mutation-only coverage classifies without fabricating `model_id`, `related`, or top-level `tool`, that direct provenance survives a direct+mutation `ai` hunk, and that the empty-mutation path is byte-identical to `build_agent_trace`.
- Tests validate golden fixtures and built payloads against the embedded schema, assert core runtime metadata directly (`version`, `timestamp`, optional `vcs`, and `metadata.sce.version`), and compare `vcs`, optional `tool`, `metadata.sce.line_changes`, and normalized `files` against fixture truth. Expected fixture URLs are normalized to the runtime `AgentTrace.id` before the existing file-shape comparison because UUIDv7 generation includes non-deterministic bits.

## Relationship to existing patch service

Consumes `intersect_patches` and `ParsedPatch`/`PatchHunk`/`TouchedLine` types from `cli/src/services/patch.rs`. Does not introduce a separate patch model.

## Out of scope

Standalone CLI command surface, OpenCode plugin behavior, non-MVP payload enrichment. Post-commit hook/runtime integration and persistence are owned by [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md) and [agent-trace-db.md](agent-trace-db.md).

## See also

- [../overview.md](../overview.md)
- [../glossary.md](../glossary.md)
- [../context-map.md](../context-map.md)
