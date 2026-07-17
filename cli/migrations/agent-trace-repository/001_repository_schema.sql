-- Repository-scoped Agent Trace database baseline.
--
-- One logical Git repository maps to one database at
-- <state-root>/sce/repos/<repository-id>/agent-trace.db. This file is the
-- complete fresh schema for that database: repository-scoped databases are
-- always created new, so the baseline stays a single file instead of an
-- incremental migration chain. Trace tables are repository-level and
-- intentionally carry no checkout_id columns; checkout identity is
-- diagnostics-only context and is never persisted on trace rows.

CREATE TABLE IF NOT EXISTS repository_metadata (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    repository_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS diff_traces (
    id INTEGER PRIMARY KEY,
    time_ms INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    patch TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    model_id TEXT,
    tool_name TEXT,
    tool_version TEXT,
    payload_type TEXT NOT NULL DEFAULT 'patch'
);

CREATE INDEX IF NOT EXISTS idx_diff_traces_time_ms_id
ON diff_traces (time_ms, id);

CREATE TABLE IF NOT EXISTS post_commit_patch_intersections (
    id INTEGER PRIMARY KEY,
    commit_id TEXT NOT NULL,
    post_commit_time_ms INTEGER NOT NULL,
    recent_window_cutoff_ms INTEGER NOT NULL,
    recent_window_end_ms INTEGER NOT NULL,
    loaded_diff_trace_count INTEGER NOT NULL CHECK (loaded_diff_trace_count >= 0),
    skipped_diff_trace_count INTEGER NOT NULL CHECK (skipped_diff_trace_count >= 0),
    intersection_patch TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS agent_traces (
    id INTEGER PRIMARY KEY,
    commit_id TEXT NOT NULL,
    commit_time_ms INTEGER NOT NULL,
    url TEXT NOT NULL,
    trace_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    agent_trace_id TEXT NOT NULL UNIQUE,
    remote_url TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_traces_agent_trace_id
ON agent_traces (agent_trace_id);

CREATE INDEX IF NOT EXISTS idx_agent_traces_remote_url
ON agent_traces (remote_url);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_session_message
ON messages (session_id, message_id);

CREATE INDEX IF NOT EXISTS idx_messages_session_order
ON messages (session_id, generated_at_unix_ms, id);

CREATE TABLE IF NOT EXISTS parts (
    id INTEGER PRIMARY KEY,
    type TEXT NOT NULL,
    text TEXT NOT NULL,
    message_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_parts_session_message_order
ON parts (session_id, message_id, generated_at_unix_ms, id);

CREATE TRIGGER IF NOT EXISTS trg_messages_updated_at
AFTER UPDATE ON messages
FOR EACH ROW
WHEN OLD.session_id IS NOT NEW.session_id
    OR OLD.message_id IS NOT NEW.message_id
    OR OLD.role IS NOT NEW.role
    OR OLD.generated_at_unix_ms IS NOT NEW.generated_at_unix_ms
BEGIN
    UPDATE messages
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_parts_updated_at
AFTER UPDATE ON parts
FOR EACH ROW
WHEN OLD.type IS NOT NEW.type
    OR OLD.text IS NOT NEW.text
    OR OLD.message_id IS NOT NEW.message_id
    OR OLD.session_id IS NOT NEW.session_id
    OR OLD.generated_at_unix_ms IS NOT NEW.generated_at_unix_ms
BEGIN
    UPDATE parts
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;
