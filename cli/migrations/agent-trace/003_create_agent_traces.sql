CREATE TABLE IF NOT EXISTS agent_traces (
    id INTEGER PRIMARY KEY,
    commit_id TEXT NOT NULL,
    commit_time_ms INTEGER NOT NULL,
    url TEXT NOT NULL,
    trace_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    agent_trace_id TEXT NOT NULL UNIQUE
);
