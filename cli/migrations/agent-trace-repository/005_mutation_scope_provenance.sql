CREATE TABLE IF NOT EXISTS mutation_trace_scope_provenance (
    scope_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    model_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
