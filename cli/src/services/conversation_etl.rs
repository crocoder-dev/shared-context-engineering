//! Conversation-level composition for the independently watermarked messages
//! and message-parts ETL runners.
//!
//! This service owns only local table-runner composition. Source extraction,
//! destination transactions, replica ownership, and table-specific identity
//! rules remain in the sibling modules; pull/push and credential handling are
//! deliberately outside this API.

use anyhow::{Context, Result};

use crate::services::{
    agent_trace_db::repository::RepositoryAgentTraceDb,
    agent_trace_dwh_db::AgentTraceDwhDb,
    agent_trace_dwh_replica::AgentTraceDwhReplica,
    conversation_messages_etl::{MessagesEtl, MessagesEtlStats},
    conversation_parts_etl::{PartsEtl, PartsEtlStats},
};

/// Configuration for both conversation table runners.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConversationEtl {
    messages: MessagesEtl,
    parts: PartsEtl,
}

/// Results for one complete conversation ETL run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConversationEtlStats {
    pub messages: MessagesEtlStats,
    pub parts: PartsEtlStats,
}

impl ConversationEtl {
    /// Create a runner with independently validated message and part batch
    /// sizes.
    pub fn with_batch_sizes(message_batch_size: u32, part_batch_size: u32) -> Result<Self> {
        Ok(Self {
            messages: MessagesEtl::with_batch_size(message_batch_size)?,
            parts: PartsEtl::with_batch_size(part_batch_size)?,
        })
    }

    /// Create a runner using one validated batch size for both tables.
    pub fn with_batch_size(batch_size: u32) -> Result<Self> {
        Self::with_batch_sizes(batch_size, batch_size)
    }

    /// Return a copy configured with a new message batch size.
    pub fn with_message_batch_size(self, batch_size: u32) -> Result<Self> {
        Ok(Self {
            messages: MessagesEtl::with_batch_size(batch_size)?,
            ..self
        })
    }

    /// Return a copy configured with a new part batch size.
    pub fn with_part_batch_size(self, batch_size: u32) -> Result<Self> {
        Ok(Self {
            parts: PartsEtl::with_batch_size(batch_size)?,
            ..self
        })
    }

    /// Return the configured message batch size.
    pub fn message_batch_size(self) -> u32 {
        self.messages.batch_size()
    }

    /// Return the configured part batch size.
    pub fn part_batch_size(self) -> u32 {
        self.parts.batch_size()
    }

