#![allow(dead_code, clippy::struct_field_names)]

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SCE_DIR_NAME: &str = ".sce";
const LOCAL_DB_FILE_NAME: &str = "local.db";

const SCHEMA_SQL: &str = r"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    ended_at TEXT
);

CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    ended_at TEXT,
    FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS prompts (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    prompt_text TEXT NOT NULL,
    prompt_sha256 TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(conversation_id) REFERENCES conversations(id)
);

CREATE TABLE IF NOT EXISTS assistant_messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    message_text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(conversation_id) REFERENCES conversations(id)
);

CREATE TABLE IF NOT EXISTS file_observations (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_sha256 TEXT,
    observed_at TEXT NOT NULL,
    FOREIGN KEY(conversation_id) REFERENCES conversations(id)
);

CREATE TABLE IF NOT EXISTS file_ranges (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    file_observation_id TEXT,
    path TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY(conversation_id) REFERENCES conversations(id),
    FOREIGN KEY(file_observation_id) REFERENCES file_observations(id)
);

CREATE TABLE IF NOT EXISTS trace_exports (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(conversation_id) REFERENCES conversations(id)
);

CREATE INDEX IF NOT EXISTS idx_conversations_session_id
    ON conversations(session_id);
CREATE INDEX IF NOT EXISTS idx_prompts_conversation_sequence
    ON prompts(conversation_id, sequence);
CREATE INDEX IF NOT EXISTS idx_assistant_messages_conversation_sequence
    ON assistant_messages(conversation_id, sequence);
CREATE INDEX IF NOT EXISTS idx_file_observations_conversation_path
    ON file_observations(conversation_id, path);
CREATE INDEX IF NOT EXISTS idx_file_ranges_conversation_path
    ON file_ranges(conversation_id, path);
CREATE INDEX IF NOT EXISTS idx_trace_exports_conversation_id
    ON trace_exports(conversation_id);
";

#[derive(Debug)]
pub enum LocalDbError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    NotFound(String),
}

impl fmt::Display for LocalDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(
                f,
                "Failed to access local database filesystem paths: {error}. Try: verify repository write permissions and retry."
            ),
            Self::Sqlite(error) => write!(
                f,
                "Failed SQLite operation for local Agent Trace persistence: {error}. Try: inspect '.sce/local.db' and retry."
            ),
            Self::NotFound(reason) => write!(
                f,
                "Required local Agent Trace record was not found: {reason}. Try: ensure the related session/conversation exists and retry."
            ),
        }
    }
}

impl std::error::Error for LocalDbError {}

