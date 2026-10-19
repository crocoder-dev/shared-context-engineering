use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::{AuthDb, AuthDbSpec};
use crate::generated_migrations;
use crate::services::db::encryption_key::AUTH_DB_ENCRYPTION_KEY_ENV;
use crate::services::db::{DatabaseKind, DbSpec, EncryptedTursoDb};
use crate::services::default_paths::auth_db_path;
use crate::services::managed_telemetry::trace_export::tests::{
    capture_requests_until, loopback_hook_telemetry, trace_spans,
};
use crate::services::managed_telemetry::trace_export::{
    record_claude_hook_payload, ClaudeHookCommand, ManagedSpanKind,
};
use crate::services::managed_telemetry::{
    load_credential_within_deadline, select_mode, ManagedTelemetryMode, RuntimeTelemetry,
    TelemetryFallbackReason,
};
use crate::services::observability::traits::Telemetry;
use crate::services::telemetry_credential_storage::{
    load_telemetry_credential, save_telemetry_credential, test_access_token, TelemetryCredential,
    TelemetryIdentity,
};

const CHILD_TEST: &str = "services::auth_db::multiprocess_tests::auth_db_child_process";
const ROLE_ENV: &str = "SCE_AUTH_DB_TEST_CHILD_ROLE";
const COLLECTOR_ENV: &str = "SCE_AUTH_DB_TEST_COLLECTOR";
const HOOK_INDEX_ENV: &str = "SCE_AUTH_DB_TEST_HOOK_INDEX";
const TEST_SECRET: &str = "auth-db-multiprocess-test-secret";
const WRONG_SECRET: &str = "auth-db-multiprocess-wrong-secret";
const TOKEN: &str = "tok-multiprocess";
const USER_ID: &str = "user_mp";
const WORKSPACE_ID: &str = "workspace_mp";
const READY: &str = "SCE_AUTH_DB_CHILD_READY";
const TOKEN_PREFIX: &str = "SCE_AUTH_DB_CHILD_TOKEN=";
const CONCURRENT_PROCESSES: usize = 32;
const CONCURRENT_MIGRATORS: usize = 4;
const READER_ROUNDS: usize = 3;
const CHILD_KILL_AFTER: Duration = Duration::from_mins(1);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);
const FAR_FUTURE: u64 = 4_102_444_800;
const HOOK_COMMANDS: [ClaudeHookCommand; 5] = [
    ClaudeHookCommand::DiffTrace,
    ClaudeHookCommand::ConversationTrace,
    ClaudeHookCommand::ClaudeMutationScope,
    ClaudeHookCommand::ClaudeModelState,
    ClaudeHookCommand::PolicyBash,
];
const PRE_BINDING_MIGRATIONS: usize = 3;
const BINDING_MIGRATION_ID: &str = "004_bind_telemetry_credentials";

struct PreBindingAuthDbSpec;

impl DbSpec for PreBindingAuthDbSpec {
    fn db_name() -> &'static str {
        AuthDbSpec::db_name()
    }

    fn db_path() -> Result<PathBuf> {
        auth_db_path()
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        &generated_migrations::AUTH_MIGRATIONS[..PRE_BINDING_MIGRATIONS]
    }

    const KIND: DatabaseKind = DatabaseKind::Auth;
}

static FAN_OUT_LOCK: Mutex<()> = Mutex::new(());

fn serialize_fan_out() -> MutexGuard<'static, ()> {
    FAN_OUT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct SchemalessAuthDbSpec;

