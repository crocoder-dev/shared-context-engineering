# Agent Trace Database Schema

Table columns, lookup indexes, and triggers of the repository-scoped Agent Trace database produced by `AGENT_TRACE_REPOSITORY_MIGRATIONS`. Migration ordering, setup-only application, and upgrade notes live in [agent-trace-db.md](agent-trace-db.md).

The `diff_traces` baseline migration creates:

- `id INTEGER PRIMARY KEY`
- `time_ms INTEGER NOT NULL`
- `session_id TEXT NOT NULL`
- `patch TEXT NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (...)`
- `model_id TEXT`
- `tool_name TEXT`
- `tool_version TEXT`
- `payload_type TEXT NOT NULL DEFAULT 'patch'` — discriminator for source payload format; `patch` for `OpenCode` unified-diff payloads, `structured` for `Claude` `PostToolUse` structured payloads.

The `post_commit_patch_intersections` baseline migration creates:

- `id INTEGER PRIMARY KEY`
- `commit_id TEXT NOT NULL`
- `post_commit_time_ms INTEGER NOT NULL`
- `recent_window_cutoff_ms INTEGER NOT NULL`
- `recent_window_end_ms INTEGER NOT NULL`
- `loaded_diff_trace_count INTEGER NOT NULL CHECK (loaded_diff_trace_count >= 0)`
- `skipped_diff_trace_count INTEGER NOT NULL CHECK (skipped_diff_trace_count >= 0)`
- `intersection_patch TEXT NOT NULL`
- `created_at TEXT NOT NULL DEFAULT (...)`

The `agent_traces` baseline migration creates:

- `id INTEGER PRIMARY KEY`
- `commit_id TEXT NOT NULL`
- `commit_time_ms INTEGER NOT NULL`
- `url TEXT NOT NULL`
- `remote_url TEXT` (nullable)
- `trace_json TEXT NOT NULL`
- `agent_trace_id TEXT NOT NULL UNIQUE`
- `created_at TEXT NOT NULL DEFAULT (...)`

Migration `007_commit_patch_id` adds:

- `commit_patch_id TEXT` (nullable) — optional Git patch ID of the original commit; `NULL` for rows created before the migration or when no fingerprint is available.

The `messages` migration creates:

- `id INTEGER PRIMARY KEY`
- `session_id TEXT NOT NULL`
- `message_id TEXT NOT NULL`
- `role TEXT NOT NULL CHECK (role IN ('user', 'assistant'))`
- `generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0)`
- `created_at TEXT NOT NULL DEFAULT (...)`
- `updated_at TEXT NOT NULL DEFAULT (...)`

The `parts` migration creates:

- `id INTEGER PRIMARY KEY`
- `type TEXT NOT NULL`
- `text TEXT NOT NULL`
- `message_id TEXT NOT NULL`
- `session_id TEXT NOT NULL`
- `generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0)`
- `created_at TEXT NOT NULL DEFAULT (...)`
- `updated_at TEXT NOT NULL DEFAULT (...)`

No foreign keys exist between `messages` and `parts`; rows may be written out of order. The data model uses natural identifiers (`session_id`, `message_id`) for joins rather than DB-level referential integrity.

Lookup indexes created by the baseline migration set:

- `idx_diff_traces_time_ms_id` on `(time_ms, id)`
- `idx_agent_traces_agent_trace_id` on `(agent_trace_id)`
- `idx_agent_traces_remote_url` on `(remote_url)`
- `idx_messages_session_message` unique index on `(session_id, message_id)` — enables duplicate-ignore parent message inserts by natural key
- `idx_messages_session_order` on `(session_id, generated_at_unix_ms, id)` — enables chronological session-scoped message retrieval
- `idx_parts_session_message_order` on `(session_id, message_id, generated_at_unix_ms, id)` — enables ordered part joins per message

`updated_at` triggers defined by the migration set:

- `trg_messages_updated_at`: fires on `UPDATE` for non-`updated_at` column changes on `messages`
- `trg_parts_updated_at`: fires on `UPDATE` for non-`updated_at` column changes on `parts`

Both triggers compare `OLD.*` vs `NEW.*` for all mutable columns (excluding `updated_at` itself) and refresh the timestamp only when a real change occurred.
