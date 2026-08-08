//! Incremental repository-source to DWH ETL for logical conversation messages.
//!
//! Source extraction is intentionally short and read-only. Transformation and
//! destination loading happen after the source snapshot ends, and each batch's
//! facts, dimensions, and `messages` watermark commit in one destination
//! transaction.

use anyhow::{bail, Context, Result};

use crate::services::{
    agent_trace_db::{repository::RepositoryAgentTraceDb, MessageRole},
    agent_trace_dwh_db::AgentTraceDwhDb,
    agent_trace_dwh_replica::AgentTraceDwhReplica,
    db::TursoTransaction,
    etl::{
        read_watermark, run_with_source_contention_retry, upsert_watermark, validate_batch_size,
        TableBatchStats,
    },
};

/// The source table represented by this pipeline.
pub const MESSAGES_SOURCE_TABLE: &str = "messages";

/// Default number of source messages processed by one batch.
pub const DEFAULT_MESSAGES_ETL_BATCH_SIZE: u32 = 500;

/// One source message copied from a short repository database snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceMessage {
    pub id: i64,
    pub session_id: String,
    pub message_id: String,
    pub role: String,
    pub generated_at_unix_ms: i64,
}

/// A source message after role validation and destination-independent
/// transformation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransformedMessage {
    pub source_row_id: i64,
    pub session_id: String,
    pub message_id: String,
    pub role: MessageRole,
    pub generated_at_unix_ms: i64,
}

/// Counts returned by one atomically loaded messages batch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MessagesBatchStats {
    pub inserted: u64,
    pub already_present: u64,
    pub watermark: i64,
}

/// Summary of one complete incremental messages ETL run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MessagesEtlStats {
    pub extracted: u64,
    pub inserted: u64,
    pub already_present: u64,
    pub batches: u64,
    pub before_watermark: i64,
    pub after_watermark: i64,
}

/// Configuration for the independently watermarked messages pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessagesEtl {
    batch_size: u32,
}

/// Descriptive alias for callers composing conversation table pipelines.
pub type ConversationMessagesEtl = MessagesEtl;

const SELECT_MESSAGES_BATCH_SQL: &str =
    "SELECT id, session_id, message_id, role, generated_at_unix_ms
FROM messages
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2";

/// Extract one bounded, ascending source batch in a short read transaction.
///
/// Only transient database/table lock contention is retried. The source read
/// transaction is committed before this function returns, so transformation
/// and destination work never hold a source snapshot open.
pub fn extract_message_batch(
    db: &RepositoryAgentTraceDb,
    watermark: i64,
    batch_size: u32,
) -> Result<Vec<SourceMessage>> {
    validate_batch_size(batch_size, "messages extraction")?;

    run_with_source_contention_retry(
        |_attempt| {
            db.read_transaction(|txn| {
                txn.query_map(
                    SELECT_MESSAGES_BATCH_SQL,
                    (watermark, i64::from(batch_size)),
                    source_message_from_row,
                )
            })
        },
        || db.rollback_best_effort(),
    )
}

fn source_message_from_row(row: &turso::Row) -> Result<SourceMessage> {
    Ok(SourceMessage {
        id: row.get(0).context("failed to read messages.id")?,
        session_id: row.get(1).context("failed to read messages.session_id")?,
        message_id: row.get(2).context("failed to read messages.message_id")?,
        role: row.get(3).context("failed to read messages.role")?,
        generated_at_unix_ms: row
            .get(4)
            .context("failed to read messages.generated_at_unix_ms")?,
    })
}

/// Validate the source role and prepare a message for destination loading.
pub fn transform_message(source: &SourceMessage) -> Result<TransformedMessage> {
    let role = match source.role.as_str() {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        other => bail!(
            "unsupported source messages.role '{other}' for message {}",
            source.message_id
        ),
    };

    Ok(TransformedMessage {
        source_row_id: source.id,
        session_id: source.session_id.clone(),
        message_id: source.message_id.clone(),
        role,
        generated_at_unix_ms: source.generated_at_unix_ms,
    })
}

/// Load one source batch atomically into the DWH messages table.
pub fn load_message_batch(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    source_rows: &[SourceMessage],
) -> Result<MessagesBatchStats> {
    let transformed = source_rows
        .iter()
        .map(transform_message)
        .collect::<Result<Vec<_>>>()?;
    load_transformed_message_batch(db, repository_id, source_instance_id, &transformed)
}

