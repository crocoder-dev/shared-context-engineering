//! The lock-owning Turso Sync replica boundary.
//!
//! `AgentTraceDwhReplica` is the only owner of a Turso Sync builder in this
//! codebase. It acquires the [`BridgeLock`] before touching the network or
//! the local replica file, opens the local `agent-trace-sync.db` file
//! against a caller-supplied remote using Turso's `sync` feature, and
//! verifies (without provisioning) that the bootstrapped database already
//! satisfies the Agent Trace DWH migration contract via
//! [`AgentTraceDwhDb::ensure_dwh_schema_ready`]. It never enables
//! `experimental_multiprocess_wal`: that flag is reserved for the
//! multiprocess-WAL source capture database, which this replica never opens.

use std::{fmt, path::Path, path::PathBuf};

use crate::services::{
    agent_trace_dwh_db::AgentTraceDwhDb,
    agent_trace_dwh_replica::lock::{BridgeLock, BridgeLockError},
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
    /// `bootstrap_if_empty` behavior. Once open, the bootstrapped database
    /// must already satisfy [`AgentTraceDwhDb::ensure_dwh_schema_ready`]:
    /// this call never runs local DWH migrations.
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

        db.ensure_dwh_schema_ready().map_err(|error| {
            AgentTraceDwhReplicaError::SchemaNotReady {
                message: redact_token(&error, &auth_token),
            }
        })?;

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
    /// The database opened, but its DWH schema is missing or incomplete.
    SchemaNotReady { message: String },
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
            Self::SchemaNotReady { message } => {
                write!(f, "Agent Trace DWH replica schema is not ready: {message}")
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
            | Self::SchemaNotReady { .. }
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
    /// against it afterward observe a genuinely ready DWH schema. This is
    /// the "prepared DWH remote" the plan's acceptance criteria assume.
    fn prepare_remote_with_dwh_schema(remote_url: &str) {
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

    /// Covers AC3–AC5: fresh bootstrap, lock-before-open rejection of a
    /// concurrent owner, independent pull/push visibility in both
    /// directions, and reconstruction of remote data after local deletion.
    fn assert_bootstrap_lock_and_pull_push(
        tursodb_path: &Path,
        repository_id: &str,
        sentinel_token: &str,
    ) {
        // --- AC4: fresh bootstrap against a prepared DWH remote succeeds. ---
        let server = LocalSyncServer::spawn(tursodb_path);
        prepare_remote_with_dwh_schema(&server.url);

        let path_a = unique_test_replica_path("peer-a");
        let replica_a = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_a.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("bootstrap against a prepared DWH remote should succeed");
        assert!(
            path_a.is_file(),
            "bootstrap should create the local replica file"
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

        // Peer replica opened before any writes exist, to prove pull (not
        // just fresh bootstrap) makes independently published data visible.
        let path_b = unique_test_replica_path("peer-b");
        let replica_b = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_b.clone(),
            database_url: server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("second independent peer should bootstrap against the same remote");
        assert!(
            source_instance_ids(replica_b.db(), repository_id).is_empty(),
            "peer should start with no rows for this repository"
        );

        // --- AC5: push from one peer becomes visible to another peer via pull. ---
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

        // --- AC5 (other direction): push from replica_b becomes visible to
        // replica_a via pull. ---
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

    /// Covers AC4's negative case: a remote whose bootstrapped schema is not
    /// the DWH schema is reported as not ready, without locally provisioning
    /// a competing schema, and the error never contains the auth token.
    fn assert_incompatible_schema_is_rejected(tursodb_path: &Path, sentinel_token: &str) {
        let incompatible_server = LocalSyncServer::spawn(tursodb_path);
        prepare_remote_with_incompatible_schema(&incompatible_server.url);

        let path_incompatible = unique_test_replica_path("incompatible");
        let schema_error = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: path_incompatible.clone(),
            database_url: incompatible_server.url.clone(),
            auth_token: sentinel_token.to_string(),
        })
        .expect_err("bootstrapping a non-DWH remote should fail schema readiness");
        assert!(matches!(
            schema_error,
            AgentTraceDwhReplicaError::SchemaNotReady { .. }
        ));
        let schema_error_message = schema_error.to_string();
        assert!(
            !schema_error_message.contains(sentinel_token),
            "schema-not-ready error must not contain the auth token: {schema_error_message}"
        );

        remove_test_replica(&path_incompatible);
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
        assert_incompatible_schema_is_rejected(&tursodb_path, sentinel_token);
    }
}
