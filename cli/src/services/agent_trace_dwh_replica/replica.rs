//! The lock-owning Turso Sync replica boundary.
//!
//! `AgentTraceDwhReplica` is the only owner of a Turso Sync builder in this
//! codebase. It acquires the [`BridgeLock`] before touching the network or
//! the local replica file, opens the local `agent-trace-sync.db` file
//! against a caller-supplied remote using Turso's `sync` feature, and
//! classifies the bootstrapped database's schema via
//! [`AgentTraceDwhDb::classify_schema_state`]. A `Ready` schema is left
//! unchanged; a genuinely `Empty` schema is initialized locally with
//! [`AgentTraceDwhDb::run_migrations`] and published to the remote; an
//! `Incompatible` schema fails loudly and is never repaired or partially
//! completed. It never enables `experimental_multiprocess_wal`: that flag is
//! reserved for the multiprocess-WAL source capture database, which this
//! replica never opens.

use std::{fmt, path::Path, path::PathBuf};

use crate::services::{
    agent_trace_db::repository::RepositoryAgentTraceDb,
    agent_trace_dwh_db::{AgentTraceDwhDb, AgentTraceDwhSchemaState},
    agent_trace_dwh_replica::lock::{BridgeLock, BridgeLockError},
    agent_trace_etl::{AgentTraceEtl, AgentTraceEtlStats},
};

/// Explicit caller-supplied configuration for opening an
/// [`AgentTraceDwhReplica`].
///
/// The replica never discovers or persists these values itself: callers
/// resolve `local_path` from the canonical
/// `agent_trace_dwh_replica_path_for_repository` helper and are responsible
/// for acquiring `database_url`/`auth_token` themselves.
pub struct AgentTraceDwhReplicaConfig {
    pub local_path: PathBuf,
    pub database_url: String,
    pub auth_token: String,
}

impl fmt::Debug for AgentTraceDwhReplicaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentTraceDwhReplicaConfig")
            .field("local_path", &self.local_path)
            .field("database_url", &self.database_url)
            .field("auth_token", &"<redacted>")
            .finish()
    }
}

/// A held bridge lock plus an open Turso Sync connection to a repository's
/// Agent Trace DWH replica.
///
/// Both the lock and the connection share this value's lifetime: dropping the
/// replica releases the lock. Only this type owns a `turso::sync::Builder`;
/// no other caller in this codebase is permitted to open a Turso Sync
/// connection to `agent-trace-sync.db`.
pub struct AgentTraceDwhReplica {
    lock: BridgeLock,
    sync_db: turso::sync::Database,
    db: AgentTraceDwhDb,
}

impl fmt::Debug for AgentTraceDwhReplica {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentTraceDwhReplica")
            .field("lock_path", &self.lock.path())
            .finish_non_exhaustive()
    }
}

impl AgentTraceDwhReplica {
    /// Acquire the bridge lock and open the replica against the caller's
    /// remote.
    ///
    /// Ordering is load-bearing: the bridge lock is acquired first and
    /// non-blockingly, so a concurrent owner is rejected before any Turso
    /// Sync builder, local file, or network access happens. A missing local
    /// file is bootstrapped from the remote using Turso Sync's normal
    /// `bootstrap_if_empty` behavior.
    ///
    /// Once open, the bootstrapped database's schema is classified via
    /// [`AgentTraceDwhDb::classify_schema_state`] and handled accordingly:
    /// a [`AgentTraceDwhSchemaState::Ready`] schema is left untouched; a
    /// [`AgentTraceDwhSchemaState::Empty`] schema is initialized locally with
    /// [`AgentTraceDwhDb::run_migrations`] and published with a single
    /// `push()` (narrowly recovering from a push conflict with one `pull()`
    /// plus re-verification); a
    /// [`AgentTraceDwhSchemaState::Incompatible`] schema fails loudly and is
    /// never repaired or partially completed.
    pub fn open(config: AgentTraceDwhReplicaConfig) -> Result<Self, AgentTraceDwhReplicaError> {
        let AgentTraceDwhReplicaConfig {
            local_path,
            database_url,
            auth_token,
        } = config;

        let lock_path = bridge_lock_path_for_replica(&local_path);
        let lock = BridgeLock::acquire(&lock_path).map_err(AgentTraceDwhReplicaError::Lock)?;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|source| AgentTraceDwhReplicaError::Runtime { source })?;

        let local_path_str = local_path
            .to_str()
            .ok_or_else(|| AgentTraceDwhReplicaError::Open {
                local_path: local_path.clone(),
                message: String::from("path is not valid UTF-8"),
            })?
            .to_string();

