use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::{AuthDb, AuthDbSpec};
use crate::generated_migrations;
use crate::services::bash_policy::{
    run_bash_policy_request, BashPolicyRequest, PolicyInputMode, PolicyOutputMode,
};
use crate::services::db::encryption_key::{
    get_or_create_encryption_key, AUTH_DB_ENCRYPTION_KEY_ENV,
};
use crate::services::db::{DatabaseKind, DatabaseLocked, DbSpec, EncryptedTursoDb};
use crate::services::default_paths::auth_db_path;
use crate::services::error::{AuthStorageFailure, CliError};
use crate::services::hooks::{run_hooks_subcommand, HookSubcommand};
use crate::services::managed_telemetry::claude_helper::repository_id_for;
use crate::services::managed_telemetry::claude_helper::tests::git_repository_with_remote;
use crate::services::managed_telemetry::trace_export::tests::{
    capture_requests_until, loopback_hook_telemetry, trace_spans,
};
use crate::services::managed_telemetry::trace_export::{
    record_claude_hook_command, record_claude_hook_payload, ClaudeHookCommand, ManagedSpanKind,
};
use crate::services::managed_telemetry::{
    inspect_health, load_credential_within_deadline, select_mode, ManagedTelemetryHealth,
    ManagedTelemetryMode, RuntimeTelemetry, TelemetryFallbackReason,
};
use crate::services::observability::traits::{NoopLogger, Telemetry};
use crate::services::telemetry_credential_storage::{
    bind_repository_telemetry, load_repository_telemetry_credential, load_telemetry_credential,
    save_telemetry_credential, test_access_token, RepositoryBindingMode, TelemetryCredential,
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
const HEALTH_PREFIX: &str = "SCE_AUTH_DB_CHILD_TELEMETRY_HEALTH=";
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
const WRITER_ENV: &str = "SCE_AUTH_DB_TEST_WRITER";
const COMMITTED_PREFIX: &str = "SCE_AUTH_DB_CHILD_COMMITTED=";
const ROW_PREFIX: &str = "SCE_AUTH_DB_CHILD_ROW=";
const REJECTED: &str = "SCE_AUTH_DB_CHILD_REJECTED";
const WRITER_ROWS: usize = 8;
const WRITER_PAUSE: Duration = Duration::from_millis(20);
const MIXED_WRITER_ROUNDS: usize = 3;
const NEW_WRITERS: usize = 4;
const SHARED_ACCESS_REJECTION: &str = "does not support shared access";
const LEGACY_LOCK_REJECTION: &str = "File is locked by another process";
const LEGACY_CIPHER: &str = "aegis256";
const INSERT_BINDING_SQL: &str = "INSERT INTO telemetry_repository_bindings \
     (repository_id, user_id, workspace_id, bound_at_unix_seconds) VALUES (?1, ?2, ?3, 1)";
const AUTH_DB_FILES: [&str; 3] = ["auth.db", "auth.db-wal", "auth.db-tshm"];
const HEADERS_REPOSITORY: &str = "bound-repo";
const UNBOUND_REPOSITORY: &str = "unbound-repo";
const HEADERS_AUTHORIZATION: &str = r#"{"Authorization":"Bearer tok-multiprocess"}"#;
const HEADERS_UNAVAILABLE: &str = "Telemetry credential is unavailable.";
const STDIN_SECRET: &str = "SECRET_STDIN_VALUE";
const FAIL_OPEN_HOOK_COMMANDS: [ClaudeHookCommand; 3] = [
    ClaudeHookCommand::DiffTrace,
    ClaudeHookCommand::ConversationTrace,
    ClaudeHookCommand::ClaudeModelState,
];

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
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("OTEL_") {
                command.env_remove(key);
            }
        }
        command
    }

    fn writer(&self, role: &str, writer: &str) -> Child {
        self.command(role)
            .env(WRITER_ENV, writer)
            .spawn()
            .expect("spawn writer")
    }

    fn rows(&self) -> BTreeSet<String> {
        prefixed_lines(&self.run("verify-rows"), ROW_PREFIX)
    }

    fn sce_state_dir(&self) -> PathBuf {
        let root = self.dir.path();
        let state_root = if cfg!(target_os = "macos") {
            root.join("Library").join("Application Support")
        } else {
            root.join("state")
        };
        state_root.join("sce")
    }

    fn auth_db_files(&self) -> Vec<String> {
        let state = self.sce_state_dir();
        AUTH_DB_FILES
            .iter()
            .filter(|name| state.join(name).exists())
            .map(ToString::to_string)
            .collect()
    }

    fn run(&self, role: &str) -> String {
        wait_success(role, self.command(role).spawn().expect("spawn child"))
    }

    fn hold(&self, role: &str) -> HeldChild {
        Self::hold_command(role, self.command(role))
    }

    fn hold_command(role: &str, mut command: Command) -> HeldChild {
        let mut child = command.stdin(Stdio::piped()).spawn().expect("spawn holder");
        let stdin = child.stdin.take().expect("holder stdin");
        let stdout = BufReader::new(child.stdout.take().expect("holder stdout"));
        let mut held = HeldChild {
            role: role.to_string(),
            child,
            stdin,
            stdout,
        };
        held.wait_ready();
        held
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
    fn signal(&mut self) {
        writeln!(self.stdin).expect("release holder");
    }

    fn wait_ready(&mut self) -> String {
        let mut captured = String::new();
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .expect("holder stdout line");
            if read == 0 {
                let _ = self.child.kill();
                self.child.wait().expect("holder status");
                let mut stderr = String::new();
                self.child
                    .stderr
                    .take()
                    .expect("holder stderr")
                    .read_to_string(&mut stderr)
                    .expect("holder stderr output");
                panic!(
                    "{} holder exited before ready:\n{captured}\n{stderr}",
                    self.role
                );
            }
            if line.trim_end().ends_with(READY) {
                return captured;
            }
            captured.push_str(&line);
        }
    }

    fn release(mut self) {
        self.signal();
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

fn prefixed_lines(stdout: &str, prefix: &str) -> BTreeSet<String> {
    stdout
        .lines()
        .filter_map(|line| line.split_once(prefix).map(|(_, value)| value.to_string()))
        .collect()
}

fn committed_rows(children: Vec<(String, Child)>) -> (BTreeSet<String>, usize) {
    let mut committed = BTreeSet::new();
    let mut rejected = 0;
    for (role, child) in children {
        let stdout = wait_success(&role, child);
        committed.extend(prefixed_lines(&stdout, COMMITTED_PREFIX));
        rejected += usize::from(stdout.contains(REJECTED));
    }
    (committed, rejected)
}

fn coordinated_writer_rows(sandbox: &Sandbox) -> BTreeSet<String> {
    // Open every connection before writes start, and keep every connection
    // alive until all writes finish so WAL startup/teardown cannot race them.
    let mut writers = (0..NEW_WRITERS)
        .map(|index| {
            let mut command = sandbox.command("coordinated-writer");
            command.env(WRITER_ENV, format!("new-{index}"));
            Sandbox::hold_command("coordinated-writer", command)
        })
        .collect::<Vec<_>>();
    for writer in &mut writers {
        writer.signal();
    }
    let mut committed = BTreeSet::new();
    for writer in &mut writers {
        committed.extend(prefixed_lines(&writer.wait_ready(), COMMITTED_PREFIX));
    }
    for writer in writers {
        writer.release();
    }
    committed
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

struct NoRepositoryContext;

impl crate::app::HasRepoRoot for NoRepositoryContext {
    fn repo_root(&self) -> Option<&Path> {
        None
    }
}

fn run_setup_lifecycle_child(sandbox: &Sandbox, role: &str) {
    wait_success(
        role,
        sandbox
            .command(role)
            .env(AUTH_DB_ENCRYPTION_KEY_ENV, "")
            .spawn()
            .expect("spawn setup child"),
    );
}

#[test]
fn setup_lifecycle_on_fresh_state_creates_no_auth_db_and_resolves_no_key() {
    let _fan_out = serialize_fan_out();
    run_setup_lifecycle_child(&Sandbox::new(), "setup-fresh");
}

#[test]
fn setup_lifecycle_leaves_existing_auth_db_unchanged_and_resolves_no_key() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();
    run_setup_lifecycle_child(&sandbox, "setup-existing");
}

#[test]
fn telemetry_lookup_never_creates_a_missing_auth_db() {
    let _fan_out = serialize_fan_out();
    Sandbox::new().run("lookup-missing");
}

fn telemetry_health(sandbox: &Sandbox, secret: &str) -> String {
    let stdout = wait_success(
        "telemetry-health",
        sandbox
            .command("telemetry-health")
            .env(AUTH_DB_ENCRYPTION_KEY_ENV, secret)
            .spawn()
            .expect("spawn health child"),
    );
    let health = prefixed_lines(&stdout, HEALTH_PREFIX);
    assert_eq!(health.len(), 1, "{stdout}");
    health.into_iter().next().expect("one health line")
}

#[test]
fn doctor_telemetry_health_on_fresh_state_resolves_no_key_and_creates_no_auth_db() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    assert_eq!(telemetry_health(&sandbox, ""), "not_configured");
    assert!(sandbox.auth_db_files().is_empty());
}

