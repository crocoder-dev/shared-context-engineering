use std::path::Path;

use anyhow::{anyhow, ensure, Result};
use turso::Builder;

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
    "CREATE INDEX IF NOT EXISTS idx_commits_repository_commit_sha ON commits(repository_id, commit_sha)",
    "CREATE INDEX IF NOT EXISTS idx_trace_records_repository_commit ON trace_records(repository_id, commit_id)",
    "CREATE INDEX IF NOT EXISTS idx_trace_ranges_record_file ON trace_ranges(trace_record_id, file_path)",
];

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

async fn connect_local(target: LocalDatabaseTarget<'_>) -> Result<turso::Connection> {
    let location = match target {
        LocalDatabaseTarget::InMemory => ":memory:".to_string(),
        LocalDatabaseTarget::Path(path) => path.to_string_lossy().into_owned(),
    };

    let db = Builder::new_local(&location).build().await?;
    let conn = db.connect()?;
    Ok(conn)
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
        let location = match target {
            LocalDatabaseTarget::InMemory => ":memory:".to_string(),
            LocalDatabaseTarget::Path(path) => path.to_string_lossy().into_owned(),
        };
        let db = turso::Builder::new_local(&location).build().await?;
        let conn = db.connect()?;
        let mut rows = conn.query(&row_exists_query(kind, name), ()).await?;
        Ok(rows.next().await?.is_some())
    }

    async fn repository_count(target: LocalDatabaseTarget<'_>) -> Result<u64> {
        let location = match target {
            LocalDatabaseTarget::InMemory => ":memory:".to_string(),
            LocalDatabaseTarget::Path(path) => path.to_string_lossy().into_owned(),
        };
        let db = turso::Builder::new_local(&location).build().await?;
        let conn = db.connect()?;
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
            outcome.executed_statements, 7,
            "expected all core migration statements to execute"
        );

        for table in ["repositories", "commits", "trace_records", "trace_ranges"] {
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
            let db = turso::Builder::new_local(path.to_string_lossy().as_ref())
                .build()
                .await?;
            let conn = db.connect()?;
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
}