        let auth_token_for_open = auth_token.clone();
        let database_url_for_open = database_url.clone();
        let open_result: Result<(turso::sync::Database, turso::Connection), turso::Error> = runtime
            .block_on(async move {
                // `experimental_multiprocess_wal` is intentionally never
                // enabled here: this replica is the single-owner Turso Sync
                // database, not the multiprocess-WAL source capture store.
                let db = turso::sync::Builder::new_remote(&local_path_str)
                    .with_remote_url(&database_url_for_open)
                    .with_auth_token(auth_token_for_open)
                    .build()
                    .await?;
                let conn = db.connect().await?;
                Ok((db, conn))
            });

        let (sync_db, conn) = open_result.map_err(|error| AgentTraceDwhReplicaError::Open {
            local_path: local_path.clone(),
            message: redact_token(&error, &auth_token),
        })?;

        let db = AgentTraceDwhDb::from_connection(conn, runtime);

        let schema_state = db.classify_schema_state().map_err(|error| {
            AgentTraceDwhReplicaError::SchemaInspection {
                message: redact_token(&error, &auth_token),
            }
        })?;

        match schema_state {
            AgentTraceDwhSchemaState::Ready => {}
            AgentTraceDwhSchemaState::Empty => {
                initialize_empty_schema(&db, &sync_db, &auth_token)?;
            }
            AgentTraceDwhSchemaState::Incompatible(reason) => {
                return Err(AgentTraceDwhReplicaError::IncompatibleSchema {
                    message: format!(
                        "{reason} (automatic initialization is only allowed for a genuinely \
                         empty remote/replica)"
                    ),
                });
            }
        }

        Ok(Self { lock, sync_db, db })
    }

    /// Lock-lifetime-bound access to the replica's DWH SQL surface.
    ///
    /// Non-mutating schema-readiness checks and application SQL both go
    /// through the same [`AgentTraceDwhDb`] this replica opened; no separate
    /// connection is created.
    pub fn db(&self) -> &AgentTraceDwhDb {
        &self.db
    }

    /// Run the incremental Agent Trace ETL while this replica owns its bridge
    /// lock. Pull/push and credential handling remain explicit caller work.
    pub fn run_agent_trace_etl(
        &self,
        repository_id: &str,
        source: &RepositoryAgentTraceDb,
        etl: AgentTraceEtl,
    ) -> anyhow::Result<AgentTraceEtlStats> {
        etl.run(repository_id, source, self)
    }

    /// Pull remote changes into the local replica.
    ///
    /// Returns `true` if any changes were applied. Non-mutating for already
    /// up-to-date replicas.
    pub fn pull(&self) -> Result<bool, AgentTraceDwhReplicaError> {
        self.db
            .block_on(self.sync_db.pull())
            .map_err(|error| AgentTraceDwhReplicaError::Pull {
                message: error.to_string(),
            })
    }

    /// Push local changes to the remote.
    pub fn push(&self) -> Result<(), AgentTraceDwhReplicaError> {
        self.db
            .block_on(self.sync_db.push())
            .map_err(|error| AgentTraceDwhReplicaError::Push {
                message: error.to_string(),
            })
    }
}

/// Initialize a genuinely [`AgentTraceDwhSchemaState::Empty`] replica by
/// running the DWH migration baseline locally and publishing it to the
/// remote.
///
/// On a push failure, this makes exactly one narrow recovery attempt: pull
/// whatever the remote now holds (best-effort; a pull failure here does not
/// change the outcome) and re-check readiness. If the schema is `Ready`
/// afterward, another initializer must have won the race and published first,
/// so this is treated as success. Otherwise the *original* push failure is
/// returned — never a swallowed or generic error.
fn initialize_empty_schema(
    db: &AgentTraceDwhDb,
    sync_db: &turso::sync::Database,
    auth_token: &str,
) -> Result<(), AgentTraceDwhReplicaError> {
    db.run_migrations()
        .map_err(|error| AgentTraceDwhReplicaError::SchemaInitialization {
            message: redact_token(&error, auth_token),
        })?;

    if let Err(push_error) = db.block_on(sync_db.push()) {
        let _ = db.block_on(sync_db.pull());

        if !matches!(db.ensure_dwh_schema_ready(), Ok(())) {
            return Err(AgentTraceDwhReplicaError::SchemaPublication {
                message: redact_token(&push_error, auth_token),
            });
        }

        return Ok(());
    }

    db.ensure_dwh_schema_ready()
        .map_err(|error| AgentTraceDwhReplicaError::ReadinessVerification {
            message: redact_token(&error, auth_token),
        })
}

/// Derive the bridge-lock path for an explicit replica path: the replica
/// path's file name with `.bridge-lock` appended, mirroring
/// `default_paths::agent_trace_dwh_bridge_lock_path_for_repository`.
fn bridge_lock_path_for_replica(local_path: &Path) -> PathBuf {
    let mut file_name = local_path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    file_name.push(".bridge-lock");
    local_path.with_file_name(file_name)
}

