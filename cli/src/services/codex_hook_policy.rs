//! Effective Codex hook-discovery *policy* readiness for SCE's project-owned
//! `.codex/hooks.json` registrations, obtained by asking the installed Codex
//! binary for its own composed effective configuration requirements rather
//! than reproducing Codex's multi-layer requirements composition in SCE.
//!
//! This is deliberately a separate dimension from `codex_hook_trust`: policy
//! answers "will Codex consider this hook *source* eligible at all?", while
//! trust answers "given an eligible source, has this handler been enabled and
//! durably trusted?". A registration is only executable when both are
//! satisfied.
//!
//! Upstream reference (`openai/codex` commit
//! `a8468330bb5f45e9f4d2ec630b01ea8c52908be3`):
//! - `hooks/src/engine/discovery.rs` `HookDiscoveryPolicy::allows`:
//!   `!allow_managed_hooks_only || source.is_managed`, applied per config
//!   layer before that layer's `hooks.json`/TOML hooks are even loaded.
//!   Project `.codex/hooks.json` hooks are `HookSource::Project`,
//!   `is_managed = false` (`hook_metadata_for_config_layer_source`), so they
//!   are entirely excluded from discovery when the policy is active,
//!   regardless of structural or trust state.
//! - `config/src/config_requirements.rs` `ConfigRequirements::allow_managed_hooks_only`
//!   (`Option<bool>`) is populated only from `requirements.toml`/managed
//!   layers; `docs/config.md` documents that putting it in `config.toml` does
//!   not enable the policy, confirmed by
//!   `hooks/src/engine/mod_tests.rs::allow_managed_hooks_only_in_config_toml_does_not_enable_policy`.
//! - Requirements are composed from multiple sources
//!   (`config/src/config_requirements.rs` `RequirementSource`: system
//!   `requirements.toml`, legacy managed `config.toml`/MDM, MDM managed
//!   preferences, backend-delivered enterprise-managed layers, and composites
//!   of these), so SCE cannot safely re-derive the effective value by reading
//!   any single file; it must ask the installed Codex binary for its own
//!   composed answer.
//! - `codex app-server` exposes that composed answer read-only via the
//!   `configRequirements/read` method
//!   (`app-server-protocol/src/protocol/common.rs`
//!   `ConfigRequirementsRead => "configRequirements/read"`, no params),
//!   returning `v2::ConfigRequirementsReadResponse { requirements: Option<ConfigRequirements> }`
//!   (`app-server-protocol/src/protocol/v2/config.rs`), where
//!   `ConfigRequirements::allow_managed_hooks_only: Option<bool>` serializes
//!   as camelCase `allowManagedHooksOnly`. `requirements` itself is `null`
//!   when no requirements are configured at all.
//! - Transport: `codex app-server --stdio` speaks newline-delimited JSON-RPC
//!   2.0 with the `"jsonrpc"` field omitted (`app-server/README.md`
//!   "Protocol"). A connection must send `initialize`
//!   (`app-server-protocol/src/protocol/v1.rs` `InitializeParams`/`ClientInfo`)
//!   and then an `initialized` notification before any other request is
//!   accepted ("Lifecycle Overview"/"Initialization").

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Effective Codex hook-discovery policy readiness for SCE's project-owned
/// `.codex/hooks.json` registrations. Independent of `codex_hook_trust`'s
/// per-handler enabled/trust bookkeeping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexHookPolicyReadiness {
    /// Effective `allow_managed_hooks_only` is absent, `false`, or no
    /// requirements are configured at all: Codex's discovery policy does not
    /// exclude project `.codex/hooks.json` handlers.
    ProjectHooksAllowed,
    /// Effective `allow_managed_hooks_only = true`: Codex's discovery policy
    /// discards every non-managed hook source, including SCE's project
    /// `.codex/hooks.json` registrations, regardless of their structural or
    /// trust state.
    PolicyBlocked,
    /// The effective policy could not be determined (no Codex executable,
    /// spawn/initialization failure, malformed response, timeout, etc.);
    /// carries a human-readable reason. Doctor must never treat this the same
    /// as `ProjectHooksAllowed`.
    Unknown(String),
}

