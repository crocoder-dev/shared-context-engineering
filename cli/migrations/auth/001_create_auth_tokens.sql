CREATE TABLE IF NOT EXISTS auth_credentials (
    id INTEGER PRIMARY KEY NOT NULL,
    access_token TEXT NOT NULL,
    token_type TEXT NOT NULL,
    expires_in INTEGER NOT NULL,
    refresh_token TEXT NOT NULL,
    scope TEXT,
    stored_at_unix_seconds INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
