use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::services::local_db;

pub const NAME: &str = "mcp";

pub fn run_mcp_server_blocking() -> Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to initialize runtime for MCP server")?;

    runtime.block_on(run_mcp_server())?;

    Ok("MCP server completed successfully.".to_string())
}

#[cfg_attr(not(test), allow(dead_code))]
const CACHE_SCHEMA_STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS file_versions (\
        id INTEGER PRIMARY KEY,\
        repository_root TEXT NOT NULL,\
        relative_path TEXT NOT NULL,\
        content_hash TEXT NOT NULL,\
        line_count INTEGER NOT NULL,\
        byte_count INTEGER NOT NULL,\
        last_read_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        UNIQUE(repository_root, relative_path)\
    )",
    "CREATE TABLE IF NOT EXISTS session_reads (\
        id INTEGER PRIMARY KEY,\
        session_id TEXT NOT NULL,\
        repository_root TEXT NOT NULL,\
        relative_path TEXT NOT NULL,\
        content_hash TEXT NOT NULL,\
        content TEXT NOT NULL DEFAULT '',\
        was_forced INTEGER NOT NULL DEFAULT 0,\
        token_savings INTEGER NOT NULL DEFAULT 0,\
        first_read_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        last_read_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        UNIQUE(session_id, repository_root, relative_path)\
    )",
    "CREATE TABLE IF NOT EXISTS cache_stats (\
        repository_root TEXT PRIMARY KEY,\
        tracked_file_count INTEGER NOT NULL DEFAULT 0,\
        session_token_savings INTEGER NOT NULL DEFAULT 0,\
        cumulative_token_savings INTEGER NOT NULL DEFAULT 0,\
        last_cleared_at TEXT,\
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
    )",
    "CREATE INDEX IF NOT EXISTS idx_file_versions_repository_path ON file_versions(repository_root, relative_path)",
    "CREATE INDEX IF NOT EXISTS idx_session_reads_session_repository ON session_reads(session_id, repository_root)",
];

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct SmartCachePaths {
    pub repository_root: PathBuf,
    pub state_root: PathBuf,
    pub cache_root: PathBuf,
    pub global_config_path: PathBuf,
    pub repository_cache_root: PathBuf,
    pub repository_db_path: PathBuf,
    pub repository_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct SmartCacheBootstrapOutcome {
    pub paths: SmartCachePaths,
    pub created_config_file: bool,
    pub executed_schema_statements: usize,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartCacheReadRequest {
    pub repository_root: PathBuf,
    pub session_id: String,
    pub relative_path: PathBuf,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub force: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartCacheReadOutcome {
    pub repository_root: PathBuf,
    pub relative_path: PathBuf,
    pub response: SmartCacheReadResponse,
    pub content_hash: String,
    pub line_count: usize,
    pub byte_count: usize,
    pub estimated_tokens: u64,
    pub saved_tokens: u64,
    pub session_saved_tokens: u64,
    pub cumulative_saved_tokens: u64,
    pub cache_hit: bool,
    pub first_read_in_session: bool,
    pub force: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartCacheBatchReadRequest {
    pub repository_root: PathBuf,
    pub session_id: String,
    pub relative_paths: Vec<PathBuf>,
    pub force: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartCacheBatchReadOutcome {
    pub repository_root: PathBuf,
    pub outputs: Vec<SmartCacheReadOutcome>,
    pub rendered_response: String,
    pub session_saved_tokens: u64,
    pub cumulative_saved_tokens: u64,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartCacheStatusRequest {
    pub repository_root: PathBuf,
    pub session_id: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartCacheStatusOutcome {
    pub repository_root: PathBuf,
    pub repository_db_path: PathBuf,
    pub tracked_file_count: u64,
    pub session_saved_tokens: u64,
    pub cumulative_saved_tokens: u64,
    pub last_cleared_at: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartCacheClearRequest {
    pub repository_root: PathBuf,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmartCacheClearOutcome {
    pub repository_root: PathBuf,
    pub repository_db_path: PathBuf,
    pub cleared_file_versions: u64,
    pub cleared_session_reads: u64,
    pub tracked_file_count: u64,
    pub session_saved_tokens: u64,
    pub cumulative_saved_tokens: u64,
    pub last_cleared_at: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SmartCacheReadResponse {
    Full {
        content: String,
    },
    Diff {
        unified_diff: String,
        changed_line_numbers: Vec<usize>,
    },
    Partial {
        content: String,
        offset: usize,
        limit: usize,
        total_lines: usize,
    },
    Unchanged {
        marker: String,
    },
    PartialUnchanged {
        marker: String,
        offset: usize,
        limit: usize,
    },
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct FileSnapshot {
    content: String,
    content_hash: String,
    line_count: usize,
    byte_count: usize,
    estimated_tokens: u64,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionReadRecord {
    content_hash: String,
    content: String,
    token_savings: u64,
}

#[allow(dead_code)]
const UNCHANGED_FILE_MARKER: &str =
    "File unchanged since the last read in this session; cached content omitted.";
#[allow(dead_code)]
const UNCHANGED_RANGE_MARKER: &str =
    "Requested line range unchanged since the last read in this session; cached content omitted.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RequestedLineRange {
    offset: usize,
    limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RenderedReadResponse {
    response: SmartCacheReadResponse,
    saved_tokens: u64,
    cache_hit: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UnifiedDiff {
    text: String,
    changed_line_numbers: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DiffOp {
    Equal(String),
    Remove(String),
    Add(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(not(test), allow(dead_code))]
struct SmartCacheGlobalConfig {
    #[serde(default)]
    repositories: BTreeMap<String, SmartCacheRepositoryConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(not(test), allow(dead_code))]
struct SmartCacheRepositoryConfig {
    repository_hash: String,
    cache_db_path: String,
}

#[allow(dead_code)]
pub fn resolve_repository_root(cwd: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .with_context(|| {
            format!(
                "Failed to resolve repository root from '{}'.",
                cwd.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.contains("not a git repository") {
            bail!(
                "Directory '{}' is not a git repository. Try: run this command inside a git working tree.",
                cwd.display()
            );
        }

        bail!(
            "Failed to resolve repository root from '{}': {}",
            cwd.display(),
            if stderr.is_empty() {
                "git rev-parse returned a non-zero exit status".to_string()
            } else {
                stderr
            }
        );
    }

    let root = String::from_utf8(output.stdout)
        .context("git returned a non-UTF-8 repository root path")?
        .trim()
        .to_string();
    if root.is_empty() {
        bail!("git returned an empty repository root path")
    }

    Ok(PathBuf::from(root))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_smart_cache_paths(repository_root: &Path) -> Result<SmartCachePaths> {
    let canonical_repository_root = fs::canonicalize(repository_root).with_context(|| {
        format!(
            "Failed to canonicalize repository root '{}'.",
            repository_root.display()
        )
    })?;
    let state_root = local_db::resolve_state_data_root()?;
    let cache_root = state_root.join("sce").join("cache");
    let repository_hash = compute_repository_hash(&canonical_repository_root);
    let repository_cache_root = cache_root.join("repos").join(&repository_hash);
    let repository_db_path = repository_cache_root.join("cache.db");
    let global_config_path = cache_root.join("config.json");

    Ok(SmartCachePaths {
        repository_root: canonical_repository_root,
        state_root,
        cache_root,
        global_config_path,
        repository_cache_root,
        repository_db_path,
        repository_hash,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn bootstrap_smart_cache_storage(repository_root: &Path) -> Result<SmartCacheBootstrapOutcome> {
    let paths = resolve_smart_cache_paths(repository_root)?;

    fs::create_dir_all(&paths.repository_cache_root).with_context(|| {
        format!(
            "Failed to create smart cache directory '{}'.",
            paths.repository_cache_root.display()
        )
    })?;

    let created_config_file = persist_global_config(&paths)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to initialize runtime for MCP cache bootstrap")?;
    let executed_schema_statements =
        runtime.block_on(apply_cache_schema(&paths.repository_db_path))?;

    Ok(SmartCacheBootstrapOutcome {
        paths,
        created_config_file,
        executed_schema_statements,
    })
}

#[allow(dead_code)]
pub fn read_cached_single_file(request: &SmartCacheReadRequest) -> Result<SmartCacheReadOutcome> {
    ensure!(
        !request.session_id.trim().is_empty(),
        "Session ID is required for Smart Cache Engine reads."
    );

    let bootstrap = bootstrap_smart_cache_storage(&request.repository_root)?;
    read_cached_single_file_with_bootstrap(
        &bootstrap.paths,
        request.session_id.trim(),
        &request.relative_path,
        request.offset,
        request.limit,
        request.force,
    )
}

#[allow(dead_code)]
pub fn read_cached_batch_files(
    request: &SmartCacheBatchReadRequest,
) -> Result<SmartCacheBatchReadOutcome> {
    ensure!(
        !request.session_id.trim().is_empty(),
        "Session ID is required for Smart Cache Engine batch reads."
    );
    ensure!(
        !request.relative_paths.is_empty(),
        "At least one relative file path is required for Smart Cache Engine batch reads."
    );

    let bootstrap = bootstrap_smart_cache_storage(&request.repository_root)?;
    let mut outputs = Vec::with_capacity(request.relative_paths.len());
    for relative_path in &request.relative_paths {
        outputs.push(read_cached_single_file_with_bootstrap(
            &bootstrap.paths,
            request.session_id.trim(),
            relative_path,
            None,
            None,
            request.force,
        )?);
    }
    let rendered_response = render_batch_read_response(&outputs);
    let status = read_smart_cache_status(&SmartCacheStatusRequest {
        repository_root: bootstrap.paths.repository_root.clone(),
        session_id: request.session_id.clone(),
    })?;

    Ok(SmartCacheBatchReadOutcome {
        repository_root: bootstrap.paths.repository_root,
        outputs,
        rendered_response,
        session_saved_tokens: status.session_saved_tokens,
        cumulative_saved_tokens: status.cumulative_saved_tokens,
    })
}

#[allow(dead_code)]
pub fn read_smart_cache_status(
    request: &SmartCacheStatusRequest,
) -> Result<SmartCacheStatusOutcome> {
    ensure!(
        !request.session_id.trim().is_empty(),
        "Session ID is required for Smart Cache Engine status reads."
    );

    let bootstrap = bootstrap_smart_cache_storage(&request.repository_root)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to initialize runtime for Smart Cache Engine status")?;
    let snapshot = runtime.block_on(query_cache_status(
        &bootstrap.paths.repository_db_path,
        &bootstrap.paths.repository_root.display().to_string(),
        request.session_id.trim(),
    ))?;

    Ok(SmartCacheStatusOutcome {
        repository_root: bootstrap.paths.repository_root,
        repository_db_path: bootstrap.paths.repository_db_path,
        tracked_file_count: snapshot.tracked_file_count,
        session_saved_tokens: snapshot.session_saved_tokens,
        cumulative_saved_tokens: snapshot.cumulative_saved_tokens,
        last_cleared_at: snapshot.last_cleared_at,
    })
}

#[allow(dead_code)]
pub fn clear_smart_cache(request: &SmartCacheClearRequest) -> Result<SmartCacheClearOutcome> {
    let bootstrap = bootstrap_smart_cache_storage(&request.repository_root)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to initialize runtime for Smart Cache Engine cache clear")?;
    let outcome = runtime.block_on(clear_cache_repository_state(
        &bootstrap.paths.repository_db_path,
        &bootstrap.paths.repository_root.display().to_string(),
    ))?;

    Ok(SmartCacheClearOutcome {
        repository_root: bootstrap.paths.repository_root,
        repository_db_path: bootstrap.paths.repository_db_path,
        cleared_file_versions: outcome.cleared_file_versions,
        cleared_session_reads: outcome.cleared_session_reads,
        tracked_file_count: outcome.status.tracked_file_count,
        session_saved_tokens: outcome.status.session_saved_tokens,
        cumulative_saved_tokens: outcome.status.cumulative_saved_tokens,
        last_cleared_at: outcome.status.last_cleared_at,
    })
}

fn read_cached_single_file_with_bootstrap(
    paths: &SmartCachePaths,
    session_id: &str,
    relative_path: &Path,
    offset: Option<usize>,
    limit: Option<usize>,
    force: bool,
) -> Result<SmartCacheReadOutcome> {
    let (relative_path, absolute_path) =
        resolve_relative_file_path(&paths.repository_root, relative_path)?;
    let snapshot = read_file_snapshot(&absolute_path)?;
    let repository_root = paths.repository_root.display().to_string();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to initialize runtime for Smart Cache Engine reads")?;
    let database_outcome = runtime.block_on(process_single_file_read(
        SingleFileReadDbRequest {
            database_path: &paths.repository_db_path,
            repository_root: repository_root.as_str(),
            session_id,
            relative_path: relative_path.as_str(),
            offset,
            limit,
            force,
        },
        &snapshot,
    ))?;

    Ok(SmartCacheReadOutcome {
        repository_root: paths.repository_root.clone(),
        relative_path: PathBuf::from(relative_path),
        response: database_outcome.response,
        content_hash: snapshot.content_hash,
        line_count: snapshot.line_count,
        byte_count: snapshot.byte_count,
        estimated_tokens: snapshot.estimated_tokens,
        saved_tokens: database_outcome.saved_tokens,
        session_saved_tokens: database_outcome.session_saved_tokens,
        cumulative_saved_tokens: database_outcome.cumulative_saved_tokens,
        cache_hit: database_outcome.cache_hit,
        first_read_in_session: database_outcome.first_read_in_session,
        force,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn compute_repository_hash(repository_root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repository_root.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

#[allow(dead_code)]
fn resolve_relative_file_path(
    repository_root: &Path,
    relative_path: &Path,
) -> Result<(String, PathBuf)> {
    ensure!(
        !relative_path.as_os_str().is_empty(),
        "Relative file path is required for Smart Cache Engine reads."
    );
    ensure!(
        !relative_path.is_absolute(),
        "File path must be relative to the repository root."
    );

    let candidate_path = repository_root.join(relative_path);
    let canonical_path = fs::canonicalize(&candidate_path).with_context(|| {
        format!(
            "Failed to resolve repository file '{}'.",
            candidate_path.display()
        )
    })?;

    ensure!(
        canonical_path.starts_with(repository_root),
        "File '{}' is outside the active repository.",
        relative_path.display()
    );
    ensure!(
        canonical_path.is_file(),
        "Path '{}' is not a readable file.",
        relative_path.display()
    );

    let normalized_path = canonical_path
        .strip_prefix(repository_root)
        .context("resolved file path was not under the repository root")?
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");

    Ok((normalized_path, canonical_path))
}

#[allow(dead_code)]
fn read_file_snapshot(path: &Path) -> Result<FileSnapshot> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read repository file '{}'.", path.display()))?;
    let byte_count = content.len();

    Ok(FileSnapshot {
        content_hash: compute_content_hash(&content),
        line_count: count_lines(&content),
        estimated_tokens: estimate_tokens(byte_count),
        byte_count,
        content,
    })
}

#[allow(dead_code)]
fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[allow(dead_code)]
fn count_lines(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

#[allow(dead_code)]
fn estimate_tokens(byte_count: usize) -> u64 {
    (byte_count.max(1) as u64).div_ceil(4)
}

#[allow(dead_code)]
fn usize_to_i64(value: usize, label: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{label} exceeds SQLite integer range"))
}

#[allow(dead_code)]
fn u64_to_i64(value: u64, label: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{label} exceeds SQLite integer range"))
}

#[cfg_attr(not(test), allow(dead_code))]
fn persist_global_config(paths: &SmartCachePaths) -> Result<bool> {
    fs::create_dir_all(&paths.cache_root).with_context(|| {
        format!(
            "Failed to create smart cache root '{}'.",
            paths.cache_root.display()
        )
    })?;

    let created_config_file = !paths.global_config_path.exists();
    let mut config = if created_config_file {
        SmartCacheGlobalConfig {
            repositories: BTreeMap::new(),
        }
    } else {
        load_global_config(&paths.global_config_path)?
    };

    config.repositories.insert(
        paths.repository_root.display().to_string(),
        SmartCacheRepositoryConfig {
            repository_hash: paths.repository_hash.clone(),
            cache_db_path: paths.repository_db_path.display().to_string(),
        },
    );

    let serialized = serde_json::to_string_pretty(&config)
        .context("Failed to serialize smart cache global config")?;
    fs::write(&paths.global_config_path, format!("{serialized}\n")).with_context(|| {
        format!(
            "Failed to write smart cache global config '{}'.",
            paths.global_config_path.display()
        )
    })?;

    Ok(created_config_file)
}

#[cfg_attr(not(test), allow(dead_code))]
fn load_global_config(path: &Path) -> Result<SmartCacheGlobalConfig> {
    let contents = fs::read_to_string(path).with_context(|| {
        format!(
            "Failed to read smart cache global config '{}'.",
            path.display()
        )
    })?;
    serde_json::from_str(&contents).with_context(|| {
        format!(
            "Failed to parse smart cache global config '{}'.",
            path.display()
        )
    })
}

#[cfg_attr(not(test), allow(dead_code))]
async fn apply_cache_schema(path: &Path) -> Result<usize> {
    let location = path.to_str().ok_or_else(|| {
        anyhow!(
            "Smart cache DB path must be valid UTF-8: {}",
            path.display()
        )
    })?;
    let db = turso::Builder::new_local(location).build().await?;
    let conn = db.connect()?;
    conn.execute("PRAGMA foreign_keys = ON", ()).await?;

    for statement in CACHE_SCHEMA_STATEMENTS {
        conn.execute(statement, ()).await?;
    }

    ensure_cache_schema_columns(&conn).await?;

    Ok(CACHE_SCHEMA_STATEMENTS.len())
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct SingleFileReadDatabaseOutcome {
    response: SmartCacheReadResponse,
    saved_tokens: u64,
    session_saved_tokens: u64,
    cumulative_saved_tokens: u64,
    cache_hit: bool,
    first_read_in_session: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SingleFileReadDbRequest<'a> {
    database_path: &'a Path,
    repository_root: &'a str,
    session_id: &'a str,
    relative_path: &'a str,
    offset: Option<usize>,
    limit: Option<usize>,
    force: bool,
}

#[allow(dead_code)]
async fn process_single_file_read(
    request: SingleFileReadDbRequest<'_>,
    snapshot: &FileSnapshot,
) -> Result<SingleFileReadDatabaseOutcome> {
    let conn = connect_cache_db(request.database_path).await?;
    let previous_session = fetch_session_read(
        &conn,
        request.session_id,
        request.repository_root,
        request.relative_path,
    )
    .await?;
    let first_read_in_session = previous_session.is_none();
    let rendered = render_read_response(
        request.relative_path,
        snapshot,
        previous_session.as_ref(),
        request.offset,
        request.limit,
        request.force,
    )?;
    let session_token_savings = previous_session
        .as_ref()
        .map_or(0, |record| record.token_savings)
        + rendered.saved_tokens;

    upsert_file_version(
        &conn,
        request.repository_root,
        request.relative_path,
        snapshot,
    )
    .await?;
    upsert_session_read(
        &conn,
        request.session_id,
        request.repository_root,
        request.relative_path,
        snapshot,
        request.force,
        session_token_savings,
    )
    .await?;
    let stats = refresh_cache_stats(&conn, request.repository_root, request.session_id).await?;

    Ok(SingleFileReadDatabaseOutcome {
        response: rendered.response,
        saved_tokens: rendered.saved_tokens,
        session_saved_tokens: stats.session_saved_tokens,
        cumulative_saved_tokens: stats.cumulative_saved_tokens,
        cache_hit: rendered.cache_hit,
        first_read_in_session,
    })
}

#[allow(dead_code)]
async fn connect_cache_db(path: &Path) -> Result<turso::Connection> {
    let location = path.to_str().ok_or_else(|| {
        anyhow!(
            "Smart cache DB path must be valid UTF-8: {}",
            path.display()
        )
    })?;
    let db = turso::Builder::new_local(location).build().await?;
    let conn = db.connect()?;
    conn.execute("PRAGMA foreign_keys = ON", ()).await?;
    Ok(conn)
}

#[allow(dead_code)]
async fn fetch_session_read(
    conn: &turso::Connection,
    session_id: &str,
    repository_root: &str,
    relative_path: &str,
) -> Result<Option<SessionReadRecord>> {
    let mut rows = conn
        .query(
            "SELECT content_hash, content, token_savings FROM session_reads WHERE session_id = ?1 AND repository_root = ?2 AND relative_path = ?3 LIMIT 1",
            (session_id, repository_root, relative_path),
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };

    let content_hash = row
        .get_value(0)?
        .as_text()
        .cloned()
        .ok_or_else(|| anyhow!("session read content hash query returned non-text"))?;
    let content = row
        .get_value(1)?
        .as_text()
        .cloned()
        .ok_or_else(|| anyhow!("session read content query returned non-text"))?;
    let token_savings = row
        .get_value(2)?
        .as_integer()
        .copied()
        .ok_or_else(|| anyhow!("session read token savings query returned non-integer"))?;
    ensure!(
        token_savings >= 0,
        "session read token savings query returned a negative value"
    );

    Ok(Some(SessionReadRecord {
        content_hash,
        content,
        token_savings: token_savings.cast_unsigned(),
    }))
}

#[allow(dead_code)]
async fn upsert_file_version(
    conn: &turso::Connection,
    repository_root: &str,
    relative_path: &str,
    snapshot: &FileSnapshot,
) -> Result<()> {
    conn.execute(
        "INSERT INTO file_versions (repository_root, relative_path, content_hash, line_count, byte_count) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(repository_root, relative_path) DO UPDATE SET content_hash = excluded.content_hash, line_count = excluded.line_count, byte_count = excluded.byte_count, last_read_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (
            repository_root,
            relative_path,
            snapshot.content_hash.as_str(),
            usize_to_i64(snapshot.line_count, "line count")?,
            usize_to_i64(snapshot.byte_count, "byte count")?,
        ),
    )
    .await?;
    Ok(())
}

#[allow(dead_code)]
async fn upsert_session_read(
    conn: &turso::Connection,
    session_id: &str,
    repository_root: &str,
    relative_path: &str,
    snapshot: &FileSnapshot,
    force: bool,
    token_savings: u64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO session_reads (session_id, repository_root, relative_path, content_hash, content, was_forced, token_savings) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(session_id, repository_root, relative_path) DO UPDATE SET content_hash = excluded.content_hash, content = excluded.content, was_forced = excluded.was_forced, token_savings = excluded.token_savings, last_read_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (
            session_id,
            repository_root,
            relative_path,
            snapshot.content_hash.as_str(),
            snapshot.content.as_str(),
            i64::from(force),
            u64_to_i64(token_savings, "token savings")?,
        ),
    )
    .await?;
    Ok(())
}

fn render_read_response(
    relative_path: &str,
    snapshot: &FileSnapshot,
    previous_session: Option<&SessionReadRecord>,
    offset: Option<usize>,
    limit: Option<usize>,
    force: bool,
) -> Result<RenderedReadResponse> {
    let requested_range = parse_requested_line_range(offset, limit)?;

    if let Some(range) = requested_range {
        return render_partial_read_response(snapshot, previous_session, range, force);
    }

    if force || previous_session.is_none() {
        return Ok(RenderedReadResponse {
            response: SmartCacheReadResponse::Full {
                content: snapshot.content.clone(),
            },
            saved_tokens: 0,
            cache_hit: false,
        });
    }

    let previous_session = previous_session.expect("checked is_some above");
    if previous_session.content_hash == snapshot.content_hash {
        return Ok(RenderedReadResponse {
            response: SmartCacheReadResponse::Unchanged {
                marker: UNCHANGED_FILE_MARKER.to_string(),
            },
            saved_tokens: snapshot.estimated_tokens,
            cache_hit: true,
        });
    }

    let diff = build_unified_diff(relative_path, &previous_session.content, &snapshot.content);
    let diff_tokens = estimate_tokens(diff.text.len());

    Ok(RenderedReadResponse {
        response: SmartCacheReadResponse::Diff {
            unified_diff: diff.text,
            changed_line_numbers: diff.changed_line_numbers,
        },
        saved_tokens: snapshot.estimated_tokens.saturating_sub(diff_tokens),
        cache_hit: false,
    })
}

fn render_partial_read_response(
    snapshot: &FileSnapshot,
    previous_session: Option<&SessionReadRecord>,
    requested_range: RequestedLineRange,
    force: bool,
) -> Result<RenderedReadResponse> {
    let sliced_content = slice_content_by_lines(
        &snapshot.content,
        requested_range.offset,
        requested_range.limit,
    )?;
    let slice_tokens = estimate_tokens(sliced_content.len());

    if force || previous_session.is_none() {
        return Ok(RenderedReadResponse {
            response: SmartCacheReadResponse::Partial {
                content: sliced_content,
                offset: requested_range.offset,
                limit: requested_range.limit,
                total_lines: snapshot.line_count,
            },
            saved_tokens: 0,
            cache_hit: false,
        });
    }

    let previous_session = previous_session.expect("checked is_some above");
    if previous_session.content_hash == snapshot.content_hash {
        return Ok(RenderedReadResponse {
            response: SmartCacheReadResponse::PartialUnchanged {
                marker: UNCHANGED_RANGE_MARKER.to_string(),
                offset: requested_range.offset,
                limit: requested_range.limit,
            },
            saved_tokens: slice_tokens,
            cache_hit: true,
        });
    }

    let diff = build_unified_diff(
        "<partial-read>",
        &previous_session.content,
        &snapshot.content,
    );
    if changed_lines_overlap(&diff.changed_line_numbers, requested_range) {
        Ok(RenderedReadResponse {
            response: SmartCacheReadResponse::Partial {
                content: sliced_content,
                offset: requested_range.offset,
                limit: requested_range.limit,
                total_lines: snapshot.line_count,
            },
            saved_tokens: 0,
            cache_hit: false,
        })
    } else {
        Ok(RenderedReadResponse {
            response: SmartCacheReadResponse::PartialUnchanged {
                marker: UNCHANGED_RANGE_MARKER.to_string(),
                offset: requested_range.offset,
                limit: requested_range.limit,
            },
            saved_tokens: slice_tokens,
            cache_hit: true,
        })
    }
}

fn render_batch_read_response(outputs: &[SmartCacheReadOutcome]) -> String {
    let mut sections = Vec::with_capacity(outputs.len() + 1);
    for output in outputs {
        sections.push(render_batch_read_section(output));
    }

    let session_saved_tokens = outputs
        .last()
        .map_or(0, |output| output.session_saved_tokens);
    sections.push(format!(
        "Session token savings: {session_saved_tokens} estimated tokens saved."
    ));
    sections.join("\n\n")
}

fn render_batch_read_section(output: &SmartCacheReadOutcome) -> String {
    let mut section = vec![format!("==> {} <==", output.relative_path.display())];
    match &output.response {
        SmartCacheReadResponse::Full { content } => push_rendered_body(&mut section, content),
        SmartCacheReadResponse::Diff { unified_diff, .. } => {
            push_rendered_body(&mut section, unified_diff);
        }
        SmartCacheReadResponse::Partial {
            content,
            offset,
            limit,
            total_lines,
        } => {
            section.push(format!(
                "lines {offset}-{} of {total_lines}",
                offset.saturating_add(limit.saturating_sub(1))
            ));
            push_rendered_body(&mut section, content);
        }
        SmartCacheReadResponse::Unchanged { marker } => section.push(marker.clone()),
        SmartCacheReadResponse::PartialUnchanged {
            marker,
            offset,
            limit,
        } => {
            section.push(format!(
                "lines {offset}-{} unchanged",
                offset.saturating_add(limit.saturating_sub(1))
            ));
            section.push(marker.clone());
        }
    }
    section.join("\n")
}

fn push_rendered_body(lines: &mut Vec<String>, content: &str) {
    if content.is_empty() {
        lines.push("<empty>".to_string());
    } else {
        lines.push(content.to_string());
    }
}

fn parse_requested_line_range(
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<Option<RequestedLineRange>> {
    match (offset, limit) {
        (None, None) => Ok(None),
        (maybe_offset, maybe_limit) => {
            let offset = maybe_offset.unwrap_or(1);
            let limit = maybe_limit.unwrap_or(usize::MAX);
            ensure!(offset > 0, "Partial read offset must be 1 or greater.");
            ensure!(limit > 0, "Partial read limit must be 1 or greater.");
            Ok(Some(RequestedLineRange { offset, limit }))
        }
    }
}

fn slice_content_by_lines(content: &str, offset: usize, limit: usize) -> Result<String> {
    ensure!(offset > 0, "Partial read offset must be 1 or greater.");
    ensure!(limit > 0, "Partial read limit must be 1 or greater.");

    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() || offset > lines.len() {
        return Ok(String::new());
    }

    let start = offset - 1;
    let end = start.saturating_add(limit).min(lines.len());
    Ok(lines[start..end].join("\n"))
}

fn changed_lines_overlap(
    changed_line_numbers: &[usize],
    requested_range: RequestedLineRange,
) -> bool {
    let requested_end = requested_range
        .offset
        .saturating_add(requested_range.limit.saturating_sub(1));
    changed_line_numbers
        .iter()
        .any(|line| *line >= requested_range.offset && *line <= requested_end)
}

fn build_unified_diff(relative_path: &str, old_content: &str, new_content: &str) -> UnifiedDiff {
    let old_lines = split_lines_for_diff(old_content);
    let new_lines = split_lines_for_diff(new_content);
    let ops = compute_diff_ops(&old_lines, &new_lines);
    let changed_line_numbers = collect_changed_line_numbers(&ops, new_lines.len());

    let mut rendered = vec![
        format!("--- a/{relative_path}"),
        format!("+++ b/{relative_path}"),
    ];
    let hunk_ranges = compute_hunk_ranges(&ops, 3);
    let (old_prefix, new_prefix) = compute_prefix_positions(&ops);

    for (start, end) in hunk_ranges {
        let old_count = count_old_lines(&ops[start..=end]);
        let new_count = count_new_lines(&ops[start..=end]);
        let old_start = hunk_range_start(old_prefix[start], old_count);
        let new_start = hunk_range_start(new_prefix[start], new_count);
        rendered.push(format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@"
        ));

        for op in &ops[start..=end] {
            match op {
                DiffOp::Equal(line) => rendered.push(format!(" {line}")),
                DiffOp::Remove(line) => rendered.push(format!("-{line}")),
                DiffOp::Add(line) => rendered.push(format!("+{line}")),
            }
        }
    }

    UnifiedDiff {
        text: rendered.join("\n"),
        changed_line_numbers,
    }
}

fn split_lines_for_diff(content: &str) -> Vec<String> {
    if content.is_empty() {
        Vec::new()
    } else {
        content.lines().map(ToOwned::to_owned).collect()
    }
}

fn compute_diff_ops(old_lines: &[String], new_lines: &[String]) -> Vec<DiffOp> {
    let mut lcs = vec![vec![0usize; new_lines.len() + 1]; old_lines.len() + 1];
    for old_index in (0..old_lines.len()).rev() {
        for new_index in (0..new_lines.len()).rev() {
            lcs[old_index][new_index] = if old_lines[old_index] == new_lines[new_index] {
                lcs[old_index + 1][new_index + 1] + 1
            } else {
                lcs[old_index + 1][new_index].max(lcs[old_index][new_index + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let mut old_index = 0;
    let mut new_index = 0;
    while old_index < old_lines.len() && new_index < new_lines.len() {
        if old_lines[old_index] == new_lines[new_index] {
            ops.push(DiffOp::Equal(old_lines[old_index].clone()));
            old_index += 1;
            new_index += 1;
        } else if lcs[old_index + 1][new_index] >= lcs[old_index][new_index + 1] {
            ops.push(DiffOp::Remove(old_lines[old_index].clone()));
            old_index += 1;
        } else {
            ops.push(DiffOp::Add(new_lines[new_index].clone()));
            new_index += 1;
        }
    }
    while old_index < old_lines.len() {
        ops.push(DiffOp::Remove(old_lines[old_index].clone()));
        old_index += 1;
    }
    while new_index < new_lines.len() {
        ops.push(DiffOp::Add(new_lines[new_index].clone()));
        new_index += 1;
    }

    ops
}

fn collect_changed_line_numbers(ops: &[DiffOp], new_line_count: usize) -> Vec<usize> {
    let mut changed = Vec::new();
    let mut new_line_number = 1usize;

    for op in ops {
        match op {
            DiffOp::Equal(_) => new_line_number += 1,
            DiffOp::Add(_) => {
                changed.push(new_line_number);
                new_line_number += 1;
            }
            DiffOp::Remove(_) => changed.push(new_line_number.min(new_line_count.max(1))),
        }
    }

    changed.dedup();
    changed
}

fn compute_hunk_ranges(ops: &[DiffOp], context_lines: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    if ops.is_empty() {
        return ranges;
    }

    for (index, op) in ops.iter().enumerate() {
        if matches!(op, DiffOp::Equal(_)) {
            continue;
        }

        let start = index.saturating_sub(context_lines);
        let end = (index + context_lines).min(ops.len() - 1);
        if let Some((_, previous_end)) = ranges.last_mut() {
            if start <= *previous_end + 1 {
                *previous_end = (*previous_end).max(end);
                continue;
            }
        }
        ranges.push((start, end));
    }

    ranges
}

fn compute_prefix_positions(ops: &[DiffOp]) -> (Vec<usize>, Vec<usize>) {
    let mut old_prefix = Vec::with_capacity(ops.len() + 1);
    let mut new_prefix = Vec::with_capacity(ops.len() + 1);
    let mut old_count = 0usize;
    let mut new_count = 0usize;

    for op in ops {
        old_prefix.push(old_count);
        new_prefix.push(new_count);
        match op {
            DiffOp::Equal(_) => {
                old_count += 1;
                new_count += 1;
            }
            DiffOp::Remove(_) => old_count += 1,
            DiffOp::Add(_) => new_count += 1,
        }
    }

    old_prefix.push(old_count);
    new_prefix.push(new_count);
    (old_prefix, new_prefix)
}

fn hunk_range_start(prefix_count: usize, line_count: usize) -> usize {
    if line_count == 0 {
        prefix_count
    } else {
        prefix_count + 1
    }
}

fn count_old_lines(ops: &[DiffOp]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, DiffOp::Equal(_) | DiffOp::Remove(_)))
        .count()
}

fn count_new_lines(ops: &[DiffOp]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, DiffOp::Equal(_) | DiffOp::Add(_)))
        .count()
}

async fn ensure_cache_schema_columns(conn: &turso::Connection) -> Result<()> {
    ensure_cache_schema_column(conn, "session_reads", "content", "TEXT NOT NULL DEFAULT ''").await
}

async fn ensure_cache_schema_column(
    conn: &turso::Connection,
    table_name: &str,
    column_name: &str,
    definition: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info({table_name})");
    let mut rows = conn.query(&pragma, ()).await?;
    while let Some(row) = rows.next().await? {
        let existing_column = row
            .get_value(1)?
            .as_text()
            .cloned()
            .ok_or_else(|| anyhow!("table info query returned non-text column name"))?;
        if existing_column == column_name {
            return Ok(());
        }
    }

    let alter_statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {definition}");
    conn.execute(&alter_statement, ()).await?;
    Ok(())
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct CacheStatsSnapshot {
    tracked_file_count: u64,
    session_saved_tokens: u64,
    cumulative_saved_tokens: u64,
    last_cleared_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClearCacheDbOutcome {
    cleared_file_versions: u64,
    cleared_session_reads: u64,
    status: CacheStatsSnapshot,
}

#[allow(dead_code)]
async fn refresh_cache_stats(
    conn: &turso::Connection,
    repository_root: &str,
    session_id: &str,
) -> Result<CacheStatsSnapshot> {
    conn.execute(
        "INSERT INTO cache_stats (repository_root) VALUES (?1) ON CONFLICT(repository_root) DO NOTHING",
        [repository_root],
    )
    .await?;

    let tracked_file_count = query_single_integer(
        conn,
        "SELECT COUNT(*) FROM file_versions WHERE repository_root = ?1",
        [repository_root],
        "tracked file count",
    )
    .await?;
    let session_saved_tokens = query_single_integer(
        conn,
        "SELECT COALESCE(SUM(token_savings), 0) FROM session_reads WHERE repository_root = ?1 AND session_id = ?2",
        (repository_root, session_id),
        "session token savings",
    )
    .await?;
    let cumulative_saved_tokens = query_single_integer(
        conn,
        "SELECT COALESCE(SUM(token_savings), 0) FROM session_reads WHERE repository_root = ?1",
        [repository_root],
        "cumulative token savings",
    )
    .await?;

    conn.execute(
        "UPDATE cache_stats SET tracked_file_count = ?2, session_token_savings = ?3, cumulative_token_savings = ?4, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE repository_root = ?1",
        (
            repository_root,
            u64_to_i64(tracked_file_count, "tracked file count")?,
            u64_to_i64(session_saved_tokens, "session token savings")?,
            u64_to_i64(cumulative_saved_tokens, "cumulative token savings")?,
        ),
    )
    .await?;

    Ok(CacheStatsSnapshot {
        tracked_file_count,
        session_saved_tokens,
        cumulative_saved_tokens,
        last_cleared_at: query_optional_text(
            conn,
            "SELECT COALESCE(last_cleared_at, '') FROM cache_stats WHERE repository_root = ?1 LIMIT 1",
            [repository_root],
            "cache clear timestamp",
        )
        .await?,
    })
}

#[allow(dead_code)]
async fn query_cache_status(
    database_path: &Path,
    repository_root: &str,
    session_id: &str,
) -> Result<CacheStatsSnapshot> {
    let conn = connect_cache_db(database_path).await?;
    refresh_cache_stats(&conn, repository_root, session_id).await
}

#[allow(dead_code)]
async fn clear_cache_repository_state(
    database_path: &Path,
    repository_root: &str,
) -> Result<ClearCacheDbOutcome> {
    let conn = connect_cache_db(database_path).await?;
    conn.execute(
        "INSERT INTO cache_stats (repository_root) VALUES (?1) ON CONFLICT(repository_root) DO NOTHING",
        [repository_root],
    )
    .await?;

    let cleared_file_versions = query_single_integer(
        &conn,
        "SELECT COUNT(*) FROM file_versions WHERE repository_root = ?1",
        [repository_root],
        "tracked file count",
    )
    .await?;
    let cleared_session_reads = query_single_integer(
        &conn,
        "SELECT COUNT(*) FROM session_reads WHERE repository_root = ?1",
        [repository_root],
        "session read count",
    )
    .await?;

    conn.execute(
        "DELETE FROM file_versions WHERE repository_root = ?1",
        [repository_root],
    )
    .await?;
    conn.execute(
        "DELETE FROM session_reads WHERE repository_root = ?1",
        [repository_root],
    )
    .await?;
    conn.execute(
        "UPDATE cache_stats SET tracked_file_count = 0, session_token_savings = 0, cumulative_token_savings = 0, last_cleared_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE repository_root = ?1",
        [repository_root],
    )
    .await?;

    let status = CacheStatsSnapshot {
        tracked_file_count: 0,
        session_saved_tokens: 0,
        cumulative_saved_tokens: 0,
        last_cleared_at: query_optional_text(
            &conn,
            "SELECT COALESCE(last_cleared_at, '') FROM cache_stats WHERE repository_root = ?1 LIMIT 1",
            [repository_root],
            "cache clear timestamp",
        )
        .await?,
    };

    Ok(ClearCacheDbOutcome {
        cleared_file_versions,
        cleared_session_reads,
        status,
    })
}

#[allow(dead_code)]
async fn query_single_integer<P>(
    conn: &turso::Connection,
    statement: &str,
    params: P,
    label: &str,
) -> Result<u64>
where
    P: turso::params::IntoParams,
{
    let mut rows = conn.query(statement, params).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow!("{label} query returned no rows"))?;
    let value = row.get_value(0)?;
    let integer = value
        .as_integer()
        .copied()
        .ok_or_else(|| anyhow!("{label} query returned non-integer"))?;
    ensure!(integer >= 0, "{label} query returned a negative value");
    Ok(integer.cast_unsigned())
}

async fn query_optional_text<P>(
    conn: &turso::Connection,
    statement: &str,
    params: P,
    label: &str,
) -> Result<Option<String>>
where
    P: turso::params::IntoParams,
{
    let mut rows = conn.query(statement, params).await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let text = row
        .get_value(0)?
        .as_text()
        .cloned()
        .ok_or_else(|| anyhow!("{label} query returned non-text"))?;
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

// MCP Server Implementation

/// Run the MCP server over stdio transport.
///
/// This function starts the MCP server and handles incoming requests from stdin,
/// responding via stdout. It implements the Model Context Protocol for the
/// Smart Cache Engine tools.
#[allow(clippy::too_many_lines)]
pub async fn run_mcp_server() -> Result<()> {
    use rmcp::{
        handler::server::{tool::ToolRouter, wrapper::Parameters},
        model::ServerInfo,
        schemars::JsonSchema,
        service::ServiceExt,
        tool, tool_router,
        transport::io::stdio,
        Json,
    };

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    struct ReadFileParams {
        /// Repository-relative path to the file to read
        path: String,
        /// Unique session identifier for cache tracking
        session_id: String,
        /// Optional 1-based line offset for partial reads
        #[serde(default)]
        offset: Option<usize>,
        /// Optional line limit for partial reads
        #[serde(default)]
        limit: Option<usize>,
        /// Force full content read, bypassing cache compression
        #[serde(default)]
        force: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    struct ReadFilesParams {
        /// List of repository-relative file paths to read
        paths: Vec<String>,
        /// Unique session identifier for cache tracking
        session_id: String,
        /// Force full content reads, bypassing cache compression
        #[serde(default)]
        force: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    struct CacheStatusParams {
        /// Unique session identifier for session-specific metrics
        session_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    struct CacheClearParams {}

    #[allow(dead_code)]
    #[derive(Debug, Clone)]
    struct SmartCacheMcpServer {
        tool_router: ToolRouter<Self>,
    }

    #[tool_router]
    impl SmartCacheMcpServer {
        fn new() -> Self {
            Self {
                tool_router: Self::tool_router(),
            }
        }

        #[tool(
            name = "read_file",
            description = "Read a file from the repository with smart caching. Returns full content on first read, unchanged markers for unchanged files, or unified diffs for changed files. Supports partial reads with offset/limit."
        )]
        async fn read_file(
            &self,
            params: Parameters<ReadFileParams>,
        ) -> Result<Json<serde_json::Value>, rmcp::ErrorData> {
            let params = params.0;
            let cwd = std::env::current_dir().map_err(|e| {
                rmcp::ErrorData::internal_error("current_dir_error", Some(e.to_string().into()))
            })?;

            let repository_root = resolve_repository_root(&cwd).map_err(|e| {
                rmcp::ErrorData::internal_error("repo_resolution_error", Some(e.to_string().into()))
            })?;

            let outcome = read_cached_single_file(&SmartCacheReadRequest {
                repository_root: repository_root.clone(),
                session_id: params.session_id,
                relative_path: PathBuf::from(&params.path),
                offset: params.offset,
                limit: params.limit,
                force: params.force,
            })
            .map_err(|e| {
                rmcp::ErrorData::internal_error("read_error", Some(e.to_string().into()))
            })?;

            let response = render_read_file_response(&outcome);
            Ok(Json(response))
        }

        #[tool(
            name = "read_files",
            description = "Read multiple files from the repository in a single batch request with smart caching. Returns per-file sections with unchanged markers or diffs, plus session token savings summary."
        )]
        async fn read_files(
            &self,
            params: Parameters<ReadFilesParams>,
        ) -> Result<Json<serde_json::Value>, rmcp::ErrorData> {
            let params = params.0;
            if params.paths.is_empty() {
                return Err(rmcp::ErrorData::invalid_params(
                    "paths_required",
                    Some("At least one file path is required".into()),
                ));
            }

            let cwd = std::env::current_dir().map_err(|e| {
                rmcp::ErrorData::internal_error("current_dir_error", Some(e.to_string().into()))
            })?;

            let repository_root = resolve_repository_root(&cwd).map_err(|e| {
                rmcp::ErrorData::internal_error("repo_resolution_error", Some(e.to_string().into()))
            })?;

            let relative_paths: Vec<PathBuf> = params.paths.iter().map(PathBuf::from).collect();

            let outcome = read_cached_batch_files(&SmartCacheBatchReadRequest {
                repository_root: repository_root.clone(),
                session_id: params.session_id,
                relative_paths,
                force: params.force,
            })
            .map_err(|e| {
                rmcp::ErrorData::internal_error("batch_read_error", Some(e.to_string().into()))
            })?;

            let response = render_read_files_response(&outcome);
            Ok(Json(response))
        }

        #[tool(
            name = "cache_status",
            description = "Report cache status for the current repository: database path, tracked file count, session token savings, and cumulative token savings."
        )]
        async fn cache_status(
            &self,
            params: Parameters<CacheStatusParams>,
        ) -> Result<Json<serde_json::Value>, rmcp::ErrorData> {
            let params = params.0;
            let cwd = std::env::current_dir().map_err(|e| {
                rmcp::ErrorData::internal_error("current_dir_error", Some(e.to_string().into()))
            })?;

            let repository_root = resolve_repository_root(&cwd).map_err(|e| {
                rmcp::ErrorData::internal_error("repo_resolution_error", Some(e.to_string().into()))
            })?;

            let outcome = read_smart_cache_status(&SmartCacheStatusRequest {
                repository_root: repository_root.clone(),
                session_id: params.session_id,
            })
            .map_err(|e| {
                rmcp::ErrorData::internal_error("status_error", Some(e.to_string().into()))
            })?;

            let response = render_cache_status_response(&outcome);
            Ok(Json(response))
        }

        #[tool(
            name = "cache_clear",
            description = "Clear cached state for the current repository. Resets file versions, session reads, and token savings while preserving the cache database scaffold."
        )]
        async fn cache_clear(
            &self,
            _params: Parameters<CacheClearParams>,
        ) -> Result<Json<serde_json::Value>, rmcp::ErrorData> {
            let cwd = std::env::current_dir().map_err(|e| {
                rmcp::ErrorData::internal_error("current_dir_error", Some(e.to_string().into()))
            })?;

            let repository_root = resolve_repository_root(&cwd).map_err(|e| {
                rmcp::ErrorData::internal_error("repo_resolution_error", Some(e.to_string().into()))
            })?;

            let outcome =
                clear_smart_cache(&SmartCacheClearRequest { repository_root }).map_err(|e| {
                    rmcp::ErrorData::internal_error("clear_error", Some(e.to_string().into()))
                })?;

            let response = render_cache_clear_response(&outcome);
            Ok(Json(response))
        }
    }

    impl rmcp::handler::server::ServerHandler for SmartCacheMcpServer {
        fn get_info(&self) -> ServerInfo {
            let mut info = ServerInfo::default();
            info.instructions = Some(
                "Smart Cache Engine MCP server for cache-aware file reads. \
                 Use read_file for single-file reads with caching, \
                 read_files for batch reads, cache_status for metrics, \
                 and cache_clear to reset cache state."
                    .into(),
            );
            info
        }
    }

    let server = SmartCacheMcpServer::new();
    let (stdin, stdout) = stdio();
    server
        .serve((stdin, stdout))
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))?;

    Ok(())
}

fn render_read_file_response(outcome: &SmartCacheReadOutcome) -> serde_json::Value {
    let response_type = match &outcome.response {
        SmartCacheReadResponse::Full { .. } => "full",
        SmartCacheReadResponse::Diff { .. } => "diff",
        SmartCacheReadResponse::Partial { .. } => "partial",
        SmartCacheReadResponse::Unchanged { .. } => "unchanged",
        SmartCacheReadResponse::PartialUnchanged { .. } => "partial_unchanged",
    };

    let mut result = json!({
        "repository_root": outcome.repository_root.display().to_string(),
        "path": outcome.relative_path.display().to_string(),
        "response_type": response_type,
        "content_hash": outcome.content_hash,
        "line_count": outcome.line_count,
        "byte_count": outcome.byte_count,
        "estimated_tokens": outcome.estimated_tokens,
        "saved_tokens": outcome.saved_tokens,
        "session_saved_tokens": outcome.session_saved_tokens,
        "cumulative_saved_tokens": outcome.cumulative_saved_tokens,
        "cache_hit": outcome.cache_hit,
        "first_read_in_session": outcome.first_read_in_session,
        "force": outcome.force,
    });

    match &outcome.response {
        SmartCacheReadResponse::Full { content } => {
            result["content"] = json!(content);
        }
        SmartCacheReadResponse::Diff {
            unified_diff,
            changed_line_numbers,
        } => {
            result["unified_diff"] = json!(unified_diff);
            result["changed_line_numbers"] = json!(changed_line_numbers);
        }
        SmartCacheReadResponse::Partial {
            content,
            offset,
            limit,
            total_lines,
        } => {
            result["content"] = json!(content);
            result["offset"] = json!(offset);
            result["limit"] = json!(limit);
            result["total_lines"] = json!(total_lines);
        }
        SmartCacheReadResponse::Unchanged { marker } => {
            result["marker"] = json!(marker);
        }
        SmartCacheReadResponse::PartialUnchanged {
            marker,
            offset,
            limit,
        } => {
            result["marker"] = json!(marker);
            result["offset"] = json!(offset);
            result["limit"] = json!(limit);
        }
    }

    result
}

fn render_read_files_response(outcome: &SmartCacheBatchReadOutcome) -> serde_json::Value {
    let outputs: Vec<serde_json::Value> = outcome
        .outputs
        .iter()
        .map(render_read_file_response)
        .collect();

    json!({
        "repository_root": outcome.repository_root.display().to_string(),
        "outputs": outputs,
        "rendered_response": outcome.rendered_response,
        "session_saved_tokens": outcome.session_saved_tokens,
        "cumulative_saved_tokens": outcome.cumulative_saved_tokens,
    })
}

fn render_cache_status_response(outcome: &SmartCacheStatusOutcome) -> serde_json::Value {
    json!({
        "repository_root": outcome.repository_root.display().to_string(),
        "repository_db_path": outcome.repository_db_path.display().to_string(),
        "tracked_file_count": outcome.tracked_file_count,
        "session_saved_tokens": outcome.session_saved_tokens,
        "cumulative_saved_tokens": outcome.cumulative_saved_tokens,
        "last_cleared_at": outcome.last_cleared_at,
    })
}

fn render_cache_clear_response(outcome: &SmartCacheClearOutcome) -> serde_json::Value {
    json!({
        "repository_root": outcome.repository_root.display().to_string(),
        "repository_db_path": outcome.repository_db_path.display().to_string(),
        "cleared_file_versions": outcome.cleared_file_versions,
        "cleared_session_reads": outcome.cleared_session_reads,
        "tracked_file_count": outcome.tracked_file_count,
        "session_saved_tokens": outcome.session_saved_tokens,
        "cumulative_saved_tokens": outcome.cumulative_saved_tokens,
        "last_cleared_at": outcome.last_cleared_at,
    })
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::NAME;

    #[test]
    fn mcp_server_name_is_mcp() {
        assert_eq!(NAME, "mcp");
    }

    #[test]
    fn resolve_repository_root_finds_git_root() -> Result<()> {
        // Skip test if not in a git repository
        let cwd = std::env::current_dir()?;
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(&cwd)
            .output();

        if output.is_err() || !output.as_ref().map(|o| o.status.success()).unwrap_or(false) {
            // Not in a git repository, skip test
            return Ok(());
        }

        let root = super::resolve_repository_root(&cwd)?;
        assert!(root.is_dir());
        assert!(root.join(".git").exists() || root.join(".git").is_file());
        Ok(())
    }

    #[test]
    fn smart_cache_paths_are_deterministic() -> Result<()> {
        // Skip test if not in a git repository
        let cwd = std::env::current_dir()?;
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(&cwd)
            .output();

        if output.is_err() || !output.as_ref().map(|o| o.status.success()).unwrap_or(false) {
            // Not in a git repository, skip test
            return Ok(());
        }

        let root = super::resolve_repository_root(&cwd)?;
        let paths1 = super::resolve_smart_cache_paths(&root)?;
        let paths2 = super::resolve_smart_cache_paths(&root)?;
        assert_eq!(paths1.repository_hash, paths2.repository_hash);
        assert_eq!(paths1.repository_db_path, paths2.repository_db_path);
        Ok(())
    }
}