impl DbSpec for SchemalessAuthDbSpec {
    fn db_name() -> &'static str {
        AuthDbSpec::db_name()
    }

    fn db_path() -> Result<PathBuf> {
        auth_db_path()
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        &[]
    }

    const KIND: DatabaseKind = DatabaseKind::Auth;
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("sandbox"),
        }
    }

    fn command(&self, role: &str) -> Command {
        let root = self.dir.path();
        let mut command = Command::new(std::env::current_exe().expect("test binary"));
        command
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(ROLE_ENV, role)
            .env("HOME", root)
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env(AUTH_DB_ENCRYPTION_KEY_ENV, TEST_SECRET)
            .env_remove("TRACEPARENT")
            .env_remove("TRACESTATE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn run(&self, role: &str) -> String {
        wait_success(role, self.command(role).spawn().expect("spawn child"))
    }

    fn hold(&self, role: &str) -> HeldChild {
        let mut child = self
            .command(role)
            .stdin(Stdio::piped())
            .spawn()
            .expect("spawn holder");
        let stdin = child.stdin.take().expect("holder stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("holder stdout"));
        let mut line = String::new();
        loop {
            line.clear();
            let read = stdout.read_line(&mut line).expect("holder stdout line");
            if read == 0 {
                let _ = child.kill();
                let output = child.wait_with_output().expect("holder output");
                panic!(
                    "{role} holder exited before ready: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            if line.trim_end().ends_with(READY) {
                break;
            }
        }
        HeldChild {
            role: role.to_string(),
            child,
            stdin,
            stdout,
        }
    }

    fn seed(&self) {
        self.run("seed");
    }
}

struct HeldChild {
    role: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl HeldChild {
    fn release(mut self) {
        writeln!(self.stdin).expect("release holder");
        drop(self.stdin);
        let mut rest = String::new();
        self.stdout
            .read_to_string(&mut rest)
            .expect("holder stdout");
        let output = self.child.wait_with_output().expect("holder output");
        assert!(
            output.status.success() && rest.contains("1 passed"),
            "{} holder failed:\n{rest}\n{}",
            self.role,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn wait_success(role: &str, mut child: Child) -> String {
    let started = Instant::now();
    while child.try_wait().expect("child status").is_none() {
        if started.elapsed() > CHILD_KILL_AFTER {
            let _ = child.kill();
            panic!("{role} child did not exit within {CHILD_KILL_AFTER:?}");
        }
        std::thread::sleep(CHILD_POLL_INTERVAL);
    }
    let output = child.wait_with_output().expect("child output");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{role} child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

fn read_token(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .find_map(|line| line.split_once(TOKEN_PREFIX).map(|(_, token)| token))
}

fn spawn_readers(sandbox: &Sandbox) -> Vec<Child> {
    (0..CONCURRENT_PROCESSES)
        .map(|_| sandbox.command("read").spawn().expect("spawn reader"))
        .collect()
}

fn wait_readers(readers: Vec<Child>) -> usize {
    readers
        .into_iter()
        .map(|child| wait_success("read", child))
        .filter(|stdout| read_token(stdout) == Some(TOKEN))
        .count()
}

#[test]
fn reader_succeeds_while_another_process_holds_auth_db_open() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let holder = sandbox.hold("hold-open");
    let reader = sandbox.run("read");
    assert_eq!(read_token(&reader), Some(TOKEN));
    holder.release();
}

#[test]
fn thirty_two_reader_processes_all_resolve_the_credential() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    for round in 0..READER_ROUNDS {
        let started = Instant::now();
        let succeeded = wait_readers(spawn_readers(&sandbox));
        println!(
            "round {round}: {succeeded}/{CONCURRENT_PROCESSES} readers in {:?}",
            started.elapsed()
        );
        assert_eq!(succeeded, CONCURRENT_PROCESSES, "round {round}");
    }
}

#[test]
fn current_schema_reader_performs_no_migration_write_under_held_write_lock() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let writer = sandbox.hold("hold-write");
    let reader = sandbox.run("read");
    assert_eq!(read_token(&reader), Some(TOKEN));
    writer.release();
    sandbox.run("verify-schema");
}

#[test]
fn telemetry_lookup_never_creates_a_missing_auth_db() {
    let _fan_out = serialize_fan_out();
    Sandbox::new().run("lookup-missing");
}

#[test]
fn concurrent_migration_of_schemaless_db_applies_each_migration_once() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.run("create-schemaless");

    let initializers = (0..CONCURRENT_MIGRATORS)
        .map(|_| sandbox.command("init").spawn().expect("spawn initializer"))
        .collect::<Vec<_>>();
    for child in initializers {
        wait_success("init", child);
    }
    sandbox.run("verify-schema");
}

#[test]
fn outdated_schema_is_migrated_and_partial_schema_fails_clearly() {
    let _fan_out = serialize_fan_out();
    Sandbox::new().run("upgrade");
    Sandbox::new().run("partial");
}

#[test]
fn concurrent_hook_processes_export_correlated_hook_spans() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(export_concurrent_hook_spans(&sandbox));
}

async fn export_concurrent_hook_spans(sandbox: &Sandbox) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("addr");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let receiver = tokio::spawn(capture_requests_until(listener, stopped));

    let hooks = (0..CONCURRENT_PROCESSES)
        .map(|index| {
            sandbox
                .command("hook")
                .env(COLLECTOR_ENV, address.to_string())
                .env(HOOK_INDEX_ENV, index.to_string())
                .spawn()
                .expect("spawn hook")
        })
        .collect::<Vec<_>>();
    let mut failing = sandbox.command("hook-credential-failure");
    failing.env(AUTH_DB_ENCRYPTION_KEY_ENV, WRONG_SECRET);
    let failing = failing.spawn().expect("spawn failing hook");
    tokio::task::spawn_blocking(move || {
        for child in hooks {
            wait_success("hook", child);
        }
        wait_success("hook-credential-failure", failing);
    })
    .await
    .expect("hook children join");
    stop.send(()).expect("stop collector");

    let spans = trace_spans(&receiver.await.expect("collector join"));
    let observed = spans
        .iter()
        .filter(|span| span.name == "sce.hook")
        .map(|span| {
            let attribute = |key: &str| {
                span.attributes
                    .iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default()
            };
            (
                attribute("sce.claude.tool_use_id"),
                attribute("sce.hook.command"),
            )
        })
        .collect::<BTreeSet<_>>();
    let expected = (0..CONCURRENT_PROCESSES)
        .map(|index| {
            (
                hook_tool_use_id(index),
                hook_command(index).as_str().to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(observed, expected);
}

fn hook_command(index: usize) -> ClaudeHookCommand {
    HOOK_COMMANDS[index % HOOK_COMMANDS.len()]
}

fn hook_tool_use_id(index: usize) -> String {
    format!("toolu_mp_{index:02}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_db_child_process() {
    let Ok(role) = std::env::var(ROLE_ENV) else {
        return;
    };
    match role.as_str() {
        "seed" => seed().await,
        "hold-open" => {
            let _db = AuthDb::new().await.expect("holder opens auth DB");
            wait_for_release();
        }
        "hold-write" => {
            let db = AuthDb::new().await.expect("writer opens auth DB");
            db.execute("BEGIN IMMEDIATE", ())
                .await
                .expect("writer takes write lock");
            wait_for_release();
            db.execute("ROLLBACK", ()).await.expect("writer rollback");
        }
        "read" => {
            let credential = load_telemetry_credential()
                .await
                .expect("credential lookup succeeds")
                .expect("credential present");
            println!("{TOKEN_PREFIX}{}", credential.token);
        }
        "lookup-missing" => {
            assert_eq!(
                load_telemetry_credential()
                    .await
                    .expect("missing auth DB is not an error"),
                None
            );
            assert!(!auth_db_path().expect("auth DB path").exists());
        }
        "create-schemaless" => {
            let db = EncryptedTursoDb::<SchemalessAuthDbSpec>::new()
                .await
                .expect("schemaless auth DB opens");
            assert!(applied_migrations(&db).await.is_empty());
        }
        "init" => {
            let db = AuthDb::new().await.expect("initializer opens auth DB");
            assert_eq!(applied_migrations(&db).await.len(), expected_migrations());
        }
        "verify-schema" => {
            let db = AuthDb::new().await.expect("verifier opens auth DB");
            assert_eq!(applied_migrations(&db).await.len(), expected_migrations());
            let check = db
                .query_map("PRAGMA integrity_check", (), |row| {
                    row.get::<String>(0).map_err(Into::into)
                })
                .await
                .expect("integrity check");
            assert_eq!(check, vec!["ok".to_string()]);
        }
        "upgrade" => upgrade_outdated_schema().await,
        "partial" => reject_partial_schema().await,
        "hook" => run_hook_child().await,
        "hook-credential-failure" => {
            let (telemetry, fallback) = RuntimeTelemetry::resolve_with(
                load_telemetry_credential(),
                ManagedSpanKind::ClaudeHook,
            )
            .await;
            assert!(matches!(telemetry, RuntimeTelemetry::Noop(_)));
            let fallback = fallback.expect("credential failure is diagnosed");
            assert_eq!(fallback.reason, TelemetryFallbackReason::DatabaseError);
            let output = telemetry
                .with_default_subscriber(&mut || async { Ok("hook-output".to_string()) })
                .await;
            assert_eq!(output.expect("hook result unaffected"), "hook-output");
        }
        other => panic!("unknown child role {other}"),
    }
}

async fn seed() {
    let db = AuthDb::new().await.expect("seed opens auth DB");
    db.execute(
        "INSERT OR REPLACE INTO auth_credentials \
         (id, access_token, token_type, expires_in, refresh_token, scope, stored_at_unix_seconds) \
         VALUES (1, ?1, 'Bearer', 3600, 'refresh', NULL, 1)",
        (test_access_token(USER_ID, Some(WORKSPACE_ID)),),
    )
    .await
    .expect("session seeds");
    drop(db);
    save_telemetry_credential(&TelemetryCredential {
        token: TOKEN.to_string(),
        accessor: "accessor-multiprocess".to_string(),
        expires_at_unix_seconds: FAR_FUTURE,
        identity: TelemetryIdentity {
            user_id: USER_ID.to_string(),
            workspace_id: WORKSPACE_ID.to_string(),
        },
    })
    .await
    .expect("telemetry credential seeds");
}

async fn run_hook_child() {
    let index = std::env::var(HOOK_INDEX_ENV)
        .expect("hook index")
        .parse::<usize>()
        .expect("numeric hook index");
    let address = std::env::var(COLLECTOR_ENV)
        .expect("collector address")
        .parse()
        .expect("socket address");
    let credential = load_credential_within_deadline(load_telemetry_credential())
        .await
        .expect("managed credential resolves");
    let ManagedTelemetryMode::Managed { .. } = select_mode(Some(credential.clone()), 0) else {
        panic!("credential selects managed telemetry");
    };
    let telemetry = loopback_hook_telemetry(address, &credential.token);
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "session_id": "session-mp",
        "tool_use_id": hook_tool_use_id(index),
    })
    .to_string();
    let output = telemetry
        .with_default_subscriber(&mut || async {
            record_claude_hook_payload(hook_command(index), &payload);
            Ok(String::new())
        })
        .await;
    assert_eq!(output.expect("hook result"), "");
    telemetry.shutdown().await;
}

fn wait_for_release() {
    println!("{READY}");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

fn expected_migrations() -> usize {
    generated_migrations::AUTH_MIGRATIONS.len()
}

async fn applied_migrations<M: DbSpec>(db: &EncryptedTursoDb<M>) -> Vec<String> {
    db.query_map("SELECT id FROM __sce_migrations ORDER BY id", (), |row| {
        row.get::<String>(0).map_err(Into::into)
    })
    .await
    .expect("migration ids")
}

async fn upgrade_outdated_schema() {
    let old = EncryptedTursoDb::<PreBindingAuthDbSpec>::new()
        .await
        .expect("pre-binding schema opens");
    assert_eq!(applied_migrations(&old).await.len(), PRE_BINDING_MIGRATIONS);
    drop(old);
    let db = AuthDb::new().await.expect("outdated schema upgrades");
    assert_eq!(applied_migrations(&db).await.len(), expected_migrations());
    db.query_map(
        "SELECT user_id, workspace_id FROM telemetry_credentials",
        (),
        |_| Ok(()),
    )
    .await
    .expect("binding columns exist");
}

async fn reject_partial_schema() {
    let old = EncryptedTursoDb::<PreBindingAuthDbSpec>::new()
        .await
        .expect("pre-binding schema opens");
    old.execute(
        "ALTER TABLE telemetry_credentials ADD COLUMN user_id TEXT NOT NULL DEFAULT ''",
        (),
    )
    .await
    .expect("partial migration applies");
    drop(old);
    let error = AuthDb::new()
        .await
        .err()
        .expect("partially migrated schema must not open");
    let chain = format!("{error:#}");
    assert!(chain.contains(BINDING_MIGRATION_ID), "{chain}");
    let old = EncryptedTursoDb::<PreBindingAuthDbSpec>::new()
        .await
        .expect("pre-binding schema reopens");
    assert_eq!(applied_migrations(&old).await.len(), PRE_BINDING_MIGRATIONS);
}
