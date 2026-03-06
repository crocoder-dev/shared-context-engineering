use std::path::{Path, PathBuf};

use anyhow::{anyhow, ensure, Context, Result};
use turso::Builder;

use crate::services::resilience::{run_with_retry, RetryPolicy};

const CORE_SCHEMA_STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS repositories (\
        id INTEGER PRIMARY KEY,\
        vcs_provider TEXT NOT NULL DEFAULT 'git',\
        canonical_root TEXT NOT NULL UNIQUE,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
    )",
    "CREATE TABLE IF NOT EXISTS commits (\
        id INTEGER PRIMARY KEY,\
        repository_id INTEGER NOT NULL,\
        commit_sha TEXT NOT NULL,\
        parent_sha TEXT,\
        committed_at TEXT,\
        author_name TEXT,\
        author_email TEXT,\
        idempotency_key TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, commit_sha),\
        UNIQUE(repository_id, idempotency_key)\
    )",
    "CREATE TABLE IF NOT EXISTS trace_records (\
        id INTEGER PRIMARY KEY,\
        repository_id INTEGER NOT NULL,\
        commit_id INTEGER NOT NULL,\
        trace_id TEXT NOT NULL UNIQUE,\
        version TEXT NOT NULL,\
        content_type TEXT NOT NULL,\
        notes_ref TEXT NOT NULL,\
        payload_json TEXT NOT NULL,\
        quality_status TEXT NOT NULL,\
        idempotency_key TEXT NOT NULL,\
        recorded_at TEXT NOT NULL,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        FOREIGN KEY(commit_id) REFERENCES commits(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, idempotency_key),\
        UNIQUE(commit_id)\
    )",
    "CREATE TABLE IF NOT EXISTS trace_ranges (\
        id INTEGER PRIMARY KEY,\
        trace_record_id INTEGER NOT NULL,\
        file_path TEXT NOT NULL,\
        conversation_url TEXT NOT NULL,\
        start_line INTEGER NOT NULL,\
        end_line INTEGER NOT NULL,\
        contributor_type TEXT NOT NULL,\
        contributor_model_id TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(trace_record_id) REFERENCES trace_records(id) ON DELETE CASCADE\
    )",
    "CREATE TABLE IF NOT EXISTS reconciliation_runs (\
        id INTEGER PRIMARY KEY,\
        repository_id INTEGER NOT NULL,\
        provider TEXT NOT NULL,\
        idempotency_key TEXT NOT NULL,\
        status TEXT NOT NULL,\
        initiated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        completed_at TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, idempotency_key)\
    )",
    "CREATE TABLE IF NOT EXISTS rewrite_mappings (\
        id INTEGER PRIMARY KEY,\
        reconciliation_run_id INTEGER NOT NULL,\
        repository_id INTEGER NOT NULL,\
        old_commit_sha TEXT NOT NULL,\
        new_commit_sha TEXT,\
        mapping_status TEXT NOT NULL,\
        confidence REAL,\
        idempotency_key TEXT NOT NULL,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(reconciliation_run_id) REFERENCES reconciliation_runs(id) ON DELETE CASCADE,\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, idempotency_key)\
    )",
    "CREATE TABLE IF NOT EXISTS conversations (\
        id INTEGER PRIMARY KEY,\
        repository_id INTEGER NOT NULL,\
        url TEXT NOT NULL,\
        source TEXT NOT NULL,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE,\
        UNIQUE(repository_id, url)\
    )",
    "CREATE TABLE IF NOT EXISTS trace_retry_queue (\
        id INTEGER PRIMARY KEY,\
        trace_id TEXT NOT NULL UNIQUE,\
        commit_sha TEXT NOT NULL,\
        failed_targets TEXT NOT NULL,\
        content_type TEXT NOT NULL,\
        notes_ref TEXT NOT NULL,\
        payload_json TEXT NOT NULL,\
        attempts INTEGER NOT NULL DEFAULT 0,\
        last_error_class TEXT,\
        last_error_message TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
    )",
    "CREATE TABLE IF NOT EXISTS reconciliation_metrics (\
        id INTEGER PRIMARY KEY,\
        run_id INTEGER,\
        mapped_count INTEGER NOT NULL,\
        unmapped_count INTEGER NOT NULL,\
        histogram_high INTEGER NOT NULL,\
        histogram_medium INTEGER NOT NULL,\
        histogram_low INTEGER NOT NULL,\
        runtime_ms INTEGER NOT NULL,\
        error_class TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
    )",
    "CREATE INDEX IF NOT EXISTS idx_commits_repository_commit_sha ON commits(repository_id, commit_sha)",
    "CREATE INDEX IF NOT EXISTS idx_trace_records_repository_commit ON trace_records(repository_id, commit_id)",
    "CREATE INDEX IF NOT EXISTS idx_trace_ranges_record_file ON trace_ranges(trace_record_id, file_path)",
    "CREATE INDEX IF NOT EXISTS idx_reconciliation_runs_repository_status ON reconciliation_runs(repository_id, status)",
    "CREATE INDEX IF NOT EXISTS idx_rewrite_mappings_run_old_sha ON rewrite_mappings(reconciliation_run_id, old_commit_sha)",
    "CREATE INDEX IF NOT EXISTS idx_rewrite_mappings_repository_old_sha ON rewrite_mappings(repository_id, old_commit_sha)",
    "CREATE INDEX IF NOT EXISTS idx_conversations_repository_source ON conversations(repository_id, source)",
    "CREATE INDEX IF NOT EXISTS idx_trace_retry_queue_created_at ON trace_retry_queue(created_at)",
    "CREATE INDEX IF NOT EXISTS idx_reconciliation_metrics_created_at ON reconciliation_metrics(created_at)",
];

