use std::time::Duration;

use super::{contention_exhausted_error, WriteContentionPolicy};
use crate::services::observability::tracing_boundary::test_capture::CapturingSubscriber;

const SECRET: &str = "sk-live-SECRET-TOKEN-9f8e7d";
const SENSITIVE_PATH: &str = "/home/victim/.local/state/sce/agent-trace.db";

fn policy(db_name: &'static str) -> WriteContentionPolicy {
    WriteContentionPolicy {
        db_name,
        max_attempts: 3,
        backoff_cap: Duration::from_millis(1),
        busy_timeout: Duration::from_millis(5),
        contention_deadline: Duration::from_millis(50),
    }
}

#[test]
fn tracing_boundary_a_contention_exhausted_event_omits_error_chain_and_classifies_strings() {
    let last_error =
        anyhow::anyhow!("open {SENSITIVE_PATH}: token {SECRET}").context(format!("outer {SECRET}"));

    let events = CapturingSubscriber::capture(|| {
        let _ = contention_exhausted_error(
            policy("repository Agent Trace DB"),
            "execute repository Agent Trace DB database query",
            3,
            Duration::from_millis(40),
            &last_error,
            "retry later",
        );
        let _ = contention_exhausted_error(
            policy(SECRET),
            SENSITIVE_PATH,
            3,
            Duration::from_millis(40),
            &last_error,
            "retry later",
        );
    });

    assert_eq!(events.len(), 2);
    for event in &events {
        let rendered = event.rendered();
        for needle in [SECRET, SENSITIVE_PATH, "outer"] {
            assert!(!rendered.contains(needle), "leaked {needle}: {rendered}");
        }
        assert!(event.field("last_error").is_none());
        assert_eq!(
            event.field("event_id"),
            Some("sce.agent_trace_db.contention_exhausted")
        );
    }
    assert_eq!(
        events[0].field("db_name"),
        Some("repository_agent_trace_db")
    );
    assert_eq!(events[0].field("operation"), Some("db.execute_query"));
    assert_eq!(events[1].field("db_name"), Some("unclassified"));
    assert_eq!(events[1].field("operation"), Some("unclassified"));
}
