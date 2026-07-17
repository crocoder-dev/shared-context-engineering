//! Deterministic discovery of Agent Trace databases.
//!
//! Discovery scans repository-scoped databases at
//! `<state_root>/sce/repos/<repository_id>/agent-trace.db`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
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
];

/// Schema-readiness verdict for a discovered Agent Trace DB.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Readiness {
    Ready,
    Skipped { missing_table: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveredAgentTraceDbKind {
    Repository { repository_id: String },
}

impl DiscoveredAgentTraceDbKind {
    pub fn identifier(&self) -> &str {
        match self {
            Self::Repository { repository_id } => repository_id,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Repository { .. } => "repository",
        }
    }
}

/// A discovered Agent Trace database with its readiness verdict.
#[derive(Clone, Debug)]
pub struct DiscoveredAgentTraceDb {
    pub alias: String,
    pub kind: DiscoveredAgentTraceDbKind,
    pub path: PathBuf,
    pub mtime: SystemTime,
    pub readiness: Readiness,
}

/// User-actionable failures while resolving an Agent Trace DB identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
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
        scope: String,
        database_id: String,
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
                scope,
                database_id,
                missing_table,
            } => format!(
                "sce trace db shell: database '{identifier}' ({alias}, {scope} {database_id}) is not schema-ready: missing table '{missing_table}'. Run `sce setup` or inspect `sce trace db list` before opening a shell."
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
pub fn resolve_agent_trace_db_identifier(
    databases: &[DiscoveredAgentTraceDb],
    identifier: &str,
) -> Result<DiscoveredAgentTraceDb, ResolveAgentTraceDbError> {
    let matches: Vec<&DiscoveredAgentTraceDb> = databases
        .iter()
        .filter(|db| db.alias == identifier || db.kind.identifier() == identifier)
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
            scope: db.kind.label().to_string(),
            database_id: db.kind.identifier().to_string(),
            missing_table: missing_table.clone(),
        }),
    }
}

/// Discover repository-scoped Agent Trace DBs under the resolved state-data root.
pub fn discover_agent_trace_dbs() -> Result<Vec<DiscoveredAgentTraceDb>> {
    let state_root = resolve_state_data_root().context("failed to resolve state data root")?;
    let sce_dir = state_root.join("sce");
    discover_repository_agent_trace_dbs_in(&sce_dir)
}

/// Discover repository-scoped Agent Trace DBs in an explicit `sce` directory.
pub fn discover_repository_agent_trace_dbs_in(
    sce_dir: &Path,
) -> Result<Vec<DiscoveredAgentTraceDb>> {
    let repos_dir = sce_dir.join("repos");
    if !repos_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<(String, PathBuf, SystemTime)> = Vec::new();
    for entry in fs::read_dir(&repos_dir)
        .with_context(|| format!("failed to read repos directory '{}'", repos_dir.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "failed to read directory entry in '{}'",
                repos_dir.display()
            )
        })?;
        let repository_id = entry.file_name().to_string_lossy().into_owned();
        if repository_id.is_empty() || !entry.metadata()?.is_dir() {
            continue;
        }
        let path = entry.path().join("agent-trace.db");
        if !path.is_file() {
            continue;
        }
        let mtime = path
            .metadata()
            .with_context(|| format!("failed to read metadata for '{}'", path.display()))?
            .modified()
            .with_context(|| format!("failed to read mtime for '{}'", path.display()))?;
        entries.push((repository_id, path, mtime));
    }

    entries.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));
    discovered_from_entries(entries, |repository_id| {
        DiscoveredAgentTraceDbKind::Repository { repository_id }
    })
}

fn discovered_from_entries(
    entries: Vec<(String, PathBuf, SystemTime)>,
    kind_for_id: impl Fn(String) -> DiscoveredAgentTraceDbKind,
) -> Result<Vec<DiscoveredAgentTraceDb>> {
    let mut discovered = Vec::with_capacity(entries.len());
    for (index, (id, path, mtime)) in entries.into_iter().enumerate() {
        let readiness = probe_readiness(&path)?;
        discovered.push(DiscoveredAgentTraceDb {
            alias: format!("agent_trace_{index}"),
            kind: kind_for_id(id),
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
    let db = RepositoryAgentTraceDb::open_for_hooks_without_migrations_at(path)
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

    use std::time::UNIX_EPOCH;

    use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;

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

    fn create_repository_schema_db(path: &Path, repository_id: &str) {
        let db = RepositoryAgentTraceDb::new_at(path).expect("repository DB should open");
        db.verify_or_initialize_repository_metadata(repository_id)
            .expect("repository metadata");
        drop(db);
    }

    #[test]
    fn repository_schema_db_reports_ready_by_default() {
        let dir = unique_temp_dir("repo-ready");
        let repository_id = "repo123";
        let db_path = dir.join("repos").join(repository_id).join("agent-trace.db");
        create_repository_schema_db(&db_path, repository_id);

        let discovered = discover_repository_agent_trace_dbs_in(&dir)
            .expect("repository discovery should succeed");

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].kind.identifier(), repository_id);
        assert_eq!(discovered[0].alias, "agent_trace_0");
        assert_eq!(discovered[0].readiness, Readiness::Ready);
        assert_eq!(discovered[0].path, db_path);
    }
}
