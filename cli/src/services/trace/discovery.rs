//! Deterministic discovery of per-checkout Agent Trace databases.
//!
//! Scans `<state_root>/sce/agent-trace-{checkout_id}.db`, sorts by file mtime
//! descending with ties broken by `checkout_id` ascending, assigns positional
//! aliases `agent_trace_{i}`, and probes each file for the required schema.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::services::agent_trace_db::AgentTraceDb;
use crate::services::default_paths::resolve_state_data_root;

const LIST_GUIDANCE: &str = "Run `sce trace db list` to see available Agent Trace databases.";

/// Tables that must exist for an Agent Trace DB to be considered `ready`.
///
/// Order is significant: the first missing table is reported as the skip
/// reason.
const REQUIRED_TABLES: &[&str] = &[
    "diff_traces",
    "post_commit_patch_intersections",
    "agent_traces",
    "messages",
    "parts",
    "session_models",
];

/// Schema-readiness verdict for a discovered Agent Trace DB.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Readiness {
    Ready,
    Skipped { missing_table: String },
}

/// A discovered per-checkout Agent Trace database with its readiness verdict.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct DiscoveredAgentTraceDb {
    pub alias: String,
    pub checkout_id: String,
    pub path: PathBuf,
    pub mtime: SystemTime,
    pub readiness: Readiness,
}

/// User-actionable failures while resolving an Agent Trace DB identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum ResolveAgentTraceDbError {
    UnknownIdentifier {
        identifier: String,
    },
    AmbiguousIdentifier {
        identifier: String,
    },
    SkippedDatabase {
        identifier: String,
        alias: String,
        checkout_id: String,
        missing_table: String,
    },
}

impl ResolveAgentTraceDbError {
    pub fn user_message(&self) -> String {
        match self {
            Self::UnknownIdentifier { identifier } => format!(
                "sce trace db shell: no agent-trace database matches '{identifier}'. {LIST_GUIDANCE}"
            ),
            Self::AmbiguousIdentifier { identifier } => format!(
                "sce trace db shell: identifier '{identifier}' matches more than one agent-trace database. {LIST_GUIDANCE}"
            ),
            Self::SkippedDatabase {
                identifier,
                alias,
                checkout_id,
                missing_table,
            } => format!(
                "sce trace db shell: database '{identifier}' ({alias}, checkout {checkout_id}) is not schema-ready: missing table '{missing_table}'. Run `sce setup` or inspect `sce trace db list` before opening a shell."
            ),
        }
    }
}

impl std::fmt::Display for ResolveAgentTraceDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for ResolveAgentTraceDbError {}

/// Resolve an alias or checkout ID to one ready discovered Agent Trace DB.
#[allow(dead_code)]
pub fn resolve_agent_trace_db_identifier(
    databases: &[DiscoveredAgentTraceDb],
    identifier: &str,
) -> Result<DiscoveredAgentTraceDb, ResolveAgentTraceDbError> {
    let matches: Vec<&DiscoveredAgentTraceDb> = databases
        .iter()
        .filter(|db| db.alias == identifier || db.checkout_id == identifier)
        .collect();

    let db = match matches.as_slice() {
        [] => {
            return Err(ResolveAgentTraceDbError::UnknownIdentifier {
                identifier: identifier.to_string(),
            });
        }
        [db] => *db,
        _ => {
            return Err(ResolveAgentTraceDbError::AmbiguousIdentifier {
                identifier: identifier.to_string(),
            });
        }
    };

    match &db.readiness {
        Readiness::Ready => Ok(db.clone()),
        Readiness::Skipped { missing_table } => Err(ResolveAgentTraceDbError::SkippedDatabase {
            identifier: identifier.to_string(),
            alias: db.alias.clone(),
            checkout_id: db.checkout_id.clone(),
            missing_table: missing_table.clone(),
        }),
    }
}

/// Discover Agent Trace DBs under the resolved state-data root.
#[allow(dead_code)]
pub fn discover_agent_trace_dbs() -> Result<Vec<DiscoveredAgentTraceDb>> {
    let state_root = resolve_state_data_root().context("failed to resolve state data root")?;
    let sce_dir = state_root.join("sce");
    discover_agent_trace_dbs_in(&sce_dir)
}

/// Discover Agent Trace DBs in an explicit `sce` directory.
///
/// Returns an empty Vec when the directory does not exist. Otherwise scans for
/// `agent-trace-{checkout_id}.db` files, sorts by mtime descending (ties broken
/// by `checkout_id` ascending), assigns positional aliases, and probes each
/// file for the required schema.
pub fn discover_agent_trace_dbs_in(sce_dir: &Path) -> Result<Vec<DiscoveredAgentTraceDb>> {
    if !sce_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<(String, PathBuf, SystemTime)> = Vec::new();

    for entry in fs::read_dir(sce_dir)
        .with_context(|| format!("failed to read sce directory '{}'", sce_dir.display()))?
    {
        let entry = entry.with_context(|| {
            format!("failed to read directory entry in '{}'", sce_dir.display())
        })?;

        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        let Some(stripped) = file_name_str.strip_prefix("agent-trace-") else {
            continue;
        };
        let Some(checkout_id) = stripped.strip_suffix(".db") else {
            continue;
        };
        if checkout_id.is_empty() {
            continue;
        }

        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata for '{}'", entry.path().display()))?;
        if !metadata.is_file() {
            continue;
        }
        let mtime = metadata
            .modified()
            .with_context(|| format!("failed to read mtime for '{}'", entry.path().display()))?;

        entries.push((checkout_id.to_string(), entry.path(), mtime));
    }

    entries.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));

    let mut discovered = Vec::with_capacity(entries.len());
    for (index, (checkout_id, path, mtime)) in entries.into_iter().enumerate() {
        let readiness = probe_readiness(&path)?;
        discovered.push(DiscoveredAgentTraceDb {
            alias: format!("agent_trace_{index}"),
            checkout_id,
            path,
            mtime,
            readiness,
        });
    }

    Ok(discovered)
}

