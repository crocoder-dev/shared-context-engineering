//! The single orchestration boundary composing an `AgentTraceDwhReplica` with
//! the three independent ETL bridges into one sync call.
//!
//! `AgentTraceDwhSync::run()` owns exactly one sequence — open the replica,
//! pull once, run `AgentTraceEtl`, `ConversationEtl`, and `CodeChangesEtl` in
//! that order through their existing `run(repository_id, source, &replica)`
//! APIs unmodified, then push once on full success — behind one bridge-lock-
//! held Turso Sync connection. It extends nothing in `agent_trace_dwh_replica`
//! or any of the three ETL modules, and it deliberately does not resolve
//! credentials, discover paths, or wrap the whole sequence in a global
//! transaction: each ETL still commits its own watermark independently, and a
//! failed push leaves those commits durable in the local replica.

use std::fmt;

use crate::services::{
    agent_trace_db::repository::RepositoryAgentTraceDb,
    agent_trace_dwh_replica::{
        AgentTraceDwhReplica, AgentTraceDwhReplicaConfig, AgentTraceDwhReplicaError,
    },
    agent_trace_etl::{AgentTraceEtl, AgentTraceEtlStats},
    code_changes_etl::{CodeChangesEtl, CodeChangesEtlStats},
    conversation_etl::{ConversationEtl, ConversationEtlStats},
};

/// One combined sync run: `open` → `pull` → `AgentTraceEtl` →
/// `ConversationEtl` → `CodeChangesEtl` → `push`.
///
/// Configuration reuses each ETL's own defaults/batch sizing; this type adds
/// no configuration of its own beyond composing the three runners.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct AgentTraceDwhSync {
    agent_trace_etl: AgentTraceEtl,
    conversation_etl: ConversationEtl,
    code_changes_etl: CodeChangesEtl,
}

impl AgentTraceDwhSync {
    /// Open the replica, pull once, run the three ETLs in order, and push
    /// once on full success.
    ///
    /// The bridge lock stays held for the whole sequence: `replica_config` is
    /// consumed by exactly one `AgentTraceDwhReplica::open()` call, and the
    /// opened replica is dropped (releasing the lock) when this call returns,
    /// whether it succeeds or fails. On any stage failure, the sequence stops
    /// immediately and `push()` is never invoked; any ETL stage that already
    /// committed within this call remains durable in the local replica.
    ///
    /// `stats.pulled_changes` reflects Turso Sync's own `pull()` semantics:
    /// because each call opens a fresh replica connection, the pull that
    /// follows any prior session's successful `push()` (including this
    /// orchestrator's own immediately preceding `run()`) observes that push
    /// as unreconciled and reports `true`, even though the pulled data
    /// already matches what is on disk. It settles to `false` only once a
    /// `run()` observes no push from any source since the previous `run()`'s
    /// own pull.
    pub fn run(
        &self,
        repository_id: &str,
        source: &RepositoryAgentTraceDb,
        replica_config: AgentTraceDwhReplicaConfig,
    ) -> Result<AgentTraceDwhSyncStats, AgentTraceDwhSyncError> {
        let replica = AgentTraceDwhReplica::open(replica_config)
            .map_err(AgentTraceDwhSyncError::ReplicaOpen)?;

        let pulled_changes = replica.pull().map_err(AgentTraceDwhSyncError::Pull)?;

        let agent_traces = self
            .agent_trace_etl
            .run(repository_id, source, &replica)
            .map_err(AgentTraceDwhSyncError::AgentTraceEtl)?;

        let conversation = self
            .conversation_etl
            .run(repository_id, source, &replica)
            .map_err(AgentTraceDwhSyncError::ConversationEtl)?;

        let code_changes = self
            .code_changes_etl
            .run(repository_id, source, &replica)
            .map_err(AgentTraceDwhSyncError::CodeChangesEtl)?;

        replica.push().map_err(AgentTraceDwhSyncError::Push)?;

        Ok(AgentTraceDwhSyncStats {
            pulled_changes,
            agent_traces,
            conversation,
            code_changes,
        })
    }
}

/// Combined stats for one complete `AgentTraceDwhSync::run()` call.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentTraceDwhSyncStats {
    /// Whether `pull()` applied any remote changes to the local replica.
    pub pulled_changes: bool,
    pub agent_traces: AgentTraceEtlStats,
    pub conversation: ConversationEtlStats,
    pub code_changes: CodeChangesEtlStats,
}