/// Replace every occurrence of `token` in `message` so caller-supplied
/// credentials never reach diagnostics.
fn redact_token(message: impl fmt::Display, token: &str) -> String {
    let message = message.to_string();
    if token.is_empty() {
        message
    } else {
        message.replace(token, "<redacted>")
    }
}

/// Failure opening or operating an [`AgentTraceDwhReplica`].
///
/// Every variant is built with the auth token already redacted; none of
/// these ever include the caller-supplied token.
#[derive(Debug)]
pub enum AgentTraceDwhReplicaError {
    /// The bridge lock is already held by another owner, or acquiring it
    /// failed.
    Lock(BridgeLockError),
    /// Building the local Tokio runtime failed.
    Runtime { source: std::io::Error },
    /// Opening the Turso Sync connection (including remote bootstrap)
    /// failed.
    Open {
        local_path: PathBuf,
        message: String,
    },
    /// Classifying the opened database's schema state failed.
    SchemaInspection { message: String },
    /// The schema is neither ready nor genuinely empty — an unrelated
    /// schema, a partial DWH schema, or a migration ledger with unexpected
    /// entries. Never repaired or partially completed automatically.
    IncompatibleSchema { message: String },
    /// Running the local DWH migration baseline on a genuinely empty schema
    /// failed.
    SchemaInitialization { message: String },
    /// Publishing a locally initialized empty schema to the remote failed,
    /// including after the narrow one-`pull()`-and-re-verify recovery
    /// attempt.
    SchemaPublication { message: String },
    /// The schema still failed readiness verification after a local
    /// initialization and successful publish.
    ReadinessVerification { message: String },
    /// Pulling remote changes failed.
    Pull { message: String },
    /// Pushing local changes failed.
    Push { message: String },
}

impl fmt::Display for AgentTraceDwhReplicaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) => write!(f, "{error}"),
            Self::Runtime { source } => {
                write!(
                    f,
                    "failed to create Agent Trace DWH replica runtime: {source}"
                )
            }
            Self::Open {
                local_path,
                message,
            } => write!(
                f,
                "failed to open Agent Trace DWH replica at {}: {message}",
                local_path.display()
            ),
            Self::SchemaInspection { message } => {
                write!(
                    f,
                    "failed to classify Agent Trace DWH replica schema state: {message}"
                )
            }
            Self::IncompatibleSchema { message } => {
                write!(
                    f,
                    "Agent Trace DWH replica schema is incompatible: {message}"
                )
            }
            Self::SchemaInitialization { message } => {
                write!(
                    f,
                    "failed to initialize Agent Trace DWH replica schema: {message}"
                )
            }
            Self::SchemaPublication { message } => {
                write!(
                    f,
                    "failed to publish Agent Trace DWH replica schema: {message}"
                )
            }
            Self::ReadinessVerification { message } => {
                write!(
                    f,
                    "Agent Trace DWH replica schema is still not ready after initialization: {message}"
                )
            }
            Self::Pull { message } => write!(f, "Agent Trace DWH replica pull failed: {message}"),
            Self::Push { message } => write!(f, "Agent Trace DWH replica push failed: {message}"),
        }
    }
}

impl std::error::Error for AgentTraceDwhReplicaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lock(error) => Some(error),
            Self::Runtime { source } => Some(source),
            Self::Open { .. }
            | Self::SchemaInspection { .. }
            | Self::IncompatibleSchema { .. }
            | Self::SchemaInitialization { .. }
            | Self::SchemaPublication { .. }
            | Self::ReadinessVerification { .. }
            | Self::Pull { .. }
            | Self::Push { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn unique_test_replica_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-agent-trace-dwh-replica-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace-sync.db")
    }

    fn remove_test_replica(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn bridge_lock_path_matches_the_replica_path_bridge_lock_suffix_convention() {
        let replica_path = PathBuf::from("/state/sce/repos/abc/agent-trace-sync.db");
        let lock_path = bridge_lock_path_for_replica(&replica_path);
        assert_eq!(
            lock_path,
            PathBuf::from("/state/sce/repos/abc/agent-trace-sync.db.bridge-lock")
        );
    }

    #[test]
    fn open_fails_before_any_turso_access_when_the_bridge_lock_is_already_held() {
        let local_path = unique_test_replica_path("lock-before-open");
        let lock_path = bridge_lock_path_for_replica(&local_path);

        let _held = BridgeLock::acquire(&lock_path).expect("first acquire should succeed");

        // An unreachable remote URL would fail during Turso Sync open if
        // reached; using it here proves the lock check happens first,
        // because acquiring the lock fails deterministically before any
        // Turso builder or network access is attempted.
        let error = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: local_path.clone(),
            database_url: String::from("http://127.0.0.1:0"),
            auth_token: String::from("sentinel-token-should-never-be-reached"),
        })
        .expect_err("open should fail while the bridge lock is held");

        assert!(matches!(error, AgentTraceDwhReplicaError::Lock(_)));

        remove_test_replica(&local_path);
    }

    #[test]
    fn redact_token_replaces_every_occurrence() {
        let redacted = redact_token("token=abc123 failed; retry with abc123", "abc123");
        assert_eq!(redacted, "token=<redacted> failed; retry with <redacted>");
    }

    #[test]
    fn redact_token_is_a_no_op_for_an_empty_token() {
        assert_eq!(redact_token("some message", ""), "some message");
    }
}