/// Probe an Agent Trace DB file for required schema readiness.
///
/// Opens the database without running migrations and queries `sqlite_master`
/// for each required table in declared order. Returns `Skipped` with the first
/// missing table reported; otherwise `Ready`.
pub(super) fn probe_readiness(path: &Path) -> Result<Readiness> {
    let db = AgentTraceDb::open_for_hooks_without_migrations_at(path)
        .with_context(|| format!("failed to open agent trace DB '{}'", path.display()))?;

    for table in REQUIRED_TABLES {
        let rows = db
            .query_map(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                (*table,),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .with_context(|| format!("failed to probe table '{table}' in '{}'", path.display()))?;

        if rows.is_empty() {
            return Ok(Readiness::Skipped {
                missing_table: (*table).to_string(),
            });
        }
    }

    Ok(Readiness::Ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::OpenOptions;
    use std::time::{Duration, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-trace-discovery-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn create_full_schema_db(path: &Path) {
        let db = AgentTraceDb::open_at(path).expect("agent trace DB should open with migrations");
        drop(db);
    }

    fn touch_mtime(path: &Path, mtime: SystemTime) {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open db file for mtime update");
        file.set_modified(mtime).expect("set mtime");
    }

    #[test]
    fn full_schema_db_reports_ready() {
        let dir = unique_temp_dir("ready");
        let db_path = dir.join("agent-trace-aaaa.db");
        create_full_schema_db(&db_path);

        let discovered = discover_agent_trace_dbs_in(&dir).expect("discovery should succeed");

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].checkout_id, "aaaa");
        assert_eq!(discovered[0].alias, "agent_trace_0");
        assert_eq!(discovered[0].readiness, Readiness::Ready);
        assert_eq!(discovered[0].path, db_path);
    }

    #[test]
    fn missing_required_table_reports_skipped_with_first_missing() {
        let dir = unique_temp_dir("skipped");
        let db_path = dir.join("agent-trace-bbbb.db");

        let db = AgentTraceDb::open_for_hooks_without_migrations_at(&db_path)
            .expect("agent trace DB should open without migrations");
        db.execute(
            "CREATE TABLE IF NOT EXISTS diff_traces (id INTEGER PRIMARY KEY)",
            (),
        )
        .expect("create diff_traces");
        db.execute(
            "CREATE TABLE post_commit_patch_intersections (id INTEGER PRIMARY KEY)",
            (),
        )
        .expect("create post_commit_patch_intersections");
        // Intentionally skip `agent_traces` to exercise the first-missing-table report.
        db.execute("CREATE TABLE messages (id INTEGER PRIMARY KEY)", ())
            .expect("create messages");
        db.execute("CREATE TABLE parts (id INTEGER PRIMARY KEY)", ())
            .expect("create parts");
        db.execute("CREATE TABLE session_models (id INTEGER PRIMARY KEY)", ())
            .expect("create session_models");
        drop(db);

        let discovered = discover_agent_trace_dbs_in(&dir).expect("discovery should succeed");

        assert_eq!(discovered.len(), 1);
        assert_eq!(
            discovered[0].readiness,
            Readiness::Skipped {
                missing_table: String::from("agent_traces"),
            }
        );
    }

    #[test]
    fn aliases_assigned_in_mtime_desc_order_with_checkout_id_tiebreak() {
        let dir = unique_temp_dir("ordering");

        let old_path = dir.join("agent-trace-old.db");
        let mid_path = dir.join("agent-trace-mid.db");
        let new_path = dir.join("agent-trace-new.db");
        let tie_a_path = dir.join("agent-trace-tie-a.db");
        let tie_b_path = dir.join("agent-trace-tie-b.db");

        create_full_schema_db(&old_path);
        create_full_schema_db(&mid_path);
        create_full_schema_db(&new_path);
        create_full_schema_db(&tie_a_path);
        create_full_schema_db(&tie_b_path);

        let base = SystemTime::now();
        touch_mtime(&old_path, base - Duration::from_secs(7));
        touch_mtime(&mid_path, base - Duration::from_secs(3));
        touch_mtime(&new_path, base);
        let tie_time = base - Duration::from_secs(5);
        touch_mtime(&tie_a_path, tie_time);
        touch_mtime(&tie_b_path, tie_time);

        let discovered = discover_agent_trace_dbs_in(&dir).expect("discovery should succeed");

        assert_eq!(discovered.len(), 5);
        assert_eq!(discovered[0].alias, "agent_trace_0");
        assert_eq!(discovered[0].checkout_id, "new");
        assert_eq!(discovered[1].alias, "agent_trace_1");
        assert_eq!(discovered[1].checkout_id, "mid");
        assert_eq!(discovered[2].alias, "agent_trace_2");
        assert_eq!(discovered[2].checkout_id, "tie-a");
        assert_eq!(discovered[3].alias, "agent_trace_3");
        assert_eq!(discovered[3].checkout_id, "tie-b");
        assert_eq!(discovered[4].alias, "agent_trace_4");
        assert_eq!(discovered[4].checkout_id, "old");
    }
}
