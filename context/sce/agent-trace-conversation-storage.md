# Agent Trace Conversation Storage (Local DB)

## Scope
- Plan: `agent-trace-conversation-storage` (T01)
- Schema lives in `cli/src/services/local_db.rs` via `apply_core_schema_migrations`.
- Adds local persistence for conversation events linked to commits.

## Tables
- `conversations`: repository-scoped conversation URL + source (existing table, reused).
- `conversation_events`: per-commit event rows linked to `commits` and `conversations`.

## conversation_events columns
- `commit_id` (FK `commits`), `conversation_id` (FK `conversations`), `event_index` (ordering)
- `role`, `event_type`, `content_text`, `payload_json`, `captured_at`, `created_at`
- Uniqueness: `UNIQUE(conversation_id, commit_id, event_index)`

## Indexes
- `idx_conversation_events_commit`
- `idx_conversation_events_conversation`
- `idx_conversation_events_commit_index`
- `idx_conversation_events_captured`

## Notes
- Post-commit runtime now ingests local prompt/transcript artifacts and persists conversation rows + events.

## Ingestion (post-commit)
- Source: `.git/sce/prompts.jsonl` plus optional `transcript_path` entries in the pre-commit checkpoint.
- Conversation URL: uses `conversation_url` when present; otherwise falls back to `sce://local-hooks/<commit_sha>`.
- Transcript parsing: JSONL lines are stored as raw `payload_json`; metadata fields (`role`, `event_type`, `content_text`, `captured_at`) are extracted best-effort.
- Persistence: `conversations` rows are upserted by `(repository_id, url)` and `conversation_events` are inserted with `event_index` (line-order) per commit.

## Range linkage
- During trace persistence, conversation URLs from Agent Trace ranges are upserted into `conversations` to guarantee joins from `trace_ranges` → `conversations`.

## Health checks
- Local DB health check now verifies `conversations` and `conversation_events` are readable.

## Related context
- `agent-trace-core-schema-migrations.md`
- `agent-trace-reconciliation-schema-ingestion.md`
