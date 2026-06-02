CREATE TRIGGER IF NOT EXISTS trg_parts_updated_at
AFTER UPDATE ON parts
FOR EACH ROW
WHEN OLD.type IS NOT NEW.type
    OR OLD.text IS NOT NEW.text
    OR OLD.message_id IS NOT NEW.message_id
    OR OLD.session_id IS NOT NEW.session_id
    OR OLD.generated_at_unix_ms IS NOT NEW.generated_at_unix_ms
BEGIN
    UPDATE parts
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;