/// A stage-tagged failure from one `AgentTraceDwhSync::run()` call.
///
/// Every variant identifies exactly which stage of the sequence failed, so a
/// caller can tell that no stage after it ran. `ReplicaOpen`/`Pull`/`Push`
/// wrap [`AgentTraceDwhReplicaError`], which already redacts the caller's
/// auth token; the three ETL stages wrap `anyhow::Error`, which never
/// observes the token in the first place.
#[derive(Debug)]
pub enum AgentTraceDwhSyncError {
    /// Opening the replica (including remote bootstrap and schema
    /// classification) failed.
    ReplicaOpen(AgentTraceDwhReplicaError),
    /// Pulling remote changes into the replica failed.
    Pull(AgentTraceDwhReplicaError),
    /// The `AgentTraceEtl` stage failed.
    AgentTraceEtl(anyhow::Error),
    /// The `ConversationEtl` stage failed.
    ConversationEtl(anyhow::Error),
    /// The `CodeChangesEtl` stage failed.
    CodeChangesEtl(anyhow::Error),
    /// Pushing local changes to the remote failed after all three ETLs
    /// committed locally.
    Push(AgentTraceDwhReplicaError),
}

impl fmt::Display for AgentTraceDwhSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReplicaOpen(source) => {
                write!(f, "agent trace DWH sync replica open failed: {source}")
            }
            Self::Pull(source) => write!(f, "agent trace DWH sync pull failed: {source}"),
            Self::AgentTraceEtl(source) => {
                write!(
                    f,
                    "agent trace DWH sync agent trace ETL stage failed: {source}"
                )
            }
            Self::ConversationEtl(source) => {
                write!(
                    f,
                    "agent trace DWH sync conversation ETL stage failed: {source}"
                )
            }
            Self::CodeChangesEtl(source) => {
                write!(
                    f,
                    "agent trace DWH sync code changes ETL stage failed: {source}"
                )
            }
            Self::Push(source) => write!(f, "agent trace DWH sync push failed: {source}"),
        }
    }
}

impl std::error::Error for AgentTraceDwhSyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReplicaOpen(source) | Self::Pull(source) | Self::Push(source) => Some(source),
            Self::AgentTraceEtl(source)
            | Self::ConversationEtl(source)
            | Self::CodeChangesEtl(source) => Some(source.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_composes_default_etl_configuration() {
        assert_eq!(
            AgentTraceDwhSync::default(),
            AgentTraceDwhSync {
                agent_trace_etl: AgentTraceEtl::default(),
                conversation_etl: ConversationEtl::default(),
                code_changes_etl: CodeChangesEtl::default(),
            }
        );
    }

    #[test]
    fn stats_default_to_zeroed_stage_stats_and_no_pulled_changes() {
        let stats = AgentTraceDwhSyncStats::default();
        assert!(!stats.pulled_changes);
        assert_eq!(stats.agent_traces, AgentTraceEtlStats::default());
        assert_eq!(stats.conversation, ConversationEtlStats::default());
        assert_eq!(stats.code_changes, CodeChangesEtlStats::default());
    }

    #[test]
    fn each_error_stage_display_names_its_stage_and_never_contains_the_sentinel_token() {
        let sentinel_token = "sentinel-must-never-leak-token";
        let replica_error = || AgentTraceDwhReplicaError::Pull {
            message: format!("boom containing {sentinel_token}")
                .replace(sentinel_token, "<redacted>"),
        };
        let etl_error = || anyhow::anyhow!("boom, no token involved");

        let cases: Vec<(AgentTraceDwhSyncError, &str)> = vec![
            (
                AgentTraceDwhSyncError::ReplicaOpen(replica_error()),
                "replica open",
            ),
            (AgentTraceDwhSyncError::Pull(replica_error()), "pull"),
            (
                AgentTraceDwhSyncError::AgentTraceEtl(etl_error()),
                "agent trace ETL",
            ),
            (
                AgentTraceDwhSyncError::ConversationEtl(etl_error()),
                "conversation ETL",
            ),
            (
                AgentTraceDwhSyncError::CodeChangesEtl(etl_error()),
                "code changes ETL",
            ),
            (AgentTraceDwhSyncError::Push(replica_error()), "push"),
        ];

        for (error, expected_fragment) in cases {
            let display = error.to_string();
            let debug = format!("{error:?}");
            assert!(
                display.contains(expected_fragment),
                "expected {display:?} to name stage {expected_fragment:?}"
            );
            assert!(
                !display.contains(sentinel_token),
                "Display output must never contain the auth token: {display}"
            );
            assert!(
                !debug.contains(sentinel_token),
                "Debug output must never contain the auth token: {debug}"
            );
        }
    }
}

/// Turso Sync integration harness proving AC1/AC2 against a real disposable
/// remote: a fresh empty-remote sync bootstraps the schema, runs all three
/// ETLs, and pushes once; a second run against the same source/remote is a
/// visible no-op. Runs only when a `tursodb` binary supporting
/// `--sync-server` is discoverable on `PATH`, matching the
/// `agent_trace_dwh_replica` integration harness convention exactly.
#[cfg(test)]
mod integration_tests {
    use std::{
        net::TcpStream,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::agent_trace_db::{
        AgentTraceInsert, DiffTraceInsert, InsertMessageInsert, InsertPartInsert, MessageRole,
        PartType, PAYLOAD_TYPE_PATCH,
    };
    use crate::services::agent_trace_dwh_db::AgentTraceDwhDb;

    fn unique_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-agent-trace-dwh-sync-integration-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create unique test directory");
        dir
    }

