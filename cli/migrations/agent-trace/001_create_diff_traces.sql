CREATE TABLE IF NOT EXISTS diff_traces (
    id INTEGER PRIMARY KEY,
    time_ms INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    patch TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    model_id TEXT,
    tool_name TEXT,
    tool_version TEXT
);
