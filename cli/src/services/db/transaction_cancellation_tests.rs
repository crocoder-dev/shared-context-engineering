use crate::services::agent_trace_db::transaction_tests::{message, open, pair_counts, part};
use crate::services::agent_trace_db::INSERT_MESSAGE_SQL;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandoned_transaction_commits_nothing_and_rolls_back_on_next_use_of_its_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent-trace.db");
    let mut writer = open(&path).await;
    let mut observer = open(&path).await;

    {
        let tx = turso::transaction::Transaction::new(
            &mut writer.core.conn,
            turso::transaction::TransactionBehavior::Immediate,
        )
        .await
        .unwrap();
        tx.execute(INSERT_MESSAGE_SQL, ("session", "abandoned", "user", 1_i64))
            .await
            .unwrap();
    }

    assert_eq!(pair_counts(&observer, "abandoned").await, (0, 0));

    let blocked = observer
        .insert_conversation_text_event(message("blocked"), part("blocked"))
        .await
        .unwrap_err()
        .to_string();
    assert!(blocked.contains("under write contention"), "{blocked}");

    assert!(writer
        .insert_conversation_text_event(message("later"), part("later"))
        .await
        .unwrap());

    assert!(observer
        .insert_conversation_text_event(message("unblocked"), part("unblocked"))
        .await
        .unwrap());

    assert_eq!(pair_counts(&observer, "abandoned").await, (0, 0));
    assert_eq!(pair_counts(&observer, "later").await, (1, 1));
    assert_eq!(pair_counts(&writer, "unblocked").await, (1, 1));
}
