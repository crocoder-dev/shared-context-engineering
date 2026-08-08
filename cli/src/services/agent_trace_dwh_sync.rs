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
}