#[test]
fn doctor_telemetry_health_classifies_real_auth_db_states_without_mutating_them() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();
    let state = sandbox.sce_state_dir();
    let auth_db = std::fs::read(state.join("auth.db")).expect("seeded auth DB");
    let readiness = state.join("telemetry-ready.json");

    assert_eq!(telemetry_health(&sandbox, TEST_SECRET), "healthy");
    assert_eq!(telemetry_health(&sandbox, WRONG_SECRET), "database_error");
    assert_eq!(telemetry_health(&sandbox, ""), "database_error");
    assert_eq!(
        std::fs::read(state.join("auth.db")).expect("auth DB"),
        auth_db
    );

    std::fs::write(&readiness, br#"{"expires_at_unix_seconds":1}"#).expect("expire readiness");
    assert_eq!(telemetry_health(&sandbox, ""), "credential_expired");
    std::fs::remove_file(&readiness).expect("remove readiness");
    assert_eq!(telemetry_health(&sandbox, ""), "not_configured");
    assert_eq!(read_token(&sandbox.run("read")), Some(TOKEN));
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

#[test]
fn legacy_holder_rejects_new_readers_and_writers_explicitly() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let legacy = sandbox.hold("legacy-hold");
    sandbox.run("expect-rejected-read");
    sandbox.run("expect-rejected-write");
    legacy.release();

    assert_eq!(sandbox.rows(), BTreeSet::from(["legacy-held".to_string()]));
    sandbox.run("legacy-roundtrip");
}