    fn clean(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

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

    fn valid_patch(path: &str, added: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n+{added}\n"
        )
    }

    /// Seed the source `RepositoryAgentTraceDb` with one row for each of the
    /// four tables the three ETLs extract from, so a first `run()` produces
    /// non-zero `inserted` counts across every stage.
    fn seed_source(source: &RepositoryAgentTraceDb) {
        source
            .insert_agent_trace(AgentTraceInsert {
                commit_id: "commit-1",
                commit_time_ms: 1_000,
                trace_json: r#"{"id":"trace-1"}"#,
                agent_trace_id: "trace-1",
                url: "https://sce.crocoder.dev/agent-trace/trace-1",
                remote_url: "https://github.com/acme/widgets",
            })
            .expect("agent trace insert should succeed");

        source
            .insert_message(InsertMessageInsert {
                session_id: String::from("session-1"),
                message_id: String::from("message-1"),
                role: MessageRole::User,
                generated_at_unix_ms: 1_000,
            })
            .expect("message insert should succeed");

        source
            .insert_part(InsertPartInsert {
                part_type: PartType::Text,
                text: String::from("hello"),
                session_id: String::from("session-1"),
                message_id: String::from("message-1"),
                generated_at_unix_ms: 1_000,
            })
            .expect("part insert should succeed");

        source
            .insert_diff_trace(DiffTraceInsert {
                time_ms: 1_000,
                session_id: "session-1",
                patch: &valid_patch("file-1.rs", "added"),
                model_id: Some("provider/model"),
                tool_name: "opencode",
                tool_version: Some("1.2.3"),
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("diff trace insert should succeed");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn agent_trace_dwh_sync_turso_sync_integration() {
        let Some(tursodb_path) = find_tursodb() else {
            println!(
                "SKIPPING agent_trace_dwh_sync_turso_sync_integration: no `tursodb` binary on \
                 PATH. Run `nix develop .#database -c ./scripts/run-cli-cargo.sh test \
                 --manifest-path cli/Cargo.toml agent_trace_dwh_sync` to exercise the real \
                 Turso Sync harness against the pinned local `tursodb --sync-server`."
            );
            return;
        };

        let repository_id = "repo-dwh-sync";
        let sentinel_token = "sentinel-dwh-sync-integration-auth-token-must-not-leak";

        let server = LocalSyncServer::spawn(&tursodb_path);

        let source_dir = unique_path("source");
        let source = RepositoryAgentTraceDb::new_at(source_dir.join("agent-trace.db"))
            .expect("source DB should open");
        seed_source(&source);

        let replica_dir = unique_path("replica");
        let replica_path = replica_dir.join("agent-trace-sync.db");

        let sync = AgentTraceDwhSync::default();

        let first = sync
            .run(
                repository_id,
                &source,
                AgentTraceDwhReplicaConfig {
                    local_path: replica_path.clone(),
                    database_url: server.url.clone(),
                    auth_token: sentinel_token.to_string(),
                },
            )
            .expect("first sync against a truly empty remote should succeed");

        assert!(
            first.agent_traces.inserted > 0,
            "first sync should insert agent trace rows: {first:?}"
        );
        assert!(
            first.conversation.messages.inserted > 0,
            "first sync should insert message rows: {first:?}"
        );
        assert!(
            first.conversation.parts.inserted > 0,
            "first sync should insert part rows: {first:?}"
        );
        assert!(
            first.code_changes.inserted > 0,
            "first sync should insert code change rows: {first:?}"
        );
        let first_debug = format!("{first:?}");
        assert!(
            !first_debug.contains(sentinel_token),
            "sync stats must never contain the auth token: {first_debug}"
        );

        let second = sync
            .run(
                repository_id,
                &source,
                AgentTraceDwhReplicaConfig {
                    local_path: replica_path.clone(),
                    database_url: server.url.clone(),
                    auth_token: sentinel_token.to_string(),
                },
            )
            .expect("second sync with no new source rows should succeed as a visible no-op");

        // Observed real Turso Sync behavior: a freshly opened replica's first
        // `pull()` after ANY session (including this orchestrator's own
        // immediately preceding `run()`) has pushed reports `pulled_changes
        // == true`, because that push was never locally marked "already
        // observed" by this new connection — it must pull once to reconcile,
        // even though the pulled data exactly matches what is already on
        // disk. This is why `second` is not asserted here: AC2's real
        // contract is "no new source rows means no new extraction/insertion,"
        // which holds regardless of this reconciliation pull. `third` below
        // proves the `pulled_changes == false` steady state once no push has
        // happened since the previous `run()`'s own reconciliation pull.
        assert_eq!(second.agent_traces.extracted, 0);
        assert_eq!(second.agent_traces.inserted, 0);
        assert_eq!(second.conversation.messages.extracted, 0);
        assert_eq!(second.conversation.messages.inserted, 0);
        assert_eq!(second.conversation.parts.extracted, 0);
        assert_eq!(second.conversation.parts.inserted, 0);
        assert_eq!(second.code_changes.extracted, 0);
        assert_eq!(second.code_changes.inserted, 0);
        let second_debug = format!("{second:?}");
        assert!(
            !second_debug.contains(sentinel_token),
            "sync stats must never contain the auth token: {second_debug}"
        );

        let third = sync
            .run(
                repository_id,
                &source,
                AgentTraceDwhReplicaConfig {
                    local_path: replica_path.clone(),
                    database_url: server.url.clone(),
                    auth_token: sentinel_token.to_string(),
                },
            )
            .expect("third sync with no new source rows should succeed as a visible no-op");

        assert!(
            !third.pulled_changes,
            "a run with no push since the previous run's reconciliation pull should observe no \
             pulled changes: {third:?}"
        );
        assert_eq!(third.agent_traces.extracted, 0);
        assert_eq!(third.agent_traces.inserted, 0);
        assert_eq!(third.conversation.messages.extracted, 0);
        assert_eq!(third.conversation.messages.inserted, 0);
        assert_eq!(third.conversation.parts.extracted, 0);
        assert_eq!(third.conversation.parts.inserted, 0);
        assert_eq!(third.code_changes.extracted, 0);
        assert_eq!(third.code_changes.inserted, 0);
        let third_debug = format!("{third:?}");
        assert!(
            !third_debug.contains(sentinel_token),
            "sync stats must never contain the auth token: {third_debug}"
        );

        clean(&source_dir);
        clean(&replica_dir);
        drop(server);
    }

    /// Count of `agent_traces`/`messages`/`message_parts`/`code_changes` rows
    /// for `repository_id` on the DWH surface a replica exposes.
    fn count_repository_rows(db: &AgentTraceDwhDb, table: &str, repository_id: &str) -> i64 {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE repository_id = ?1");
        db.query_map(&sql, (repository_id,), |row| {
            row.get::<i64>(0).map_err(Into::into)
        })
        .expect("row count query should succeed")
        .into_iter()
        .next()
        .unwrap_or(0)
    }

    /// `[agent_traces, messages, message_parts, code_changes]` row counts for
    /// `repository_id`, in that fixed order, matching every assertion below.
    fn stage_row_counts(replica: &AgentTraceDwhReplica, repository_id: &str) -> [i64; 4] {
        [
            count_repository_rows(replica.db(), "agent_traces", repository_id),
            count_repository_rows(replica.db(), "messages", repository_id),
            count_repository_rows(replica.db(), "message_parts", repository_id),
            count_repository_rows(replica.db(), "code_changes", repository_id),
        ]
    }

    /// Open a disposable peer replica against the live remote, pull, and read
    /// its row counts. Proves what has actually been pushed, independent of
    /// whatever any other replica holds locally.
    fn remote_row_counts(
        server_url: &str,
        sentinel_token: &str,
        repository_id: &str,
        peer_label: &str,
    ) -> [i64; 4] {
        let peer_dir = unique_path(peer_label);
        let replica = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: peer_dir.join("agent-trace-sync.db"),
            database_url: server_url.to_string(),
            auth_token: sentinel_token.to_string(),
        })
        .expect("peer replica open for remote inspection should succeed");
        replica.pull().expect("peer pull should succeed");
        let counts = stage_row_counts(&replica, repository_id);
        drop(replica);
        clean(&peer_dir);
        counts
    }

    /// Reopen the same local replica path directly to inspect exactly what is
    /// durable in the local spool after a failed run.
    ///
    /// Empirically confirmed (see the observed-behavior note on
    /// `AgentTraceDwhSync::run`'s pull-failure test below): once a replica's
    /// local schema has classified `Ready`, `AgentTraceDwhReplica::open()`
    /// never needs to reach the network, so an unreachable `database_url`
    /// here is safe and does not affect what is read.
    fn local_row_counts(local_path: &Path, repository_id: &str) -> [i64; 4] {
        let replica = AgentTraceDwhReplica::open(AgentTraceDwhReplicaConfig {
            local_path: local_path.to_path_buf(),
            database_url: String::from("http://127.0.0.1:1"),
            auth_token: String::from("unused-for-local-only-inspection"),
        })
        .expect(
            "reopening an already-Ready local replica for direct spool inspection should \
             succeed without contacting the network",
        );
        let counts = stage_row_counts(&replica, repository_id);
        drop(replica);
        counts
    }

    /// Covers AC3(a): opening against an unreachable remote fails with
    /// `ReplicaOpen`, the auth token never leaks, and the live remote (never
    /// actually addressed by this call) is left with zero rows for this
    /// repository.
    fn assert_replica_open_failure_stops_before_any_stage(
        tursodb_path: &Path,
        sentinel_token: &str,
    ) {
        let server = LocalSyncServer::spawn(tursodb_path);
        let repository_id = "repo-stage-open-failure";

        let source_dir = unique_path("stage-open-source");
        let source = RepositoryAgentTraceDb::new_at(source_dir.join("agent-trace.db"))
            .expect("source DB should open");
        seed_source(&source);

        let unreachable_dir = unique_path("stage-open-replica");
        let sync = AgentTraceDwhSync::default();
        let error = sync
            .run(
                repository_id,
                &source,
                AgentTraceDwhReplicaConfig {
                    local_path: unreachable_dir.join("agent-trace-sync.db"),
                    database_url: String::from("http://127.0.0.1:0"),
                    auth_token: sentinel_token.to_string(),
                },
            )
            .expect_err("open against an unreachable remote should fail");
        assert!(
            matches!(error, AgentTraceDwhSyncError::ReplicaOpen(_)),
            "expected ReplicaOpen, got {error:?}"
        );
        assert!(!error.to_string().contains(sentinel_token));

        let after_remote = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-open-check",
        );
        assert_eq!(
            after_remote,
            [0, 0, 0, 0],
            "a replica-open failure must never reach any ETL or push"
        );

        clean(&source_dir);
        clean(&unreachable_dir);
        drop(server);
    }

