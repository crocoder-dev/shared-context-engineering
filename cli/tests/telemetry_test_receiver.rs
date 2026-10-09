#![cfg(feature = "telemetry-test-receiver")]

mod support;

use support::{contains, standalone_env, Observed, Receiver, Sandbox};

const MODE: &str = "test-receiver";

fn receiver_env(endpoint: &str) -> Vec<(&'static str, String)> {
    vec![
        ("SCE_TELEMETRY", MODE.to_string()),
        ("SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT", endpoint.to_string()),
    ]
}

fn baseline(sandbox: &Sandbox, args: &[&str]) -> Observed {
    sandbox.run(args, &[]).observed
}

#[test]
fn telemetry_test_receiver_exports_to_the_loopback_receiver_without_credentials() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    let expected = baseline(&sandbox, &["version"]);
    let run = sandbox.run(&["version"], &receiver_env(&receiver.endpoint()));
    assert_eq!(run.observed, expected);
    assert!(receiver.wait_for_request());
    let request = &receiver.requests()[0];
    assert!(!request.head.contains("authorization"));
    assert!(contains(&request.body, "sce.command"));
}

#[test]
fn telemetry_test_receiver_rejects_hostname_https_and_non_loopback_endpoints() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    let port = receiver.addr().port();
    let expected = baseline(&sandbox, &["version"]);
    for rejected in [
        format!("http://localhost:{port}"),
        format!("https://127.0.0.1:{port}"),
        format!("http://0.0.0.0:{port}"),
        format!("http://127.0.0.1:{port}/v1/traces"),
        format!("http://user@127.0.0.1:{port}"),
        format!("127.0.0.1:{port}"),
        "http://192.0.2.1:4318".to_string(),
    ] {
        let run = sandbox.run(&["version"], &receiver_env(&rejected));
        assert_eq!(run.observed, expected, "{rejected}");
    }
    assert_eq!(receiver.connections(), 0);
}

#[test]
fn telemetry_test_receiver_ignores_otel_endpoint_and_rejects_header_overrides() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    let ambient = Receiver::start(true);
    let mut env = receiver_env(&receiver.endpoint());
    env.push(("OTEL_EXPORTER_OTLP_ENDPOINT", ambient.endpoint()));
    env.push((
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        format!("{}/v1/traces", ambient.endpoint()),
    ));
    sandbox.run(&["version"], &env);
    assert!(receiver.wait_for_request());
    assert_eq!(ambient.connections(), 0);

    let rejected = Receiver::start(true);
    let mut env = receiver_env(&rejected.endpoint());
    env.push((
        "OTEL_EXPORTER_OTLP_HEADERS",
        "authorization=Bearer managed-secret".to_string(),
    ));
    let expected = baseline(&sandbox, &["version"]);
    assert_eq!(sandbox.run(&["version"], &env).observed, expected);
    assert_eq!(rejected.connections(), 0);
}

#[test]
fn telemetry_test_receiver_never_touches_the_auth_database() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    sandbox.run(&["version"], &receiver_env(&receiver.endpoint()));
    assert!(receiver.wait_for_request());
    let mut stack = vec![sandbox.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            assert!(!name.contains("auth"), "{}", path.display());
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
}

#[test]
fn telemetry_test_receiver_wins_over_standalone_and_inherited_otel_variables() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    let standalone = Receiver::start(true);
    let mut env = standalone_env(&standalone.endpoint());
    env.retain(|(key, _)| *key != "SCE_TELEMETRY");
    env.extend(receiver_env(&receiver.endpoint()));
    sandbox.run(&["version"], &env);
    assert!(receiver.wait_for_request());
    assert_eq!(standalone.connections(), 0);
}

#[test]
fn telemetry_hook_export_gated_test_receiver_hook_reaches_only_loopback_receiver() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    let inherited = Receiver::start(true);
    for hook in [&["hooks", "pre-commit"][..], &["policy", "bash"][..]] {
        let expected = baseline(&sandbox, hook);
        let mut env = receiver_env(&receiver.endpoint());
        env.push(("OTEL_EXPORTER_OTLP_ENDPOINT", inherited.endpoint()));
        let run = sandbox.run(hook, &env);
        assert_eq!(run.observed, expected, "{hook:?}");
    }
    assert!(receiver.wait_for_request());
    assert_eq!(inherited.connections(), 0);
}