#[test]
fn new_holder_rejects_legacy_writer_explicitly() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let holder = sandbox.hold("hold-open");
    sandbox.run("legacy-expect-rejected");
    holder.release();

    sandbox.run("legacy-roundtrip");
    assert_eq!(
        sandbox.rows(),
        BTreeSet::from(["legacy-roundtrip".to_string()])
    );
}

#[test]
fn concurrent_legacy_and_new_writers_never_lose_committed_rows() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let mut committed = BTreeSet::new();
    let mut rejected = 0;
    for round in 0..MIXED_WRITER_ROUNDS {
        let writers = vec![
            (
                "legacy-writer".to_string(),
                sandbox.writer("legacy-writer", &format!("legacy-{round}")),
            ),
            (
                "new-writer".to_string(),
                sandbox.writer("new-writer", &format!("new-{round}-a")),
            ),
            (
                "new-writer".to_string(),
                sandbox.writer("new-writer", &format!("new-{round}-b")),
            ),
        ];
        let (round_committed, round_rejected) = committed_rows(writers);
        committed.extend(round_committed);
        rejected += round_rejected;
    }
    println!(
        "mixed writers: {} committed rows, {rejected} rejected opens",
        committed.len()
    );

    let persisted = sandbox.rows();
    let lost = committed.difference(&persisted).collect::<Vec<_>>();
    assert!(lost.is_empty(), "committed rows lost: {lost:?}");
    sandbox.run("legacy-roundtrip");
}

