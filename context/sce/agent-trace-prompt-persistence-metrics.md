# Agent Trace prompt persistence and metrics

T05 implements post-commit prompt persistence for `agent-trace-prompt-capture`.

## Current-state contract

- Policy entrypoint: `cli/src/services/hooks.rs` -> `build_post_commit_input` + `load_post_commit_prompt_records` + `finalize_post_commit_trace`.
- Prompt rows are persisted only to the local Agent Trace DB `prompts` table; prompts are not mirrored into git notes.
- Prompt persistence reuses the existing post-commit DB write path by attaching prompt rows to `PersistedTraceRecord` and writing them through `LocalDbTraceRecordStore`.
- Retry-queue entries now carry prompt payloads so a DB retry can replay prompt inserts along with the trace record.

## Prompt metric derivation

- Prompt source of truth for commit persistence is the finalized pre-commit checkpoint artifact at Git path `sce/pre-commit-checkpoint.json`.
- Each checkpoint prompt may carry `transcript_path`, captured by the Claude `UserPromptSubmit` hook and preserved through checkpoint finalization.
- `tool_call_count` is derived from the Claude transcript JSONL by counting assistant `message.content[]` items with `type = tool_use` whose event timestamps fall within the prompt window.
- Prompt windows are computed as:
  - current prompt `captured_at`
  - next prompt `captured_at`, or commit time for the last prompt
- `duration_ms` is the non-negative millisecond delta across that window.

## Truncation and persistence rules

- Prompt text is truncated at `10 * 1024` bytes before DB insertion.
- Truncation preserves UTF-8 boundaries and stores:
  - truncated `prompt_text`
  - original byte length in `prompt_length`
  - `is_truncated = true`
- If the checkpoint or transcript cannot be read, the runtime keeps post-commit fail-open behavior; prompt metrics are best-effort and do not block commit completion.
- Existing commit-level idempotency still applies: if the trace record already exists for the commit/idempotency key, the DB write path returns `AlreadyExists` and prompt rows are not duplicated.

## Verification coverage

- `cargo test post_commit_finalization`
- `cargo test load_post_commit_prompt_records`
- `cargo test load_pending_prompts`
- `cargo test pre_commit_finalization`
- `cargo test prompt_capture_flow_persists_and_queries_end_to_end`

## Related context

- `context/sce/agent-trace-prompt-capture-hook.md`
- `context/sce/agent-trace-prompt-query-command.md`
- `context/sce/agent-trace-pre-commit-staged-checkpoint.md`
- `context/sce/agent-trace-post-commit-dual-write.md`
- `context/plans/agent-trace-prompt-capture.md`
