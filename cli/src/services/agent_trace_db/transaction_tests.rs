use std::path::Path;
use std::time::Duration;

use super::repository::RepositoryAgentTraceDb;
use super::{InsertMessageInsert, InsertPartInsert, MessageRole, PartType};

pub(crate) fn message(id: &str) -> InsertMessageInsert {
    InsertMessageInsert {
        session_id: "session".to_string(),
        message_id: id.to_string(),
        role: MessageRole::User,
        generated_at_unix_ms: 1,
    }
}

pub(crate) fn part(id: &str) -> InsertPartInsert {
    InsertPartInsert {
        part_type: PartType::Text,
        text: "text".to_string(),
        session_id: "session".to_string(),
        message_id: id.to_string(),
        generated_at_unix_ms: 1,
    }
}

async fn count(db: &RepositoryAgentTraceDb, table: &str, id: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE message_id = ?1");
    db.query_map(&sql, (id.to_string(),), |row| {
        row.get::<i64>(0).map_err(Into::into)
    })
    .await
    .unwrap()[0]
}

pub(crate) async fn pair_counts(db: &RepositoryAgentTraceDb, id: &str) -> (i64, i64) {
    (
        count(db, "messages", id).await,
        count(db, "parts", id).await,
    )
}

pub(crate) async fn open(path: &Path) -> RepositoryAgentTraceDb {
    RepositoryAgentTraceDb::new_at(path).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn write_contention_retries_whole_transaction_once_and_persists_one_pair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent-trace.db");
    let mut writer = open(&path).await;
    let holder = open(&path).await;

    holder.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1300)).await;
        holder.execute("COMMIT", ()).await.unwrap();
    });

    let inserted = writer
        .insert_conversation_text_event(message("contended"), part("contended"))
        .await
        .unwrap();
    release.await.unwrap();

    assert!(inserted);
    assert_eq!(pair_counts(&writer, "contended").await, (1, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn write_contention_exhausts_bounded_attempts_without_persisting_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent-trace.db");
    let mut writer = open(&path).await;
    let holder = open(&path).await;

    holder.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    let error = writer
        .insert_conversation_text_event(message("exhausted"), part("exhausted"))
        .await
        .unwrap_err()
        .to_string();
    holder.execute("COMMIT", ()).await.unwrap();

    assert!(error.contains("attempts=2"), "{error}");
    assert!(error.contains("under write contention"), "{error}");
    assert_eq!(pair_counts(&writer, "exhausted").await, (0, 0));
    assert!(writer
        .insert_conversation_text_event(message("after"), part("after"))
        .await
        .unwrap());
}
