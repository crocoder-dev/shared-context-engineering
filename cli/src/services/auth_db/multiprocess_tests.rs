use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::{AuthDb, AuthDbSpec};
use crate::generated_migrations;
use crate::services::bash_policy::{
    run_bash_policy_request, BashPolicyRequest, PolicyInputMode, PolicyOutputMode,
};
use crate::services::db::encryption_key::AUTH_DB_ENCRYPTION_KEY_ENV;
use crate::services::db::{DatabaseKind, DbSpec, EncryptedTursoDb};
use crate::services::default_paths::auth_db_path;
use crate::services::error::CliError;
use crate::services::hooks::{run_hooks_subcommand, HookSubcommand};
use crate::services::managed_telemetry::claude_helper::repository_id_for;
use crate::services::managed_telemetry::claude_helper::tests::git_repository_with_remote;
use crate::services::managed_telemetry::trace_export::tests::{
    capture_requests_until, loopback_hook_telemetry, trace_spans,
};
use crate::services::managed_telemetry::trace_export::ClaudeHookCommand;
use crate::services::managed_telemetry::{inspect_health, ManagedTelemetryHealth};
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
const WRITER_ROWS: usize = 8;
const WRITER_PAUSE: Duration = Duration::from_millis(20);
const CONCURRENT_WRITERS: usize = 4;
const INSERT_BINDING_SQL: &str = "INSERT INTO telemetry_repository_bindings \
     (repository_id, user_id, workspace_id, bound_at_unix_seconds) VALUES (?1, ?2, ?3, 1)";
const AUTH_DB_FILES: [&str; 3] = ["auth.db", "auth.db-wal", "auth.db-tshm"];
const HEADERS_REPOSITORY: &str = "bound-repo";
const UNBOUND_REPOSITORY: &str = "unbound-repo";
const HEADERS_AUTHORIZATION: &str = r#"{"Authorization":"Bearer tok-multiprocess"}"#;
const HEADERS_UNAVAILABLE: &str = "Telemetry credential is unavailable.";
const STDIN_SECRET: &str = "SECRET_STDIN_VALUE";
const TELEMETRY_FALLBACK_EVENT: &str = "event_id=sce.telemetry.fallback";
const FALLBACK_REASON_PREFIX: &str = "reason=";
const CLI_ARGS_ENV: &str = "SCE_AUTH_DB_TEST_CLI_ARGS";
const CLI_SUCCESS_PREFIX: &str = "SCE_AUTH_DB_CHILD_CLI_SUCCESS=";
const AUTH_DISABLED_BY_REPOSITORY: &str =
    "Authentication is disabled by `auth.mode: disabled` in the repository SCE config";
