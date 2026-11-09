use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexHookPolicyReadiness {
    ProjectHooksAllowed,
    PolicyBlocked,
    Unknown(String),
}

const DEFAULT_CODEX_COMMAND: &str = "codex";

pub(crate) const DEFAULT_POLICY_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn probe_default() -> CodexHookPolicyReadiness {
    probe_effective_policy(DEFAULT_CODEX_COMMAND.as_ref(), DEFAULT_POLICY_PROBE_TIMEOUT)
}

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

    #[cfg(unix)]
    fn temp_script(name: &str, contents: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};
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
