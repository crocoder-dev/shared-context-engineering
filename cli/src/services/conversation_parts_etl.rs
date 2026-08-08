//! Incremental repository-source to DWH ETL for message parts.
//!
//! Parts retain source lineage and the exact source text. Source extraction is
//! short and read-only; transformation and destination loading happen after
//! the source snapshot ends, and each batch's facts, dimensions, and `parts`
//! watermark commit in one destination transaction.

use std::fmt::Write;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::services::{
    agent_trace_db::{repository::RepositoryAgentTraceDb, PartType},
    agent_trace_dwh_db::AgentTraceDwhDb,
    agent_trace_dwh_replica::AgentTraceDwhReplica,
    db::TursoTransaction,
    etl::{
        read_watermark, run_with_source_contention_retry, upsert_watermark, validate_batch_size,
        TableBatchStats,
    },
};

/// The source table represented by this pipeline.
pub const PARTS_SOURCE_TABLE: &str = "parts";

/// Default number of source parts processed by one batch.
pub const DEFAULT_PARTS_ETL_BATCH_SIZE: u32 = 500;

/// One source message part copied from a short repository database snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceMessagePart {
    pub id: i64,
    pub part_type: String,
    pub text: String,
    pub message_id: String,
    pub session_id: String,
    pub generated_at_unix_ms: i64,
}

/// A source part after type validation and exact-text hashing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransformedMessagePart {
    pub source_part_id: i64,
    pub part_type: PartType,
    pub text: String,
    pub text_sha256: String,
    pub message_id: String,
    pub session_id: String,
    pub generated_at_unix_ms: i64,
}

/// Counts returned by one atomically loaded parts batch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PartsBatchStats {
    pub inserted: u64,
    pub already_present: u64,
    pub watermark: i64,
}

/// Summary of one complete incremental parts ETL run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PartsEtlStats {
    pub extracted: u64,
    pub inserted: u64,
    pub already_present: u64,
    pub batches: u64,
    pub before_watermark: i64,
    pub after_watermark: i64,
}

/// Configuration for the independently watermarked parts pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartsEtl {
    batch_size: u32,
}

const SELECT_PARTS_BATCH_SQL: &str =
    "SELECT id, type, text, message_id, session_id, generated_at_unix_ms
FROM parts
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2";

/// Extract one bounded, ascending source batch in a short read transaction.
///
/// Only transient database/table lock contention is retried. The source read
/// transaction is committed before this function returns, so transformation
/// and destination work never hold a source snapshot open.
pub fn extract_part_batch(
    db: &RepositoryAgentTraceDb,
    watermark: i64,
    batch_size: u32,
) -> Result<Vec<SourceMessagePart>> {
    validate_batch_size(batch_size, "parts extraction")?;

    run_with_source_contention_retry(
        |_attempt| {
            db.read_transaction(|txn| {
                txn.query_map(
                    SELECT_PARTS_BATCH_SQL,
                    (watermark, i64::from(batch_size)),
                    source_message_part_from_row,
                )
            })
        },
        || db.rollback_best_effort(),
    )
}

fn source_message_part_from_row(row: &turso::Row) -> Result<SourceMessagePart> {
    Ok(SourceMessagePart {
        id: row.get(0).context("failed to read parts.id")?,
        part_type: row.get(1).context("failed to read parts.type")?,
        text: row.get(2).context("failed to read parts.text")?,
        message_id: row.get(3).context("failed to read parts.message_id")?,
        session_id: row.get(4).context("failed to read parts.session_id")?,
        generated_at_unix_ms: row
            .get(5)
            .context("failed to read parts.generated_at_unix_ms")?,
    })
}