const CORE_SCHEMA_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 3,
    timeout_ms: 5_000,
    initial_backoff_ms: 150,
    max_backoff_ms: 600,
};

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum LocalDatabaseTarget<'a> {
    InMemory,
    Path(&'a Path),
}

#[derive(Clone, Copy, Debug)]
pub struct SmokeCheckOutcome {
    pub inserted_rows: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreSchemaMigrationOutcome {
    pub executed_statements: usize,
}

pub fn resolve_agent_trace_local_db_path() -> Result<PathBuf> {
    let state_root = resolve_state_data_root()?;
    Ok(state_root.join("sce").join("agent-trace").join("local.db"))
}

pub fn ensure_agent_trace_local_db_ready_blocking() -> Result<PathBuf> {
    let db_path = resolve_agent_trace_local_db_path()?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Agent Trace local DB directory '{}'.",
                parent.display()
            )
        })?;
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    runtime.block_on(run_with_retry(
        CORE_SCHEMA_RETRY_POLICY,
        "local_db.apply_core_schema_migrations",
        "retry the command; if it persists, verify state-directory permissions and available disk space.",
        |_| apply_core_schema_migrations(LocalDatabaseTarget::Path(&db_path)),
    ))?;
    Ok(db_path)
}

async fn connect_local(target: LocalDatabaseTarget<'_>) -> Result<turso::Connection> {
    let location = target_location(target)?;
    let db = Builder::new_local(location).build().await?;
    let conn = db.connect()?;
    conn.execute("PRAGMA foreign_keys = ON", ()).await?;
    Ok(conn)
}

fn target_location(target: LocalDatabaseTarget<'_>) -> Result<&str> {
    match target {
        LocalDatabaseTarget::InMemory => Ok(":memory:"),
        LocalDatabaseTarget::Path(path) => path
            .to_str()
            .ok_or_else(|| anyhow!("Local DB path must be valid UTF-8: {}", path.display())),
    }
}

pub(crate) fn resolve_state_data_root() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local_app_data));
        }
        if let Some(app_data) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(app_data));
        }

        return Ok(resolve_home_dir()?.join("AppData").join("Local"));
    }

    #[cfg(target_os = "macos")]
    {
        return Ok(resolve_home_dir()?
            .join("Library")
            .join("Application Support"));
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(xdg_state_home) = std::env::var_os("XDG_STATE_HOME") {
            return Ok(PathBuf::from(xdg_state_home));
        }
        Ok(resolve_home_dir()?.join(".local").join("state"))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Ok(resolve_home_dir()?.join(".local").join("state"))
    }
}