impl From<std::io::Error> for LocalDbError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<rusqlite::Error> for LocalDbError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalDb {
    database_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub session_id: String,
    pub created_at: String,
    pub ended_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prompt {
    pub id: String,
    pub conversation_id: String,
    pub sequence: i64,
    pub prompt_text: String,
    pub prompt_sha256: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantMessage {
    pub id: String,
    pub conversation_id: String,
    pub sequence: i64,
    pub message_text: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObservation {
    pub id: String,
    pub conversation_id: String,
    pub path: String,
    pub content_sha256: Option<String>,
    pub observed_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRange {
    pub id: String,
    pub conversation_id: String,
    pub file_observation_id: Option<String>,
    pub path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub reason: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceExport {
    pub id: String,
    pub conversation_id: String,
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendPromptRequest {
    pub conversation_id: String,
    pub sequence: i64,
    pub prompt_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendAssistantMessageRequest {
    pub conversation_id: String,
    pub sequence: i64,
    pub message_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordFileObservationRequest {
    pub conversation_id: String,
    pub path: String,
    pub content_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordFileRangeRequest {
    pub conversation_id: String,
    pub file_observation_id: Option<String>,
    pub path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordTraceExportRequest {
    pub conversation_id: String,
    pub payload_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MinimalTraceInputs {
    pub prompts: Vec<Prompt>,
    pub assistant_messages: Vec<AssistantMessage>,
    pub file_observations: Vec<FileObservation>,
    pub file_ranges: Vec<FileRange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoPersistedPrompt {
    pub session: Session,
    pub conversation: Conversation,
    pub prompt: Prompt,
}

pub fn init_db(repository_root: &Path) -> Result<LocalDb, LocalDbError> {
    let database_path = repository_local_db_path(repository_root);
    let database_parent = database_path.parent().ok_or_else(|| {
        LocalDbError::Io(std::io::Error::other(format!(
            "database path '{}' has no parent directory",
            database_path.display()
        )))
    })?;

    fs::create_dir_all(database_parent)?;

    let db = LocalDb { database_path };
    let connection = db.open_connection()?;
    bootstrap_schema(&connection)?;

    Ok(db)
}

pub fn ensure_db_initialized(repository_root: &Path) -> Result<LocalDb, LocalDbError> {
    init_db(repository_root)
}

pub fn append_prompt_with_auto_init(
    repository_root: &Path,
    prompt_text: &str,
) -> Result<AutoPersistedPrompt, LocalDbError> {
    let db = ensure_db_initialized(repository_root)?;
    db.append_prompt_with_active_context(prompt_text)
}

impl LocalDb {
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn create_session(&self) -> Result<Session, LocalDbError> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;

        let session = Session {
            id: new_uuid_v4_string(),
            started_at: now_rfc3339_utc(),
            ended_at: None,
        };

        transaction.execute(
            "INSERT INTO sessions (id, started_at, ended_at) VALUES (?1, ?2, ?3)",
            params![&session.id, &session.started_at, &session.ended_at],
        )?;

        transaction.commit()?;
        Ok(session)
    }

    pub fn ensure_active_session(&self) -> Result<Session, LocalDbError> {
        let connection = self.open_connection()?;
        if let Some(existing) = query_active_session(&connection)? {
            return Ok(existing);
        }

        self.create_session()
    }

    pub fn end_session(&self, session_id: &str) -> Result<Session, LocalDbError> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;

        let ended_at = now_rfc3339_utc();
        let changed_rows = transaction.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
            params![ended_at, session_id],
        )?;

        if changed_rows == 0 {
            return Err(LocalDbError::NotFound(format!("session id '{session_id}'")));
        }

        let session = query_session_by_id(&transaction, session_id)?.ok_or_else(|| {
            LocalDbError::NotFound(format!("session id '{session_id}' after update"))
        })?;

        transaction.commit()?;
        Ok(session)
    }

    pub fn create_conversation(&self, session_id: &str) -> Result<Conversation, LocalDbError> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;

        let conversation = Conversation {
            id: new_uuid_v4_string(),
            session_id: session_id.to_string(),
            created_at: now_rfc3339_utc(),
            ended_at: None,
        };

        transaction.execute(
            "INSERT INTO conversations (id, session_id, created_at, ended_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                &conversation.id,
                &conversation.session_id,
                &conversation.created_at,
                &conversation.ended_at
            ],
        )?;

        transaction.commit()?;
        Ok(conversation)
    }

    pub fn ensure_active_conversation(
        &self,
        session_id: &str,
    ) -> Result<Conversation, LocalDbError> {
        let connection = self.open_connection()?;
        if let Some(existing) = query_active_conversation(&connection, session_id)? {
            return Ok(existing);
        }

        self.create_conversation(session_id)
    }

    pub fn append_prompt_with_active_context(
        &self,
        prompt_text: &str,
    ) -> Result<AutoPersistedPrompt, LocalDbError> {
        let session = self.ensure_active_session()?;
        let conversation = self.ensure_active_conversation(&session.id)?;
        let sequence = self.next_prompt_sequence(&conversation.id)?;
        let prompt = self.append_prompt(&AppendPromptRequest {
            conversation_id: conversation.id.clone(),
            sequence,
            prompt_text: prompt_text.to_string(),
        })?;

        Ok(AutoPersistedPrompt {
            session,
            conversation,
            prompt,
        })
    }

    pub fn append_prompt(&self, request: &AppendPromptRequest) -> Result<Prompt, LocalDbError> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;

        let prompt = Prompt {
            id: new_uuid_v4_string(),
            conversation_id: request.conversation_id.clone(),
            sequence: request.sequence,
            prompt_sha256: sha256_hex(request.prompt_text.as_bytes()),
            prompt_text: request.prompt_text.clone(),
            created_at: now_rfc3339_utc(),
        };

        transaction.execute(
            "INSERT INTO prompts (id, conversation_id, sequence, prompt_text, prompt_sha256, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                &prompt.id,
                &prompt.conversation_id,
                prompt.sequence,
                &prompt.prompt_text,
                &prompt.prompt_sha256,
                &prompt.created_at
            ],
        )?;

        transaction.commit()?;
        Ok(prompt)
    }

    pub fn append_assistant_message(
        &self,
        request: &AppendAssistantMessageRequest,
    ) -> Result<AssistantMessage, LocalDbError> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;

        let message = AssistantMessage {
            id: new_uuid_v4_string(),
            conversation_id: request.conversation_id.clone(),
            sequence: request.sequence,
            message_text: request.message_text.clone(),
            created_at: now_rfc3339_utc(),
        };

        transaction.execute(
            "INSERT INTO assistant_messages (id, conversation_id, sequence, message_text, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &message.id,
                &message.conversation_id,
                message.sequence,
                &message.message_text,
                &message.created_at
            ],
        )?;

        transaction.commit()?;
        Ok(message)
    }

    pub fn record_file_observation(
        &self,
        request: &RecordFileObservationRequest,
    ) -> Result<FileObservation, LocalDbError> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;

        let observation = FileObservation {
            id: new_uuid_v4_string(),
            conversation_id: request.conversation_id.clone(),
            path: request.path.clone(),
            content_sha256: request.content_sha256.clone(),
            observed_at: now_rfc3339_utc(),
        };

        transaction.execute(
            "INSERT INTO file_observations (id, conversation_id, path, content_sha256, observed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &observation.id,
                &observation.conversation_id,
                &observation.path,
                &observation.content_sha256,
                &observation.observed_at
            ],
        )?;

        transaction.commit()?;
        Ok(observation)
    }

    pub fn record_file_range(
        &self,
        request: &RecordFileRangeRequest,
    ) -> Result<FileRange, LocalDbError> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;

        let range = FileRange {
            id: new_uuid_v4_string(),
            conversation_id: request.conversation_id.clone(),
            file_observation_id: request.file_observation_id.clone(),
            path: request.path.clone(),
            start_line: request.start_line,
            end_line: request.end_line,
            reason: request.reason.clone(),
            created_at: now_rfc3339_utc(),
        };

        transaction.execute(
            "INSERT INTO file_ranges (id, conversation_id, file_observation_id, path, start_line, end_line, reason, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &range.id,
                &range.conversation_id,
                &range.file_observation_id,
                &range.path,
                range.start_line,
                range.end_line,
                &range.reason,
                &range.created_at
            ],
        )?;