    /// Run messages and parts through the lock-owning DWH replica.
    ///
    /// Metadata is verified once before the two table runners execute. Each
    /// table still reads and commits its own watermark independently: a
    /// successful messages run is not part of a transaction with the parts
    /// run, and a parts failure cannot roll it back.
    pub fn run(
        self,
        repository_id: &str,
        source: &RepositoryAgentTraceDb,
        replica: &AgentTraceDwhReplica,
    ) -> Result<ConversationEtlStats> {
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

pub(crate) fn run_with_destination(
    config: ConversationEtl,
    repository_id: &str,
    source_instance_id: &str,
    source: &RepositoryAgentTraceDb,
    destination: &AgentTraceDwhDb,
) -> Result<ConversationEtlStats> {
    let messages = crate::services::conversation_messages_etl::run_with_destination(
        config.messages,
        repository_id,
        source_instance_id,
        source,
        destination,
    )?;
    let parts = crate::services::conversation_parts_etl::run_with_destination(
        config.parts,
        repository_id,
        source_instance_id,
        source,
        destination,
    )?;

    Ok(ConversationEtlStats { messages, parts })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::mpsc,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::agent_trace_db::{
        repository::RepositoryAgentTraceDb, InsertMessageInsert, InsertPartInsert, MessageRole,
        PartType,
    };

    fn unique_path(label: &str, file: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-conversation-etl-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join(file)
    }

    fn clean(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn insert_message(db: &RepositoryAgentTraceDb, message_id: &str, timestamp: i64) {
        db.insert_message(InsertMessageInsert {
            session_id: String::from("session-1"),
            message_id: message_id.to_string(),
            role: MessageRole::User,
            generated_at_unix_ms: timestamp,
        })
        .expect("source message insert should succeed");
    }

    fn insert_part(db: &RepositoryAgentTraceDb, message_id: &str, text: &str, timestamp: i64) {
        db.insert_part(InsertPartInsert {
            part_type: PartType::Text,
            text: text.to_string(),
            session_id: String::from("session-1"),
            message_id: message_id.to_string(),
            generated_at_unix_ms: timestamp,
        })
        .expect("source part insert should succeed");
    }

    fn open_source_and_destination(
        label: &str,
    ) -> (
        PathBuf,
        RepositoryAgentTraceDb,
        PathBuf,
        AgentTraceDwhDb,
        String,
    ) {
        let source_path = unique_path(label, "agent-trace.db");
        let dwh_path = unique_path(label, "agent-trace-dwh.db");
        let source = RepositoryAgentTraceDb::new_at(&source_path).unwrap();
        let destination = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let metadata = source
            .verify_or_initialize_repository_metadata("repo-a")
            .unwrap();
        (
            source_path,
            source,
            dwh_path,
            destination,
            metadata.source_instance_id,
        )
    }

    #[test]
    fn conversation_etl_defaults_to_independent_500_row_batches() {
        let config = ConversationEtl::default();
        assert_eq!(config.message_batch_size(), 500);
        assert_eq!(config.part_batch_size(), 500);

        let config = ConversationEtl::with_batch_sizes(2, 3).unwrap();
        assert_eq!(config.message_batch_size(), 2);
        assert_eq!(config.part_batch_size(), 3);
    }

    #[test]
    fn conversation_etl_rejects_invalid_batch_sizes() {
        assert!(ConversationEtl::with_batch_sizes(0, 1).is_err());
        assert!(ConversationEtl::with_batch_sizes(1, 0).is_err());
        assert!(ConversationEtl::default()
            .with_message_batch_size(0)
            .is_err());
        assert!(ConversationEtl::default().with_part_batch_size(0).is_err());
    }

    #[test]
    fn conversation_etl_runs_initial_incremental_and_noop_batches() {
        let (source_path, source, dwh_path, destination, source_instance_id) =
            open_source_and_destination("growth");
        insert_message(&source, "message-1", 1_000);
        insert_part(&source, "message-1", "one", 1_001);

        let config = ConversationEtl::with_batch_sizes(1, 1).unwrap();
        let first =
            run_with_destination(config, "repo-a", &source_instance_id, &source, &destination)
                .unwrap();
        assert_eq!(first.messages.extracted, 1);
        assert_eq!(first.messages.inserted, 1);
        assert_eq!(first.parts.extracted, 1);
        assert_eq!(first.parts.inserted, 1);
        assert_eq!(first.messages.after_watermark, 1);
        assert_eq!(first.parts.after_watermark, 1);

        insert_message(&source, "message-2", 2_000);
        insert_part(&source, "message-2", "two", 2_001);
        let second =
            run_with_destination(config, "repo-a", &source_instance_id, &source, &destination)
                .unwrap();
        assert_eq!(second.messages.before_watermark, 1);
        assert_eq!(second.messages.extracted, 1);
        assert_eq!(second.messages.after_watermark, 2);
        assert_eq!(second.parts.before_watermark, 1);
        assert_eq!(second.parts.extracted, 1);
        assert_eq!(second.parts.after_watermark, 2);

        let noop =
            run_with_destination(config, "repo-a", &source_instance_id, &source, &destination)
                .unwrap();
        assert_eq!(noop.messages.extracted, 0);
        assert_eq!(noop.messages.batches, 0);
        assert_eq!(noop.messages.before_watermark, 2);
        assert_eq!(noop.messages.after_watermark, 2);
        assert_eq!(noop.parts.extracted, 0);
        assert_eq!(noop.parts.batches, 0);
        assert_eq!(noop.parts.before_watermark, 2);
        assert_eq!(noop.parts.after_watermark, 2);

        clean(&source_path);
        clean(&dwh_path);
    }

    #[test]
    fn conversation_etl_advances_message_and_part_watermarks_independently() {
        let (source_path, source, dwh_path, destination, source_instance_id) =
            open_source_and_destination("independent-progress");
        insert_message(&source, "message-1", 1_000);
        insert_part(&source, "message-1", "one", 1_000);

        let config = ConversationEtl::with_batch_size(10).unwrap();
        run_with_destination(config, "repo-a", &source_instance_id, &source, &destination).unwrap();

        insert_part(&source, "message-1", "two", 1_001);
        let parts_only =
            run_with_destination(config, "repo-a", &source_instance_id, &source, &destination)
                .unwrap();
        assert_eq!(parts_only.messages.extracted, 0);
        assert_eq!(parts_only.messages.before_watermark, 1);
        assert_eq!(parts_only.parts.extracted, 1);
        assert_eq!(parts_only.parts.before_watermark, 1);
        assert_eq!(parts_only.parts.after_watermark, 2);

        insert_message(&source, "message-2", 2_000);
        let messages_only =
            run_with_destination(config, "repo-a", &source_instance_id, &source, &destination)
                .unwrap();
        assert_eq!(messages_only.messages.extracted, 1);
        assert_eq!(messages_only.messages.before_watermark, 1);
        assert_eq!(messages_only.messages.after_watermark, 2);
        assert_eq!(messages_only.parts.extracted, 0);
        assert_eq!(messages_only.parts.before_watermark, 2);
        assert_eq!(messages_only.parts.after_watermark, 2);

        clean(&source_path);
        clean(&dwh_path);
    }

    #[test]
    fn conversation_etl_allows_parts_before_messages_and_reconstructs_equal_timestamps() {
        let (source_path, source, dwh_path, destination, source_instance_id) =
            open_source_and_destination("out-of-order");
        insert_part(&source, "message-later", "first", 1_000);
        insert_part(&source, "message-later", "second", 1_000);

        let config = ConversationEtl::with_batch_size(10).unwrap();
        let parts_first =
            run_with_destination(config, "repo-a", &source_instance_id, &source, &destination)
                .unwrap();
        assert_eq!(parts_first.messages.extracted, 0);
        assert_eq!(parts_first.parts.inserted, 2);

        let before_parent = destination
            .query_map("SELECT COUNT(*) FROM messages", (), |row| {
                row.get::<i64>(0).map_err(Into::into)
            })
            .unwrap();
        assert_eq!(before_parent, vec![0]);

        let ordered = destination
            .query_map(
                "SELECT text FROM message_parts
                 WHERE repository_id = ?1 AND session_id = ?2 AND message_id = ?3
                 ORDER BY generated_at_unix_ms ASC, source_part_id ASC",
                ("repo-a", "session-1", "message-later"),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .unwrap();
        assert_eq!(ordered, vec![String::from("first"), String::from("second")]);

        insert_message(&source, "message-later", 1_001);
        let message_after_parts =
            run_with_destination(config, "repo-a", &source_instance_id, &source, &destination)
                .unwrap();
        assert_eq!(message_after_parts.messages.inserted, 1);
        assert_eq!(message_after_parts.parts.extracted, 0);

        clean(&source_path);
        clean(&dwh_path);
    }

    #[test]
    fn conversation_etl_source_read_transactions_do_not_block_message_writers() {
        let source_path = unique_path("messages-writer", "agent-trace.db");
        let source = RepositoryAgentTraceDb::new_at(&source_path).unwrap();
        insert_message(&source, "message-1", 1_000);
        let (reader_ready_tx, reader_ready_rx) = mpsc::channel::<()>();
        let (release_reader_tx, release_reader_rx) = mpsc::channel::<()>();
        let reader_path = source_path.clone();
        let reader = thread::spawn(move || {
            let reader_db = RepositoryAgentTraceDb::open_without_migrations_at(&reader_path)
                .expect("reader connection should reopen");
            reader_db.read_transaction(|txn| {
                txn.query_map("SELECT id FROM messages ORDER BY id", (), |row| {
                    row.get::<i64>(0).map_err(Into::into)
                })?;
                reader_ready_tx
                    .send(())
                    .expect("reader should signal readiness");
                release_reader_rx
                    .recv()
                    .expect("reader should wait while holding the snapshot");
                Ok(())
            })
        });

        reader_ready_rx
            .recv()
            .expect("test should observe the open message snapshot");
        let writer = RepositoryAgentTraceDb::open_without_migrations_at(&source_path).unwrap();
        insert_message(&writer, "message-2", 1_001);
        release_reader_tx.send(()).unwrap();
        reader
            .join()
            .expect("reader thread should not panic")
            .expect("reader transaction should commit");
        clean(&source_path);
    }

    #[test]
    fn conversation_etl_source_read_transactions_do_not_block_part_writers() {
        let source_path = unique_path("parts-writer", "agent-trace.db");
        let source = RepositoryAgentTraceDb::new_at(&source_path).unwrap();
        insert_part(&source, "message-1", "one", 1_000);
        let (reader_ready_tx, reader_ready_rx) = mpsc::channel::<()>();
        let (release_reader_tx, release_reader_rx) = mpsc::channel::<()>();
        let reader_path = source_path.clone();
        let reader = thread::spawn(move || {
            let reader_db = RepositoryAgentTraceDb::open_without_migrations_at(&reader_path)
                .expect("reader connection should reopen");
            reader_db.read_transaction(|txn| {
                txn.query_map("SELECT id FROM parts ORDER BY id", (), |row| {
                    row.get::<i64>(0).map_err(Into::into)
                })?;
                reader_ready_tx
                    .send(())
                    .expect("reader should signal readiness");
                release_reader_rx
                    .recv()
                    .expect("reader should wait while holding the snapshot");
                Ok(())
            })
        });

        reader_ready_rx
            .recv()
            .expect("test should observe the open parts snapshot");
        let writer = RepositoryAgentTraceDb::open_without_migrations_at(&source_path).unwrap();
        insert_part(&writer, "message-1", "two", 1_001);
        release_reader_tx.send(()).unwrap();
        reader
            .join()
            .expect("reader thread should not panic")
            .expect("reader transaction should commit");
        clean(&source_path);
    }
}
