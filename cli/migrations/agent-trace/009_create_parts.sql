CREATE TABLE IF NOT EXISTS parts (
    id INTEGER PRIMARY KEY,
    type TEXT NOT NULL CHECK (type IN ('text', 'reasoning', 'patch')),
    text TEXT NOT NULL,
    message_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    generated_at_unix_ms INTEGER NOT NULL CHECK (generated_at_unix_ms >= 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