const AUTH_DISABLED_BY_GLOBAL: &str =
    "Authentication is disabled by `auth.mode: disabled` in the global SCE config";
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

    fn global_config_file(&self) -> PathBuf {
        let root = self.dir.path();
        let config_root = if cfg!(target_os = "macos") {
            root.join("Library").join("Application Support")
        } else {
            root.join("config")
        };
        config_root.join("sce").join("config.json")
    }

    fn write_global_config(&self, config: &serde_json::Value) {
        write_config_file(&self.global_config_file(), config);
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
        let stdout = child.stdout.take().expect("holder stdout");
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut held = HeldChild {
            role: role.to_string(),
            child,
            stdin,
            lines,
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
    lines: Receiver<String>,
}

impl HeldChild {
    fn signal(&mut self) {
        writeln!(self.stdin).expect("release holder");
    }

    fn wait_ready(&mut self) -> String {
        let deadline = Instant::now() + CHILD_KILL_AFTER;
        let mut captured = String::new();
        loop {
            match self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(line) if line.trim_end().ends_with(READY) => return captured,
                Ok(line) => {
                    captured.push_str(&line);
                    captured.push('\n');
                }
                Err(RecvTimeoutError::Timeout) => self.abort(
                    &format!("was not ready within {CHILD_KILL_AFTER:?}"),
                    &captured,
                ),
                Err(RecvTimeoutError::Disconnected) => self.abort("exited before ready", &captured),
            }
        }
    }

    fn abort(&mut self, reason: &str, stdout: &str) -> ! {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        panic!("{} holder {reason}:\n{stdout}\n{stderr}", self.role);
    }

    fn release(mut self) {
        self.signal();
        wait_bounded(&self.role, &mut self.child);
        let rest = self.lines.iter().collect::<Vec<_>>().join("\n");
        let output = self.child.wait_with_output().expect("holder output");
        assert!(
            output.status.success() && rest.contains("1 passed"),
            "{} holder failed:\n{rest}\n{}",
            self.role,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn wait_bounded(role: &str, child: &mut Child) {
    let started = Instant::now();
    while child.try_wait().expect("child status").is_none() {
        if started.elapsed() > CHILD_KILL_AFTER {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{role} child did not exit within {CHILD_KILL_AFTER:?}");
        }
        std::thread::sleep(CHILD_POLL_INTERVAL);
    }
}

fn wait_success(role: &str, mut child: Child) -> String {
    wait_bounded(role, &mut child);
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

fn coordinated_writer_rows(sandbox: &Sandbox) -> BTreeSet<String> {
    // Open every connection before writes start, and keep every connection
    // alive until all writes finish so WAL startup/teardown cannot race them.
    let mut writers = (0..CONCURRENT_WRITERS)
        .map(|index| {
            let mut command = sandbox.command("coordinated-writer");
            command.env(WRITER_ENV, format!("writer-{index}"));
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

fn expected_writer_rows() -> BTreeSet<String> {
    (0..CONCURRENT_WRITERS)
        .flat_map(|index| (0..WRITER_ROWS).map(move |row| format!("writer-{index}-{row:02}")))
        .collect()
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

    assert_eq!(wait_readers(spawn_readers(&sandbox)), CONCURRENT_PROCESSES);
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
fn concurrent_current_version_writers_persist_every_row() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    assert_eq!(coordinated_writer_rows(&sandbox), expected_writer_rows());
    assert_eq!(sandbox.rows(), expected_writer_rows());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_current_version_processes_share_the_lock_concurrently() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();

    let holders = [sandbox.hold("hold-open"), sandbox.hold("hold-open")];
    assert_eq!(coordinated_writer_rows(&sandbox), expected_writer_rows());
    assert_eq!(read_token(&sandbox.run("read")), Some(TOKEN));
    for holder in holders {
        holder.release();
    }
    assert_eq!(sandbox.rows(), expected_writer_rows());
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

    let config = headers_policy_config(&sandbox, true);
    let helpers = (0..CONCURRENT_PROCESSES)
        .map(|_| spawn_headers(&sandbox, &repository, Some(&config)))
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

    let enabled = headers_policy_config(&sandbox, true);
    let disabled = headers_policy_config(&sandbox, false);
    assert_headers_served(spawn_headers(&sandbox, &bound, Some(&enabled)));
    assert_headers_unavailable(spawn_headers(&sandbox, &unbound, Some(&enabled)));
    assert_headers_unavailable(spawn_headers(&sandbox, &bound, Some(&disabled)));
}

fn write_config_file(path: &Path, config: &serde_json::Value) {
    std::fs::create_dir_all(path.parent().expect("config parent")).expect("config dir");
    std::fs::write(path, config.to_string()).expect("config file");
}

fn headers_policy_config(sandbox: &Sandbox, telemetry_enabled: bool) -> PathBuf {
    let config = sandbox
        .dir
        .path()
        .join(format!("headers-policy-{telemetry_enabled}.json"));
    std::fs::write(
        &config,
        serde_json::json!({ "telemetry": { "enabled": telemetry_enabled } }).to_string(),
    )
    .expect("headers policy config");
    config
}

fn spawn_headers(sandbox: &Sandbox, repository: &Path, config: Option<&Path>) -> Child {
    let mut helper = sandbox.command("headers");
    helper.current_dir(repository);
    if let Some(config) = config {
        helper.env("SCE_CONFIG_FILE", config);
    }
    helper.spawn().expect("spawn headers")
}

fn assert_headers_served(helper: Child) {
    let output = helper.wait_with_output().expect("headers output");
    assert!(output.status.success(), "headers child failed");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(HEADERS_AUTHORIZATION),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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

fn policy_hook_fallback_reasons(sandbox: &Sandbox, telemetry_enabled: bool) -> Vec<String> {
    let root = sandbox.dir.path();
    let work = root.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
    let config = root.join(format!("policy-{telemetry_enabled}.json"));
    std::fs::write(
        &config,
        serde_json::json!({ "telemetry": { "enabled": telemetry_enabled } }).to_string(),
    )
    .expect("policy config");
    hook_fallback_reasons(
        sandbox,
        &work,
        Some(&config),
        &format!("logs-{telemetry_enabled}"),
    )
}

fn hook_fallback_reasons(
    sandbox: &Sandbox,
    working_dir: &Path,
    explicit_config: Option<&Path>,
    log_label: &str,
) -> Vec<String> {
    let log_dir = sandbox.dir.path().join(log_label);
    std::fs::create_dir_all(&log_dir).expect("log dir");
    let mut hook = sandbox.command("policy-hook");
    hook.current_dir(working_dir)
        .env(AUTH_DB_ENCRYPTION_KEY_ENV, "")
        .env("SCE_LOG_LEVEL", "debug")
        .env("SCE_LOG_FORMAT", "text")
        .env("SCE_LOG_DIR", &log_dir)
        .env("CLAUDECODE", "1")
        .env("CLAUDE_PROJECT_DIR", working_dir);
    if let Some(config) = explicit_config {
        hook.env("SCE_CONFIG_FILE", config);
    }
    wait_success("policy-hook", hook.spawn().expect("spawn policy hook"));
    let mut reasons = Vec::new();
    for entry in std::fs::read_dir(&log_dir).expect("log entries") {
        let log = std::fs::read_to_string(entry.expect("log entry").path()).expect("log file");
        reasons.extend(
            log.lines()
                .filter(|line| line.contains(TELEMETRY_FALLBACK_EVENT))
                .filter_map(|line| {
                    line.split_whitespace()
                        .find_map(|token| token.strip_prefix(FALLBACK_REASON_PREFIX))
                        .map(ToString::to_string)
                }),
        );
    }
    reasons
}

struct CliRun {
    success: bool,
    stdout: String,
    stderr: String,
}

fn run_cli(
    sandbox: &Sandbox,
    working_dir: &Path,
    args: &[&str],
    explicit_config: Option<&Path>,
) -> CliRun {
    let mut command = sandbox.command("cli");
    command
        .current_dir(working_dir)
        .env(CLI_ARGS_ENV, args.join("\n"));
    if let Some(config) = explicit_config {
        command.env("SCE_CONFIG_FILE", config);
    }
    let output = command
        .spawn()
        .expect("spawn cli")
        .wait_with_output()
        .expect("cli output");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "cli child failed: {stderr}");
    CliRun {
        success: stdout.contains(&format!("{CLI_SUCCESS_PREFIX}true")),
        stdout,
        stderr,
    }
}

fn bound_repository(sandbox: &Sandbox) -> PathBuf {
    let repository = git_repository_with_remote(
        sandbox.dir.path(),
        HEADERS_REPOSITORY,
        "https://github.com/acme/bound-repo",
    );
    sandbox.seed();
    let mut bind = sandbox.command("bind");
    bind.current_dir(&repository);
    wait_success("bind", bind.spawn().expect("spawn bind"));
    repository
}

#[test]
fn cli_launched_from_a_repository_subdirectory_honors_repository_policy() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.write_global_config(&serde_json::json!({
        "auth": { "mode": "optional" },
        "telemetry": { "enabled": true },
    }));
    let repository = bound_repository(&sandbox);
    let nested = repository.join("src").join("module");
    let repository_config = repository.join(".sce").join("config.json");

    assert_headers_served(spawn_headers(&sandbox, &nested, None));
    assert_eq!(
        hook_fallback_reasons(&sandbox, &nested, None, "logs-consented"),
        ["database_error"]
    );

    write_config_file(
        &repository_config,
        &serde_json::json!({ "telemetry": { "enabled": false } }),
    );
    assert_headers_unavailable(spawn_headers(&sandbox, &nested, None));
    assert_eq!(
        hook_fallback_reasons(&sandbox, &nested, None, "logs-repository-opt-out"),
        ["disabled_by_config"]
    );

    write_config_file(
        &repository_config,
        &serde_json::json!({ "auth": { "mode": "disabled" } }),
    );
    let sync = run_cli(
        &sandbox,
        &nested,
        &["sce", "sync", "--format", "json"],
        None,
    );
    assert!(!sync.success, "{}", sync.stdout);
    assert!(
        sync.stderr.contains(AUTH_DISABLED_BY_REPOSITORY),
        "{}",
        sync.stderr
    );
    assert_headers_unavailable(spawn_headers(&sandbox, &nested, None));
}

#[test]
fn explicit_config_cannot_enable_access_the_global_config_forbids() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    let repository = bound_repository(&sandbox);
    let explicit = sandbox.dir.path().join("explicit-config.json");
    write_config_file(
        &explicit,
        &serde_json::json!({
            "auth": { "mode": "optional" },
            "telemetry": { "enabled": true },
        }),
    );
    assert_headers_served(spawn_headers(&sandbox, &repository, Some(&explicit)));

    sandbox.write_global_config(&serde_json::json!({
        "auth": { "mode": "disabled" },
        "telemetry": { "enabled": false },
    }));
    assert_headers_unavailable(spawn_headers(&sandbox, &repository, Some(&explicit)));
    assert_eq!(
        hook_fallback_reasons(&sandbox, &repository, Some(&explicit), "logs-explicit"),
        ["disabled_by_config"]
    );
    let sync = run_cli(
        &sandbox,
        &repository,
        &["sce", "sync", "--format", "json"],
        Some(&explicit),
    );
    assert!(!sync.success, "{}", sync.stdout);
    assert!(
        sync.stderr.contains(AUTH_DISABLED_BY_GLOBAL),
        "{}",
        sync.stderr
    );

    let show = run_cli(
        &sandbox,
        &repository,
        &[
            "sce",
            "config",
            "show",
            "--config",
            explicit.to_str().expect("utf-8 path"),
            "--format",
            "json",
        ],
        None,
    );
    assert!(show.success, "{}", show.stderr);
    for expected in [
        r#""state": "disabled""#,
        r#""reason": "auth_disabled""#,
        r#""config_source": "default_discovered_global""#,
    ] {
        assert!(
            show.stdout.contains(expected),
            "{expected}: {}",
            show.stdout
        );
    }
}

#[test]
fn claude_hook_with_telemetry_disabled_by_config_never_resolves_the_credential() {
    let _fan_out = serialize_fan_out();
    let sandbox = Sandbox::new();
    sandbox.seed();
    let state = sandbox.sce_state_dir();
    let auth_db = std::fs::read(state.join("auth.db")).expect("seeded auth DB");
    assert!(state.join("telemetry-ready.json").is_file());

    assert_eq!(
        policy_hook_fallback_reasons(&sandbox, false),
        ["disabled_by_config"]
    );
    assert_eq!(
        policy_hook_fallback_reasons(&sandbox, true),
        ["database_error"]
    );
    assert_eq!(
        std::fs::read(state.join("auth.db")).expect("auth DB"),
        auth_db
    );
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
            let label = match inspect_health(true).await {
                ManagedTelemetryHealth::Healthy => "healthy",
                ManagedTelemetryHealth::Disabled(reason) => reason.as_str(),
                ManagedTelemetryHealth::DisabledByConfig { .. } => "disabled_by_config",
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
        "verify-rows" => print_verified_rows().await,
        "coordinated-writer" => run_coordinated_writer_child().await,
        "hook-entry" => run_hook_entry_child().await,
        "policy-hook" => {
            let _ = Box::pin(crate::app::run(
                ["sce", "hooks", "claude-model-state"]
                    .into_iter()
                    .map(String::from),
            ))
            .await;
        }
        other => run_binding_child(other).await,
    }
}

async fn run_binding_child(role: &str) {
    match role {
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
        "cli" => {
            let args = std::env::var(CLI_ARGS_ENV).expect("cli args");
            let code = Box::pin(crate::app::run(args.split('\n').map(String::from))).await;
            println!();
            println!(
                "{CLI_SUCCESS_PREFIX}{}",
                code == std::process::ExitCode::SUCCESS
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

fn writer_rows() -> Vec<String> {
    let writer = std::env::var(WRITER_ENV).expect("writer id");
    (0..WRITER_ROWS)
        .map(|row| format!("{writer}-{row:02}"))
        .collect()
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
