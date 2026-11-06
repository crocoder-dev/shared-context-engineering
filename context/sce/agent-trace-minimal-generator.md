# Minimal agent-trace generator seam

Rust library seam at `cli/src/services/agent_trace.rs` that produces the minimal agent-trace JSON shape from patch data and is consumed by the active post-commit hook flow before AgentTraceDb persistence.

## Contract

Given per-row direct candidate patches, mutation lineage evidence, and a `post_commit_patch` (canonical source of truth):

1. Resolve every `post_commit_patch` touched line independently with the precedence in [../cli/mutation-trace-agent-attribution.md](../cli/mutation-trace-agent-attribution.md#evidence-precedence): AI-exclusive mutation lineage wins; non-exclusive mutation lineage keeps the line non-AI; otherwise a fresh, exact `(file, kind, line_number, content)`, single-session direct candidate attributes it; otherwise the line is unattributed. Historical `(kind, content)` similarity never attributes a line.
2. Classify each `post_commit_patch` hunk from its attributed lines:
   - **`ai`** — every touched line in the hunk is attributed.
   - **`mixed`** — a non-empty proper subset of the hunk's touched lines is attributed.
   - **`unknown`** — no touched line in the hunk is attributed.
3. Map `Conversation.contributor.model_id` for `ai` / `mixed` hunks only when every attributed line has the same known model. A mutation-attributed line uses its scope model, falling back to a same-session exact direct candidate's model.
4. Derive optional `conversation.related` entries from the sessions of attributed lines only, emitted as `{ "type": "session", "url": "https://sce.crocoder.dev/sessions/<session_id>" }`, deduplicated and deterministically ordered. Overridden or conflicting direct candidates never appear.
5. Derive the top-level `tool` object only from the direct sources that support finally accepted lines (after mutation reconciliation). It is omitted when no direct source survived, when surviving sources name different tools, or when any surviving source lacks a tool name; `version` is kept only when every surviving source reports the same one. Row order never picks a winner, and rejected or overridden sources never contribute.
6. Emit one `Conversation` per `post_commit_patch` hunk, each carrying the trace lookup `url`, one `TraceFile` per `post_commit_patch` file, and one range per hunk with a deterministic `content_hash` computed from that hunk's touched-line kind/content.

## Separated direct/mutation evidence

`build_agent_trace_from_evidence(evidence, post_commit_patch, metadata)` is the only supported generator entrypoint. `AgentTraceEvidence` keeps the sources separate:

- `repository_root` — `RepositoryRoot` anchoring attribution-grade file identity (`agent_trace/repository_path.rs`).
- `direct_sources` — one `DirectEvidenceSource` per freshness-bounded `diff_traces` row (patch, optional absolute `base_dir`, `tool_name`, `tool_version`), never combined, so conflicting sessions, duplicate occurrences, and per-row tool metadata stay attached until resolution. Matching lives in `agent_trace/evidence.rs`.

Only *accepted* direct evidence may influence authorship, co-author eligibility, or tool provenance: a candidate is accepted when its file resolves to the same repository identity as the target file (`agent_trace/repository_path.rs`), its touched line matches exactly on `(kind, line_number, content)`, and every matching candidate for that line comes from one session. Conflicting candidates are rejected everywhere.
- `mutation_ai_patch` — target-shaped committed lines that causal lineage attributed to an AI-exclusive scope, with resolved scope provenance.
- `mutation_non_ai_patch` — target-shaped committed lines that lineage resolved to a non-exclusive transition; these block direct attribution.

## Domain types

| Type                    | Purpose                                                                                                      |
| ----------------------- | ------------------------------------------------------------------------------------------------------------ |
| `HunkContributor`       | Enum: `Ai`, `Mixed`, `Unknown`                                                                               |
| `AgentTraceEvidence`    | Builder input: `repository_root`, borrowed per-row `direct_sources`, target-shaped `mutation_ai_patch` (with scope provenance), and `mutation_non_ai_patch` |
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

All types are `serde`-serializable with `snake_case` field naming. `Conversation.url` is always serialized as `https://sce.crocoder.dev/conversations/{agent_trace.id}` for the generated top-level trace ID. `Conversation.contributor` serializes as a nested object with a JSON field named `type`; `model_id` is present only when every attributed line in the hunk has one agreed concrete value. `Conversation.related` is optional and omitted when `None` (`skip_serializing_if = "Option::is_none"`) and populated from the sessions of attributed lines as deduplicated session links.

The persisted Agent Trace path is covered by
`cli/src/services/hooks/commit_hooks/attribution_tests.rs`: lineage-attributed
hunks retain their resolved model and canonical session link, stale or
overridden direct candidates never reach the payload, and mutation protocol
attribution is not itself scope provenance.

## Payload shape

Current output includes top-level metadata fields with this contract:

- `version` is fixed to `"0.1.0"` and remains the Agent Trace payload/schema version
- `id` is generated per `build_agent_trace_from_evidence(...)` call as a UUIDv7 string derived from the same commit-time moment used for `timestamp`
- `timestamp` is sourced from explicit commit metadata input (`AgentTraceMetadataInput.commit_timestamp`) and must be RFC 3339
- `vcs` is emitted only when explicit commit metadata input includes `AgentTraceMetadataInput.vcs_type`
- when `vcs` is emitted, `vcs.type` is sourced from the schema-aligned enum (`git | jj | hg | svn`) and `vcs.revision` is sourced from `AgentTraceMetadataInput.commit_revision`
- `tool` is derived from accepted direct sources only (see rule 5 above); `AgentTraceMetadataInput` no longer carries tool metadata. When emitted it serializes as `{ "name": string, "version"?: string }`
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

- `range_content_hash(hunk) -> String` — internal helper that computes the serialized range-level `murmur3:<lowercase-hex>` content fingerprint from `PatchHunk.lines` using versioned, length-delimited touched-line serialization in patch order. The hash input includes touched-line kind and content, and excludes hunk positions, line numbers, file paths, trace metadata, contributor/model metadata, VCS metadata, tool metadata, and database IDs.
- `build_agent_trace_from_evidence(evidence: AgentTraceEvidence, post_commit_patch, metadata) -> Result<AgentTrace>` — separated-evidence entrypoint (see [Separated direct/mutation evidence](#separated-directmutation-evidence)): identical top-level metadata behavior, classifies each hunk from the union of direct and mutation-derived AI coverage, unions direct/mutation related sessions, and emits a model only when the present evidence sources agree under the combined rule; top-level `tool` is derived from the direct sources supporting finally accepted lines.

## Test fixture contract

- Golden fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json` pin deterministic literal values for top-level `id`, `timestamp`, optional `vcs`, `metadata.sce.version`, `metadata.sce.line_changes`, per-conversation `url`, range-level `content_hash`, and expected file/conversation shapes.
- Evidence fixtures (`direct_only`, `exclusive_without_direct`, `direct_plus_mutation`, `partial_combined`, `newer_nonexclusive_blocks`, `mutation_only_no_provenance`) pair `direct.patch` + `mutation_ai.patch` + `post_commit.patch` and drive `build_agent_trace_from_evidence`, pinning mutation-only coverage with and without resolved provenance, the combined model-agreement and session-union rules, the absence of fabricated metadata when provenance is unknown, and the direct-only case (empty `mutation_ai_patch`), whose classification and provenance match the direct-intersection rule.
- Tests validate golden fixtures and built payloads against the embedded schema, assert core runtime metadata directly (`version`, `timestamp`, optional `vcs`, and `metadata.sce.version`), and compare `vcs`, optional `tool`, `metadata.sce.line_changes`, and normalized `files` against fixture truth. Expected fixture URLs are normalized to the runtime `AgentTrace.id` before the existing file-shape comparison because UUIDv7 generation includes non-deterministic bits.

## Relationship to existing patch service

Consumes `ParsedPatch`/`PatchHunk`/`TouchedLine` types from `cli/src/services/patch.rs`; file identity comes from `agent_trace/repository_path.rs`. It does not introduce a separate patch model.

## Out of scope

Standalone CLI command surface, OpenCode plugin behavior, non-MVP payload enrichment. Post-commit hook/runtime integration and persistence are owned by [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md) and [agent-trace-db.md](agent-trace-db.md).

## See also

- [../overview.md](../overview.md)
- [../glossary.md](../glossary.md)
- [../context-map.md](../context-map.md)
