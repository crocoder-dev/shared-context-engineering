# Minimal agent-trace generator seam

Rust library seam at `cli/src/services/agent_trace.rs` that produces the minimal agent-trace JSON shape from patch data and is consumed by the active post-commit hook flow before AgentTraceDb persistence.

## Contract

Given per-row direct candidate patches, mutation lineage evidence, and a `post_commit_patch` (canonical source of truth):

1. Resolve every `post_commit_patch` touched line independently to exactly one final `LineAttribution` decision with the precedence in [../cli/mutation-trace-agent-attribution.md](../cli/mutation-trace-agent-attribution.md#evidence-precedence): AI-exclusive mutation lineage wins (`Ai`); non-exclusive mutation lineage keeps the line non-AI (`NonExclusive`); otherwise a fresh, exact `(file, kind, line_number, content)`, single-session direct candidate attributes it (`Ai`); otherwise the line is unresolved (`Unknown`). Historical `(kind, content)` similarity never attributes a line. Hunk classification, metadata, and `metadata.sce.line_changes` all derive from this one decision map.
2. Build revision-positioned ranges from those same decisions (see [Ranges and conversations](#ranges-and-conversations)). Git hunks are not consulted for ownership, range boundaries, contributors, or hashes.
3. Map contributor `model_id` only from accepted provenance of the lines a range covers. A mutation-attributed line uses its scope model, falling back to a same-session exact direct candidate's model.
4. Derive optional `conversation.related` only from the accepted session of the conversation's own lines, emitted as `{ "type": "session", "url": "https://sce.crocoder.dev/sessions/<session_id>" }`. Unresolved conversations never inherit neighbouring sessions; overridden or conflicting direct candidates never appear.
5. Derive the top-level `tool` object only from the direct sources that support finally accepted lines (after mutation reconciliation). It is omitted when no direct source survived, when surviving sources name different tools, or when any surviving source lacks a tool name; `version` is kept only when every surviving source reports the same one. Row order never picks a winner, and rejected or overridden sources never contribute.
6. Emit one `TraceFile` per committed file that has surviving added lines, with conversations ordered by first line and every range carrying the trace lookup `url` of its conversation and a `content_hash` of its exact committed bytes.

## Ranges and conversations

**Why hunks are not authorship boundaries.** A Git hunk is a diff presentation artifact: it bundles context lines, removals, and additions from possibly different authors. Earlier output emitted one range per hunk from `new_start`/`new_count`, which included unchanged context, placed removed-only hunks at old-file coordinates, and labelled any hunk with partial AI coverage `mixed`. SCE now derives every range from individual committed lines.

**Range construction** (`agent_trace/ranges.rs`, wired by `build_trace_files` in `agent_trace.rs`):

1. For each committed file, enumerate the canonical touched lines and keep only `Added` lines; their `TouchedLine.line_number` is the post-commit (new-file) position. Removed lines and context lines never occupy a range.
2. Look up each line's final `LineAttribution` and map it to a `RangeAttribution`: `Ai { session_id, model_id }` (empty session treated as absent), `NonExclusive`, or `Unknown`. `NonExclusive` and `Unknown` both serialize as `{ "type": "unknown" }` but stay distinct internally. `NonExclusive` is never reported as `human`.
3. Validate positions: line numbers must be ≥ 1 and strictly increasing; when the committed blob is available each line must exist there and carry the text the patch recorded (ignoring its `\n`/`\r\n` terminator). Any disagreement fails trace generation with `Invalid Agent Trace range position for '<path>' ...` instead of emitting an invalid coordinate.
4. Coalesce contiguous lines whose `RangeAttribution` is identical (same classification, same accepted session, same model including absence). Ranges never cross files, gaps, sessions, models, or decision kinds; hunk boundaries play no role, so re-partitioning hunks cannot change output.
5. Group ranges of a file into conversations by provenance: one per accepted session; one per model for session-less AI evidence; one for `NonExclusive`; one for `Unknown`. Adjacent lines from different sessions are therefore separate conversations, and different models never share fabricated model provenance.

**Contributors.**

- A homogeneous conversation carries the conversation-level contributor (`{ "type": "ai", "model_id"? }` or `{ "type": "unknown" }`) and no range overrides.
- A conversation that is one established session but whose ranges carry different models keeps a single entry (splitting it would misrepresent one session as several) with conversation contributor `{ "type": "ai" }` and an explicit `contributor` override on every range.
- If a conversation had no common classification, the conversation-level contributor would be omitted; range overrides carry the attribution.
- SCE never generates `mixed` (partial AI coverage of a hunk is not evidence of mixed authorship) and never generates `human`. Both remain valid standard types and are read back from existing records.

**Git path identity** (`patch/git_path.rs`). Git C-quotes a path that contains control characters, `"`, `\`, or (with `core.quotePath`, the default) any byte ≥ 0x80, writing each non-ASCII byte as its own `\ooo` octal escape. The patch parser decodes quoted names in `diff --git` headers, `---`/`+++` lines, and `rename`/`copy from`/`to` lines with one decoder: escapes become bytes first, then the whole path is validated as UTF-8 (never per escape, never lossily). Malformed escapes, unterminated quotes, NUL, or non-UTF-8 results fail parsing with `invalid path in ...` rather than inventing an identity. The `diff --git` header is split without looking inside quoted names; two unquoted names are split at the unique midpoint where they agree, so a name containing ` b/` is safe, and an ambiguous rename header must be resolved by the following `---`/`+++` or `rename` lines or parsing fails. Git's `a/`/`b/` prefixes are stripped only after decoding, `/dev/null` still marks created and deleted files, and on unquoted `---`/`+++` lines only the tab suffix Git appends (to names containing a space) is dropped, so significant trailing spaces survive. The decoded repository-relative path is the identity used by attribution lookup, `files[].path`, and `git ls-tree`; repository-root normalization and boundary checks are unchanged and run after decoding.

**Content hashes** (`agent_trace/committed_content.rs`). For an emitted inclusive range `[start_line, end_line]` the hash input is exactly the contiguous bytes of those lines in the committed blob at `vcs.revision`, including each line's own terminator (LF or CRLF) and excluding a final newline that does not exist. Algorithm: Murmur3 x86 32-bit, seed `0`, formatted `murmur3:<8 lowercase hex digits>`. Paths, line numbers, hunk headers, `+`/`-` markers, attribution, session/model/trace IDs, and any version prefix are not part of the input, so identical byte spans hash identically wherever they appear. Bytes are never reconstructed from `TouchedLine.content`, which has lost terminators.

**Committed-blob retrieval.** `build_agent_trace_from_evidence` takes a `CommittedFileReader`. The post-commit hook passes `GitCommittedFiles::new(repository_root, commit_oid)` for the captured commit ID (the same ID whose patch is parsed, captured with `git show --no-ext-diff --no-textconv <oid> --` so patch lines describe raw blobs). It runs `git --literal-pathspecs ls-tree -z --full-tree <oid> -- <path>` and `git cat-file blob <object>`: committed objects only, no working tree, index, network, or object writes. Each path is read at most once per trace (`CommittedFileCache`), and only for files with surviving added lines. The NUL-delimited `ls-tree` listing is parsed strictly: every record must be `<mode> <type> <hex-object>\t<path>`, the path must equal the requested path byte for byte (no prefix or suffix matching), and more than one exact match is an error.

`CommittedFileReader::read_committed_file` returns `Result<CommittedFile, CommittedFileReadError>`. Mode-aware classification (`GitEntryKind`) checks both the Git mode and the object type:

| Mode | Type | Kind | Result |
| --- | --- | --- | --- |
| `100644`, `100755` | `blob` | regular / executable file | `Blob(bytes)` |
| `120000` | `blob` | symlink | `NotLineAddressable` (the target string is not source) |
| `160000` | `commit` | gitlink / submodule | `NotLineAddressable` |
| `040000` | `tree` | directory | `NotLineAddressable` |
| anything else | | | `MalformedTreeEntry` error |

Outcomes and policy:

- `Blob(bytes)` — every added line is validated against the blob and every range gets a hash.
- `NotLineAddressable` — the file emits no ranges and no hashes; its touched lines still count in `metadata.sce.line_changes`.
- `Missing` — the path is absent from the recorded revision. A file with surviving additions must exist there, so this fails trace generation with `Invalid Agent Trace file '<path>': ... absent from revision '<rev>'`; the file is neither silently dropped nor emitted unverified.
- `Unverified` — an explicit caller decision that no committed content is available (for example `UnverifiedCommittedFiles` in tests, or a future non-Git caller). Ranges keep their patch positions and omit `content_hash`, which the schema allows; nothing is fabricated, and such positions are **not** independently verified.
- `Err(CommittedFileReadError)` — an operational failure, kept distinct from a missing file: `TreeLookupFailed` (`git ls-tree` could not run or exited non-zero), `MalformedTreeEntry` (unparseable, ambiguous, or unsupported mode/type entry), or `BlobReadFailed` (`git cat-file` failed, for example a missing object). Each names the path, revision, and Git diagnostic, and fails trace generation.

`GitCommittedFiles` never returns `Unverified`, so the production post-commit hook is strict: a missing path or Git read error aborts the flow before validation and persistence, and no hashless range is ever persisted because of a failed read. Contradictory positions fail closed as before.

Range building and hashing are pure and unit-tested with in-memory readers.

**Deletions.** Removed lines have no position at the recorded revision, so they never produce ranges, are never moved onto a nearby surviving line, and never become zero-length ranges. Their decisions still count in `metadata.sce.line_changes`. A deletion-only commit, a deleted file, or a pure rename can legitimately produce `"files": []` while its counters are non-zero. Mutation-lineage deletion attribution remains out of scope.

**Deleted `.patch` files.** A deleted `*.patch` file is an ordinary deleted file: its literal deleted lines are counted and nothing else. The former expansion of its embedded diff into synthetic `TraceFile` entries was removed because those changes cannot be verified against the recorded revision.

## Separated direct/mutation evidence

`build_agent_trace_from_evidence(evidence, post_commit_patch, metadata)` is the only supported generator entrypoint. `AgentTraceEvidence` keeps the sources separate:

- `repository_root` — `RepositoryRoot` anchoring attribution-grade file identity (`agent_trace/repository_path.rs`).
- `direct_sources` — one `DirectEvidenceSource` per freshness-bounded `diff_traces` row (patch, `path_origin`, `tool_name`, `tool_version`), never combined, so conflicting sessions, duplicate occurrences, and per-row tool metadata stay attached until resolution. Matching lives in `agent_trace/evidence.rs`.

Only *accepted* direct evidence may influence authorship, co-author eligibility, or tool provenance: a candidate is accepted when its file resolves to the same repository identity as the target file (`agent_trace/repository_path.rs`; the committed target is always repository-relative), its touched line matches exactly on `(kind, line_number, content)`, and every matching candidate for that line comes from one session. Conflicting candidates are rejected everywhere.

File identity requires an established coordinate system (`DiffTracePathOrigin`, see [agent-trace-db.md](agent-trace-db.md)). `RepositoryRoot::repository_path` resolves absolute paths directly and only inside the repository root; a relative path resolves as repository-relative for Git patch rows, against the recorded absolute `cwd` for Claude structured rows (then checked for repository membership, so `..` cannot escape), and is rejected for Claude rows without an absolute `cwd`. An unknown origin fails closed rather than defaulting to the repository root, so a Claude edit to `<root>/cli/src/lib.rs` recorded as `src/lib.rs` without `cwd` cannot credit `<root>/src/lib.rs`. These rules are shared by post-commit attribution, the persisted `post_commit_patch_intersections`, staged co-author detection, and accepted-source tool metadata.
- `mutation_ai_patch` — target-shaped committed lines that causal lineage attributed to an AI-exclusive scope, with resolved scope provenance.
- `mutation_non_ai_patch` — target-shaped committed lines that lineage resolved to a non-exclusive transition; these block direct attribution.

## Domain types

| Type                    | Purpose                                                                                                      |
| ----------------------- | ------------------------------------------------------------------------------------------------------------ |
| `ContributorType`       | Standard contributor enum: `Human`, `Ai`, `Mixed`, `Unknown` (SCE generates only `Ai` and `Unknown`)         |
| `AgentTraceEvidence`    | Builder input: `repository_root`, borrowed per-row `direct_sources`, target-shaped `mutation_ai_patch` (with scope provenance), and `mutation_non_ai_patch` |
| `Contributor`           | Conversation or range contributor: `type: ContributorType` and optional `model_id` omitted when absent        |
| `ConversationRelated`   | Schema-aligned related-link entry shape (`type` as free-form string + `url`) for optional `conversation.related` |
| `LineRange`             | Inclusive 1-indexed post-commit span: `start_line`, `end_line`, optional `content_hash`, optional `contributor` override |
| `Conversation`          | Ranges sharing provenance: trace lookup `url`, optional `contributor`, `ranges`, optional `related`          |
| `TraceFile`             | Per-file entry: post-commit path + conversations ordered by first line                                       |
| `CommittedFileReader`   | Seam returning `Result<CommittedFile::{Blob, NotLineAddressable, Missing, Unverified}, CommittedFileReadError>` for a post-commit path; `GitCommittedFiles` reads one exact commit and never returns `Unverified` |
| `AgentTraceVcs`         | Optional top-level VCS metadata object carrying `type` + `revision` when present                             |
| `AgentTraceTool`        | Optional top-level tool metadata object carrying optional `name` + optional `version`                        |
| `AgentTraceMetadata`    | Top-level implementation metadata object carrying SCE-owned metadata                                         |
| `AgentTraceSceMetadata` | Nested `metadata.sce` object carrying the compiled SCE CLI package `version` plus `line_changes`             |
| `LineChangeCounts`      | `{ added, removed }` `u64` touched-line counters for one per-line attribution bucket                         |
| `LineChangeAttribution` | `metadata.sce.line_changes` shape: `{ ai, non_exclusive, unknown }`, each a `LineChangeCounts`, `#[serde(default)]` |
| `AgentTrace`            | Top-level payload: `version`, `id`, `timestamp`, optional `vcs`, optional `tool`, `metadata`, `files`        |

All types are `serde`-serializable with `snake_case` field naming. `Conversation.url` is always serialized as `https://sce.crocoder.dev/conversations/{agent_trace.id}` for the generated top-level trace ID. `Conversation.contributor`, `LineRange.contributor`, `LineRange.content_hash`, and `Conversation.related` are optional and omitted when `None`; existing records with a required hash or a `mixed` contributor still deserialize.

The persisted Agent Trace path is covered by
`cli/src/services/hooks/commit_hooks/attribution_tests.rs`: lineage-attributed
ranges retain their resolved model and canonical session link, stale or
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
- `metadata.sce.line_changes` is always emitted (`{ ai, non_exclusive, unknown }`, each `{ added, removed }`) and carries exact **per-line** attribution counts: `line_change_attribution(...)` walks every touched line of the canonical `post_commit_patch` once and increments exactly one bucket from that line's final `LineAttribution` decision — the same decision map ranges are built from, never a second attribution pass and never derived from conversations or ranges. Buckets mean:
  - `ai` — accepted AI-attributed lines (AI-exclusive mutation lineage, or accepted direct evidence for an unresolved line);
  - `non_exclusive` — mutation evidence established a non-exclusive or unscoped (contended/ineligible) transition; this is not a claim of human authorship, and direct AI evidence cannot move these lines to `ai`;
  - `unknown` — attribution is unresolved (no accepted evidence, or only conflicting direct candidates).

  There is no `mixed` bucket. Counter totals exactly cover the canonical committed patch (`ai + non_exclusive + unknown` equals its added and removed touched-line counts), are independent of hunk boundaries and range presentation, never count hunk context lines, and never deduplicate identical lines. Removed lines are not yet resolved by mutation lineage, so they count as `ai` only with accepted exact direct evidence and are otherwise `unknown`. A deleted `.patch` file counts only its literal deleted lines. `#[serde(default)]` on `line_changes`, its buckets, and its parent keeps older payloads deserializing (the previous `mixed` key is ignored and `non_exclusive` defaults to zero); historical traces are not migrated
- every `Conversation.url` is the absolute URI `https://sce.crocoder.dev/conversations/{agent_trace.id}` derived from the generated top-level `AgentTrace.id`; all conversations in one payload therefore share the same URL

Example for a hunk `@@ -10,2 +10,4 @@` that removes one line, adds two session-A lines and one unresolved line, and keeps one context line: the context and removed lines produce no ranges, and nothing is `mixed`.

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
        "ai": { "added": 2, "removed": 0 },
        "non_exclusive": { "added": 0, "removed": 0 },
        "unknown": { "added": 1, "removed": 1 }
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
            { "start_line": 10, "end_line": 11, "content_hash": "murmur3:<hash of lines 10-11>" }
          ],
          "related": [
            { "type": "session", "url": "https://sce.crocoder.dev/sessions/<session-a>" }
          ]
        },
        {
          "url": "https://sce.crocoder.dev/conversations/01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1",
          "contributor": { "type": "unknown" },
          "ranges": [
            { "start_line": 12, "end_line": 12, "content_hash": "murmur3:<hash of line 12>" }
          ]
        }
      ]
    }
  ]
}
```

## Public API

- `build_agent_trace_from_evidence(evidence: AgentTraceEvidence, post_commit_patch, metadata, committed_files: &dyn CommittedFileReader) -> Result<AgentTrace>` — separated-evidence entrypoint (see [Separated direct/mutation evidence](#separated-directmutation-evidence)); resolves per-line decisions, builds counters and revision-positioned ranges from them, and derives `tool` from the direct sources supporting finally accepted lines. Fails when a range position contradicts the committed blob, when a file with added lines is `Missing` at the revision, or when the reader returns a `CommittedFileReadError`.
- `GitCommittedFiles::new(repository_root, revision)` — production `CommittedFileReader` for one exact commit.

## Test fixture contract

- Golden fixtures under `cli/src/services/agent_trace/fixtures/**/golden.json` pin the complete exported `files` structure, including exact range positions, contributors, range overrides, related sessions, and `content_hash` values, plus `metadata.sce.line_changes`. Fixture tests supply committed content through a test reader that rebuilds new-side bytes from the raw `post_commit.patch` text (exact terminators and `\ No newline at end of file`; lines outside hunks become placeholders) without the production parser. The golden hashes were cross-checked by an independent Murmur3 implementation that re-derived every range from the raw patch.
- Evidence fixtures (`direct_only`, `exclusive_without_direct`, `direct_plus_mutation`, `partial_combined`, `newer_nonexclusive_blocks`, `mutation_only_no_provenance`) pair `direct.patch` + `mutation_ai.patch` + `post_commit.patch` and drive `build_agent_trace_from_evidence`, pinning mutation-only coverage with and without resolved provenance, the absence of fabricated metadata when provenance is unknown, and the direct-only case.
- `agent_trace/tests.rs` covers: hunks with context and AI/unresolved additions (no `mixed`); additions separated by context; new, deleted, renamed, and pure-rename files; deletion-only commits; sessions and models never merged; one session with several models using range overrides; `Unknown`/`NonExclusive` never coalesced; legacy `mixed`/`human` record deserialization; Murmur3 vectors; LF/CRLF/missing-final-newline/whitespace/Unicode byte spans; hash stability across paths, positions, and hunks; explicitly unverified and non-line-addressable blobs; rejection of positions that contradict the blob; per-line counters; the `resolve_line` precedence table; and seeded randomized tests that check ranges, hashes, provenance, schema validity, and hunk re-partitioning invariance against an independently computed model.
- `agent_trace/committed_content/tests.rs` builds real Git tree entries (regular, executable, symlink, gitlink, directory, absent blob object) and covers missing paths, failed `ls-tree`/`cat-file`, malformed or ambiguous listings, mode/type agreement, and fail-closed trace building for `Missing` and every read error.
- `hooks/commit_hooks/path_identity_tests.rs` runs the real Git + Turso post-commit flow with `core.quotePath=true` over Unicode, space, tab, newline, quote, backslash, ` b/`, and trailing-space filenames plus an executable file and a symlink; it checks decoded `files[].path`, hashes against independently read blobs, direct evidence on the quoted Unicode file, symlink exclusion, and counters, and proves that a missing path, failed `ls-tree`, or missing blob persists no trace.
- `hooks/commit_hooks/attribution_tests.rs` runs the real Git + Turso post-commit flow, including a commit with CRLF, Unicode, a missing final newline, a deleted file, and a deleted `.patch` file, and checks every persisted range and hash against blobs read independently with `git cat-file`.
- Tests validate golden fixtures and built payloads against the embedded schema, assert core runtime metadata directly (`version`, `timestamp`, optional `vcs`, and `metadata.sce.version`), and compare `vcs`, optional `tool`, `metadata.sce.line_changes`, and normalized `files` against fixture truth. Expected fixture URLs are normalized to the runtime `AgentTrace.id` before the existing file-shape comparison because UUIDv7 generation includes non-deterministic bits.

## Relationship to existing patch service

Consumes `ParsedPatch`/`PatchHunk`/`TouchedLine` types from `cli/src/services/patch.rs`; file identity comes from `agent_trace/repository_path.rs`. It does not introduce a separate patch model.

## Out of scope

Standalone CLI command surface, OpenCode plugin behavior, non-MVP payload enrichment. Post-commit hook/runtime integration and persistence are owned by [agent-trace-hooks-command-routing.md](agent-trace-hooks-command-routing.md) and [agent-trace-db.md](agent-trace-db.md).

## See also

- [../overview.md](../overview.md)
- [../glossary.md](../glossary.md)
- [../context-map.md](../context-map.md)