#[test]
fn concurrent_new_writers_all_commit_and_legacy_reopens_afterwards() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();
    let committed = coordinated_writer_rows(&sandbox);
    assert_eq!(committed.len(), NEW_WRITERS * WRITER_ROWS);
    assert_eq!(sandbox.rows(), committed);

    sandbox.run("legacy-roundtrip");
    let mut expected = committed;
    expected.insert("legacy-roundtrip".to_string());
    assert_eq!(sandbox.rows(), expected);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_new_shared_lock_rejects_legacy_exclusive_lock_until_released() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let holder = sandbox.hold("hold-open");
    sandbox.run("legacy-lock-expect-rejected");
    sandbox.run("legacy-expect-rejected");
    holder.release();

    sandbox.run("legacy-lock-acquire");
    sandbox.run("legacy-roundtrip");
}

#[cfg(target_os = "macos")]
#[test]
fn macos_legacy_exclusive_lock_rejects_new_shared_lock_until_released() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let legacy = sandbox.hold("legacy-lock-hold");
    sandbox.run("expect-rejected-read");
    sandbox.run("expect-rejected-write");
    legacy.release();

    assert_eq!(read_token(&sandbox.run("read")), Some(TOKEN));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_new_shared_lock_survives_unrelated_descriptor_close() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let holder = sandbox.hold("hold-open-close-other");
    sandbox.run("legacy-lock-expect-rejected");
    sandbox.run("legacy-expect-rejected");
    holder.release();

    sandbox.run("legacy-roundtrip");
}

#[cfg(target_os = "macos")]
#[test]
fn macos_new_processes_share_the_lock_concurrently() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let holders = [
        sandbox.hold("hold-open"),
        sandbox.hold("hold-open-close-other"),
    ];
    let committed = coordinated_writer_rows(&sandbox);
    assert_eq!(committed.len(), NEW_WRITERS * WRITER_ROWS);
    sandbox.run("legacy-lock-expect-rejected");
    for holder in holders {
        holder.release();
    }

    assert_eq!(sandbox.rows(), committed);
    sandbox.run("legacy-roundtrip");
}

#[test]
fn repository_telemetry_lookup_never_creates_a_missing_auth_db() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    let mut lookup = sandbox.command("repository-lookup-missing");
    lookup.env(AUTH_DB_ENCRYPTION_KEY_ENV, "");
    wait_success(
        "repository-lookup-missing",
        lookup.spawn().expect("spawn lookup"),
    );
    assert!(sandbox.auth_db_files().is_empty());
}

#[test]
fn concurrent_telemetry_headers_on_missing_auth_db_fail_closed_without_creating_it() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    let repository = git_repository_with_remote(
        sandbox.dir.path(),
        HEADERS_REPOSITORY,
        "https://github.com/acme/bound-repo",
    );

    let helpers = (0..CONCURRENT_PROCESSES)
        .map(|_| spawn_headers(&sandbox, &repository))
        .collect::<Vec<_>>();
    for helper in helpers {
        assert_headers_unavailable(helper);
    }
    assert!(sandbox.auth_db_files().is_empty());
}

#[test]
fn telemetry_headers_serve_only_the_bound_repository() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    let bound = git_repository_with_remote(
        sandbox.dir.path(),
        HEADERS_REPOSITORY,
        "https://github.com/acme/bound-repo",
    );
    let unbound = git_repository_with_remote(
        sandbox.dir.path(),
        UNBOUND_REPOSITORY,
        "https://github.com/acme/unbound-repo",
    );
    sandbox.seed();
    let mut bind = sandbox.command("bind");
    bind.current_dir(&bound);
    wait_success("bind", bind.spawn().expect("spawn bind"));

    let output = spawn_headers(&sandbox, &bound)
        .wait_with_output()
        .expect("bound headers output");
    assert!(output.status.success(), "bound headers failed");
    assert!(String::from_utf8_lossy(&output.stdout).contains(HEADERS_AUTHORIZATION));
    assert_headers_unavailable(spawn_headers(&sandbox, &unbound));
}