/// Default command used to probe Codex's effective policy in production:
/// `codex app-server --stdio`, invoked directly (no shell).
const DEFAULT_CODEX_COMMAND: &str = "codex";

/// Upper bound on the whole probe's wall-clock lifetime (spawn, initialize,
/// `configRequirements/read`, and teardown). `sce doctor` must never hang
/// because Codex is broken, slow, or unavailable.
pub(crate) const DEFAULT_POLICY_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Production entry point: probe the installed `codex` binary on `PATH`.
pub(crate) fn probe_default() -> CodexHookPolicyReadiness {
    probe_effective_policy(DEFAULT_CODEX_COMMAND.as_ref(), DEFAULT_POLICY_PROBE_TIMEOUT)
}

/// Probe one Codex executable's effective hook-discovery policy over
/// `codex app-server --stdio`, bounded by `timeout`. Never panics on
/// malformed output; always terminates and reaps the child process before
/// returning, on every exit path (success, protocol error, or timeout).
pub(crate) fn probe_effective_policy(
    codex_command: &std::ffi::OsStr,
    timeout: Duration,
) -> CodexHookPolicyReadiness {
    let deadline = Instant::now() + timeout;

    let mut command = Command::new(codex_command);
    command
        .arg("app-server")
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CodexHookPolicyReadiness::Unknown(format!(
                "Codex executable '{}' was not found while probing the effective hook-discovery \
                 policy: {error}",
                codex_command.to_string_lossy()
            ));
        }
        Err(error) => {
            return CodexHookPolicyReadiness::Unknown(format!(
                "Unable to start 'codex app-server --stdio' to probe the effective hook-discovery \
                 policy: {error}"
            ));
        }
    };

    let Some(stdin) = child.stdin.take() else {
        return terminate_and_reap_with_unknown(
            child,
            "Unable to open stdin for the Codex app-server probe process".to_string(),
        );
    };
    let Some(stdout) = child.stdout.take() else {
        return terminate_and_reap_with_unknown(
            child,
            "Unable to open stdout for the Codex app-server probe process".to_string(),
        );
    };
    let mut guard = ChildGuard(child);
    let mut stdin = stdin;

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let readiness = run_policy_probe_session(&mut stdin, &rx, deadline);
    guard.terminate_and_reap();
    readiness
}

/// Drives the initialize/initialized/`configRequirements/read` exchange over
/// an already-spawned child's stdin/stdout channel. Split out from
/// `probe_effective_policy` so the child-process teardown in the caller is
/// unconditional (this function only ever returns a readiness value, never
/// panics or leaks the process).
fn run_policy_probe_session(
    stdin: &mut ChildStdin,
    rx: &mpsc::Receiver<String>,
    deadline: Instant,
) -> CodexHookPolicyReadiness {
    let initialize_request = json!({
        "method": "initialize",
        "id": 1,
        "params": {
            "clientInfo": {
                "name": "sce-doctor",
                "version": env!("CARGO_PKG_VERSION"),
            }
        }
    });
    if let Err(error) = write_jsonl(stdin, &initialize_request) {
        return CodexHookPolicyReadiness::Unknown(format!(
            "Unable to send 'initialize' to the Codex app-server probe: {error}"
        ));
    }

    match read_response_for_id(rx, 1, deadline) {
        Ok(ResponseOutcome::Result(_)) => {}
        Ok(ResponseOutcome::Error(message)) => {
            return CodexHookPolicyReadiness::Unknown(format!(
                "Codex app-server rejected 'initialize' while probing the effective \
                 hook-discovery policy: {message}"
            ));
        }
        Err(reason) => return CodexHookPolicyReadiness::Unknown(reason),
    }

    let initialized_notification = json!({ "method": "initialized" });
    if let Err(error) = write_jsonl(stdin, &initialized_notification) {
        return CodexHookPolicyReadiness::Unknown(format!(
            "Unable to send 'initialized' to the Codex app-server probe: {error}"
        ));
    }

    let config_requirements_request = json!({
        "method": "configRequirements/read",
        "id": 2,
    });
    if let Err(error) = write_jsonl(stdin, &config_requirements_request) {
        return CodexHookPolicyReadiness::Unknown(format!(
            "Unable to send 'configRequirements/read' to the Codex app-server probe: {error}"
        ));
    }

    match read_response_for_id(rx, 2, deadline) {
        Ok(ResponseOutcome::Result(result)) => parse_config_requirements_result(&result),
        Ok(ResponseOutcome::Error(message)) => CodexHookPolicyReadiness::Unknown(format!(
            "Codex app-server rejected 'configRequirements/read': {message}"
        )),
        Err(reason) => CodexHookPolicyReadiness::Unknown(reason),
    }
}

