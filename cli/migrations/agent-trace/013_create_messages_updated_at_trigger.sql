CREATE TRIGGER IF NOT EXISTS trg_messages_updated_at
AFTER UPDATE ON messages
FOR EACH ROW
WHEN OLD.session_id IS NOT NEW.session_id
    OR OLD.message_id IS NOT NEW.message_id
    OR OLD.role IS NOT NEW.role
    OR OLD.generated_at_unix_ms IS NOT NEW.generated_at_unix_ms
BEGIN
    UPDATE messages
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;
