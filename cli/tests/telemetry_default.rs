mod support;

use support::{
    closed_endpoint, contains, count_occurrences, standalone_env, Observed, Receiver, Sandbox,
    NON_ROUTABLE_ENDPOINT,
};

const WORKLOADS: [&[&str]; 2] = [&["version"], &["doctor"]];
const TRACE_SPAN_NAME_FIELD: &[u8] = b"\x2a\x0bsce.command";

fn baseline(sandbox: &Sandbox, args: &[&str]) -> Observed {
    sandbox.run(args, &[]).observed
}

fn assert_offline_parity(label: &str, endpoint: &str) {
    let sandbox = Sandbox::new();
    for args in WORKLOADS {
        let expected = baseline(&sandbox, args);
        let enabled = sandbox.run(args, &standalone_env(endpoint));
        assert_eq!(enabled.observed, expected, "{label} {args:?}");
        assert!(
            enabled.elapsed < support::HARD_TIMEOUT,
            "{label} {args:?} {:?}",
            enabled.elapsed
        );
    }
}

#[test]
fn telemetry_offline_parity_reachable_receiver() {
    let receiver = Receiver::start(true);
    assert_offline_parity("reachable", &receiver.endpoint());
    assert!(receiver.wait_for_request());
}

#[test]
fn telemetry_offline_parity_connection_refused() {
    assert_offline_parity("refused", &closed_endpoint());
}

#[test]
fn telemetry_offline_parity_accept_but_never_respond() {
    let receiver = Receiver::start(false);
    assert_offline_parity("blackhole", &receiver.endpoint());
    assert!(receiver.connections() > 0);
}

#[test]
fn telemetry_offline_parity_non_routable_address() {
    assert_offline_parity("non-routable", NON_ROUTABLE_ENDPOINT);
}

#[test]
fn sce_command_span_standalone_exports_one_root_span_without_credentials() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    let run = sandbox.run(&["doctor"], &standalone_env(&receiver.endpoint()));
    assert!(receiver.wait_for_request());

    let requests = receiver.requests();
    assert_eq!(requests.len(), 1, "one batch for one command");
    let request = &requests[0];
    assert!(request
        .head
        .contains("content-type: application/x-protobuf"));
    assert!(!request.head.contains("authorization"));
    assert!(!request.head.contains("x-sce"));
    assert_eq!(
        count_occurrences(&request.body, TRACE_SPAN_NAME_FIELD),
        1,
        "exactly one sce.command span"
    );
    for key in ["sce.command.name", "sce.outcome", "doctor"] {
        assert!(contains(&request.body, key), "{key}");
    }
    let outcome = if run.observed.code == Some(0) {
        "success"
    } else {
        "failure"
    };
    assert!(contains(&request.body, outcome), "{outcome}");
    for other in ["sce.mutation_scope", "sce.worktree", "sce.db.operation"] {
        assert!(!contains(&request.body, other), "{other}");
    }
}

#[test]
fn sce_command_span_never_exports_arguments_paths_or_error_text() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    let run = sandbox.run(
        &[
            "config",
            "validate",
            "--config",
            "/nonexistent-SECRETPATH-sce-config.json",
        ],
        &standalone_env(&receiver.endpoint()),
    );
    assert_ne!(run.observed.code, Some(0));
    assert!(receiver.wait_for_request());

    let body = &receiver.requests()[0].body;
    assert_eq!(count_occurrences(body, TRACE_SPAN_NAME_FIELD), 1);
    for key in ["sce.outcome", "failure", "sce.error.category", "runtime"] {
        assert!(contains(body, key), "{key}");
    }
    for leaked in [
        "SECRETPATH",
        "nonexistent",
        "validate",
        "exception",
        "stack",
    ] {
        assert!(!contains(body, leaked), "{leaked}");
    }
}

#[cfg(not(feature = "telemetry-test-receiver"))]
#[test]
fn telemetry_test_receiver_inert_without_the_feature() {
    let sandbox = Sandbox::new();
    let receiver = Receiver::start(true);
    let other = Receiver::start(true);
    for args in [&["version"][..], &["hooks", "pre-commit"][..]] {
        let expected = baseline(&sandbox, args);
        let run = sandbox.run(
            args,
            &[
                ("SCE_TELEMETRY", "test-receiver".to_string()),
                ("SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT", receiver.endpoint()),
                ("SCE_TELEMETRY_TEST_LIFECYCLE_FILE", {
                    sandbox.marker_file("inert").to_string_lossy().into_owned()
                }),
                ("OTEL_EXPORTER_OTLP_ENDPOINT", other.endpoint()),
            ],
        );
        assert_eq!(run.observed, expected, "{args:?}");
    }
    assert_eq!(receiver.connections(), 0);
    assert_eq!(other.connections(), 0);
    assert!(!sandbox.marker_file("inert").exists());
}
