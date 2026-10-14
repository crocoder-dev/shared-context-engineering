ALTER TABLE telemetry_credentials
ADD COLUMN user_id TEXT NOT NULL DEFAULT '';

ALTER TABLE telemetry_credentials
ADD COLUMN workspace_id TEXT NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS telemetry_repository_bindings (
    repository_id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    bound_at_unix_seconds INTEGER NOT NULL
);
