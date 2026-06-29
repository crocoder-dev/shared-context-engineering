CREATE TABLE IF NOT EXISTS session_models (
    id INTEGER PRIMARY KEY,
    tool_name TEXT NOT NULL,
    session_id TEXT NOT NULL,
    model_id TEXT,
    tool_version TEXT,
    session_start_time_ms INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (tool_name, session_id)
);
