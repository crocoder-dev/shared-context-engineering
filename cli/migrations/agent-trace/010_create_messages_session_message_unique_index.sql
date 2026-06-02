CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_session_message
ON messages (session_id, message_id);