    /// Covers AC3(b): once a replica is locally `Ready`, opening it again
    /// against an unreachable `database_url` still succeeds (no network
    /// access is needed to read an already-synced local schema), but the
    /// following `pull()` fails with `Pull`, no ETL runs, and the live
    /// remote — addressed only by the earlier, successful baseline run, never
    /// by the failing one — is unaffected.
    fn assert_pull_failure_stops_before_any_stage(tursodb_path: &Path, sentinel_token: &str) {
        let server = LocalSyncServer::spawn(tursodb_path);
        let repository_id = "repo-stage-pull-failure";
        let sync = AgentTraceDwhSync::default();

        let source_dir = unique_path("stage-pull-source");
        let source = RepositoryAgentTraceDb::new_at(source_dir.join("agent-trace.db"))
            .expect("source DB should open");
        seed_source(&source);

        let replica_dir = unique_path("stage-pull-replica");
        let replica_path = replica_dir.join("agent-trace-sync.db");
        sync.run(
            repository_id,
            &source,
            AgentTraceDwhReplicaConfig {
                local_path: replica_path.clone(),
                database_url: server.url.clone(),
                auth_token: sentinel_token.to_string(),
            },
        )
        .expect("baseline sync should succeed and populate the remote");

        let baseline = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-pull-baseline",
        );
        assert_eq!(baseline, [1, 1, 1, 1]);