/// Validate the source part type and hash the exact UTF-8 text bytes.
pub fn transform_message_part(source: &SourceMessagePart) -> Result<TransformedMessagePart> {
    let part_type = match source.part_type.as_str() {
        "text" => PartType::Text,
        "reasoning" => PartType::Reasoning,
        "patch" => PartType::Patch,
        "question" => PartType::Question,
        other => bail!(
            "unsupported source parts.type '{other}' for part {}",
            source.id
        ),
    };

    Ok(TransformedMessagePart {
        source_part_id: source.id,
        part_type,
        text: source.text.clone(),
        text_sha256: sha256_hex(source.text.as_bytes()),
        message_id: source.message_id.clone(),
        session_id: source.session_id.clone(),
        generated_at_unix_ms: source.generated_at_unix_ms,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().fold(
        String::with_capacity(digest.len() * 2),
        |mut output, byte| {
            write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
            output
        },
    )
}

/// Load one source batch atomically into the DWH `message_parts` table.
pub fn load_part_batch(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    source_rows: &[SourceMessagePart],
) -> Result<PartsBatchStats> {
    let transformed = source_rows
        .iter()
        .map(transform_message_part)
        .collect::<Result<Vec<_>>>()?;
    load_transformed_part_batch(db, repository_id, source_instance_id, &transformed)
}

fn load_transformed_part_batch(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    rows: &[TransformedMessagePart],
) -> Result<PartsBatchStats> {
    load_transformed_part_batch_with_failure(db, repository_id, source_instance_id, rows, None)
}

fn load_transformed_part_batch_with_failure(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    rows: &[TransformedMessagePart],
    fail_after_row: Option<usize>,
) -> Result<PartsBatchStats> {
    let Some(last_row) = rows.last() else {
        return Ok(PartsBatchStats::default());
    };

    db.transaction(|txn| {
        ensure_lineage(txn, repository_id, source_instance_id)?;

        let mut stats = TableBatchStats {
            watermark: last_row.source_part_id,
            ..Default::default()
        };

        for (index, row) in rows.iter().enumerate() {
            let existing = txn.query_map(
                "SELECT session_id, message_id, part_type, text, text_sha256, generated_at_unix_ms
                 FROM message_parts
                 WHERE repository_id = ?1 AND source_instance_id = ?2 AND source_part_id = ?3",
                (repository_id, source_instance_id, row.source_part_id),
                |db_row| {
                    Ok((
                        db_row.get::<String>(0)?,
                        db_row.get::<String>(1)?,
                        db_row.get::<String>(2)?,
                        db_row.get::<String>(3)?,
                        db_row.get::<String>(4)?,
                        db_row.get::<i64>(5)?,
                    ))
                },
            )?;

            if let Some((
                existing_session_id,
                existing_message_id,
                existing_part_type,
                existing_text,
                existing_hash,
                existing_timestamp,
            )) = existing.into_iter().next()
            {
                let incoming_part_type = row.part_type.to_string();
                if existing_session_id != row.session_id
                    || existing_message_id != row.message_id
                    || existing_part_type != incoming_part_type
                    || existing_text != row.text
                    || existing_hash != row.text_sha256
                    || existing_timestamp != row.generated_at_unix_ms
                {
                    bail!(
                        "message part integrity conflict for repository {repository_id}, source instance {source_instance_id}, source part {}",
                        row.source_part_id
                    );
                }
                stats.already_present += 1;
            } else {
                txn.execute(
                    "INSERT INTO message_parts (repository_id, source_instance_id, session_id, message_id, source_part_id, part_type, text, text_sha256, generated_at_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    (
                        repository_id,
                        source_instance_id,
                        row.session_id.as_str(),
                        row.message_id.as_str(),
                        row.source_part_id,
                        row.part_type.to_string(),
                        row.text.as_str(),
                        row.text_sha256.as_str(),
                        row.generated_at_unix_ms,
                    ),
                )?;
                stats.inserted += 1;
            }

            if fail_after_row == Some(index + 1) {
                bail!(
                    "injected message parts destination failure after row {}",
                    index + 1
                );
            }
        }

        upsert_watermark(
            txn,
            repository_id,
            source_instance_id,
            PARTS_SOURCE_TABLE,
            last_row.source_part_id,
        )?;

        Ok(PartsBatchStats {
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

impl Default for PartsEtl {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_PARTS_ETL_BATCH_SIZE,
        }
    }
}

impl PartsEtl {
    /// Create a parts runner with a positive bounded source batch size.
    pub fn with_batch_size(batch_size: u32) -> Result<Self> {
        validate_batch_size(batch_size, "parts ETL")?;
        Ok(Self { batch_size })
    }

    /// Return the configured source batch size.
    pub fn batch_size(self) -> u32 {
        self.batch_size
    }

    /// Run the independently watermarked parts ETL through an open replica.
    pub fn run(
        self,
        repository_id: &str,
        source: &RepositoryAgentTraceDb,
        replica: &AgentTraceDwhReplica,
    ) -> Result<PartsEtlStats> {
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
    config: PartsEtl,
    repository_id: &str,
    source_instance_id: &str,
    source: &RepositoryAgentTraceDb,
    destination: &AgentTraceDwhDb,
) -> Result<PartsEtlStats> {
    let before_watermark = read_parts_watermark(destination, repository_id, source_instance_id)?;
    let mut watermark = before_watermark;
    let mut stats = PartsEtlStats {
        before_watermark,
        after_watermark: before_watermark,
        ..Default::default()
    };

    loop {
        let rows = extract_part_batch(source, watermark, config.batch_size)?;
        if rows.is_empty() {
            break;
        }

        let batch = load_part_batch(destination, repository_id, source_instance_id, &rows)?;
        watermark = batch.watermark;
        stats.extracted += rows.len() as u64;
        stats.inserted += batch.inserted;
        stats.already_present += batch.already_present;
        stats.batches += 1;
        stats.after_watermark = watermark;
    }

    Ok(stats)
}

/// Run the default-sized parts ETL through an open DWH replica.
pub fn run_parts_etl(
    repository_id: &str,
    source: &RepositoryAgentTraceDb,
    replica: &AgentTraceDwhReplica,
) -> Result<PartsEtlStats> {
    PartsEtl::default().run(repository_id, source, replica)
}

/// Read the parts watermark, treating an absent row as zero.
pub fn read_parts_watermark(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
) -> Result<i64> {
    read_watermark(db, repository_id, source_instance_id, PARTS_SOURCE_TABLE)
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
    use crate::services::agent_trace_db::{InsertPartInsert, PartType};

    fn unique_path(label: &str, file: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-conversation-parts-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join(file)
    }

    fn clean(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn source_row(
        id: i64,
        part_type: &str,
        text: &str,
        session_id: &str,
        message_id: &str,
        timestamp: i64,
    ) -> SourceMessagePart {
        SourceMessagePart {
            id,
            part_type: part_type.to_string(),
            text: text.to_string(),
            message_id: message_id.to_string(),
            session_id: session_id.to_string(),
            generated_at_unix_ms: timestamp,
        }
    }

    fn transformed_row(
        id: i64,
        part_type: PartType,
        text: &str,
        session_id: &str,
        message_id: &str,
        timestamp: i64,
    ) -> TransformedMessagePart {
        TransformedMessagePart {
            source_part_id: id,
            part_type,
            text: text.to_string(),
            text_sha256: sha256_hex(text.as_bytes()),
            message_id: message_id.to_string(),
            session_id: session_id.to_string(),
            generated_at_unix_ms: timestamp,
        }
    }

    fn insert_part(
        db: &RepositoryAgentTraceDb,
        part_type: PartType,
        text: &str,
        session_id: &str,
        message_id: &str,
        timestamp: i64,
    ) {
        db.insert_part(InsertPartInsert {
            part_type,
            text: text.to_string(),
            session_id: session_id.to_string(),
            message_id: message_id.to_string(),
            generated_at_unix_ms: timestamp,
        })
        .expect("source part insert should succeed");
    }

    #[test]
    fn conversation_parts_etl_extracts_ordered_bounded_batches_from_zero() {
        let source_path = unique_path("extract", "agent-trace.db");
        let source = RepositoryAgentTraceDb::new_at(&source_path).unwrap();
        insert_part(
            &source,
            PartType::Text,
            "one",
            "session-1",
            "message-1",
            1_000,
        );
        insert_part(
            &source,
            PartType::Reasoning,
            "two",
            "session-1",
            "message-1",
            1_001,
        );
        insert_part(
            &source,
            PartType::Patch,
            "three",
            "session-1",
            "message-1",
            1_002,
        );

        let rows = extract_part_batch(&source, 0, 2).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(rows[0].part_type, "text");
        assert_eq!(rows[1].part_type, "reasoning");
        assert!(extract_part_batch(&source, 3, 10).unwrap().is_empty());

        clean(&source_path);
    }

    #[test]
    fn conversation_parts_etl_accepts_only_supported_types() {
        for part_type in ["text", "reasoning", "patch", "question"] {
            let transformed = transform_message_part(&source_row(
                1,
                part_type,
                "content",
                "session-1",
                "message-1",
                1_000,
            ))
            .unwrap();
            assert_eq!(transformed.part_type.to_string(), part_type);
        }

        let error = transform_message_part(&source_row(
            1,
            "tool",
            "content",
            "session-1",
            "message-1",
            1_000,
        ))
        .expect_err("unknown part types must fail");
        assert!(error
            .to_string()
            .contains("unsupported source parts.type 'tool'"));
    }

    #[test]
    fn conversation_parts_etl_preserves_text_and_uses_lowercase_sha256() {
        let text = "line 1\r\n\u{0000}unicode: café";
        let transformed = transform_message_part(&source_row(
            7,
            "text",
            text,
            "session-1",
            "message-1",
            1_000,
        ))
        .unwrap();
        assert_eq!(transformed.text, text);
        assert_eq!(transformed.text_sha256, sha256_hex(text.as_bytes()));
        assert_eq!(
            transformed.text_sha256,
            "5d6fb926a3bfcd8394b33e5a5aecaa23a5feb92d5d13a596818f929b26ee3221"
        );
    }

    #[test]
    fn conversation_parts_etl_inserts_lineage_content_and_watermark_without_parent() {
        let dwh_path = unique_path("insert", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let rows = vec![
            source_row(1, "text", "hello", "session-1", "missing-message", 1_000),
            source_row(2, "patch", "diff", "session-1", "missing-message", 1_001),
        ];

        let stats = load_part_batch(&dwh, "repo-a", "instance-a", &rows).unwrap();
        assert_eq!(stats.inserted, 2);
        assert_eq!(stats.watermark, 2);
        assert_eq!(
            read_parts_watermark(&dwh, "repo-a", "instance-a").unwrap(),
            2
        );
        let values = dwh
            .query_map(
                "SELECT source_instance_id, session_id, message_id, source_part_id, part_type, text, text_sha256
                 FROM message_parts ORDER BY source_part_id",
                (),
                |row| {
                    Ok((
                        row.get::<String>(0)?,
                        row.get::<String>(1)?,
                        row.get::<String>(2)?,
                        row.get::<i64>(3)?,
                        row.get::<String>(4)?,
                        row.get::<String>(5)?,
                        row.get::<String>(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(values[0].0, "instance-a");
        assert_eq!(values[0].2, "missing-message");
        assert_eq!(values[0].4, "text");
        assert_eq!(values[0].5, "hello");
        assert_eq!(values[0].6, sha256_hex(b"hello"));

        clean(&dwh_path);
    }

    #[test]
    fn conversation_parts_identity_matching_replay_counts_already_present() {
        let dwh_path = unique_path("replay", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let row = source_row(7, "text", "hello", "session-1", "message-1", 1_000);

        assert_eq!(
            load_part_batch(&dwh, "repo-a", "instance-a", std::slice::from_ref(&row))
                .unwrap()
                .inserted,
            1
        );
        let replay = load_part_batch(&dwh, "repo-a", "instance-a", &[row]).unwrap();
        assert_eq!(replay.already_present, 1);
        assert_eq!(
            dwh.query_map("SELECT COUNT(*) FROM message_parts", (), |db_row| db_row
                .get::<i64>(0)
                .map_err(Into::into))
                .unwrap(),
            vec![1]
        );

        clean(&dwh_path);
    }

    #[test]
    fn conversation_parts_identity_conflict_fails_without_overwrite() {
        let dwh_path = unique_path("conflict", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let first = source_row(1, "text", "hello", "session-1", "message-1", 1_000);
        load_part_batch(&dwh, "repo-a", "instance-a", &[first]).unwrap();

        let error = load_part_batch(
            &dwh,
            "repo-a",
            "instance-a",
            &[source_row(
                1,
                "text",
                "changed",
                "session-1",
                "message-1",
                1_000,
            )],
        )
        .expect_err("changed source-lineage content must fail");
        assert!(error
            .to_string()
            .contains("message part integrity conflict"));
        assert_eq!(
            read_parts_watermark(&dwh, "repo-a", "instance-a").unwrap(),
            1
        );
        assert_eq!(
            dwh.query_map("SELECT text FROM message_parts", (), |row| row
                .get::<String>(0)
                .map_err(Into::into))
                .unwrap(),
            vec![String::from("hello")]
        );

        clean(&dwh_path);
    }

    #[test]
    fn conversation_parts_identity_rolls_back_facts_dimensions_and_watermark() {
        let dwh_path = unique_path("rollback", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let rows = vec![
            transformed_row(1, PartType::Text, "one", "session-1", "message-1", 1_000),
            transformed_row(2, PartType::Patch, "two", "session-1", "message-1", 1_001),
        ];

        load_transformed_part_batch_with_failure(&dwh, "repo-a", "instance-a", &rows, Some(1))
            .expect_err("injected failure should roll back the complete batch");

        for table in [
            "repositories",
            "source_instances",
            "message_parts",
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

        let replay = load_part_batch(
            &dwh,
            "repo-a",
            "instance-a",
            &[
                source_row(1, "text", "one", "session-1", "message-1", 1_000),
                source_row(2, "patch", "two", "session-1", "message-1", 1_001),
            ],
        )
        .unwrap();
        assert_eq!(replay.inserted, 2);
        assert_eq!(replay.watermark, 2);

        clean(&dwh_path);
    }

    #[test]
    fn conversation_parts_same_local_ids_from_different_source_instances_coexist() {
        let dwh_path = unique_path("lineage", "agent-trace-dwh.db");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).unwrap();
        let row = source_row(1, "text", "hello", "session-1", "message-1", 1_000);

        load_part_batch(&dwh, "repo-a", "instance-a", std::slice::from_ref(&row)).unwrap();
        load_part_batch(&dwh, "repo-a", "instance-b", &[row]).unwrap();

        assert_eq!(
            dwh.query_map(
                "SELECT COUNT(*) FROM message_parts WHERE repository_id = 'repo-a'",
                (),
                |db_row| db_row.get::<i64>(0).map_err(Into::into)
            )
            .unwrap(),
            vec![2]
        );
        clean(&dwh_path);
    }

    #[test]
    fn conversation_parts_etl_rejects_zero_batch_size() {
        let source_path = unique_path("zero-batch", "agent-trace.db");
        let source = RepositoryAgentTraceDb::new_at(&source_path).unwrap();
        let error = extract_part_batch(&source, 0, 0).expect_err("zero batch must fail");
        assert!(error.to_string().contains("batch_size"));
        clean(&source_path);
    }

    #[test]
    fn conversation_parts_etl_config_rejects_zero_batch_size() {
        let error = PartsEtl::with_batch_size(0).expect_err("zero batch must fail");
        assert!(error.to_string().contains("batch_size"));
    }

    #[test]
    fn concurrent_source_writer_is_not_blocked_by_parts_read_transaction() {
        let source_path = unique_path("concurrent-writer", "agent-trace.db");
        let source = RepositoryAgentTraceDb::new_at(&source_path).unwrap();
        insert_part(
            &source,
            PartType::Text,
            "first",
            "session-1",
            "message-1",
            1_000,
        );

        let (reader_ready_tx, reader_ready_rx) = mpsc::channel::<()>();
        let (release_reader_tx, release_reader_rx) = mpsc::channel::<()>();
        let reader_db_path = source_path.clone();
        let reader_handle = thread::spawn(move || {
            let reader_db = RepositoryAgentTraceDb::open_without_migrations_at(&reader_db_path)
                .expect("reader connection should reopen");
            reader_db.read_transaction(|txn| {
                let rows = txn.query_map(
                    SELECT_PARTS_BATCH_SQL,
                    (0i64, 10i64),
                    source_message_part_from_row,
                )?;
                reader_ready_tx
                    .send(())
                    .expect("reader should signal it holds an open read transaction");
                release_reader_rx
                    .recv()
                    .expect("reader should wait to be released while holding the transaction open");
                Ok(rows)
            })
        });

        reader_ready_rx
            .recv()
            .expect("test should observe the parts reader transaction");
        let writer_db = RepositoryAgentTraceDb::open_without_migrations_at(&source_path)
            .expect("writer connection should reopen");
        insert_part(
            &writer_db,
            PartType::Question,
            "second",
            "session-1",
            "message-1",
            1_001,
        );

        release_reader_tx
            .send(())
            .expect("reader should be released");
        let rows = reader_handle
            .join()
            .expect("reader thread should not panic")
            .expect("reader transaction should commit");
        assert_eq!(rows.len(), 1);

        clean(&source_path);
    }
}
