use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use services::command_registry::build_default_registry;
use services::observability::otel_policy::OtelName;
use services::observability::otel_runtime::{
    resolve_telemetry_mode, RuntimeTelemetry, TelemetryMode, SCE_TELEMETRY_ENV,
};
use services::observability::tracing_boundary::test_spans::in_otel_span;
use services::parse::command_runtime::parse_runtime_command;

const CHILD_MARKER: &str = "SCE_TELEMETRY_TEST_CHILD";
const CHILD_ARGS: &str = "SCE_TELEMETRY_TEST_ARGS";
const CHILD_OUT: &str = "SCE_TELEMETRY_TEST_OUT";
const HOOK_INVOCATIONS: [&[&str]; 3] = [
    &["sce", "hooks", "pre-commit"],
    &["sce", "hooks", "mutation-scope"],
    &["sce", "hooks", "codex"],
];

struct Receiver {
    addr: SocketAddr,
    requests: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Receiver {
    fn start(respond: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let held = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let (requests, connections, stop) =
                (requests.clone(), connections.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    connections.fetch_add(1, Ordering::SeqCst);
                    if !respond {
                        held.lock().unwrap().push(stream);
                        continue;
                    }
                    let requests = requests.clone();
                    std::thread::spawn(move || serve(&mut stream, &requests));
                }
            })
        };
        Self {
            addr,
            requests,
            connections,
            stop,
            thread: Some(thread),
        }
    }

    fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    fn wait_for_request(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.requests() > 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(stream: &mut std::net::TcpStream, requests: &AtomicUsize) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let Ok(read) = stream.read(&mut chunk) else {
            break;
        };
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    if head.starts_with("post /v1/traces") {
        requests.fetch_add(1, Ordering::SeqCst);
    }
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );
}

fn closed_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

fn env_map(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect();
    move |key| map.get(key).cloned()
}

fn standalone_env(endpoint: &str) -> impl Fn(&str) -> Option<String> {
    env_map(&[
        (SCE_TELEMETRY_ENV, "standalone".to_string()),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.to_string()),
        (
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            format!("{endpoint}/v1/traces"),
        ),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc".to_string()),
    ])
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

