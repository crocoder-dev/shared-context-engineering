# Structured Patch Service

`cli/src/services/structured_patch.rs` owns synchronous, pure conversion from structured editor hook payloads into the canonical `ParsedPatch` model from `cli/src/services/patch.rs`.

## Current scope

- Supported source: Claude `PostToolUse` structured payloads.
- Supported tools:
  - `Write` payloads with non-empty `tool_response.structuredPatch` / `structured_patch` hunks.
  - `Write` create payloads that omit usable structured hunks and provide `tool_input.content`.
  - `Edit` payloads with `tool_response.structuredPatch` / `structured_patch` hunks.
- Output: `ClaudeStructuredPatch` with `session_id`, `ParsedPatch`, fixed caller-provided `time`, `tool_name="claude"`, and nullable `tool_version`.
- Failure mode: `ClaudeStructuredPatchDerivationResult::Skipped(...)` with deterministic skip reasons for unsupported events/tools/payload shapes or missing required fields.

## ParsedPatch contract

- Write payloads prefer structured hunks when present, including updates with non-null `tool_response.originalFile` / `original_file`.
- Write content fallback accepts only `tool_input.content`; missing content skips with `MissingFileContent` when no structured hunks are usable.
- Write-create content fallback produces `FileChangeKind::Added` file entries.
- Empty Write content produces an added file with no hunks.
- Write and Edit structured hunks produce `FileChangeKind::Modified` file entries.
- File paths come from `tool_input.file_path` / `tool_input.filePath`, with Write also accepting `tool_response.file_path` / `tool_response.filePath`; structured hunks are not path sources.
- Structured hunk context lines advance old and new line counters but are not stored as touched lines.
- Added/removed structured lines become `TouchedLineKind::Added` / `TouchedLineKind::Removed` entries with line numbers derived from hunk starts.
- The service does not render unified-diff text; downstream consumers can serialize or intersect `ParsedPatch` directly.

## Runtime wiring status

The module is wired into `sce hooks diff-trace` for Claude payload classification at intake (T04): when `hook_event_name` is present and the event is a supported `PostToolUse` (`Write` structured update, `Write` content create fallback, or `Edit` structured patch), the raw JSON is persisted as a `structured` payload type in `diff_traces` without conversion to unified-diff text. Unsupported Claude events (non-`PostToolUse`, unsupported tools) produce deterministic no-op results. OpenCode normalized payloads continue to be stored as `patch` payloads unchanged.

Post-commit parsing dispatch through `structured_patch.rs` is implemented (T05): `AgentTraceDb::recent_diff_trace_patches` now reads `payload_type` from each `diff_traces` row and dispatches `patch` rows through existing `parse_patch` while dispatching `structured` rows through `derive_claude_structured_patch` at read time, producing `ParsedPatch` for both paths before hunk `model_id` injection and downstream combine/intersect operations.

## Test status

Golden fixture coverage lives in `cli/src/services/structured_patch/tests.rs` as `claude_derivation_golden_tests`. The test discovers all scenarios under `cli/src/services/structured_patch/fixtures/`, validates the expected eight scenarios, and asserts derived `ParsedPatch` equality against `parse_patch(expected.patch)` with fixed time/tool-version inputs. Current Write coverage includes structured update hunks plus content fallback cases. No generated helper tests are kept for the T01 derivation slice.

## See also

- [patch-service.md](patch-service.md)
- [agent-trace-hooks-command-routing.md](../sce/agent-trace-hooks-command-routing.md)
- [context-map.md](../context-map.md)