fn load_transformed_message_batch(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    rows: &[TransformedMessage],
) -> Result<MessagesBatchStats> {
    load_transformed_message_batch_with_failure(db, repository_id, source_instance_id, rows, None)
}

fn load_transformed_message_batch_with_failure(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    rows: &[TransformedMessage],
    fail_after_row: Option<usize>,
) -> Result<MessagesBatchStats> {
    let Some(last_row) = rows.last() else {
        return Ok(MessagesBatchStats::default());
    };

    db.transaction(|txn| {
        ensure_lineage(txn, repository_id, source_instance_id)?;

        let mut stats = TableBatchStats {
            watermark: last_row.source_row_id,
            ..Default::default()
        };

        for (index, row) in rows.iter().enumerate() {
            let existing = txn.query_map(
                "SELECT role, generated_at_unix_ms FROM messages
                 WHERE repository_id = ?1 AND session_id = ?2 AND message_id = ?3",
                (repository_id, row.session_id.as_str(), row.message_id.as_str()),
                |db_row| {
                    Ok((
                        db_row.get::<String>(0)?,
                        db_row.get::<i64>(1)?,
                    ))
                },
            )?;

            if let Some((existing_role, existing_timestamp)) = existing.into_iter().next() {
                let incoming_role = row.role.to_string();
                if existing_role != incoming_role || existing_timestamp != row.generated_at_unix_ms
                {
                    bail!(
                        "message integrity conflict for repository {repository_id}, session {}, message {}: existing role/timestamp {existing_role}/{existing_timestamp}, incoming {incoming_role}/{}",
                        row.session_id,
                        row.message_id,
                        row.generated_at_unix_ms
                    );
                }
                stats.already_present += 1;
            } else {
                txn.execute(
                    "INSERT INTO messages (repository_id, source_instance_id, session_id, message_id, role, generated_at_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        repository_id,
                        source_instance_id,
                        row.session_id.as_str(),
                        row.message_id.as_str(),
                        row.role.to_string(),
                        row.generated_at_unix_ms,
                    ),
                )?;
                stats.inserted += 1;
            }

            if fail_after_row == Some(index + 1) {
                bail!("injected messages destination failure after row {}", index + 1);
            }
        }

        upsert_watermark(
            txn,
            repository_id,
            source_instance_id,
            MESSAGES_SOURCE_TABLE,
            last_row.source_row_id,
        )?;

        Ok(MessagesBatchStats {
            inserted: stats.inserted,
            already_present: stats.already_present,
            watermark: stats.watermark,
        })
    })
}

fn ensure_lineage(
    txn: &TursoTransaction<'_, crate::services::agent_trace_dwh_db::AgentTraceDwhDbSpec>,
    repository_id: &str,
    source_instance_id: &str,
) -> Result<()> {
    txn.execute(
        "INSERT INTO repositories (repository_id) VALUES (?1)
         ON CONFLICT (repository_id) DO NOTHING",
        (repository_id,),
    )?;
    txn.execute(
        "INSERT INTO source_instances (repository_id, source_instance_id) VALUES (?1, ?2)
         ON CONFLICT (repository_id, source_instance_id) DO NOTHING",
        (repository_id, source_instance_id),
    )?;
    Ok(())
}

impl Default for MessagesEtl {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_MESSAGES_ETL_BATCH_SIZE,
        }
    }
}

impl MessagesEtl {
    /// Create a messages runner with a positive bounded source batch size.
    pub fn with_batch_size(batch_size: u32) -> Result<Self> {
        validate_batch_size(batch_size, "messages ETL")?;
        Ok(Self { batch_size })
    }

    /// Return the configured source batch size.
    pub fn batch_size(self) -> u32 {
        self.batch_size
    }

    /// Run the independently watermarked messages ETL through an open replica.
    pub fn run(
        self,
        repository_id: &str,
        source: &RepositoryAgentTraceDb,
        replica: &AgentTraceDwhReplica,
    ) -> Result<MessagesEtlStats> {
        let metadata = source
            .verify_or_initialize_repository_metadata(repository_id)
            .context("failed to verify Agent Trace source metadata")?;
        run_with_destination(
            self,
            repository_id,
            &metadata.source_instance_id,
            source,
            replica.db(),
        )
    }
}