fn resolve_home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home));
    }

    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        return Ok(PathBuf::from(user_profile));
    }

    Err(anyhow!(
        "Unable to resolve home directory from HOME or USERPROFILE environment variables"
    ))
}

pub async fn apply_core_schema_migrations(
    target: LocalDatabaseTarget<'_>,
) -> Result<CoreSchemaMigrationOutcome> {
    let conn = connect_local(target).await?;
    for statement in CORE_SCHEMA_STATEMENTS {
        conn.execute(statement, ()).await?;
    }

    Ok(CoreSchemaMigrationOutcome {
        executed_statements: CORE_SCHEMA_STATEMENTS.len(),
    })
}

pub async fn run_smoke_check(target: LocalDatabaseTarget<'_>) -> Result<SmokeCheckOutcome> {
    let conn = connect_local(target).await?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS sce_smoke (id INTEGER PRIMARY KEY, label TEXT NOT NULL)",
        (),
    )
    .await?;

    let inserted_rows = conn
        .execute("INSERT INTO sce_smoke (label) VALUES (?1)", ["connected"])
        .await?;

    let mut rows = conn
        .query("SELECT label FROM sce_smoke ORDER BY id DESC LIMIT 1", ())
        .await?;

    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("Turso smoke query returned no rows"))?;
    let label = row.get_value(0)?;
    ensure!(
        label.as_text().is_some(),
        "Turso smoke query returned a non-text label"
    );

    Ok(SmokeCheckOutcome { inserted_rows })
}

#[cfg(test)]
mod tests {
    use crate::test_support::TestTempDir;
    use anyhow::Result;

    use super::{apply_core_schema_migrations, run_smoke_check, LocalDatabaseTarget};

    fn row_exists_query(kind: &str, name: &str) -> String {
        format!("SELECT 1 FROM sqlite_master WHERE type = '{kind}' AND name = '{name}' LIMIT 1")
    }

    async fn sqlite_object_exists(
        target: LocalDatabaseTarget<'_>,
        kind: &str,
        name: &str,
    ) -> Result<bool> {
        let conn = super::connect_local(target).await?;
        let mut rows = conn.query(&row_exists_query(kind, name), ()).await?;
        Ok(rows.next().await?.is_some())
    }

