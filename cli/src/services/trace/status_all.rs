//! Aggregation for `sce trace status --all` across every discovered DB.
//!
//! Walks the `services::trace::discovery` output, runs
//! `collect_agent_trace_db_stats` over each `Ready` DB, and aggregates totals
//! plus a per-database breakdown for downstream renderers. `Skipped` DBs are
//! counted in the discovery summary but excluded from totals.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::services::default_paths::resolve_state_data_root;
use crate::services::trace::discovery::{discover_agent_trace_dbs_in, Readiness};
use crate::services::trace::stats::{collect_agent_trace_db_stats, AgentTraceDbStats};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoverySummary {
    pub discovered: usize,
    pub ready: usize,
    pub skipped: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Totals {
    pub diff_traces: u64,
    pub messages: u64,
    pub parts: u64,
    pub session_models: u64,
    pub agent_traces: u64,
    pub post_commit_patch_intersections: u64,
    pub last_activity: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DatabaseRowStatus {
    Ready { stats: AgentTraceDbStats },
    Skipped { missing_table: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseRow {
    pub alias: String,
    pub checkout_id: String,
    pub path: PathBuf,
    pub status: DatabaseRowStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusAllReport {
    pub discovery: DiscoverySummary,
    pub totals: Totals,
    pub databases: Vec<DatabaseRow>,
}

/// Aggregate `sce trace status --all` using the default state-data root.
pub fn aggregate_current_status_all() -> Result<StatusAllReport> {
    let state_root = resolve_state_data_root().context("failed to resolve state data root")?;
    let sce_dir = state_root.join("sce");
    aggregate_status_all_in(&sce_dir)
}

/// Aggregate `sce trace status --all` against an explicit `sce` directory.
pub fn aggregate_status_all_in(sce_dir: &Path) -> Result<StatusAllReport> {
    let discovered = discover_agent_trace_dbs_in(sce_dir)?;

    let mut discovery = DiscoverySummary {
        discovered: discovered.len(),
        ready: 0,
        skipped: 0,
    };
    let mut totals = Totals::default();
    let mut databases: Vec<DatabaseRow> = Vec::with_capacity(discovered.len());

    for db in discovered {
        match db.readiness {
            Readiness::Ready => {
                discovery.ready += 1;
                let stats = collect_agent_trace_db_stats(&db.path)?;
                totals.diff_traces += stats.diff_traces;
                totals.messages += stats.messages;
                totals.parts += stats.parts;
                totals.session_models += stats.session_models;
                totals.agent_traces += stats.agent_traces;
                totals.post_commit_patch_intersections += stats.post_commit_patch_intersections;
                if let Some(dt) = stats.last_activity {
                    totals.last_activity =
                        Some(totals.last_activity.map_or(dt, |prev| prev.max(dt)));
                }
                databases.push(DatabaseRow {
                    alias: db.alias,
                    checkout_id: db.checkout_id,
                    path: db.path,
                    status: DatabaseRowStatus::Ready { stats },
                });
            }
            Readiness::Skipped { missing_table } => {
                discovery.skipped += 1;
                databases.push(DatabaseRow {
                    alias: db.alias,
                    checkout_id: db.checkout_id,
                    path: db.path,
                    status: DatabaseRowStatus::Skipped { missing_table },
                });
            }
        }
    }

    Ok(StatusAllReport {
        discovery,
        totals,
        databases,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::OpenOptions;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::services::agent_trace_db::{
        AgentTraceDb, DiffTraceInsert, InsertMessageInsert, InsertPartInsert, MessageRole, PartType,
    };

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-trace-status-all-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn touch_mtime(path: &Path, mtime: SystemTime) {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open db file for mtime update");
        file.set_modified(mtime).expect("set mtime");
    }

    fn seed_ready_db(path: &Path, diff_count: u64, message_count: u64, parts_per_message: u64) {
        let db = AgentTraceDb::open_at(path).expect("migrated DB should open");
        for i in 0..diff_count {
            db.insert_diff_trace(DiffTraceInsert {
                time_ms: 1_000 + i64::try_from(i).expect("diff index fits in i64"),
                session_id: "s1",
                patch: "diff",
                model_id: Some("m1"),
                tool_name: "claude",
                tool_version: Some("1"),
                payload_type: "patch",
            })
            .expect("diff trace");
        }
        for i in 0..message_count {
            let mid = format!("m{i}");
            db.insert_message(InsertMessageInsert {
                session_id: "s1".into(),
                message_id: mid.clone(),
                role: MessageRole::User,
                generated_at_unix_ms: 1_000 + i64::try_from(i).expect("msg index fits in i64"),
            })
            .expect("message");
            for j in 0..parts_per_message {
                db.insert_part(InsertPartInsert {
                    part_type: PartType::Text,
                    text: format!("p{i}{j}"),
                    session_id: "s1".into(),
                    message_id: mid.clone(),
                    generated_at_unix_ms: 1_000
                        + i64::try_from(i * parts_per_message + j).expect("part index fits in i64"),
                })
                .expect("part");
            }
        }
    }

    fn seed_partial_db(path: &Path) {
        let db = AgentTraceDb::open_for_hooks_without_migrations_at(path)
            .expect("open without migrations");
        db.execute("CREATE TABLE diff_traces (id INTEGER PRIMARY KEY)", ())
            .expect("create diff_traces");
        // Intentionally missing post_commit_patch_intersections so readiness
        // probe reports skipped with the first missing table.
        drop(db);
    }

    #[test]
    fn empty_sce_dir_reports_zero_discovery_and_totals() {
        let dir = unique_temp_dir("empty");
        let report = aggregate_status_all_in(&dir).expect("empty aggregation should succeed");
        assert_eq!(report.discovery.discovered, 0);
        assert_eq!(report.discovery.ready, 0);
        assert_eq!(report.discovery.skipped, 0);
        assert_eq!(report.totals, Totals::default());
        assert!(report.databases.is_empty());
    }

    #[test]
    fn mixed_fixture_aggregates_ready_and_lists_skipped() {
        let dir = unique_temp_dir("mixed");

        let ready_newest = dir.join("agent-trace-aaaa.db");
        let ready_older = dir.join("agent-trace-bbbb.db");
        let skipped = dir.join("agent-trace-cccc.db");

        seed_ready_db(&ready_newest, 3, 2, 2); // diffs=3, msgs=2, parts=4
        seed_ready_db(&ready_older, 1, 1, 1); // diffs=1, msgs=1, parts=1
        seed_partial_db(&skipped);

        let base = SystemTime::now();
        touch_mtime(&ready_newest, base);
        touch_mtime(&ready_older, base - Duration::from_secs(10));
        touch_mtime(&skipped, base - Duration::from_secs(20));

        let report = aggregate_status_all_in(&dir).expect("aggregation should succeed");

        assert_eq!(report.discovery.discovered, 3);
        assert_eq!(report.discovery.ready, 2);
        assert_eq!(report.discovery.skipped, 1);

        assert_eq!(report.totals.diff_traces, 4);
        assert_eq!(report.totals.messages, 3);
        assert_eq!(report.totals.parts, 5);
        assert_eq!(report.totals.session_models, 0);
        assert_eq!(report.totals.agent_traces, 0);
        assert_eq!(report.totals.post_commit_patch_intersections, 0);

        assert_eq!(report.databases.len(), 3);
        // Discovery is mtime-desc, so newest ready first, then older ready, then skipped.
        assert_eq!(report.databases[0].alias, "agent_trace_0");
        assert_eq!(report.databases[0].checkout_id, "aaaa");
        match &report.databases[0].status {
            DatabaseRowStatus::Ready { stats } => assert_eq!(stats.diff_traces, 3),
            DatabaseRowStatus::Skipped { .. } => panic!("expected ready row"),
        }
        assert_eq!(report.databases[1].alias, "agent_trace_1");
        assert_eq!(report.databases[1].checkout_id, "bbbb");
        match &report.databases[1].status {
            DatabaseRowStatus::Ready { stats } => assert_eq!(stats.diff_traces, 1),
            DatabaseRowStatus::Skipped { .. } => panic!("expected ready row"),
        }
        assert_eq!(report.databases[2].alias, "agent_trace_2");
        assert_eq!(report.databases[2].checkout_id, "cccc");
        match &report.databases[2].status {
            DatabaseRowStatus::Skipped { missing_table } => {
                assert_eq!(missing_table, "post_commit_patch_intersections");
            }
            DatabaseRowStatus::Ready { .. } => panic!("expected skipped row"),
        }
    }
}