fn run_with_destination(
    config: MessagesEtl,
    repository_id: &str,
    source_instance_id: &str,
    source: &RepositoryAgentTraceDb,
    destination: &AgentTraceDwhDb,
) -> Result<MessagesEtlStats> {
    let before_watermark = read_messages_watermark(destination, repository_id, source_instance_id)?;
    let mut watermark = before_watermark;
    let mut stats = MessagesEtlStats {
        before_watermark,
        after_watermark: before_watermark,
        ..Default::default()
    };

    loop {
        let rows = extract_message_batch(source, watermark, config.batch_size)?;
        if rows.is_empty() {
            break;
        }

        let batch = load_message_batch(destination, repository_id, source_instance_id, &rows)?;
        watermark = batch.watermark;
        stats.extracted += rows.len() as u64;
        stats.inserted += batch.inserted;
        stats.already_present += batch.already_present;
        stats.batches += 1;
        stats.after_watermark = watermark;
    }

    Ok(stats)
}

/// Run the default-sized messages ETL through an open DWH replica.
pub fn run_messages_etl(
    repository_id: &str,
    source: &RepositoryAgentTraceDb,
    replica: &AgentTraceDwhReplica,
) -> Result<MessagesEtlStats> {
    MessagesEtl::default().run(repository_id, source, replica)
}

