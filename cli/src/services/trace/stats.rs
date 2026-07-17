//! Agent Trace DB row-count and last-activity stats.
//!
//! Issues read-only `COUNT(*)` and `MAX(...)` queries against a single
//! Agent Trace DB and returns the aggregated counts plus the most recent
//! activity timestamp derived from `diff_traces.time_ms`,
//! `messages.updated_at`, and `agent_traces.created_at`.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

/// Aggregated Agent Trace DB row counts and last activity.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(dead_code)]
pub struct AgentTraceDbStats {
    pub diff_traces: u64,
    pub messages: u64,
    pub parts: u64,
    pub agent_traces: u64,
    pub post_commit_patch_intersections: u64,
    pub last_activity: Option<DateTime<Utc>>,
}

/// Collect row counts and last activity for a single Agent Trace DB.
///
/// Opens the DB read-only (without running migrations) and issues one
/// `SELECT COUNT(*)` per required table plus three `MAX(...)` queries to
/// compute the last activity timestamp. The caller is expected to have
/// already verified schema readiness via `discover_agent_trace_dbs`.
#[allow(dead_code)]
pub fn collect_agent_trace_db_stats(path: &Path) -> Result<AgentTraceDbStats> {
    let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(path)
        .with_context(|| format!("failed to open agent trace DB '{}'", path.display()))?;

    let diff_traces = count_rows(&db, "diff_traces", path)?;
    let messages = count_rows(&db, "messages", path)?;
    let parts = count_rows(&db, "parts", path)?;
    let agent_traces = count_rows(&db, "agent_traces", path)?;
    let post_commit_patch_intersections = count_rows(&db, "post_commit_patch_intersections", path)?;

    let diff_max_ms = query_optional_i64(&db, "SELECT MAX(time_ms) FROM diff_traces", path)
        .context("failed to query MAX(diff_traces.time_ms)")?;
    let messages_max_iso = query_optional_string(&db, "SELECT MAX(updated_at) FROM messages", path)
        .context("failed to query MAX(messages.updated_at)")?;
    let agent_traces_max_iso =
        query_optional_string(&db, "SELECT MAX(created_at) FROM agent_traces", path)
            .context("failed to query MAX(agent_traces.created_at)")?;

    let mut last_activity: Option<DateTime<Utc>> = None;
    if let Some(ms) = diff_max_ms {
        if let Some(dt) = DateTime::<Utc>::from_timestamp_millis(ms) {
            last_activity = Some(last_activity.map_or(dt, |prev| prev.max(dt)));
        }
    }
    for iso in [messages_max_iso, agent_traces_max_iso]
        .into_iter()
        .flatten()
    {
        if let Some(dt) = parse_iso_millis(&iso) {
            last_activity = Some(last_activity.map_or(dt, |prev| prev.max(dt)));
        }
    }

    Ok(AgentTraceDbStats {
        diff_traces,
        messages,
        parts,
        agent_traces,
        post_commit_patch_intersections,
        last_activity,
    })
}

fn count_rows(db: &RepositoryAgentTraceDb, table: &str, path: &Path) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let rows = db
        .query_map(sql.as_str(), (), |row| {
            row.get::<i64>(0).map_err(Into::into)
        })
        .with_context(|| {
            format!(
                "failed to count rows in '{table}' for agent trace DB '{}'",
                path.display()
            )
        })?;
    let count = rows.into_iter().next().unwrap_or(0);
    Ok(u64::try_from(count).unwrap_or(0))
}

fn query_optional_i64(db: &RepositoryAgentTraceDb, sql: &str, path: &Path) -> Result<Option<i64>> {
    let rows = db
        .query_map(sql, (), |row| row.get::<Option<i64>>(0).map_err(Into::into))
        .with_context(|| {
            format!(
                "failed to query '{sql}' on agent trace DB '{}'",
                path.display()
            )
        })?;
    Ok(rows.into_iter().next().flatten())
}

fn query_optional_string(
    db: &RepositoryAgentTraceDb,
    sql: &str,
    path: &Path,
) -> Result<Option<String>> {
    let rows = db
        .query_map(sql, (), |row| {
            row.get::<Option<String>>(0).map_err(Into::into)
        })
        .with_context(|| {
            format!(
                "failed to query '{sql}' on agent trace DB '{}'",
                path.display()
            )
        })?;
    Ok(rows.into_iter().next().flatten())
}