fn spawn_headers(sandbox: &Sandbox, repository: &Path) -> Child {
    let mut helper = sandbox.command("headers");
    helper.current_dir(repository);
    helper.spawn().expect("spawn headers")
}

fn assert_headers_unavailable(helper: Child) {
    let output = helper.wait_with_output().expect("headers output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "headers child failed: {stderr}");
    assert!(stdout.contains("1 passed"), "{stdout}");
    assert!(!stdout.contains("Authorization"), "{stdout}");
    assert!(stderr.contains(HEADERS_UNAVAILABLE), "{stderr}");
}

#[test]
fn every_claude_hook_entry_point_records_its_command_identity() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.dir.path().join("work")).expect("work dir");
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(export_hook_entry_spans(&sandbox));
}

fn hook_stdin_cases(command: ClaudeHookCommand) -> [(Vec<u8>, String); 4] {
    let tool_use_id = format!("toolu_entry_{}", command.as_str());
    let valid = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "session_id": "session-entry",
        "tool_use_id": tool_use_id,
        "tool_input": { "command": "true" },
    })
    .to_string();
    [
        (valid.into_bytes(), tool_use_id),
        (Vec::new(), String::new()),
        (
            format!("{{\"tool_use_id\":\"{STDIN_SECRET}").into_bytes(),
            String::new(),
        ),
        (
            [b"\xff\xfe".as_slice(), STDIN_SECRET.as_bytes()].concat(),
            String::new(),
        ),
    ]
}

async fn export_hook_entry_spans(sandbox: &Sandbox) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("addr");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let receiver = tokio::spawn(capture_requests_until(listener, stopped));

    let mut expected = Vec::new();
    let mut hooks = Vec::new();
    for (index, command) in HOOK_COMMANDS.iter().enumerate() {
        for (stdin, tool_use_id) in hook_stdin_cases(*command) {
            let mut child = sandbox
                .command("hook-entry")
                .current_dir(sandbox.dir.path().join("work"))
                .env(COLLECTOR_ENV, address.to_string())
                .env(HOOK_INDEX_ENV, index.to_string())
                .stdin(Stdio::piped())
                .spawn()
                .expect("spawn hook entry");
            let mut child_stdin = child.stdin.take().expect("hook entry stdin");
            child_stdin.write_all(&stdin).expect("write hook stdin");
            drop(child_stdin);
            hooks.push(child);
            expected.push((command.as_str().to_string(), tool_use_id));
        }
    }
    tokio::task::spawn_blocking(move || {
        for child in hooks {
            wait_success("hook-entry", child);
        }
    })
    .await
    .expect("hook entry children join");
    stop.send(()).expect("stop collector");

    let requests = receiver.await.expect("collector join");
    for request in &requests {
        let body = String::from_utf8_lossy(&request.body);
        assert!(!body.contains(STDIN_SECRET), "hook payload leaked");
        assert!(!body.contains("bao-token-value"), "token leaked");
    }
    let mut observed = Vec::new();
    for span in trace_spans(&requests)
        .iter()
        .filter(|span| span.name == "sce.hook")
    {
        let attribute = |key: &str| {
            span.attributes
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        let command = attribute("sce.hook.command");
        let tool_use_id = attribute("sce.claude.tool_use_id");
        let outcome = attribute("sce.outcome");
        if FAIL_OPEN_HOOK_COMMANDS
            .iter()
            .any(|hook| hook.as_str() == command)
        {
            assert_eq!(outcome, "success", "{command} must fail open");
        }
        if command == ClaudeHookCommand::PolicyBash.as_str() && tool_use_id.is_empty() {
            assert_eq!(outcome, "failure", "malformed policy-bash input must fail");
        }
        observed.push((command, tool_use_id));
    }
    observed.sort();
    expected.sort();
    assert_eq!(observed, expected);
}

