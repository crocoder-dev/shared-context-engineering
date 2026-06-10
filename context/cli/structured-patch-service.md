# Structured Patch Service

`cli/src/services/structured_patch.rs` owns synchronous, pure conversion from structured editor hook payloads into the canonical `ParsedPatch` model from `cli/src/services/patch.rs`.

## Current scope

- Supported source: Claude `PostToolUse` structured payloads.
- Supported tools:
  - `Write` create payloads where `tool_response.originalFile` / `original_file` is `null`.
  - `Edit` payloads with `tool_response.structuredPatch` / `structured_patch` hunks.
- Output: `ClaudeStructuredPatch` with `session_id`, `ParsedPatch`, fixed caller-provided `time`, `tool_name="claude"`, and nullable `tool_version`.
- Failure mode: `ClaudeStructuredPatchDerivationResult::Skipped(...)` with deterministic skip reasons for unsupported events/tools/payload shapes or missing required fields.

## ParsedPatch contract

- Write-create payloads produce `FileChangeKind::Added` file entries.
- Empty Write content produces an added file with no hunks.
- Edit structured hunks produce `FileChangeKind::Modified` file entries.
- Structured hunk context lines advance old and new line counters but are not stored as touched lines.
- Added/removed structured lines become `TouchedLineKind::Added` / `TouchedLineKind::Removed` entries with line numbers derived from hunk starts.
- The service does not render unified-diff text; downstream consumers can serialize or intersect `ParsedPatch` directly.

## Runtime wiring status

The module is not wired into `sce hooks diff-trace` yet. Current hook runtime still accepts normalized diff-trace JSON with raw diff text. The `diff_traces` table now supports a `payload_type` discriminator (`patch` for `OpenCode` unified-diff payloads, `structured` for `Claude` `PostToolUse` payloads) so structured payloads can be persisted and later parsed through `structured_patch.rs` during post-commit processing. Runtime intake wiring is planned in T04; post-commit parsing dispatch is planned in T05.

## Test status

Golden fixture coverage lives in `cli/src/services/structured_patch/tests.rs` as `claude_derivation_golden_tests`. The test discovers all scenarios under `cli/src/services/structured_patch/fixtures/`, validates the expected eight scenarios, and asserts derived `ParsedPatch` equality against `parse_patch(expected.patch)` with fixed time/tool-version inputs. No generated helper tests are kept for the T01 derivation slice.

## See also

- [patch-service.md](patch-service.md)
- [agent-trace-hooks-command-routing.md](../sce/agent-trace-hooks-command-routing.md)
- [context-map.md](../context-map.md)
