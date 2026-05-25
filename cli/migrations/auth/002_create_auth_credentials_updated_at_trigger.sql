CREATE TRIGGER IF NOT EXISTS auth_credentials_set_updated_at
AFTER UPDATE ON auth_credentials
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE auth_credentials
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;
