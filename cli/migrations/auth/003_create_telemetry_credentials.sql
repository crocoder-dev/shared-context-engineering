CREATE TABLE IF NOT EXISTS telemetry_credentials (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    token TEXT NOT NULL,
    accessor TEXT NOT NULL,
    expires_at_unix_seconds INTEGER NOT NULL,
    stored_at_unix_seconds INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE TRIGGER IF NOT EXISTS telemetry_credentials_set_updated_at
AFTER UPDATE ON telemetry_credentials
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE telemetry_credentials
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;
