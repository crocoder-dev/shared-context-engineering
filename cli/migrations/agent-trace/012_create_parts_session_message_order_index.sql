CREATE INDEX IF NOT EXISTS idx_parts_session_message_order
ON parts (session_id, message_id, generated_at_unix_ms, id);