fn terminate_and_reap_with_unknown(child: Child, reason: String) -> CodexHookPolicyReadiness {
    let mut guard = ChildGuard(child);
    guard.terminate_and_reap();
    CodexHookPolicyReadiness::Unknown(reason)
}

/// Ensures the probed Codex process is always terminated and reaped,
/// regardless of which path through `probe_effective_policy` returns.
struct ChildGuard(Child);

impl ChildGuard {
    fn terminate_and_reap(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate_and_reap();
    }
}

fn write_jsonl(stdin: &mut ChildStdin, value: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    stdin.write_all(&line)?;
    stdin.flush()
}

enum ResponseOutcome {
    Result(Value),
    Error(String),
}

/// Reads JSONL lines from `rx` until one whose `id` matches `expected_id` is
/// found, skipping unrelated notifications/responses (bounded, so a chatty or
/// malicious process cannot spin this loop forever), or until `deadline`
/// elapses.
fn read_response_for_id(
    rx: &mpsc::Receiver<String>,
    expected_id: i64,
    deadline: Instant,
) -> Result<ResponseOutcome, String> {
    const MAX_UNRELATED_LINES: usize = 200;
    let mut unrelated = 0usize;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Timed out waiting for a response from 'codex app-server'".to_string());
        }
        let line = match rx.recv_timeout(remaining) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                return Err("Timed out waiting for a response from 'codex app-server'".to_string());
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err("'codex app-server' exited before responding".to_string());
            }
        };

        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                return Err(format!(
                    "Received malformed JSON from 'codex app-server': {error}"
                ));
            }
        };

        let matches_expected_id = value
            .get("id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id == expected_id);
        if !matches_expected_id {
            unrelated += 1;
            if unrelated > MAX_UNRELATED_LINES {
                return Err(
                    "Too many unrelated messages from 'codex app-server' while waiting for a \
                     response"
                        .to_string(),
                );
            }
            continue;
        }

        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string();
            return Ok(ResponseOutcome::Error(message));
        }
        let Some(result) = value.get("result") else {
            return Err("'codex app-server' response had neither 'result' nor 'error'".to_string());
        };
        return Ok(ResponseOutcome::Result(result.clone()));
    }
}