async fn run_hook_entry_child() {
    let index = std::env::var(HOOK_INDEX_ENV)
        .expect("hook index")
        .parse::<usize>()
        .expect("numeric hook index");
    let address = std::env::var(COLLECTOR_ENV)
        .expect("collector address")
        .parse()
        .expect("socket address");
    let telemetry = loopback_hook_telemetry(address, "bao-token-value");
    let command = HOOK_COMMANDS[index];
    let _ = telemetry
        .with_default_subscriber(&mut || async move {
            let subcommand = match command {
                ClaudeHookCommand::DiffTrace => HookSubcommand::DiffTrace,
                ClaudeHookCommand::ConversationTrace => HookSubcommand::ConversationTrace,
                ClaudeHookCommand::ClaudeMutationScope => HookSubcommand::ClaudeMutationScope,
                ClaudeHookCommand::ClaudeModelState => HookSubcommand::ClaudeModelState,
                ClaudeHookCommand::PolicyBash => {
                    return run_bash_policy_request(&BashPolicyRequest {
                        input: PolicyInputMode::ClaudePreToolUse,
                        output: PolicyOutputMode::ClaudeHook,
                    });
                }
            };
            run_hooks_subcommand(&subcommand, None::<&NoopLogger>)
                .await
                .map_err(CliError::runtime)
        })
        .await;
    telemetry.shutdown().await;
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
        "telemetry-health" => {
            let label = match inspect_health().await {
                ManagedTelemetryHealth::Healthy => "healthy",
                ManagedTelemetryHealth::Disabled(reason) => reason.as_str(),
            };
            println!("{HEALTH_PREFIX}{label}");
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
        "setup-fresh" | "setup-existing" => {
            use crate::services::lifecycle::ServiceLifecycle;
            let path = auth_db_path().expect("auth DB path");
            let before = std::fs::read(&path).ok();
            assert_eq!(before.is_some(), role == "setup-existing");
            super::lifecycle::AuthDbLifecycle
                .setup(&NoRepositoryContext)
                .await
                .expect("setup needs no auth DB key");
            assert!(path.parent().expect("auth DB parent").is_dir());
            assert_eq!(std::fs::read(&path).ok(), before);
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
        "coordinated-writer" => run_coordinated_writer_child().await,
        "hook" => run_hook_child().await,
        "hook-entry" => run_hook_entry_child().await,
        other => run_compatibility_child(other).await,
    }
}

async fn run_compatibility_child(role: &str) {
    match role {
        "legacy-hold" => {
            let conn = open_legacy_auth_db().await.expect("legacy holder opens");
            wait_for_release();
            insert_binding(&conn, "legacy-held")
                .await
                .expect("legacy holder writes after release");
        }
        "legacy-expect-rejected" => {
            let Err(error) = open_legacy_auth_db().await else {
                panic!("legacy open must be rejected while a new process holds the DB");
            };
            assert!(
                format!("{error:#}").contains(LEGACY_LOCK_REJECTION),
                "{error:#}"
            );
        }
        "legacy-roundtrip" => legacy_roundtrip().await,
        #[cfg(target_os = "macos")]
        "hold-open-close-other" => {
            let _db = AuthDb::new().await.expect("holder opens auth DB");
            let path = auth_db_path().expect("auth DB path");
            drop(std::fs::File::open(&path).expect("unrelated auth DB descriptor"));
            wait_for_release();
        }
        #[cfg(target_os = "macos")]
        "legacy-lock-hold" => {
            let file = auth_db_file();
            legacy_exclusive_lock(&file).expect("legacy lock acquires");
            wait_for_release();
        }
        #[cfg(target_os = "macos")]
        "legacy-lock-expect-rejected" => {
            let error = legacy_exclusive_lock(&auth_db_file())
                .expect_err("legacy lock must be rejected while a new process holds the DB");
            assert_eq!(error.raw_os_error(), Some(libc::EAGAIN), "{error}");
        }
        #[cfg(target_os = "macos")]
        "legacy-lock-acquire" => {
            legacy_exclusive_lock(&auth_db_file()).expect("legacy lock acquires");
        }
        "legacy-writer" => match open_legacy_auth_db().await {
            Ok(conn) => {
                for row in writer_rows() {
                    if insert_binding(&conn, &row).await.is_err() {
                        break;
                    }
                    println!("{COMMITTED_PREFIX}{row}");
                    tokio::time::sleep(WRITER_PAUSE).await;
                }
            }
            Err(error) => {
                assert!(
                    format!("{error:#}").contains(LEGACY_LOCK_REJECTION),
                    "{error:#}"
                );
                println!("{REJECTED}");
            }
        },
        other => run_new_version_child(other).await,
    }
}

async fn run_new_version_child(role: &str) {
    match role {
        "new-writer" => match AuthDb::new().await {
            Ok(db) => {
                for row in writer_rows() {
                    if db
                        .execute(INSERT_BINDING_SQL, (row.as_str(), USER_ID, WORKSPACE_ID))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    println!("{COMMITTED_PREFIX}{row}");
                    tokio::time::sleep(WRITER_PAUSE).await;
                }
            }
            Err(error) => {
                assert!(
                    format!("{error:#}").contains(SHARED_ACCESS_REJECTION),
                    "{error:#}"
                );
                println!("{REJECTED}");
            }
        },
        "expect-rejected-read" => {
            let error = load_telemetry_credential()
                .await
                .expect_err("lookup must fail while a legacy process holds the DB");
            assert!(
                error.to_string().contains(SHARED_ACCESS_REJECTION),
                "{error}"
            );
            assert_eq!(error.failure(), AuthStorageFailure::DatabaseLocked);
        }
        "expect-rejected-write" => {
            let error = AuthDb::new()
                .await
                .err()
                .expect("writer open must fail while a legacy process holds the DB");
            assert!(
                format!("{error:#}").contains(SHARED_ACCESS_REJECTION),
                "{error:#}"
            );
            assert!(error.is::<DatabaseLocked>(), "{error:#}");
        }
        "verify-rows" => print_verified_rows().await,
        "repository-lookup-missing" => {
            assert_eq!(
                load_repository_telemetry_credential("repo-missing".to_string())
                    .await
                    .expect("missing auth DB is not an error"),
                None
            );
            assert_eq!(
                load_telemetry_credential()
                    .await
                    .expect("missing auth DB is not an error"),
                None
            );
        }
        "bind" => {
            let repository_id = repository_id_for(&std::env::current_dir().expect("cwd"))
                .expect("repository identity");
            assert!(
                bind_repository_telemetry(repository_id, RepositoryBindingMode::Enroll)
                    .await
                    .expect("binding succeeds")
            );
        }
        "headers" => {
            let code = Box::pin(crate::app::run(
                ["sce", "auth", "telemetry-headers"]
                    .into_iter()
                    .map(String::from),
            ))
            .await;
            println!();
            if code != std::process::ExitCode::SUCCESS {
                assert_eq!(code, std::process::ExitCode::from(4));
            }
        }
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

async fn run_coordinated_writer_child() {
    let db = AuthDb::new()
        .await
        .expect("coordinated writer opens auth DB");
    wait_for_release();
    for row in writer_rows() {
        db.execute(INSERT_BINDING_SQL, (row.as_str(), USER_ID, WORKSPACE_ID))
            .await
            .unwrap_or_else(|error| panic!("writer failed at {row}: {error:#}"));
        println!("{COMMITTED_PREFIX}{row}");
        tokio::time::sleep(WRITER_PAUSE).await;
    }
    wait_for_release();
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
    crate::services::telemetry_readiness::mark_ready(FAR_FUTURE).expect("readiness seeds");
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
            record_claude_hook_command(hook_command(index));
            record_claude_hook_payload(&payload);
            Ok(String::new())
        })
        .await;
    assert_eq!(output.expect("hook result"), "");
    telemetry.shutdown().await;
}

fn writer_rows() -> Vec<String> {
    let writer = std::env::var(WRITER_ENV).expect("writer id");
    (0..WRITER_ROWS)
        .map(|row| format!("{writer}-{row:02}"))
        .collect()
}

async fn open_legacy_auth_db() -> Result<turso::Connection> {
    let path = auth_db_path()?;
    let hexkey = get_or_create_encryption_key(
        &path,
        AuthDbSpec::db_name(),
        crate::services::db::encryption_key::CredentialAccess::Interactive,
    )?;
    let db = turso::Builder::new_local(path.to_str().expect("UTF-8 auth DB path"))
        .experimental_encryption(true)
        .with_encryption(turso::EncryptionOpts {
            hexkey,
            cipher: LEGACY_CIPHER.to_string(),
        })
        .build()
        .await?;
    Ok(db.connect()?)
}

#[cfg(target_os = "macos")]
fn auth_db_file() -> std::fs::File {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(auth_db_path().expect("auth DB path"))
        .expect("auth DB file")
}

#[cfg(target_os = "macos")]
fn legacy_exclusive_lock(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::c_short::try_from(libc::F_WRLCK).expect("lock type");
    lock.l_whence = libc::c_short::try_from(libc::SEEK_SET).expect("lock whence");
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &raw const lock) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

async fn insert_binding(conn: &turso::Connection, row: &str) -> Result<()> {
    conn.execute(INSERT_BINDING_SQL, (row, USER_ID, WORKSPACE_ID))
        .await?;
    Ok(())
}

async fn legacy_roundtrip() {
    let conn = open_legacy_auth_db()
        .await
        .expect("legacy binary reopens the auth DB");
    let mut rows = conn
        .query("SELECT token FROM telemetry_credentials WHERE id = 1", ())
        .await
        .expect("legacy reads telemetry credential");
    let row = rows
        .next()
        .await
        .expect("legacy credential row fetch")
        .expect("legacy sees credential row");
    assert_eq!(row.get::<String>(0).expect("token"), TOKEN);
    drop(rows);
    insert_binding(&conn, "legacy-roundtrip")
        .await
        .expect("legacy writes after new processes exit");
}

async fn print_verified_rows() {
    let db = AuthDb::new().await.expect("verifier opens auth DB");
    assert_credential_rows_intact(&db).await;
    for row in db
        .query_map(
            "SELECT repository_id FROM telemetry_repository_bindings ORDER BY repository_id",
            (),
            |row| row.get::<String>(0).map_err(Into::into),
        )
        .await
        .expect("binding rows")
    {
        println!("{ROW_PREFIX}{row}");
    }
}

async fn assert_credential_rows_intact(db: &AuthDb) {
    let tokens = db
        .query_map(
            "SELECT token, user_id, workspace_id FROM telemetry_credentials",
            (),
            |row| {
                Ok((
                    row.get::<String>(0)?,
                    row.get::<String>(1)?,
                    row.get::<String>(2)?,
                ))
            },
        )
        .await
        .expect("telemetry credential rows");
    assert_eq!(
        tokens,
        vec![(
            TOKEN.to_string(),
            USER_ID.to_string(),
            WORKSPACE_ID.to_string()
        )]
    );
    let sessions = db
        .query_map("SELECT access_token FROM auth_credentials", (), |row| {
            row.get::<String>(0).map_err(Into::into)
        })
        .await
        .expect("session rows");
    assert_eq!(
        sessions,
        vec![test_access_token(USER_ID, Some(WORKSPACE_ID))]
    );
    let check = db
        .query_map("PRAGMA integrity_check", (), |row| {
            row.get::<String>(0).map_err(Into::into)
        })
        .await
        .expect("integrity check");
    assert_eq!(check, vec!["ok".to_string()]);
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