        // A new source row that would be extracted if the sequence wrongly
        // proceeded past the pull failure, so a false no-op can't hide a
        // stop-on-failure bypass.
        source
            .insert_agent_trace(AgentTraceInsert {
                commit_id: "commit-2",
                commit_time_ms: 2_000,
                trace_json: r#"{"id":"trace-2"}"#,
                agent_trace_id: "trace-2",
                url: "https://sce.crocoder.dev/agent-trace/trace-2",
                remote_url: "https://github.com/acme/widgets",
            })
            .expect("second agent trace insert should succeed");

        let error = sync
            .run(
                repository_id,
                &source,
                AgentTraceDwhReplicaConfig {
                    local_path: replica_path.clone(),
                    // Deliberately different from `server.url`: the live
                    // remote above is never touched by this call, so any
                    // change there would prove a real stop-on-failure bypass
                    // rather than an artifact of killing the real server.
                    database_url: String::from("http://127.0.0.1:1"),
                    auth_token: sentinel_token.to_string(),
                },
            )
            .expect_err("pull against an unreachable remote should fail");
        assert!(
            matches!(error, AgentTraceDwhSyncError::Pull(_)),
            "expected Pull, got {error:?}"
        );
        assert!(!error.to_string().contains(sentinel_token));

        let after_local = local_row_counts(&replica_path, repository_id);
        assert_eq!(
            after_local,
            [1, 1, 1, 1],
            "no ETL should run past a pull failure; the new source row must not appear locally"
        );