/// Parse the `SQLite` `strftime('%Y-%m-%dT%H:%M:%fZ', ...)` format into UTC.
fn parse_iso_millis(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.with_timezone(&Utc));
    }
    // SQLite emits `YYYY-MM-DDTHH:MM:SS.sssZ`; fall back to a naive parse if
    // the upstream format ever drops the timezone designator.
    let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.fZ").ok()?;
    Some(Utc.from_utc_datetime(&naive))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
    use crate::services::agent_trace_db::{
        AgentTraceInsert, DiffTraceInsert, InsertMessageInsert, InsertPartInsert, MessageRole,
        PartType, PostCommitPatchIntersectionInsert,
    };

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-trace-stats-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn seed_db(path: &Path) -> i64 {
        let db = RepositoryAgentTraceDb::new_at(path).expect("repository DB should open");

        // 2 diff traces
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: 1_000,
            session_id: "s1",
            patch: "diff1",
            model_id: Some("m1"),
            tool_name: "claude",
            tool_version: Some("1"),
            payload_type: "patch",
        })
        .expect("diff trace 1");
        let latest_diff_ms = 2_500;
        db.insert_diff_trace(DiffTraceInsert {
            time_ms: latest_diff_ms,
            session_id: "s1",
            patch: "diff2",
            model_id: Some("m1"),
            tool_name: "claude",
            tool_version: Some("1"),
            payload_type: "patch",
        })
        .expect("diff trace 2");

        // 1 post_commit_patch_intersection
        db.insert_post_commit_patch_intersection(PostCommitPatchIntersectionInsert {
            commit_id: "c1",
            post_commit_time_ms: 3_000,
            recent_window_cutoff_ms: 0,
            recent_window_end_ms: 3_000,
            loaded_diff_trace_count: 2,
            skipped_diff_trace_count: 0,
            intersection_patch: "patch",
        })
        .expect("intersection");

        // 1 agent trace
        db.insert_agent_trace(AgentTraceInsert {
            commit_id: "c1",
            commit_time_ms: 3_000,
            trace_json: "{}",
            agent_trace_id: "at1",
            url: "https://example.test/at1",
            remote_url: "https://example.test/repo",
        })
        .expect("agent trace");

        // 2 messages, 3 parts
        db.insert_message(InsertMessageInsert {
            session_id: "s1".into(),
            message_id: "m1".into(),
            role: MessageRole::User,
            generated_at_unix_ms: 1_000,
        })
        .expect("message 1");
        db.insert_message(InsertMessageInsert {
            session_id: "s1".into(),
            message_id: "m2".into(),
            role: MessageRole::Assistant,
            generated_at_unix_ms: 1_100,
        })
        .expect("message 2");
        let parts = ["p1", "p2", "p3"]
            .iter()
            .enumerate()
            .map(|(i, part_id)| InsertPartInsert {
                part_type: PartType::Text,
                text: format!("part {part_id}"),
                session_id: "s1".into(),
                message_id: if i < 2 { "m1".into() } else { "m2".into() },
                generated_at_unix_ms: 1_000 + i64::try_from(i).expect("part index fits in i64"),
            })
            .collect();
        db.insert_parts(parts).expect("parts");

        latest_diff_ms
    }

    #[test]
    fn collect_stats_returns_counts_and_last_activity() {
        let dir = unique_temp_dir("counts");
        let db_path = dir.join("agent-trace-aaaa.db");
        let latest_diff_ms = seed_db(&db_path);

        let stats =
            collect_agent_trace_db_stats(&db_path).expect("stats collection should succeed");

        assert_eq!(stats.diff_traces, 2);
        assert_eq!(stats.messages, 2);
        assert_eq!(stats.parts, 3);
        assert_eq!(stats.agent_traces, 1);
        assert_eq!(stats.post_commit_patch_intersections, 1);

        let last = stats.last_activity.expect("last activity should be set");
        let diff_dt = DateTime::<Utc>::from_timestamp_millis(latest_diff_ms)
            .expect("latest diff time should convert");
        assert!(
            last >= diff_dt,
            "last_activity {last} should be >= latest diff trace {diff_dt}"
        );
    }

    #[test]
    fn collect_stats_on_empty_db_returns_zero_counts_and_no_activity() {
        let dir = unique_temp_dir("empty");
        let db_path = dir.join("agent-trace-bbbb.db");
        drop(RepositoryAgentTraceDb::new_at(&db_path).expect("repository DB should open"));

        let stats =
            collect_agent_trace_db_stats(&db_path).expect("stats collection should succeed");

        assert_eq!(stats.diff_traces, 0);
        assert_eq!(stats.messages, 0);
        assert_eq!(stats.parts, 0);
        assert_eq!(stats.agent_traces, 0);
        assert_eq!(stats.post_commit_patch_intersections, 0);
        assert!(stats.last_activity.is_none());
    }

    #[test]
    fn parse_iso_millis_handles_sqlite_strftime_output() {
        let parsed = parse_iso_millis("2026-06-28T12:34:56.789Z").expect("parse strftime output");
        assert_eq!(parsed.timestamp_millis(), 1_782_650_096_789);
    }
}