/// Turso Sync integration harness.
///
/// Exercises fresh bootstrap, schema-readiness classification, independent
/// pull/push visibility, and local-deletion reconstruction against a real
/// disposable Turso Sync remote. Runs only when a `tursodb` binary
/// supporting `--sync-server` is discoverable on `PATH` (the pinned
/// `nix develop .#database` shell provides one); otherwise it records why it
/// was skipped and passes trivially, so `cargo test` outside that shell does
/// not fail.
#[cfg(test)]
mod integration_tests {
    use std::{
        net::TcpStream,
        process::{Child, Command, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::{agent_trace_dwh_db::AgentTraceDwhDbSpec, db::DbSpec};

    fn unique_test_replica_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-agent-trace-dwh-replica-integration-{label}-{}-{nonce}",
            std::process::id()
        ));
        // `AgentTraceDwhReplica::open` creates its parent directory via the
        // bridge lock, but the schema-preparation helpers below open a raw
        // Turso Sync connection directly, so the directory must already
        // exist for them too.
        std::fs::create_dir_all(&dir).expect("create unique test replica directory");
        dir.join("agent-trace-sync.db")
    }

    fn remove_test_replica(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    fn build_test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("build test tokio runtime")
    }

    /// Locate a `tursodb` binary on `PATH`. This is the same pinned build
    /// `nix develop .#database` layers into the shell; it is intentionally
    /// not required in the default shell so ordinary `cargo test` runs stay
    /// fast and hermetic.
    fn find_tursodb() -> Option<PathBuf> {
        let path_var = std::env::var_os("PATH")?;
        std::env::split_paths(&path_var).find_map(|dir| {
            let candidate = dir.join("tursodb");
            candidate.is_file().then_some(candidate)
        })
    }

    struct LocalSyncServer {
        child: Child,
        url: String,
    }

    impl LocalSyncServer {
        fn spawn(tursodb_path: &Path) -> Self {
            let port = {
                let listener = std::net::TcpListener::bind("127.0.0.1:0")
                    .expect("bind an ephemeral port to pick a free one for the sync server");
                listener
                    .local_addr()
                    .expect("resolve the bound ephemeral port")
                    .port()
            };
            let addr = format!("127.0.0.1:{port}");

            let child = Command::new(tursodb_path)
                .args(["--sync-server", &addr])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn tursodb --sync-server");

            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if TcpStream::connect(&addr).is_ok() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "tursodb sync server did not become ready in time"
                );
                std::thread::sleep(Duration::from_millis(50));
            }

            Self {
                child,
                url: format!("http://{addr}"),
            }
        }
    }

    impl Drop for LocalSyncServer {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Publish the real Agent Trace DWH migration baseline to `remote_url`
    /// through a disposable local sync connection, so replicas opened
    /// against it afterward observe a genuinely ready DWH schema without
    /// `open()` itself needing to initialize anything. Used only for the
    /// already-`Ready`/unchanged-on-open case: the main fresh-bootstrap
    /// coverage instead starts from a truly empty remote and lets `open()`
    /// initialize and publish it itself.
    fn prepare_remote_with_ready_dwh_schema(remote_url: &str) {
        let local_path = unique_test_replica_path("prep-dwh-schema");
        let runtime = build_test_runtime();
        let local_path_str = local_path.to_str().unwrap().to_string();
        let remote_url = remote_url.to_string();

        let (sync_db, conn) = runtime
            .block_on(async move {
                let db = turso::sync::Builder::new_remote(&local_path_str)
                    .with_remote_url(&remote_url)
                    .build()
                    .await?;
                let conn = db.connect().await?;
                Ok::<_, turso::Error>((db, conn))
            })
            .expect("open the schema-preparation connection should succeed");

        let db = AgentTraceDwhDb::from_connection(conn, runtime);
        db.run_migrations()
            .expect("running the DWH migration baseline should succeed");
        db.block_on(sync_db.push())
            .expect("publishing the DWH schema baseline should succeed");

        remove_test_replica(&local_path);
    }

    /// Publish an unrelated, non-DWH table to `remote_url`, so a replica
    /// bootstrapped from it has a non-empty local file that still fails
    /// `ensure_dwh_schema_ready`.
    fn prepare_remote_with_incompatible_schema(remote_url: &str) {
        let local_path = unique_test_replica_path("prep-incompatible-schema");
        let runtime = build_test_runtime();
        let local_path_str = local_path.to_str().unwrap().to_string();
        let remote_url_owned = remote_url.to_string();

        let (sync_db, _conn) = runtime
            .block_on(async move {
                let db = turso::sync::Builder::new_remote(&local_path_str)
                    .with_remote_url(&remote_url_owned)
                    .build()
                    .await?;
                let conn = db.connect().await?;
                conn.execute("CREATE TABLE unrelated_schema (x INTEGER)", ())
                    .await?;
                Ok::<_, turso::Error>((db, conn))
            })
            .expect("open the incompatible-schema preparation connection should succeed");

        runtime
            .block_on(sync_db.push())
            .expect("publishing the incompatible schema should succeed");

        remove_test_replica(&local_path);
    }

    /// Publish only a subset of the DWH contract tables (no `__sce_migrations`
    /// ledger) to `remote_url`, so a replica bootstrapped from it has a
    /// non-empty, non-ready local file that is neither a fresh empty schema
    /// nor a fully-applied one.
    fn prepare_remote_with_partial_dwh_schema(remote_url: &str) {
        let local_path = unique_test_replica_path("prep-partial-dwh-schema");
        let runtime = build_test_runtime();
        let local_path_str = local_path.to_str().unwrap().to_string();
        let remote_url_owned = remote_url.to_string();

        let (sync_db, _conn) = runtime
            .block_on(async move {
                let db = turso::sync::Builder::new_remote(&local_path_str)
                    .with_remote_url(&remote_url_owned)
                    .build()
                    .await?;
                let conn = db.connect().await?;
                conn.execute(
                    "CREATE TABLE repositories (repository_id TEXT PRIMARY KEY)",
                    (),
                )
                .await?;
                Ok::<_, turso::Error>((db, conn))
            })
            .expect("open the partial-schema preparation connection should succeed");

        runtime
            .block_on(sync_db.push())
            .expect("publishing the partial schema should succeed");

        remove_test_replica(&local_path);
    }

    /// Publish a fully-applied DWH schema to `remote_url`, then append an
    /// unexpected/unknown migration ID to its `__sce_migrations` ledger, so a
    /// replica bootstrapped from it has an otherwise-ready schema that still
    /// fails readiness verification.
    fn prepare_remote_with_unexpected_ledger_entry(remote_url: &str) {
        let local_path = unique_test_replica_path("prep-unexpected-ledger");
        let runtime = build_test_runtime();
        let local_path_str = local_path.to_str().unwrap().to_string();
        let remote_url_owned = remote_url.to_string();

        let (sync_db, conn) = runtime
            .block_on(async move {
                let db = turso::sync::Builder::new_remote(&local_path_str)
                    .with_remote_url(&remote_url_owned)
                    .build()
                    .await?;
                let conn = db.connect().await?;
                Ok::<_, turso::Error>((db, conn))
            })
            .expect("open the unexpected-ledger preparation connection should succeed");

        let db = AgentTraceDwhDb::from_connection(conn, runtime);
        db.run_migrations()
            .expect("running the DWH migration baseline should succeed");
        db.execute(
            "INSERT INTO __sce_migrations (id) VALUES ('999_unknown_migration')",
            (),
        )
        .expect("unexpected ledger entry insert should succeed");
        db.block_on(sync_db.push())
            .expect("publishing the unexpected-ledger schema should succeed");

        remove_test_replica(&local_path);
    }

    /// The non-internal table set currently published on `remote_url`, used
    /// to prove a rejected `open()` never adds, removes, or modifies any
    /// remote table.
    fn remote_table_set(remote_url: &str, label: &str) -> Vec<String> {
        let local_path = unique_test_replica_path(label);
        let runtime = build_test_runtime();
        let local_path_str = local_path.to_str().unwrap().to_string();
        let remote_url_owned = remote_url.to_string();

        let conn = runtime
            .block_on(async move {
                let db = turso::sync::Builder::new_remote(&local_path_str)
                    .with_remote_url(&remote_url_owned)
                    .build()
                    .await?;
                db.connect().await
            })
            .expect("open the table-set inspection connection should succeed");
        let db = AgentTraceDwhDb::from_connection(conn, runtime);

        let mut tables = db
            .query_map(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name ASC",
                (),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("table-set query should succeed");
        tables.sort();

        remove_test_replica(&local_path);
        tables
    }

    fn source_instance_ids(db: &AgentTraceDwhDb, repository_id: &str) -> Vec<String> {
        let mut ids = db
            .query_map(
                "SELECT source_instance_id FROM source_instances WHERE repository_id = ?1",
                (repository_id,),
                |row| row.get::<String>(0).map_err(Into::into),
            )
            .expect("source_instances query should succeed");
        ids.sort();
        ids
    }

    /// Covers AC3–AC5 plus T05's empty-remote auto-initialization: a truly
    /// empty remote (no prior schema preparation) is initialized and
    /// published by `open()` itself, lock-before-open rejects a concurrent
    /// owner, independent pull/push visibility works in both directions, and
    /// local deletion reconstructs remote data.
    fn assert_bootstrap_lock_and_pull_push(
        tursodb_path: &Path,
        repository_id: &str,
        sentinel_token: &str,
    ) {
        // --- T05: `open()` against a truly empty remote (nothing published
        // to it yet) auto-initializes and publishes the DWH schema itself,
        // without any prior `prepare_remote_with_*` call. ---
        let server = LocalSyncServer::spawn(tursodb_path);

        let path_a = unique_test_replica_path("peer-a");
        let replica_a = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_a.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("opening a truly empty remote should auto-initialize and publish the DWH schema");
        assert!(
            path_a.is_file(),
            "bootstrap should create the local replica file"
        );
        assert_eq!(
            replica_a
                .db()
                .classify_schema_state()
                .expect("schema classification should succeed after auto-initialization"),
            AgentTraceDwhSchemaState::Ready,
            "the schema replica_a initialized locally should classify as Ready"
        );

        // --- AC3: a concurrent open against the same local path is rejected
        // before any Turso Sync access, while the first replica is alive. ---
        let concurrent_error = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_a.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect_err("a second open against the same replica path should be rejected");
        assert!(matches!(
            concurrent_error,
            AgentTraceDwhReplicaError::Lock(_)
        ));

        // Peer replica opened before any domain writes exist, to prove both
        // that a second independent fresh replica observes replica_a's
        // published schema as Ready (without re-running migrations) and that
        // pull (not just fresh bootstrap) makes independently published data
        // visible.
        let path_b = unique_test_replica_path("peer-b");
        let replica_b = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_b.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("second independent peer should bootstrap against the same remote");
        assert_eq!(
            replica_b
                .db()
                .classify_schema_state()
                .expect("schema classification should succeed for the second peer"),
            AgentTraceDwhSchemaState::Ready,
            "a second fresh replica should observe replica_a's published schema as Ready"
        );
        assert!(
            source_instance_ids(replica_b.db(), repository_id).is_empty(),
            "peer should start with no rows for this repository"
        );

        assert_bidirectional_pull_push(&replica_a, &replica_b, repository_id);

        // --- AC5: deleting the local replica plus Turso sidecars, then
        // reopening fresh, reconstructs all previously published data. ---
        drop(replica_a);
        remove_test_replica(&path_a);

        let reconstructed = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_a.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("reopening a deleted replica should reconstruct it via fresh bootstrap");
        assert_eq!(
            source_instance_ids(reconstructed.db(), repository_id),
            vec![String::from("instance-a"), String::from("instance-b")],
            "reconstruction should recover both peers' previously published writes"
        );

        drop(reconstructed);
        drop(replica_b);
        remove_test_replica(&path_a);
        remove_test_replica(&path_b);
        drop(server);
    }

    /// Covers AC5: a push from one peer becomes visible to another peer only
    /// after `pull()`, in both directions.
    fn assert_bidirectional_pull_push(
        replica_a: &AgentTraceDwhReplica,
        replica_b: &AgentTraceDwhReplica,
        repository_id: &str,
    ) {
        replica_a
            .db()
            .execute(
                "INSERT INTO repositories (repository_id) VALUES (?1) ON CONFLICT (repository_id) DO NOTHING",
                (repository_id,),
            )
            .expect("repository dimension insert should succeed");
        replica_a
            .db()
            .execute(
                "INSERT INTO source_instances (repository_id, source_instance_id) VALUES (?1, ?2)",
                (repository_id, "instance-a"),
            )
            .expect("source instance insert should succeed");
        replica_a
            .push()
            .expect("push from replica_a should succeed");

        replica_b.pull().expect("pull on replica_b should succeed");
        assert_eq!(
            source_instance_ids(replica_b.db(), repository_id),
            vec![String::from("instance-a")],
            "replica_b should observe replica_a's independently published write"
        );

        replica_b
            .db()
            .execute(
                "INSERT INTO source_instances (repository_id, source_instance_id) VALUES (?1, ?2)",
                (repository_id, "instance-b"),
            )
            .expect("second source instance insert should succeed");
        replica_b
            .push()
            .expect("push from replica_b should succeed");

        replica_a.pull().expect("pull on replica_a should succeed");
        assert_eq!(
            source_instance_ids(replica_a.db(), repository_id),
            vec![String::from("instance-a"), String::from("instance-b")],
            "replica_a should observe replica_b's independently published write"
        );
    }

    /// Covers T06: opening `local_path` against `remote_url` fails with
    /// `IncompatibleSchema`, never leaks `sentinel_token`, and never adds,
    /// removes, or modifies any table on the remote (proven by comparing the
    /// remote's non-internal table set before and after the rejected open).
    fn assert_open_is_rejected_without_mutating_remote(
        remote_url: &str,
        sentinel_token: &str,
        local_path_label: &str,
        table_set_label: &str,
    ) {
        let tables_before = remote_table_set(remote_url, &format!("{table_set_label}-before"));

        let local_path = unique_test_replica_path(local_path_label);
        let schema_error = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: local_path.clone(),
            database_url: remote_url.to_string(),
            auth_token: sentinel_token.to_string(),
        })
        .expect_err("bootstrapping an incompatible remote should fail schema readiness");
        assert!(matches!(
            schema_error,
            AgentTraceDwhReplicaError::IncompatibleSchema { .. }
        ));
        let schema_error_message = schema_error.to_string();
        assert!(
            !schema_error_message.contains(sentinel_token),
            "incompatible-schema error must not contain the auth token: {schema_error_message}"
        );

        let tables_after = remote_table_set(remote_url, &format!("{table_set_label}-after"));
        assert_eq!(
            tables_before, tables_after,
            "a rejected open must not add, remove, or modify any remote table"
        );

        remove_test_replica(&local_path);
    }

    /// Covers AC4's negative case and T06: a remote whose bootstrapped schema
    /// is not the DWH schema is reported incompatible, without locally
    /// provisioning a competing schema or mutating the remote, and the error
    /// never contains the auth token.
    fn assert_unrelated_schema_is_rejected(tursodb_path: &Path, sentinel_token: &str) {
        let server = LocalSyncServer::spawn(tursodb_path);
        prepare_remote_with_incompatible_schema(&server.url);

        assert_open_is_rejected_without_mutating_remote(
            &server.url,
            sentinel_token,
            "incompatible",
            "unrelated-schema",
        );

        drop(server);
    }

    /// Covers T06: a remote carrying only some of the seven DWH contract
    /// tables, with no valid `__sce_migrations` ledger, is rejected as
    /// incompatible rather than being completed or repaired.
    fn assert_partial_dwh_schema_is_rejected(tursodb_path: &Path, sentinel_token: &str) {
        let server = LocalSyncServer::spawn(tursodb_path);
        prepare_remote_with_partial_dwh_schema(&server.url);

        assert_open_is_rejected_without_mutating_remote(
            &server.url,
            sentinel_token,
            "partial-dwh",
            "partial-dwh-schema",
        );

        drop(server);
    }

    /// Covers T06: a remote whose migration ledger contains an unexpected or
    /// unknown migration ID is rejected as incompatible, even though every
    /// DWH contract table is present.
    fn assert_unexpected_ledger_entry_is_rejected(tursodb_path: &Path, sentinel_token: &str) {
        let server = LocalSyncServer::spawn(tursodb_path);
        prepare_remote_with_unexpected_ledger_entry(&server.url);

        assert_open_is_rejected_without_mutating_remote(
            &server.url,
            sentinel_token,
            "unexpected-ledger",
            "unexpected-ledger",
        );

        drop(server);
    }

    /// Covers T07: two distinct local replicas racing to `open()` against the
    /// same truly empty remote both converge safely — either both succeed
    /// (the loser recovering through T05's one-`pull()`-and-re-verify path in
    /// `initialize_empty_schema`) — and the remote ends up with exactly one
    /// valid, non-duplicated DWH schema/ledger, which a third fresh replica
    /// then observes as `Ready`.
    fn assert_concurrent_first_initializers_converge(tursodb_path: &Path, sentinel_token: &str) {
        // Racing more than two initializers, released simultaneously by a
        // `Barrier`, raises the odds that at least one push actually
        // collides with another on the shared remote (rather than the two
        // threads happening to interleave without ever overlapping their
        // network round trips), so this concretely exercises
        // `initialize_empty_schema`'s one-`pull()`-and-re-verify recovery
        // path against real observed Turso Sync behavior instead of relying
        // on it remaining untested whenever no collision occurs.
        const RACER_COUNT: usize = 6;

        use std::sync::Barrier;

        let server = LocalSyncServer::spawn(tursodb_path);
        let expected_migration_count = AgentTraceDwhDbSpec::migrations().len();
        let barrier = std::sync::Arc::new(Barrier::new(RACER_COUNT));
        let paths: Vec<PathBuf> = (0..RACER_COUNT)
            .map(|index| unique_test_replica_path(&format!("concurrent-{index}")))
            .collect();

        let handles: Vec<_> = paths
            .iter()
            .map(|path| {
                let local_path = path.clone();
                let database_url = server.url.clone();
                let auth_token = sentinel_token.to_string();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
                        local_path,
                        database_url,
                        auth_token,
                    })
                })
            })
            .collect();

        let replicas: Vec<AgentTraceDwhReplica> = handles
            .into_iter()
            .enumerate()
            .map(|(index, handle)| {
                handle
                    .join()
                    .unwrap_or_else(|_| {
                        panic!("concurrent initializer thread {index} should not panic")
                    })
                    .unwrap_or_else(|error| {
                        panic!(
                            "concurrent initializer {index} should either win the race or \
                             recover through the one-pull-and-re-verify path, got: {error}"
                        )
                    })
            })
            .collect();

        for (index, replica) in replicas.iter().enumerate() {
            assert_eq!(
                replica
                    .db()
                    .classify_schema_state()
                    .unwrap_or_else(|_| panic!(
                        "schema classification should succeed for initializer {index}"
                    )),
                AgentTraceDwhSchemaState::Ready,
                "concurrent initializer {index} should observe a Ready schema after converging"
            );
            assert_eq!(
                applied_migration_count(replica.db()),
                expected_migration_count,
                "concurrent initializer {index}'s converged view must not duplicate any migration"
            );
        }

        drop(replicas);
        for path in &paths {
            remove_test_replica(path);
        }

        // A third, fresh replica bootstraps `Ready` against the converged
        // remote, with exactly one (not duplicated) applied-migration ledger.
        let path_z = unique_test_replica_path("concurrent-z");
        let replica_z = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_z.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("a third fresh replica should bootstrap Ready against the converged remote");
        assert_eq!(
            replica_z
                .db()
                .classify_schema_state()
                .expect("schema classification should succeed for the third replica"),
            AgentTraceDwhSchemaState::Ready,
        );
        assert_eq!(
            applied_migration_count(replica_z.db()),
            expected_migration_count,
            "the third replica must observe exactly one converged migration ledger"
        );

        drop(replica_z);
        remove_test_replica(&path_z);
        drop(server);
    }

    /// Covers T05's unchanged-on-open case: an already-`Ready` remote opens
    /// without `open()` running any migration or performing any push merely
    /// because it was called, and the applied-migration ledger is identical
    /// (not duplicated) whether observed by the remote's own preparation
    /// connection or by a freshly opened replica.
    fn assert_ready_remote_opens_unchanged(tursodb_path: &Path, sentinel_token: &str) {
        let server = LocalSyncServer::spawn(tursodb_path);
        prepare_remote_with_ready_dwh_schema(&server.url);

        let expected_migration_count = AgentTraceDwhDbSpec::migrations().len();

        let path = unique_test_replica_path("already-ready");
        let replica = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("opening an already-ready remote should succeed without initializing anything");
        assert_eq!(
            replica
                .db()
                .classify_schema_state()
                .expect("schema classification should succeed"),
            AgentTraceDwhSchemaState::Ready,
        );
        assert_eq!(
            applied_migration_count(replica.db()),
            expected_migration_count,
            "opening an already-ready remote must not duplicate or rerun any migration"
        );
        drop(replica);
        remove_test_replica(&path);

        // A second independent replica against the same already-ready remote
        // must observe the exact same, unchanged migration ledger.
        let path_again = unique_test_replica_path("already-ready-again");
        let replica_again = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_again.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("reopening the same already-ready remote should succeed unchanged");
        assert_eq!(
            applied_migration_count(replica_again.db()),
            expected_migration_count,
            "a second open against the same already-ready remote must not duplicate any migration"
        );

        drop(replica_again);
        remove_test_replica(&path_again);
        drop(server);
    }

    fn applied_migration_count(db: &AgentTraceDwhDb) -> usize {
        db.query_map("SELECT id FROM __sce_migrations", (), |row| {
            row.get::<String>(0).map_err(Into::into)
        })
        .expect("applied-migration query should succeed")
        .len()
    }

    #[test]
    fn agent_trace_dwh_replica_turso_sync_integration() {
        let Some(tursodb_path) = find_tursodb() else {
            println!(
                "SKIPPING agent_trace_dwh_replica_turso_sync_integration: no `tursodb` binary \
                 on PATH. Run `nix develop .#database -c ./scripts/run-cli-cargo.sh test \
                 --manifest-path cli/Cargo.toml agent_trace_dwh_replica` to exercise the real \
                 Turso Sync harness against the pinned local `tursodb --sync-server`."
            );
            return;
        };
        println!("Using the pinned local `tursodb --sync-server` at {tursodb_path:?} as the disposable prepared DWH remote.");

        let repository_id = "repo-integration";
        let sentinel_token = "sentinel-integration-auth-token-must-not-leak";

        assert_bootstrap_lock_and_pull_push(&tursodb_path, repository_id, sentinel_token);
        assert_ready_remote_opens_unchanged(&tursodb_path, sentinel_token);
        assert_unrelated_schema_is_rejected(&tursodb_path, sentinel_token);
        assert_partial_dwh_schema_is_rejected(&tursodb_path, sentinel_token);
        assert_unexpected_ledger_entry_is_rejected(&tursodb_path, sentinel_token);
        assert_concurrent_first_initializers_converge(&tursodb_path, sentinel_token);
    }
}