/// Parses a `configRequirements/read` response's `result` value into policy
/// readiness. Pure and independent of process I/O so it can be tested with
/// captured fixtures.
fn parse_config_requirements_result(result: &Value) -> CodexHookPolicyReadiness {
    let Some(requirements) = result.get("requirements") else {
        return CodexHookPolicyReadiness::Unknown(
            "'configRequirements/read' response was missing the 'requirements' field".to_string(),
        );
    };
    if requirements.is_null() {
        return CodexHookPolicyReadiness::ProjectHooksAllowed;
    }
    let Some(requirements) = requirements.as_object() else {
        return CodexHookPolicyReadiness::Unknown(
            "'configRequirements/read' response 'requirements' was not an object or null"
                .to_string(),
        );
    };
    match requirements.get("allowManagedHooksOnly") {
        None | Some(Value::Null | Value::Bool(false)) => {
            CodexHookPolicyReadiness::ProjectHooksAllowed
        }
        Some(Value::Bool(true)) => CodexHookPolicyReadiness::PolicyBlocked,
        Some(other) => CodexHookPolicyReadiness::Unknown(format!(
            "'configRequirements/read' response 'allowManagedHooksOnly' was not a boolean: {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_config_requirements_result: pure-parser fixtures --------------

    #[test]
    fn null_requirements_allows_project_hooks() {
        let result = json!({ "requirements": null });
        assert_eq!(
            parse_config_requirements_result(&result),
            CodexHookPolicyReadiness::ProjectHooksAllowed
        );
    }

    #[test]
    fn missing_allow_managed_hooks_only_allows_project_hooks() {
        let result = json!({ "requirements": { "allowAppshots": true } });
        assert_eq!(
            parse_config_requirements_result(&result),
            CodexHookPolicyReadiness::ProjectHooksAllowed
        );
    }

    #[test]
    fn null_allow_managed_hooks_only_allows_project_hooks() {
        let result = json!({ "requirements": { "allowManagedHooksOnly": null } });
        assert_eq!(
            parse_config_requirements_result(&result),
            CodexHookPolicyReadiness::ProjectHooksAllowed
        );
    }

    #[test]
    fn false_allow_managed_hooks_only_allows_project_hooks() {
        let result = json!({ "requirements": { "allowManagedHooksOnly": false } });
        assert_eq!(
            parse_config_requirements_result(&result),
            CodexHookPolicyReadiness::ProjectHooksAllowed
        );
    }

    #[test]
    fn true_allow_managed_hooks_only_blocks_project_hooks() {
        let result = json!({ "requirements": { "allowManagedHooksOnly": true } });
        assert_eq!(
            parse_config_requirements_result(&result),
            CodexHookPolicyReadiness::PolicyBlocked
        );
    }

    #[test]
    fn non_boolean_allow_managed_hooks_only_is_unknown() {
        let result = json!({ "requirements": { "allowManagedHooksOnly": "true" } });
        assert!(matches!(
            parse_config_requirements_result(&result),
            CodexHookPolicyReadiness::Unknown(_)
        ));
    }

    #[test]
    fn non_object_requirements_is_unknown() {
        let result = json!({ "requirements": "not-an-object" });
        assert!(matches!(
            parse_config_requirements_result(&result),
            CodexHookPolicyReadiness::Unknown(_)
        ));
    }

    #[test]
    fn missing_requirements_field_is_unknown() {
        let result = json!({});
        assert!(matches!(
            parse_config_requirements_result(&result),
            CodexHookPolicyReadiness::Unknown(_)
        ));
    }

    // -- probe_effective_policy: real subprocess spawn, using fake `codex` --
    // scripts (executed directly, never through `sh -c`/`bash -c`/`eval`) so
    // these tests never depend on a real installed Codex binary.

    #[cfg(unix)]
    fn temp_script(name: &str, contents: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        // A coarse-resolution clock in some sandboxes can make
        // `SystemTime::now()` collide across concurrently-running test
        // threads that write to the same OS temp directory; an atomic
        // counter guarantees uniqueness regardless of clock resolution
        // (`Command::spawn` on a path another thread's child process is
        // still executing otherwise fails with `ETXTBSY`).
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sce-codex-hook-policy-{name}-{}-{}-{nonce}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Some sandboxed/overlay filesystems spuriously return `ETXTBSY`
    /// ("Text file busy") when `exec`ing a script that was written and
    /// `chmod`ed moments earlier by a concurrently-running test thread, even
    /// with a guaranteed-unique path. Retrying a couple of times is the
    /// standard mitigation for this known transient OS race and keeps these
    /// tests meaningful (they still fail on a genuine, persistent spawn
    /// failure).
    #[cfg(unix)]
    fn probe_effective_policy_retrying_on_transient_busy_text(
        codex_command: &std::ffi::OsStr,
        timeout: Duration,
    ) -> CodexHookPolicyReadiness {
        for attempt in 0..3 {
            let readiness = probe_effective_policy(codex_command, timeout);
            let is_transient_busy_text = matches!(
                &readiness,
                CodexHookPolicyReadiness::Unknown(reason) if reason.contains("Text file busy")
            );
            if !is_transient_busy_text || attempt == 2 {
                return readiness;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        unreachable!()
    }

    #[cfg(unix)]
    #[test]
    fn missing_codex_executable_is_unknown() {
        let readiness = probe_effective_policy(
            std::ffi::OsStr::new("/nonexistent/sce-doctor-fixture/codex"),
            Duration::from_secs(2),
        );
        assert!(matches!(readiness, CodexHookPolicyReadiness::Unknown(_)));
    }

    #[cfg(unix)]
    #[test]
    fn full_handshake_with_allow_managed_hooks_only_true_is_policy_blocked() {
        let script = temp_script(
            "blocked",
            r#"#!/bin/sh
read -r init_line
printf '{"id":1,"result":{}}\n'
read -r initialized_line
read -r config_line
printf '{"id":2,"result":{"requirements":{"allowManagedHooksOnly":true}}}\n'
# Keep running briefly so the probe's kill/reap path is exercised too.
sleep 5
"#,
        );

        let readiness = probe_effective_policy_retrying_on_transient_busy_text(
            script.as_os_str(),
            Duration::from_secs(3),
        );
        assert_eq!(readiness, CodexHookPolicyReadiness::PolicyBlocked);
        std::fs::remove_file(&script).ok();
    }

    #[cfg(unix)]
    #[test]
    fn full_handshake_with_no_requirements_is_project_hooks_allowed() {
        let script = temp_script(
            "allowed",
            r#"#!/bin/sh
read -r init_line
printf '{"id":1,"result":{}}\n'
read -r initialized_line
read -r config_line
printf '{"id":2,"result":{"requirements":null}}\n'
"#,
        );

        let readiness = probe_effective_policy_retrying_on_transient_busy_text(
            script.as_os_str(),
            Duration::from_secs(3),
        );
        assert_eq!(readiness, CodexHookPolicyReadiness::ProjectHooksAllowed);
        std::fs::remove_file(&script).ok();
    }

    #[cfg(unix)]
    #[test]
    fn process_exiting_before_responding_is_unknown() {
        let script = temp_script(
            "exits-early",
            r"#!/bin/sh
exit 1
",
        );

        let readiness = probe_effective_policy(script.as_os_str(), Duration::from_secs(3));
        assert!(matches!(readiness, CodexHookPolicyReadiness::Unknown(_)));
        std::fs::remove_file(&script).ok();
    }

    #[cfg(unix)]
    #[test]
    fn malformed_json_response_is_unknown() {
        let script = temp_script(
            "malformed",
            r"#!/bin/sh
read -r init_line
printf 'not json at all\n'
",
        );

        let readiness = probe_effective_policy(script.as_os_str(), Duration::from_secs(3));
        assert!(matches!(readiness, CodexHookPolicyReadiness::Unknown(_)));
        std::fs::remove_file(&script).ok();
    }

    #[cfg(unix)]
    #[test]
    fn timeout_is_unknown_and_child_is_terminated() {
        let script = temp_script(
            "hangs",
            r"#!/bin/sh
# Never reads or writes anything; the probe must time out and kill this.
sleep 30
",
        );

        let started = Instant::now();
        let readiness = probe_effective_policy(script.as_os_str(), Duration::from_millis(500));
        assert!(matches!(readiness, CodexHookPolicyReadiness::Unknown(_)));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the probe must not wait anywhere near the child's own 30s sleep"
        );
        std::fs::remove_file(&script).ok();
    }

    #[cfg(unix)]
    #[test]
    fn config_requirements_error_response_is_unknown() {
        let script = temp_script(
            "error-response",
            r#"#!/bin/sh
read -r init_line
printf '{"id":1,"result":{}}\n'
read -r initialized_line
read -r config_line
printf '{"id":2,"error":{"code":-32601,"message":"method not found"}}\n'
"#,
        );

        let readiness = probe_effective_policy(script.as_os_str(), Duration::from_secs(3));
        assert!(matches!(readiness, CodexHookPolicyReadiness::Unknown(_)));
        std::fs::remove_file(&script).ok();
    }
}