    async fn repository_count(target: LocalDatabaseTarget<'_>) -> Result<u64> {
        let conn = super::connect_local(target).await?;
        let mut rows = conn.query("SELECT COUNT(*) FROM repositories", ()).await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("repository count query returned no rows"))?;
        let count = row.get_value(0)?;
        let count = *count
            .as_integer()
            .ok_or_else(|| anyhow::anyhow!("repository count query returned non-integer"))?;
        Ok(count as u64)
    }

    async fn fetch_single_integer(target: LocalDatabaseTarget<'_>, query: &str) -> Result<i64> {
        let conn = super::connect_local(target).await?;
        let mut rows = conn.query(query, ()).await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("integer query returned no rows"))?;
        let value = row.get_value(0)?;
        let value = *value
            .as_integer()
            .ok_or_else(|| anyhow::anyhow!("integer query returned non-integer"))?;
        Ok(value)
    }

    #[test]
    fn in_memory_smoke_check_succeeds() -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let outcome = runtime.block_on(run_smoke_check(LocalDatabaseTarget::InMemory))?;
        assert_eq!(outcome.inserted_rows, 1);
        Ok(())
    }

    #[test]
    fn file_backed_smoke_check_succeeds() -> Result<()> {
        let temp = TestTempDir::new("sce-smoke-tests")?;
        let path = temp.path().join("local.db");
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let outcome = runtime.block_on(run_smoke_check(LocalDatabaseTarget::Path(&path)))?;
        assert_eq!(outcome.inserted_rows, 1);
        assert!(path.exists());
        Ok(())
    }

    #[test]
    fn core_schema_migrations_create_required_tables_and_indexes() -> Result<()> {
        let temp = TestTempDir::new("sce-core-schema-tests")?;
        let path = temp.path().join("core-schema.db");
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;

        let outcome = runtime.block_on(apply_core_schema_migrations(LocalDatabaseTarget::Path(
            &path,
        )))?;
        assert_eq!(
            outcome.executed_statements,
            super::CORE_SCHEMA_STATEMENTS.len(),
            "expected all core migration statements to execute"
        );

        for table in [
            "repositories",
            "commits",
            "trace_records",
            "trace_ranges",
            "reconciliation_runs",
            "rewrite_mappings",
            "conversations",
            "trace_retry_queue",
            "reconciliation_metrics",
        ] {
            assert!(runtime.block_on(sqlite_object_exists(
                LocalDatabaseTarget::Path(&path),
                "table",
                table,
            ))?);
        }

        for index in [
            "idx_commits_repository_commit_sha",
            "idx_trace_records_repository_commit",
            "idx_trace_ranges_record_file",
            "idx_reconciliation_runs_repository_status",
            "idx_rewrite_mappings_run_old_sha",
            "idx_rewrite_mappings_repository_old_sha",
            "idx_conversations_repository_source",
            "idx_trace_retry_queue_created_at",
            "idx_reconciliation_metrics_created_at",
        ] {
            assert!(runtime.block_on(sqlite_object_exists(
                LocalDatabaseTarget::Path(&path),
                "index",
                index,
            ))?);
        }

        Ok(())
    }

    #[test]
    fn core_schema_migrations_are_upgrade_safe_for_preexisting_state() -> Result<()> {
        let temp = TestTempDir::new("sce-core-schema-upgrade-tests")?;
        let path = temp.path().join("preexisting.db");
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;

        runtime.block_on(async {
            let conn = super::connect_local(LocalDatabaseTarget::Path(&path)).await?;
            conn.execute(
                "CREATE TABLE IF NOT EXISTS repositories (\
                    id INTEGER PRIMARY KEY,\
                    vcs_provider TEXT NOT NULL DEFAULT 'git',\
                    canonical_root TEXT NOT NULL UNIQUE,\
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
                )",
                (),
            )
            .await?;
            conn.execute(
                "INSERT INTO repositories (canonical_root) VALUES (?1)",
                ["/tmp/example-repo"],
            )
            .await?;
            Ok::<(), anyhow::Error>(())
        })?;

        runtime.block_on(apply_core_schema_migrations(LocalDatabaseTarget::Path(
            &path,
        )))?;
        runtime.block_on(apply_core_schema_migrations(LocalDatabaseTarget::Path(
            &path,
        )))?;

        let repository_rows =
            runtime.block_on(repository_count(LocalDatabaseTarget::Path(&path)))?;
        assert_eq!(
            repository_rows, 1,
            "preexisting repository rows should remain"
        );
        Ok(())
    }

    #[test]
    fn reconciliation_schema_supports_replay_safe_runs_and_mapping_queries() -> Result<()> {
        let temp = TestTempDir::new("sce-reconciliation-schema-tests")?;
        let path = temp.path().join("reconciliation.db");
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;

        runtime.block_on(apply_core_schema_migrations(LocalDatabaseTarget::Path(
            &path,
        )))?;

        runtime.block_on(async {
            let location = super::target_location(LocalDatabaseTarget::Path(&path))?;
            let db = turso::Builder::new_local(location).build().await?;
            let conn = db.connect()?;

            conn.execute(
                "INSERT INTO repositories (canonical_root) VALUES (?1)",
                ["/tmp/reconciliation-repo"],
            )
            .await?;

            conn.execute(
                "INSERT INTO reconciliation_runs (repository_id, provider, idempotency_key, status) \
                 VALUES (?1, ?2, ?3, ?4)",
                (1_i64, "github", "run:key:1", "completed"),
            )
            .await?;

            conn.execute(
                "INSERT INTO conversations (repository_id, url, source) VALUES (?1, ?2, ?3)",
                (1_i64, "https://example.dev/conversations/abc", "github"),
            )
            .await?;

            conn.execute(
                "INSERT INTO rewrite_mappings (\
                    reconciliation_run_id, repository_id, old_commit_sha, new_commit_sha,\
                    mapping_status, confidence, idempotency_key\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    1_i64,
                    1_i64,
                    "1111111111111111111111111111111111111111",
                    "2222222222222222222222222222222222222222",
                    "mapped",
                    0.98_f64,
                    "map:key:1",
                ),
            )
            .await?;

            let duplicate_run = conn
                .execute(
                    "INSERT INTO reconciliation_runs (repository_id, provider, idempotency_key, status) \
                     VALUES (?1, ?2, ?3, ?4)",
                    (1_i64, "github", "run:key:1", "completed"),
                )
                .await;
            assert!(duplicate_run.is_err(), "run idempotency key should be unique");

            let duplicate_mapping = conn
                .execute(
                    "INSERT INTO rewrite_mappings (\
                        reconciliation_run_id, repository_id, old_commit_sha, new_commit_sha,\
                        mapping_status, confidence, idempotency_key\
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    (
                        1_i64,
                        1_i64,
                        "1111111111111111111111111111111111111111",
                        "3333333333333333333333333333333333333333",
                        "mapped",
                        0.70_f64,
                        "map:key:1",
                    ),
                )
                .await;
            assert!(
                duplicate_mapping.is_err(),
                "mapping idempotency key should be unique"
            );

            Ok::<(), anyhow::Error>(())
        })?;

        let run_count = runtime.block_on(fetch_single_integer(
            LocalDatabaseTarget::Path(&path),
            "SELECT COUNT(*) FROM reconciliation_runs WHERE repository_id = 1 AND status = 'completed'",
        ))?;
        assert_eq!(run_count, 1);

        let mapped_count = runtime.block_on(fetch_single_integer(
            LocalDatabaseTarget::Path(&path),
            "SELECT COUNT(*) FROM rewrite_mappings WHERE repository_id = 1 AND old_commit_sha = '1111111111111111111111111111111111111111'",
        ))?;
        assert_eq!(mapped_count, 1);

        let joined_mapping_count = runtime.block_on(fetch_single_integer(
            LocalDatabaseTarget::Path(&path),
            "SELECT COUNT(*) FROM rewrite_mappings m JOIN reconciliation_runs r ON r.id = m.reconciliation_run_id JOIN repositories repo ON repo.id = m.repository_id WHERE r.repository_id = repo.id AND m.old_commit_sha = '1111111111111111111111111111111111111111'",
        ))?;
        assert_eq!(joined_mapping_count, 1);

        let conversation_count = runtime.block_on(fetch_single_integer(
            LocalDatabaseTarget::Path(&path),
            "SELECT COUNT(*) FROM conversations WHERE repository_id = 1 AND source = 'github'",
        ))?;
        assert_eq!(conversation_count, 1);

        Ok(())
    }

    #[test]
    fn persistent_target_survives_process_restart() -> Result<()> {
        let temp = TestTempDir::new("sce-persistent-local-db-tests")?;
        let path = temp.path().join("persistent.db");

        {
            let runtime = tokio::runtime::Builder::new_current_thread().build()?;
            runtime.block_on(apply_core_schema_migrations(LocalDatabaseTarget::Path(
                &path,
            )))?;
            runtime.block_on(async {
                let conn = super::connect_local(LocalDatabaseTarget::Path(&path)).await?;
                conn.execute(
                    "INSERT INTO repositories (canonical_root) VALUES (?1)",
                    ["/tmp/restart-proof-repo"],
                )
                .await?;
                Ok::<(), anyhow::Error>(())
            })?;
        }

        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let repository_rows =
            runtime.block_on(repository_count(LocalDatabaseTarget::Path(&path)))?;
        assert_eq!(repository_rows, 1);

        Ok(())
    }
}
