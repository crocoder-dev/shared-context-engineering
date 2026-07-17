//! Aggregation for `sce trace status --all` across every discovered DB.
//!
//! Walks the `services::trace::discovery` output, runs
//! `collect_agent_trace_db_stats` over each `Ready` DB, and aggregates totals
//! plus a per-database breakdown for downstream renderers. `Skipped` DBs are
//! counted in the discovery summary but excluded from totals.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::services::default_paths::resolve_state_data_root;
use crate::services::trace::discovery::{
    discover_repository_agent_trace_dbs_in, DiscoveredAgentTraceDbKind, Readiness,
};
use crate::services::trace::stats::{collect_agent_trace_db_stats, AgentTraceDbStats};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoverySummary {
    pub discovered: usize,
    pub ready: usize,
    pub skipped: usize,
}

/// Aggregated totals across all discovered Agent Trace DBs.
pub type Totals = AgentTraceDbStats;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DatabaseRowStatus {
    Ready { stats: AgentTraceDbStats },
    Skipped { missing_table: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseRow {
    pub alias: String,
    pub kind: DiscoveredAgentTraceDbKind,
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
    let discovered = discover_repository_agent_trace_dbs_in(sce_dir)?;

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
                totals.agent_traces += stats.agent_traces;
                totals.post_commit_patch_intersections += stats.post_commit_patch_intersections;
                if let Some(dt) = stats.last_activity {
                    totals.last_activity =
                        Some(totals.last_activity.map_or(dt, |prev| prev.max(dt)));
                }
                databases.push(DatabaseRow {
                    alias: db.alias,
                    kind: db.kind,
                    path: db.path,
                    status: DatabaseRowStatus::Ready { stats },
                });
            }
            Readiness::Skipped { missing_table } => {
                discovery.skipped += 1;
                databases.push(DatabaseRow {
                    alias: db.alias,
                    kind: db.kind,
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

    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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
}