        let after_remote = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-pull-after",
        );
        assert_eq!(
            after_remote, baseline,
            "the live remote must be unaffected by a run that failed at pull using a different, \
             unreachable url"
        );

        clean(&source_dir);
        clean(&replica_dir);
        drop(server);
    }

    /// Covers AC3(c): a same-repository, cross-source `agent_trace_id`
    /// collision with a different `trace_json` fails `AgentTraceEtl` with an
    /// identity-integrity conflict. `AgentTraceEtl` is both the first stage
    /// and internally atomic per batch, so the failing run commits nothing
    /// locally, and push never runs.
    #[allow(clippy::too_many_lines)]
    fn assert_agent_trace_etl_failure_stops_before_conversation_code_changes_and_push(
        tursodb_path: &Path,
        sentinel_token: &str,
    ) {
        let server = LocalSyncServer::spawn(tursodb_path);
        let repository_id = "repo-stage-agent-trace-failure";
        let sync = AgentTraceDwhSync::default();

        let first_writer_source_dir = unique_path("stage-agent-trace-source-a");
        let first_writer_source =
            RepositoryAgentTraceDb::new_at(first_writer_source_dir.join("agent-trace.db"))
                .expect("source A DB should open");
        first_writer_source
            .insert_agent_trace(AgentTraceInsert {
                commit_id: "commit-a",
                commit_time_ms: 1_000,
                trace_json: r#"{"id":"conflict","variant":"a"}"#,
                agent_trace_id: "trace-conflict",
                url: "https://sce.crocoder.dev/agent-trace/trace-conflict",
                remote_url: "https://github.com/acme/widgets",
            })
            .expect("source A agent trace insert should succeed");

        let first_writer_replica_dir = unique_path("stage-agent-trace-replica-a");
        sync.run(
            repository_id,
            &first_writer_source,
            AgentTraceDwhReplicaConfig {
                local_path: first_writer_replica_dir.join("agent-trace-sync.db"),
                database_url: server.url.clone(),
                auth_token: sentinel_token.to_string(),
            },
        )
        .expect("source A's sync should succeed and publish trace-conflict");

        let baseline = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-agent-trace-baseline",
        );
        assert_eq!(baseline, [1, 0, 0, 0]);

        let conflicting_writer_source_dir = unique_path("stage-agent-trace-source-b");
        let conflicting_writer_source =
            RepositoryAgentTraceDb::new_at(conflicting_writer_source_dir.join("agent-trace.db"))
                .expect("source B DB should open");
        conflicting_writer_source
            .insert_agent_trace(AgentTraceInsert {
                commit_id: "commit-b",
                commit_time_ms: 2_000,
                trace_json: r#"{"id":"conflict","variant":"b"}"#,
                agent_trace_id: "trace-conflict",
                url: "https://sce.crocoder.dev/agent-trace/trace-conflict",
                remote_url: "https://github.com/acme/widgets",
            })
            .expect("source B agent trace insert should succeed");
        // Later-stage source rows that would be extracted if the sequence
        // wrongly proceeded past the AgentTraceEtl failure.
        conflicting_writer_source
            .insert_message(InsertMessageInsert {
                session_id: String::from("session-b"),
                message_id: String::from("message-b"),
                role: MessageRole::User,
                generated_at_unix_ms: 2_000,
            })
            .expect("source B message insert should succeed");
        conflicting_writer_source
            .insert_part(InsertPartInsert {
                part_type: PartType::Text,
                text: String::from("hello from b"),
                session_id: String::from("session-b"),
                message_id: String::from("message-b"),
                generated_at_unix_ms: 2_000,
            })
            .expect("source B part insert should succeed");
        conflicting_writer_source
            .insert_diff_trace(DiffTraceInsert {
                time_ms: 2_000,
                session_id: "session-b",
                patch: &valid_patch("file-b.rs", "added-b"),
                model_id: Some("provider/model"),
                tool_name: "opencode",
                tool_version: Some("1.2.3"),
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("source B diff trace insert should succeed");

        let conflicting_writer_replica_dir = unique_path("stage-agent-trace-replica-b");
        let conflicting_writer_replica_path =
            conflicting_writer_replica_dir.join("agent-trace-sync.db");
        let error = sync
            .run(
                repository_id,
                &conflicting_writer_source,
                AgentTraceDwhReplicaConfig {
                    local_path: conflicting_writer_replica_path.clone(),
                    database_url: server.url.clone(),
                    auth_token: sentinel_token.to_string(),
                },
            )
            .expect_err(
                "a conflicting agent_trace_id with a different trace_json should fail the \
                 AgentTraceEtl stage",
            );
        assert!(
            matches!(error, AgentTraceDwhSyncError::AgentTraceEtl(_)),
            "expected AgentTraceEtl, got {error:?}"
        );
        assert!(!error.to_string().contains(sentinel_token));

        let after_local = local_row_counts(&conflicting_writer_replica_path, repository_id);
        assert_eq!(
            after_local, baseline,
            "AgentTraceEtl's atomic per-batch failure must leave the local replica exactly as \
             pull() left it (source A's already-published row, pulled before the conflict was \
             detected), with no partial facts from source B and nothing from later stages"
        );

        let after_remote = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-agent-trace-after",
        );
        assert_eq!(
            after_remote, baseline,
            "a failed first-stage run must never push"
        );

        clean(&first_writer_source_dir);
        clean(&first_writer_replica_dir);
        clean(&conflicting_writer_source_dir);
        clean(&conflicting_writer_replica_dir);
        drop(server);
    }

    /// Covers AC3(d): an invalid `parts.type` value (no source-side CHECK
    /// constraint permits inserting it through raw SQL) fails the parts half
    /// of `ConversationEtl`. The messages half commits independently within
    /// the same `ConversationEtl::run()` call, and `AgentTraceEtl` (run
    /// before it) commits too; `CodeChangesEtl` never runs and push never
    /// runs.
    fn assert_conversation_etl_failure_leaves_agent_trace_committed_and_stops_before_code_changes_and_push(
        tursodb_path: &Path,
        sentinel_token: &str,
    ) {
        let server = LocalSyncServer::spawn(tursodb_path);
        let repository_id = "repo-stage-conversation-failure";
        let sync = AgentTraceDwhSync::default();

        let source_dir = unique_path("stage-conversation-source");
        let source = RepositoryAgentTraceDb::new_at(source_dir.join("agent-trace.db"))
            .expect("source DB should open");
        seed_source(&source);

        let replica_dir = unique_path("stage-conversation-replica");
        let replica_path = replica_dir.join("agent-trace-sync.db");
        sync.run(
            repository_id,
            &source,
            AgentTraceDwhReplicaConfig {
                local_path: replica_path.clone(),
                database_url: server.url.clone(),
                auth_token: sentinel_token.to_string(),
            },
        )
        .expect("baseline sync should succeed");

        let baseline = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-conversation-baseline",
        );
        assert_eq!(baseline, [1, 1, 1, 1]);

        source
            .insert_agent_trace(AgentTraceInsert {
                commit_id: "commit-2",
                commit_time_ms: 2_000,
                trace_json: r#"{"id":"trace-2"}"#,
                agent_trace_id: "trace-2",
                url: "https://sce.crocoder.dev/agent-trace/trace-2",
                remote_url: "https://github.com/acme/widgets",
            })
            .expect("second agent trace insert should succeed");
        source
            .insert_message(InsertMessageInsert {
                session_id: String::from("session-1"),
                message_id: String::from("message-2"),
                role: MessageRole::Assistant,
                generated_at_unix_ms: 2_000,
            })
            .expect("second message insert should succeed");
        source
            .execute(
                "INSERT INTO parts (type, text, message_id, session_id, generated_at_unix_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    "bogus-type",
                    "invalid part",
                    "message-2",
                    "session-1",
                    2_000_i64,
                ),
            )
            .expect("raw invalid parts insert should succeed: parts.type has no CHECK constraint");
        // A later-stage source row that would be extracted if the sequence
        // wrongly proceeded past the ConversationEtl failure.
        source
            .insert_diff_trace(DiffTraceInsert {
                time_ms: 2_000,
                session_id: "session-1",
                patch: &valid_patch("file-2.rs", "added-2"),
                model_id: Some("provider/model"),
                tool_name: "opencode",
                tool_version: Some("1.2.3"),
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("second diff trace insert should succeed");

        let error = sync
            .run(
                repository_id,
                &source,
                AgentTraceDwhReplicaConfig {
                    local_path: replica_path.clone(),
                    database_url: server.url.clone(),
                    auth_token: sentinel_token.to_string(),
                },
            )
            .expect_err("an invalid parts.type value should fail the ConversationEtl stage");
        assert!(
            matches!(error, AgentTraceDwhSyncError::ConversationEtl(_)),
            "expected ConversationEtl, got {error:?}"
        );
        assert!(!error.to_string().contains(sentinel_token));

        let after_local = local_row_counts(&replica_path, repository_id);
        assert_eq!(
            after_local,
            [2, 2, 1, 1],
            "AgentTraceEtl and the messages half of ConversationEtl should commit locally even \
             though parts failed and CodeChangesEtl never ran"
        );

        let after_remote = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-conversation-after",
        );
        assert_eq!(
            after_remote, baseline,
            "a ConversationEtl failure must never push"
        );

        clean(&source_dir);
        clean(&replica_dir);
        drop(server);
    }

    /// Covers AC3(e): a malformed `diff_traces.patch` payload fails
    /// `CodeChangesEtl`'s strict transformation. Both prior ETLs commit
    /// locally within the same run, and push never runs.
    #[allow(clippy::too_many_lines)]
    fn assert_code_changes_etl_failure_leaves_prior_etls_committed_and_stops_before_push(
        tursodb_path: &Path,
        sentinel_token: &str,
    ) {
        let server = LocalSyncServer::spawn(tursodb_path);
        let repository_id = "repo-stage-code-changes-failure";
        let sync = AgentTraceDwhSync::default();

        let source_dir = unique_path("stage-code-changes-source");
        let source = RepositoryAgentTraceDb::new_at(source_dir.join("agent-trace.db"))
            .expect("source DB should open");
        seed_source(&source);

        let replica_dir = unique_path("stage-code-changes-replica");
        let replica_path = replica_dir.join("agent-trace-sync.db");
        sync.run(
            repository_id,
            &source,
            AgentTraceDwhReplicaConfig {
                local_path: replica_path.clone(),
                database_url: server.url.clone(),
                auth_token: sentinel_token.to_string(),
            },
        )
        .expect("baseline sync should succeed");

        let baseline = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-code-changes-baseline",
        );
        assert_eq!(baseline, [1, 1, 1, 1]);

        source
            .insert_agent_trace(AgentTraceInsert {
                commit_id: "commit-2",
                commit_time_ms: 2_000,
                trace_json: r#"{"id":"trace-2"}"#,
                agent_trace_id: "trace-2",
                url: "https://sce.crocoder.dev/agent-trace/trace-2",
                remote_url: "https://github.com/acme/widgets",
            })
            .expect("second agent trace insert should succeed");
        source
            .insert_message(InsertMessageInsert {
                session_id: String::from("session-1"),
                message_id: String::from("message-2"),
                role: MessageRole::Assistant,
                generated_at_unix_ms: 2_000,
            })
            .expect("second message insert should succeed");
        source
            .insert_part(InsertPartInsert {
                part_type: PartType::Text,
                text: String::from("second part"),
                session_id: String::from("session-1"),
                message_id: String::from("message-2"),
                generated_at_unix_ms: 2_000,
            })
            .expect("second part insert should succeed");
        source
            .insert_diff_trace(DiffTraceInsert {
                time_ms: 2_000,
                session_id: "session-1",
                patch: "Index: notes/malformed.md\n===================================================================\n--- notes/malformed.md\n+++ notes/malformed.md\n@@ malformed @@\n+bad\n",
                model_id: None,
                tool_name: "opencode",
                tool_version: None,
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect(
                "malformed diff trace insert should succeed at the source layer; strict \
                 validation happens during ETL transformation",
            );

        let error = sync
            .run(
                repository_id,
                &source,
                AgentTraceDwhReplicaConfig {
                    local_path: replica_path.clone(),
                    database_url: server.url.clone(),
                    auth_token: sentinel_token.to_string(),
                },
            )
            .expect_err(
                "a malformed diff_traces.patch payload should fail the CodeChangesEtl stage",
            );
        assert!(
            matches!(error, AgentTraceDwhSyncError::CodeChangesEtl(_)),
            "expected CodeChangesEtl, got {error:?}"
        );
        assert!(!error.to_string().contains(sentinel_token));

        let after_local = local_row_counts(&replica_path, repository_id);
        assert_eq!(
            after_local,
            [2, 2, 2, 1],
            "AgentTraceEtl and ConversationEtl should commit locally even though CodeChangesEtl \
             failed and push never ran"
        );

        let after_remote = remote_row_counts(
            &server.url,
            sentinel_token,
            repository_id,
            "stage-code-changes-after",
        );
        assert_eq!(
            after_remote, baseline,
            "a CodeChangesEtl failure must never push"
        );

        clean(&source_dir);
        clean(&replica_dir);
        drop(server);
    }

    /// Proves AC3: each of the five sequence stages, when it fails, is
    /// identifiable through a distinct `AgentTraceDwhSyncError` stage
    /// variant, and none of them ever reaches `push()`.
    #[test]
    fn agent_trace_dwh_sync_stage_failure_turso_sync_integration() {
        let Some(tursodb_path) = find_tursodb() else {
            println!(
                "SKIPPING agent_trace_dwh_sync_stage_failure_turso_sync_integration: no \
                 `tursodb` binary on PATH. Run `nix develop .#database -c \
                 ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml \
                 agent_trace_dwh_sync` to exercise the real Turso Sync harness against the \
                 pinned local `tursodb --sync-server`."
            );
            return;
        };

        let sentinel_token = "sentinel-dwh-sync-stage-failure-token-must-not-leak";

        assert_replica_open_failure_stops_before_any_stage(&tursodb_path, sentinel_token);
        assert_pull_failure_stops_before_any_stage(&tursodb_path, sentinel_token);
        assert_agent_trace_etl_failure_stops_before_conversation_code_changes_and_push(
            &tursodb_path,
            sentinel_token,
        );
        assert_conversation_etl_failure_leaves_agent_trace_committed_and_stops_before_code_changes_and_push(
            &tursodb_path,
            sentinel_token,
        );
        assert_code_changes_etl_failure_leaves_prior_etls_committed_and_stops_before_push(
            &tursodb_path,
            sentinel_token,
        );
    }
}