/// Read the messages watermark, treating an absent row as zero.
pub fn read_messages_watermark(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
) -> Result<i64> {
    read_watermark(db, repository_id, source_instance_id, MESSAGES_SOURCE_TABLE)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::agent_trace_db::{InsertMessageInsert, MessageRole};

    fn unique_path(label: &str, file: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-conversation-messages-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join(file)
    }

    fn clean(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn insert_message(db: &RepositoryAgentTraceDb, id: &str, role: MessageRole, timestamp: i64) {
        db.insert_message(InsertMessageInsert {
            session_id: String::from("session-1"),
            message_id: id.to_string(),
            role,
            generated_at_unix_ms: timestamp,
        })
        .expect("source message insert should succeed");
    }

    fn source_row(
        id: i64,
        session_id: &str,
        message_id: &str,
        role: &str,
        timestamp: i64,
    ) -> SourceMessage {
        SourceMessage {
            id,
            session_id: session_id.to_string(),
            message_id: message_id.to_string(),
            role: role.to_string(),
            generated_at_unix_ms: timestamp,
        }
    }

    fn transformed_row(
        id: i64,
        session_id: &str,
        message_id: &str,
        role: MessageRole,
        timestamp: i64,
    ) -> TransformedMessage {
        TransformedMessage {
            source_row_id: id,
            session_id: session_id.to_string(),
            message_id: message_id.to_string(),
            role,
            generated_at_unix_ms: timestamp,
        }
    }

    #[test]
    fn conversation_messages_etl_extracts_ordered_bounded_batches_from_zero() {
        let source_path = unique_path("extract", "agent-trace.db");
        let source = RepositoryAgentTraceDb::new_at(&source_path).unwrap();
        insert_message(&source, "message-1", MessageRole::User, 1_000);
        insert_message(&source, "message-2", MessageRole::Assistant, 1_001);
        insert_message(&source, "message-3", MessageRole::User, 1_002);

        let rows = extract_message_batch(&source, 0, 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(rows[1].role, "assistant");
        assert!(extract_message_batch(&source, 3, 10).unwrap().is_empty());

        clean(&source_path);
    }

    #[test]
    fn conversation_messages_etl_rejects_unknown_roles() {
        let error = transform_message(&source_row(1, "session-1", "message-1", "system", 1_000))
            .expect_err("unsupported source roles must fail");
        assert!(error
            .to_string()
            .contains("unsupported source messages.role 'system'"));
    }

    #[test]
    fn conversation_messages_etl_inserts_lineage_content_and_watermark() {
        let dwh_path = unique_path("insert", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let rows = vec![
            source_row(1, "session-1", "message-1", "user", 1_000),
            source_row(2, "session-1", "message-2", "assistant", 1_001),
        ];

        let stats = load_message_batch(&dwh, "repo-a", "instance-a", &rows).unwrap();
        assert_eq!(stats.inserted, 2);
        assert_eq!(stats.watermark, 2);
        assert_eq!(
            read_messages_watermark(&dwh, "repo-a", "instance-a").unwrap(),
            2
        );
        let values = dwh
            .query_map(
                "SELECT source_instance_id, session_id, message_id, role, generated_at_unix_ms
                 FROM messages ORDER BY id",
                (),
                |row| {
                    Ok((
                        row.get::<String>(0)?,
                        row.get::<String>(1)?,
                        row.get::<String>(2)?,
                        row.get::<String>(3)?,
                        row.get::<i64>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            values[0],
            (
                "instance-a".into(),
                "session-1".into(),
                "message-1".into(),
                "user".into(),
                1_000
            )
        );
        assert_eq!(values[1].3, "assistant");

        clean(&dwh_path);
    }

    #[test]
    fn conversation_messages_identity_matching_replay_counts_already_present() {
        let dwh_path = unique_path("replay", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let row = source_row(7, "session-1", "message-1", "assistant", 1_000);

        assert_eq!(
            load_message_batch(&dwh, "repo-a", "instance-a", &[row.clone()])
                .unwrap()
                .inserted,
            1
        );
        let replay = load_message_batch(&dwh, "repo-a", "instance-a", &[row]).unwrap();
        assert_eq!(replay.already_present, 1);
        assert_eq!(
            dwh.query_map("SELECT COUNT(*) FROM messages", (), |db_row| db_row
                .get::<i64>(0)
                .map_err(Into::into))
                .unwrap(),
            vec![1]
        );

        clean(&dwh_path);
    }

    #[test]
    fn conversation_messages_identity_conflict_contains_logical_identity_and_rolls_back() {
        let dwh_path = unique_path("conflict", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let first = source_row(1, "session-1", "message-1", "user", 1_000);
        load_message_batch(&dwh, "repo-a", "instance-a", &[first]).unwrap();

        let error = load_message_batch(
            &dwh,
            "repo-a",
            "instance-a",
            &[source_row(2, "session-1", "message-1", "assistant", 1_001)],
        )
        .expect_err("a changed logical message must fail");
        let message = error.to_string();
        assert!(message.contains("repo-a"));
        assert!(message.contains("session-1"));
        assert!(message.contains("message-1"));
        assert_eq!(
            read_messages_watermark(&dwh, "repo-a", "instance-a").unwrap(),
            1
        );

        clean(&dwh_path);
    }

    #[test]
    fn conversation_messages_identity_rolls_back_facts_dimensions_and_watermark() {
        let dwh_path = unique_path("rollback", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let rows = vec![
            transformed_row(1, "session-1", "message-1", MessageRole::User, 1_000),
            transformed_row(2, "session-1", "message-2", MessageRole::Assistant, 1_001),
        ];

        load_transformed_message_batch_with_failure(&dwh, "repo-a", "instance-a", &rows, Some(1))
            .expect_err("injected failure should roll back the complete batch");

        for table in [
            "repositories",
            "source_instances",
            "messages",
            "etl_watermarks",
        ] {
            assert_eq!(
                dwh.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |row| row
                    .get::<i64>(0)
                    .map_err(Into::into))
                    .unwrap(),
                vec![0],
                "{table} should be rolled back"
            );
        }

        let replay = load_message_batch(
            &dwh,
            "repo-a",
            "instance-a",
            &[
                source_row(1, "session-1", "message-1", "user", 1_000),
                source_row(2, "session-1", "message-2", "assistant", 1_001),
            ],
        )
        .unwrap();
        assert_eq!(replay.inserted, 2);
        assert_eq!(replay.watermark, 2);

        clean(&dwh_path);
    }

    #[test]
    fn conversation_messages_etl_rejects_zero_batch_size() {
        let source_path = unique_path("zero-batch", "agent-trace.db");
        let source = RepositoryAgentTraceDb::new_at(&source_path).unwrap();
        let error = extract_message_batch(&source, 0, 0).expect_err("zero batch must fail");
        assert!(error.to_string().contains("batch_size"));
        clean(&source_path);
    }

    #[test]
    fn conversation_messages_etl_config_rejects_zero_batch_size() {
        let error = MessagesEtl::with_batch_size(0).expect_err("zero batch must fail");
        assert!(error.to_string().contains("batch_size"));
    }
}
