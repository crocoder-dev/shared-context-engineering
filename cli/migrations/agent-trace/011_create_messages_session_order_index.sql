CREATE INDEX IF NOT EXISTS idx_messages_session_order
ON messages (session_id, generated_at_unix_ms, id);