async fn emit_command_span(telemetry: &RuntimeTelemetry) {
    telemetry
        .with_default_subscriber(&mut || async {
            in_otel_span(OtelName::Command, async {
                tokio::task::yield_now().await;
            })
            .await;
            Ok(String::new())
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn telemetry_hook_export_gated_standalone_hook_invocations_export_nothing() {
    let receiver = Receiver::start(true);
    let env = standalone_env(&receiver.endpoint());
    let registry = build_default_registry();
    for hook in HOOK_INVOCATIONS {
        let args = owned(hook);
        let parsed = parse_runtime_command(args.iter().cloned(), &registry, None).unwrap();
        assert!(parsed.is_hook_invocation(), "{hook:?}");
        let telemetry = select_runtime_telemetry(&args, &registry, &env);
        assert!(matches!(telemetry, RuntimeTelemetry::Noop(_)), "{hook:?}");
        emit_command_span(&telemetry).await;
        drop(telemetry);
    }
    let policy = owned(&["sce", "policy", "bash"]);
    assert!(
        parse_runtime_command(policy.iter().cloned(), &registry, None)
            .unwrap()
            .is_hook_invocation()
    );
    assert!(matches!(
        select_runtime_telemetry(&policy, &registry, &env),
        RuntimeTelemetry::Noop(_)
    ));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(receiver.connections(), 0);
    assert_eq!(receiver.requests(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn telemetry_hook_export_gated_unparseable_invocation_fails_closed() {
    let receiver = Receiver::start(true);
    let env = standalone_env(&receiver.endpoint());
    let telemetry = select_runtime_telemetry(
        &owned(&["sce", "--definitely-not-a-flag"]),
        &build_default_registry(),
        &env,
    );
    assert!(matches!(telemetry, RuntimeTelemetry::Noop(_)));
    assert_eq!(receiver.connections(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn telemetry_hook_export_gated_non_hook_standalone_command_still_exports() {
    let receiver = Receiver::start(true);
    let env = standalone_env(&receiver.endpoint());
    let telemetry =
        select_runtime_telemetry(&owned(&["sce", "version"]), &build_default_registry(), &env);
    assert!(matches!(telemetry, RuntimeTelemetry::Otel(_)));
    emit_command_span(&telemetry).await;
    drop(telemetry);
    assert!(
        receiver.wait_for_request(),
        "standalone trace was not exported"
    );
}

#[test]
fn telemetry_hook_export_gated_resolution_never_inspects_invocation_unless_requested() {
    let inspected = std::cell::Cell::new(false);
    let disabled = env_map(&[(
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "http://127.0.0.1:1".to_string(),
    )]);
    assert_eq!(
        resolve_telemetry_mode(&disabled, || {
            inspected.set(true);
            true
        }),
        TelemetryMode::Disabled
    );
    assert!(!inspected.get());
    let standalone = env_map(&[(SCE_TELEMETRY_ENV, "standalone".to_string())]);
    assert_eq!(
        resolve_telemetry_mode(&standalone, || false),
        TelemetryMode::Disabled
    );
    assert_eq!(
        resolve_telemetry_mode(&standalone, || true),
        TelemetryMode::Standalone
    );
}

#[test]
fn telemetry_disabled_baseline_resolution_reads_only_the_environment_switch() {
    let keys = Mutex::new(Vec::new());
    let env = |key: &str| {
        keys.lock().unwrap().push(key.to_string());
        None
    };
    assert_eq!(
        resolve_telemetry_mode(&env, || true),
        TelemetryMode::Disabled
    );
    assert_eq!(*keys.lock().unwrap(), vec![SCE_TELEMETRY_ENV.to_string()]);
    for value in [
        "",
        "Standalone",
        "STANDALONE",
        "on",
        "true",
        "1",
        "managed",
        "test-receiver",
    ] {
        let env = env_map(&[(SCE_TELEMETRY_ENV, value.to_string())]);
        assert_eq!(
            resolve_telemetry_mode(&env, || true),
            TelemetryMode::Disabled,
            "{value}"
        );
    }
}

fn run_child(args: &str, envs: &[(&str, String)], sandbox: &Path, label: &str) -> String {
    let out = sandbox.join(format!("{label}.out"));
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "app::telemetry_tests::isolated_command_output",
            "--nocapture",
        ])
        .current_dir(sandbox)
        .env(CHILD_MARKER, "1")
        .env(CHILD_ARGS, args)
        .env(CHILD_OUT, &out)
        .env("HOME", sandbox)
        .env("XDG_CONFIG_HOME", sandbox.join("config"))
        .env("XDG_STATE_HOME", sandbox.join("state"))
        .env("XDG_CACHE_HOME", sandbox.join("cache"))
        .env("NO_COLOR", "1")
        .env_remove("SCE_CONFIG_FILE")
        .env_remove(SCE_TELEMETRY_ENV);
    for (key, value) in envs {
        command.env(key, value);
    }
    let started = Instant::now();
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "child process did not exit promptly"
    );
    std::fs::read_to_string(out).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_command_output() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let args = std::env::var(CHILD_ARGS).unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = Box::pin(run_with_dependency_check_and_streams(
        args.split_whitespace().map(String::from),
        || Ok(()),
        &mut stdout,
        &mut stderr,
    ))
    .await;
    let rendered = format!(
        "{}\n---\n{}\n---\n{}",
        if code == ExitCode::SUCCESS {
            "success"
        } else {
            "failure"
        },
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    std::fs::write(std::env::var(CHILD_OUT).unwrap(), rendered).unwrap();
}

#[test]
fn telemetry_disabled_baseline_ignores_otlp_environment_and_other_switch_values() {
    let sandbox = tempfile::tempdir().unwrap();
    let receiver = Receiver::start(true);
    let endpoint = receiver.endpoint();
    let baseline = run_child("sce version", &[], sandbox.path(), "baseline");
    assert!(baseline.starts_with("success\n---\nshared-context-engineering "));

    let otlp_only = [
        ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.clone()),
        ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", endpoint.clone()),
    ];
    assert_eq!(
        run_child("sce version", &otlp_only, sandbox.path(), "otlp"),
        baseline
    );
    for (index, value) in ["off", "Standalone", "1"].into_iter().enumerate() {
        let mut envs = otlp_only.to_vec();
        envs.push((SCE_TELEMETRY_ENV, value.to_string()));
        assert_eq!(
            run_child(
                "sce version",
                &envs,
                sandbox.path(),
                &format!("switch-{index}")
            ),
            baseline,
            "{value}"
        );
    }
    assert_eq!(receiver.connections(), 0);
    assert_eq!(receiver.requests(), 0);
}

#[test]
fn telemetry_hook_export_gated_unreachable_receiver_preserves_hook_behavior() {
    let sandbox = tempfile::tempdir().unwrap();
    let baseline = run_child("sce hooks pre-commit", &[], sandbox.path(), "hook-baseline");

    let blackhole = Receiver::start(false);
    let refused = closed_endpoint();
    let cases = [("blackhole", blackhole.endpoint()), ("refused", refused)];
    for (label, endpoint) in cases {
        let envs = [
            (SCE_TELEMETRY_ENV, "standalone".to_string()),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint),
        ];
        assert_eq!(
            run_child("sce hooks pre-commit", &envs, sandbox.path(), label),
            baseline,
            "{label}"
        );
    }
    assert_eq!(blackhole.connections(), 0);
}