        transaction.commit()?;
        Ok(range)
    }

    pub fn record_trace_export(
        &self,
        request: &RecordTraceExportRequest,
    ) -> Result<TraceExport, LocalDbError> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;

        let trace_export = TraceExport {
            id: new_uuid_v4_string(),
            conversation_id: request.conversation_id.clone(),
            payload_json: request.payload_json.clone(),
            created_at: now_rfc3339_utc(),
        };

        transaction.execute(
            "INSERT INTO trace_exports (id, conversation_id, payload_json, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                &trace_export.id,
                &trace_export.conversation_id,
                &trace_export.payload_json,
                &trace_export.created_at
            ],
        )?;

        transaction.commit()?;
        Ok(trace_export)
    }

    pub fn get_conversation_prompts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<Prompt>, LocalDbError> {
        let connection = self.open_connection()?;

        let mut statement = connection.prepare(
            "SELECT id, conversation_id, sequence, prompt_text, prompt_sha256, created_at
             FROM prompts
             WHERE conversation_id = ?1
             ORDER BY sequence ASC, id ASC",
        )?;

        let rows = statement.query_map(params![conversation_id], |row| {
            Ok(Prompt {
                id: row.get(0)?,
                conversation_id: row.get(1)?,
                sequence: row.get(2)?,
                prompt_text: row.get(3)?,
                prompt_sha256: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;

        collect_rows(rows)
    }

    pub fn get_conversation_ranges(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<FileRange>, LocalDbError> {
        let connection = self.open_connection()?;

        let mut statement = connection.prepare(
            "SELECT id, conversation_id, file_observation_id, path, start_line, end_line, reason, created_at
             FROM file_ranges
             WHERE conversation_id = ?1
             ORDER BY path ASC, start_line ASC, end_line ASC, id ASC",
        )?;

        let rows = statement.query_map(params![conversation_id], |row| {
            Ok(FileRange {
                id: row.get(0)?,
                conversation_id: row.get(1)?,
                file_observation_id: row.get(2)?,
                path: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                reason: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;

        collect_rows(rows)
    }

    pub fn get_trace_exports(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<TraceExport>, LocalDbError> {
        let connection = self.open_connection()?;

        let mut statement = connection.prepare(
            "SELECT id, conversation_id, payload_json, created_at
             FROM trace_exports
             WHERE conversation_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;

        let rows = statement.query_map(params![conversation_id], |row| {
            Ok(TraceExport {
                id: row.get(0)?,
                conversation_id: row.get(1)?,
                payload_json: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;

        collect_rows(rows)
    }

    pub fn get_minimal_trace_inputs(
        &self,
        conversation_id: &str,
    ) -> Result<MinimalTraceInputs, LocalDbError> {
        let connection = self.open_connection()?;

        let prompts = query_prompts(&connection, conversation_id)?;
        let assistant_messages = query_assistant_messages(&connection, conversation_id)?;
        let file_observations = query_file_observations(&connection, conversation_id)?;
        let file_ranges = query_file_ranges(&connection, conversation_id)?;

        Ok(MinimalTraceInputs {
            prompts,
            assistant_messages,
            file_observations,
            file_ranges,
        })
    }

    fn open_connection(&self) -> Result<Connection, LocalDbError> {
        let connection = Connection::open(&self.database_path)?;
        connection.execute("PRAGMA foreign_keys = ON", [])?;
        Ok(connection)
    }

    fn next_prompt_sequence(&self, conversation_id: &str) -> Result<i64, LocalDbError> {
        let connection = self.open_connection()?;
        query_next_prompt_sequence(&connection, conversation_id)
    }
}

fn bootstrap_schema(connection: &Connection) -> Result<(), LocalDbError> {
    connection.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

fn query_session_by_id(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<Session>, LocalDbError> {
    let mut statement =
        connection.prepare("SELECT id, started_at, ended_at FROM sessions WHERE id = ?1")?;

    let mut rows = statement.query(params![session_id])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(Session {
            id: row.get(0)?,
            started_at: row.get(1)?,
            ended_at: row.get(2)?,
        }));
    }

    Ok(None)
}

fn query_active_session(connection: &Connection) -> Result<Option<Session>, LocalDbError> {
    let mut statement = connection.prepare(
        "SELECT id, started_at, ended_at
         FROM sessions
         WHERE ended_at IS NULL
         ORDER BY started_at DESC, id DESC
         LIMIT 1",
    )?;

    let mut rows = statement.query([])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(Session {
            id: row.get(0)?,
            started_at: row.get(1)?,
            ended_at: row.get(2)?,
        }));
    }

    Ok(None)
}

fn query_active_conversation(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<Conversation>, LocalDbError> {
    let mut statement = connection.prepare(
        "SELECT id, session_id, created_at, ended_at
         FROM conversations
         WHERE session_id = ?1
           AND ended_at IS NULL
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )?;

    let mut rows = statement.query(params![session_id])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(Conversation {
            id: row.get(0)?,
            session_id: row.get(1)?,
            created_at: row.get(2)?,
            ended_at: row.get(3)?,
        }));
    }

    Ok(None)
}

fn query_next_prompt_sequence(
    connection: &Connection,
    conversation_id: &str,
) -> Result<i64, LocalDbError> {
    let next_sequence = connection.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1
         FROM prompts
         WHERE conversation_id = ?1",
        params![conversation_id],
        |row| row.get(0),
    )?;

    Ok(next_sequence)
}

fn query_prompts(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Vec<Prompt>, LocalDbError> {
    let mut statement = connection.prepare(
        "SELECT id, conversation_id, sequence, prompt_text, prompt_sha256, created_at
         FROM prompts
         WHERE conversation_id = ?1
         ORDER BY sequence ASC, id ASC",
    )?;
    let rows = statement.query_map(params![conversation_id], |row| {
        Ok(Prompt {
            id: row.get(0)?,
            conversation_id: row.get(1)?,
            sequence: row.get(2)?,
            prompt_text: row.get(3)?,
            prompt_sha256: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;

    collect_rows(rows)
}

fn query_assistant_messages(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Vec<AssistantMessage>, LocalDbError> {
    let mut statement = connection.prepare(
        "SELECT id, conversation_id, sequence, message_text, created_at
         FROM assistant_messages
         WHERE conversation_id = ?1
         ORDER BY sequence ASC, id ASC",
    )?;

    let rows = statement.query_map(params![conversation_id], |row| {
        Ok(AssistantMessage {
            id: row.get(0)?,
            conversation_id: row.get(1)?,
            sequence: row.get(2)?,
            message_text: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;

    collect_rows(rows)
}

fn query_file_observations(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Vec<FileObservation>, LocalDbError> {
    let mut statement = connection.prepare(
        "SELECT id, conversation_id, path, content_sha256, observed_at
         FROM file_observations
         WHERE conversation_id = ?1
         ORDER BY path ASC, observed_at ASC, id ASC",
    )?;

    let rows = statement.query_map(params![conversation_id], |row| {
        Ok(FileObservation {
            id: row.get(0)?,
            conversation_id: row.get(1)?,
            path: row.get(2)?,
            content_sha256: row.get(3)?,
            observed_at: row.get(4)?,
        })
    })?;

    collect_rows(rows)
}

fn query_file_ranges(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Vec<FileRange>, LocalDbError> {
    let mut statement = connection.prepare(
        "SELECT id, conversation_id, file_observation_id, path, start_line, end_line, reason, created_at
         FROM file_ranges
         WHERE conversation_id = ?1
         ORDER BY path ASC, start_line ASC, end_line ASC, id ASC",
    )?;

    let rows = statement.query_map(params![conversation_id], |row| {
        Ok(FileRange {
            id: row.get(0)?,
            conversation_id: row.get(1)?,
            file_observation_id: row.get(2)?,
            path: row.get(3)?,
            start_line: row.get(4)?,
            end_line: row.get(5)?,
            reason: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;

    collect_rows(rows)
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>, LocalDbError> {
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(LocalDbError::from)
}

fn repository_local_db_path(repository_root: &Path) -> PathBuf {
    repository_root.join(SCE_DIR_NAME).join(LOCAL_DB_FILE_NAME)
}

fn now_rfc3339_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn new_uuid_v4_string() -> String {
    Uuid::new_v4().to_string()
}

fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRepository {
        root: PathBuf,
    }

    impl TempRepository {
        fn create() -> Self {
            let root =
                std::env::temp_dir().join(format!("sce-local-db-service-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("temp repository root should be created");
            Self { root }
        }

        fn root(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for TempRepository {
        fn drop(&mut self) {
            if self.root.exists() {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }

    #[test]
    fn init_db_creates_expected_schema_tables_and_indexes() {
        let fixture = TempRepository::create();
        let db = init_db(fixture.root()).expect("database init should succeed");

        let connection = Connection::open(db.database_path())
            .expect("database path should be openable after init");

        let table_names = sqlite_object_names(&connection, "table");
        for expected_table in [
            "sessions",
            "conversations",
            "prompts",
            "assistant_messages",
            "file_observations",
            "file_ranges",
            "trace_exports",
        ] {
            assert!(
                table_names.contains(&expected_table.to_string()),
                "expected table '{expected_table}' to exist; got: {table_names:?}"
            );
        }

        let index_names = sqlite_object_names(&connection, "index");
        for expected_index in [
            "idx_conversations_session_id",
            "idx_prompts_conversation_sequence",
            "idx_assistant_messages_conversation_sequence",
            "idx_file_observations_conversation_path",
            "idx_file_ranges_conversation_path",
            "idx_trace_exports_conversation_id",
        ] {
            assert!(
                index_names.contains(&expected_index.to_string()),
                "expected index '{expected_index}' to exist; got: {index_names:?}"
            );
        }
    }

    #[test]
    fn local_db_crud_round_trip_uses_uuid_and_rfc3339_utc_timestamps() {
        let fixture = TempRepository::create();
        let db = init_db(fixture.root()).expect("database init should succeed");

        let session = db.create_session().expect("session insert should succeed");
        assert_uuid_v4(&session.id);
        assert_rfc3339_timestamp(&session.started_at);

        let ended_session = db
            .end_session(&session.id)
            .expect("ending existing session should succeed");
        assert_eq!(ended_session.id, session.id);
        assert!(ended_session.ended_at.is_some());
        assert_rfc3339_timestamp(
            ended_session
                .ended_at
                .as_deref()
                .expect("ended_at is present"),
        );

        let conversation = db
            .create_conversation(&session.id)
            .expect("conversation insert should succeed");
        assert_uuid_v4(&conversation.id);
        assert_rfc3339_timestamp(&conversation.created_at);

        let prompt_one = db
            .append_prompt(&AppendPromptRequest {
                conversation_id: conversation.id.clone(),
                sequence: 1,
                prompt_text: "Draft a release summary".to_string(),
            })
            .expect("first prompt insert should succeed");
        assert_uuid_v4(&prompt_one.id);
        assert_rfc3339_timestamp(&prompt_one.created_at);

        let prompt_two = db
            .append_prompt(&AppendPromptRequest {
                conversation_id: conversation.id.clone(),
                sequence: 2,
                prompt_text: "Now include generated artifacts".to_string(),
            })
            .expect("second prompt insert should succeed");
        assert_uuid_v4(&prompt_two.id);
        assert_rfc3339_timestamp(&prompt_two.created_at);

        let assistant_message = db
            .append_assistant_message(&AppendAssistantMessageRequest {
                conversation_id: conversation.id.clone(),
                sequence: 1,
                message_text: "Added release summary".to_string(),
            })
            .expect("assistant message insert should succeed");
        assert_uuid_v4(&assistant_message.id);
        assert_rfc3339_timestamp(&assistant_message.created_at);

        let file_observation = db
            .record_file_observation(&RecordFileObservationRequest {
                conversation_id: conversation.id.clone(),
                path: "context/overview.md".to_string(),
                content_sha256: Some(sha256_hex(b"overview content")),
            })
            .expect("file observation insert should succeed");
        assert_uuid_v4(&file_observation.id);
        assert_rfc3339_timestamp(&file_observation.observed_at);

        let file_range = db
            .record_file_range(&RecordFileRangeRequest {
                conversation_id: conversation.id.clone(),
                file_observation_id: Some(file_observation.id.clone()),
                path: "context/overview.md".to_string(),
                start_line: 5,
                end_line: 12,
                reason: Some("local-db contract update".to_string()),
            })
            .expect("file range insert should succeed");
        assert_uuid_v4(&file_range.id);
        assert_rfc3339_timestamp(&file_range.created_at);

        let trace_export = db
            .record_trace_export(&RecordTraceExportRequest {
                conversation_id: conversation.id.clone(),
                payload_json: "{\"trace\":\"minimal\"}".to_string(),
            })
            .expect("trace export insert should succeed");
        assert_uuid_v4(&trace_export.id);
        assert_rfc3339_timestamp(&trace_export.created_at);

        let prompts = db
            .get_conversation_prompts(&conversation.id)
            .expect("prompt query should succeed");
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].sequence, 1);
        assert_eq!(prompts[1].sequence, 2);

        let ranges = db
            .get_conversation_ranges(&conversation.id)
            .expect("range query should succeed");
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].path, "context/overview.md");

        let trace_exports = db
            .get_trace_exports(&conversation.id)
            .expect("trace export query should succeed");
        assert_eq!(trace_exports, vec![trace_export]);

        let minimal = db
            .get_minimal_trace_inputs(&conversation.id)
            .expect("minimal trace query should succeed");
        assert_eq!(minimal.prompts, prompts);
        assert_eq!(minimal.assistant_messages, vec![assistant_message]);
        assert_eq!(minimal.file_observations, vec![file_observation]);
        assert_eq!(minimal.file_ranges, vec![file_range]);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn minimal_trace_inputs_and_ranges_are_scoped_and_sorted_deterministically() {
        let fixture = TempRepository::create();
        let db = init_db(fixture.root()).expect("database init should succeed");

        let session = db.create_session().expect("session insert should succeed");
        let conversation = db
            .create_conversation(&session.id)
            .expect("conversation insert should succeed");
        let other_conversation = db
            .create_conversation(&session.id)
            .expect("second conversation insert should succeed");

        for (sequence, prompt_text) in [(1, "First prompt"), (2, "Second prompt")] {
            append_prompt_for_test(&db, &conversation.id, sequence, prompt_text);
        }
        append_prompt_for_test(&db, &other_conversation.id, 1, "Other conversation prompt");

        for (sequence, message_text) in [(1, "Assistant reply one"), (2, "Assistant reply two")] {
            append_assistant_message_for_test(&db, &conversation.id, sequence, message_text);
        }
        append_assistant_message_for_test(
            &db,
            &other_conversation.id,
            1,
            "Other conversation assistant reply",
        );

        let z_observation =
            record_observation_for_test(&db, &conversation.id, "z/file.rs", b"z-file-content");
        let a_observation =
            record_observation_for_test(&db, &conversation.id, "a/file.rs", b"a-file-content");
        record_observation_for_test(
            &db,
            &other_conversation.id,
            "m/other.rs",
            b"other-file-content",
        );

        record_range_for_test(
            &db,
            &conversation.id,
            Some(&z_observation.id),
            "z/file.rs",
            20,
            30,
            "z-path range",
        );
        record_range_for_test(
            &db,
            &conversation.id,
            Some(&a_observation.id),
            "a/file.rs",
            10,
            12,
            "a-path later range",
        );
        record_range_for_test(
            &db,
            &conversation.id,
            Some(&a_observation.id),
            "a/file.rs",
            1,
            3,
            "a-path earliest range",
        );
        record_range_for_test(
            &db,
            &other_conversation.id,
            None,
            "m/other.rs",
            50,
            60,
            "other conversation range",
        );

        let ranges = db
            .get_conversation_ranges(&conversation.id)
            .expect("conversation ranges query should succeed");
        assert_eq!(ranges.len(), 3);
        assert_eq!(
            ranges
                .iter()
                .map(|range| range.path.as_str())
                .collect::<Vec<_>>(),
            vec!["a/file.rs", "a/file.rs", "z/file.rs"]
        );
        assert_eq!(
            ranges
                .iter()
                .map(|range| range.start_line)
                .collect::<Vec<_>>(),
            vec![1, 10, 20]
        );
        assert!(
            ranges
                .iter()
                .all(|range| range.conversation_id == conversation.id),
            "range query should stay scoped to the requested conversation"
        );

        let minimal = db
            .get_minimal_trace_inputs(&conversation.id)
            .expect("minimal trace inputs query should succeed");
        assert_eq!(minimal.prompts.len(), 2);
        assert_eq!(
            minimal
                .prompts
                .iter()
                .map(|prompt| prompt.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(minimal.assistant_messages.len(), 2);
        assert_eq!(
            minimal
                .assistant_messages
                .iter()
                .map(|message| message.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(minimal.file_observations.len(), 2);
        assert_eq!(
            minimal
                .file_observations
                .iter()
                .map(|observation| observation.path.as_str())
                .collect::<Vec<_>>(),
            vec!["a/file.rs", "z/file.rs"]
        );
        assert_eq!(minimal.file_ranges, ranges);
        assert!(
            minimal
                .prompts
                .iter()
                .all(|prompt| prompt.conversation_id == conversation.id),
            "minimal trace prompts should stay scoped to the requested conversation"
        );
        assert!(
            minimal
                .assistant_messages
                .iter()
                .all(|message| message.conversation_id == conversation.id),
            "minimal trace assistant messages should stay scoped to the requested conversation"
        );
        assert!(
            minimal
                .file_observations
                .iter()
                .all(|observation| observation.conversation_id == conversation.id),
            "minimal trace file observations should stay scoped to the requested conversation"
        );
    }

    #[test]
    fn ensure_active_session_creates_then_reuses_until_session_is_ended() {
        let fixture = TempRepository::create();
        let db = init_db(fixture.root()).expect("database init should succeed");

        let first = db
            .ensure_active_session()
            .expect("initial active session ensure should succeed");
        let reused = db
            .ensure_active_session()
            .expect("second active session ensure should reuse the same session");
        assert_eq!(reused.id, first.id);

        db.end_session(&first.id)
            .expect("ending session should succeed");

        let after_end = db
            .ensure_active_session()
            .expect("active session ensure should create a new open session");
        assert_ne!(after_end.id, first.id);
        assert!(after_end.ended_at.is_none());
    }

    #[test]
    fn append_prompt_with_auto_init_creates_missing_db_and_first_submit_records_prompt() {
        let fixture = TempRepository::create();
        let expected_db_path = fixture.root().join(SCE_DIR_NAME).join(LOCAL_DB_FILE_NAME);
        assert!(
            !expected_db_path.exists(),
            "database file should not exist before first submit"
        );

        let persisted = append_prompt_with_auto_init(fixture.root(), "First persisted prompt")
            .expect("auto-init submit helper should succeed");

        assert!(
            expected_db_path.exists(),
            "database file should be created on first submit"
        );
        assert_eq!(persisted.prompt.sequence, 1);
        assert_eq!(persisted.prompt.prompt_text, "First persisted prompt");

        let connection = Connection::open(&expected_db_path)
            .expect("created database should be openable after first submit");
        assert_eq!(sqlite_count(&connection, "sessions"), 1);
        assert_eq!(sqlite_count(&connection, "conversations"), 1);
        assert_eq!(sqlite_count(&connection, "prompts"), 1);
    }

    #[test]
    fn append_prompt_with_auto_init_reuses_active_session_and_conversation() {
        let fixture = TempRepository::create();

        let first = append_prompt_with_auto_init(fixture.root(), "first")
            .expect("first submit should persist prompt");
        let second = append_prompt_with_auto_init(fixture.root(), "second")
            .expect("second submit should persist prompt");

        assert_eq!(second.session.id, first.session.id);
        assert_eq!(second.conversation.id, first.conversation.id);
        assert_eq!(first.prompt.sequence, 1);
        assert_eq!(second.prompt.sequence, 2);

        let connection =
            Connection::open(fixture.root().join(SCE_DIR_NAME).join(LOCAL_DB_FILE_NAME))
                .expect("database path should be openable after second submit");
        assert_eq!(sqlite_count(&connection, "sessions"), 1);
        assert_eq!(sqlite_count(&connection, "conversations"), 1);
        assert_eq!(sqlite_count(&connection, "prompts"), 2);
    }

    fn sqlite_object_names(connection: &Connection, object_type: &str) -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = ?1 ORDER BY name ASC")
            .expect("sqlite_master query should prepare");

        let rows = statement
            .query_map(params![object_type], |row| row.get::<_, String>(0))
            .expect("sqlite_master query should run");

        rows.collect::<Result<Vec<_>, _>>()
            .expect("sqlite_master names should be collected")
    }

    fn sqlite_count(connection: &Connection, table_name: &str) -> i64 {
        let query = format!("SELECT COUNT(*) FROM {table_name}");
        connection
            .query_row(&query, [], |row| row.get(0))
            .expect("COUNT query should succeed")
    }

    fn assert_uuid_v4(value: &str) {
        let parsed = Uuid::parse_str(value).expect("identifier must be UUID");
        assert_eq!(parsed.get_version_num(), 4, "identifier must be UUID v4");
    }

    fn assert_rfc3339_timestamp(value: &str) {
        let parsed =
            chrono::DateTime::parse_from_rfc3339(value).expect("timestamp must parse as RFC3339");
        assert_eq!(
            parsed.offset().local_minus_utc(),
            0,
            "timestamp must be UTC"
        );
    }

    fn append_prompt_for_test(
        db: &LocalDb,
        conversation_id: &str,
        sequence: i64,
        prompt_text: &str,
    ) {
        db.append_prompt(&AppendPromptRequest {
            conversation_id: conversation_id.to_string(),
            sequence,
            prompt_text: prompt_text.to_string(),
        })
        .expect("prompt insert should succeed");
    }

    fn append_assistant_message_for_test(
        db: &LocalDb,
        conversation_id: &str,
        sequence: i64,
        message_text: &str,
    ) {
        db.append_assistant_message(&AppendAssistantMessageRequest {
            conversation_id: conversation_id.to_string(),
            sequence,
            message_text: message_text.to_string(),
        })
        .expect("assistant message insert should succeed");
    }

    fn record_observation_for_test(
        db: &LocalDb,
        conversation_id: &str,
        path: &str,
        content: &[u8],
    ) -> FileObservation {
        db.record_file_observation(&RecordFileObservationRequest {
            conversation_id: conversation_id.to_string(),
            path: path.to_string(),
            content_sha256: Some(sha256_hex(content)),
        })
        .expect("file observation insert should succeed")
    }

    #[allow(clippy::too_many_arguments)]
    fn record_range_for_test(
        db: &LocalDb,
        conversation_id: &str,
        file_observation_id: Option<&str>,
        path: &str,
        start_line: i64,
        end_line: i64,
        reason: &str,
    ) {
        db.record_file_range(&RecordFileRangeRequest {
            conversation_id: conversation_id.to_string(),
            file_observation_id: file_observation_id.map(str::to_string),
            path: path.to_string(),
            start_line,
            end_line,
            reason: Some(reason.to_string()),
        })
        .expect("file range insert should succeed");
    }
}
