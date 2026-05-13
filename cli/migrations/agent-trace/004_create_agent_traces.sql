CREATE TABLE IF NOT EXISTS agent_traces (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    commit_id TEXT NOT NULL,
    commit_time_ms INTEGER NOT NULL,
    trace_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
