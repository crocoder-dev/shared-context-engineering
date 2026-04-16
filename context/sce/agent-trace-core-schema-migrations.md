# Local DB SQLite milestone-1 schema baseline

## Scope

- Current state after `opencode-local-agent-trace-db-m1` tasks `T01`, `T02`, and `T03`.
- Defines the canonical local Agent Trace SQLite persistence baseline in `cli/src/services/local_db.rs`.
- Covers schema bootstrap, typed persistence/query helpers, submit-time orchestration helpers, and the active submit bridge.
- Real prompt submit-path wiring is active through the OpenCode plugin `chat.message` bridge invoking `sce trace append-prompt`.

## Code ownership

- Canonical persistence module: `cli/src/services/local_db.rs`.
- Module export seam: `cli/src/services/mod.rs`.
- Runtime dependency surface: `cli/Cargo.toml` (`rusqlite`, `uuid`).
- Submit bridge command seam: `cli/src/services/trace.rs` (`sce trace append-prompt`).
- OpenCode runtime seam: `config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts` `chat.message` hook.

## Current contract

- `init_db(repository_root)` resolves repo-local `.sce/local.db`, creates parent directories, opens SQLite, enables foreign keys, and runs idempotent schema bootstrap.
- Schema bootstrap (`SCHEMA_SQL`) creates milestone tables:
  - `sessions`
  - `conversations`
  - `prompts`
  - `assistant_messages`
  - `file_observations`
  - `file_ranges`
  - `trace_exports`
- Schema bootstrap also creates deterministic lookup indexes for session/conversation and conversation/path access patterns.
- IDs are UUID v4 strings.
- Timestamps are RFC3339 UTC strings.
- Prompt rows include deterministic SHA-256 hash material (`prompt_sha256`) owned by the persistence module.
- Submit-time orchestration helpers provide one-call lazy persistence flow:
  - `ensure_db_initialized(repository_root)` wraps idempotent DB bootstrap.
  - `ensure_active_session()` reuses the latest open session (`ended_at IS NULL`) or creates one when none exists.
  - `ensure_active_conversation(session_id)` reuses the latest open conversation for the session or creates one when none exists.
  - `append_prompt_with_active_context(prompt_text)` persists the next prompt sequence in the active conversation.
  - `append_prompt_with_auto_init(repository_root, prompt_text)` combines lazy init + active-session/conversation ensure + prompt append.

## Typed API surface (T01/T02)

- Initialization:
  - `init_db`
  - `ensure_db_initialized`
  - `append_prompt_with_auto_init`
- Writes:
  - `create_session`
  - `end_session`
  - `ensure_active_session`
  - `create_conversation`
  - `ensure_active_conversation`
  - `append_prompt`
  - `append_prompt_with_active_context`
  - `append_assistant_message`
  - `record_file_observation`
  - `record_file_range`
  - `record_trace_export`
- Reads:
  - `get_conversation_prompts`
  - `get_conversation_ranges`
  - `get_trace_exports`
  - `get_minimal_trace_inputs`

## Explicit non-goals in this slice

- No git-hook runtime persistence wiring (`pre-commit`/`commit-msg`/`post-commit`/`post-rewrite` remain out of scope for prompt submit persistence).
- No trace export command surface.

## Verification evidence

- `nix flake check`
- `nix run .#pkl-check-generated`
- `cli/src/services/local_db.rs` tests validate:
  - schema + index creation
  - CRUD/query round trips
  - lazy DB auto-create on first submit helper call
  - first-submit create path and subsequent-submit active session/conversation reuse
  - UUID v4 ID shape
  - RFC3339 UTC timestamp shape

## Related context

- `context/plans/opencode-local-agent-trace-db-m1.md`
- `context/overview.md`
- `context/architecture.md`
- `context/glossary.md`
