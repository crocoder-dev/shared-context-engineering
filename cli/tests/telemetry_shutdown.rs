#![cfg(feature = "telemetry-test-receiver")]

mod support;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use support::{
    closed_endpoint, standalone_env, Observed, Receiver, Sandbox, NON_ROUTABLE_ENDPOINT,
};

const WARMUP_RUNS: usize = 5;
const MEASURED_RUNS: usize = 30;
const FLUSH_BUDGET: Duration = Duration::from_secs(1);
const POST_COMMAND_SLACK: Duration = Duration::from_millis(250);
const END_TO_END_SLACK: Duration = Duration::from_millis(250);
const END_TO_END_MAX_SLACK: Duration = Duration::from_millis(500);
static TIMING_LOCK: Mutex<()> = Mutex::new(());
const WORKLOADS: [&[&str]; 2] = [&["version"], &["doctor"]];

#[derive(Clone, Copy, Debug)]
enum Mode {
    Reachable,
    Refused,
    Blackhole,
    NonRoutable,
}

struct Bounds {
    p50: Duration,
    p95: Duration,
    max: Duration,
}

impl Mode {
    fn bounds(self) -> Bounds {
        match self {
            Self::Blackhole | Self::NonRoutable => Bounds {
                p50: FLUSH_BUDGET + END_TO_END_SLACK,
                p95: FLUSH_BUDGET + END_TO_END_SLACK,
                max: FLUSH_BUDGET + END_TO_END_MAX_SLACK,
            },
            Self::Reachable | Self::Refused => Bounds {
                p50: Duration::from_millis(250),
                p95: Duration::from_millis(400),
                max: Duration::from_millis(750),
            },
        }
    }
}

struct Target {
    env: Vec<(&'static str, String)>,
    _receiver: Option<Receiver>,
}

fn target(mode: Mode) -> Target {
    let loopback = |endpoint: String, receiver| Target {
        env: vec![
            ("SCE_TELEMETRY", "test-receiver".to_string()),
            ("SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT", endpoint),
        ],
        _receiver: receiver,
    };
    match mode {
        Mode::Reachable => {
            let receiver = Receiver::start(true);
            loopback(receiver.endpoint(), Some(receiver))
        }
        Mode::Refused => loopback(closed_endpoint(), None),
        Mode::Blackhole => {
            let receiver = Receiver::start(false);
            loopback(receiver.endpoint(), Some(receiver))
        }
        Mode::NonRoutable => Target {
            env: standalone_env(NON_ROUTABLE_ENDPOINT),
            _receiver: None,
        },
    }
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    let rank = (sorted.len() * percent).div_ceil(100);
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn markers(contents: &str) -> HashMap<String, u128> {
    contents
        .lines()
        .filter_map(|line| {
            let (name, nanos) = line.split_once(' ')?;
            Some((name.to_string(), nanos.parse().ok()?))
        })
        .collect()
}

fn post_command(markers: &HashMap<String, u128>) -> Duration {
    let started = markers["command_complete"];
    let requested = markers["process_exit_requested"];
    assert!(markers["shutdown_begin"] >= started);
    assert!(markers["shutdown_end"] >= markers["shutdown_begin"]);
    assert!(requested >= markers["shutdown_end"]);
    Duration::from_nanos(u64::try_from(requested - started).unwrap())
}

struct Measurement {
    overhead: Vec<Duration>,
    post_command: Vec<Duration>,
}

struct SampleContext<'a> {
    workload: &'a str,
    mode: &'a str,
    iteration: usize,
    budget: Duration,
}

fn load_diagnostics() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "loadavg unavailable".to_string(),
        |load| load.trim().to_string(),
    )
}

fn require_completed(
    result: Result<support::Run, String>,
    context: &SampleContext<'_>,
) -> support::Run {
    result.unwrap_or_else(|failure| {
        panic!(
            "hard timeout: workload={} mode={} iteration={} flush_budget={:?} hard_timeout={:?} loadavg={}\n{failure}",
            context.workload,
            context.mode,
            context.iteration,
            context.budget,
            support::HARD_TIMEOUT,
            load_diagnostics(),
        )
    })
}

fn assert_strict_max(sorted: &[Duration], bound: Duration, what: &str, summary: &str) {
    let worst = *sorted.last().unwrap();
    assert!(
        worst <= bound,
        "{what}: maximum sample {worst:?} exceeds bound {bound:?} (loadavg={}); {summary}",
        load_diagnostics(),
    );
}

fn measure(
    sandbox: &Sandbox,
    args: &[&str],
    target: &Target,
    extra_env: &[(&'static str, String)],
    label: &str,
    budget: Duration,
) -> Measurement {
    let _serial = TIMING_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut expected: Option<Observed> = None;
    let mut measurement = Measurement {
        overhead: Vec::new(),
        post_command: Vec::new(),
    };
    for iteration in 0..WARMUP_RUNS + MEASURED_RUNS {
        let context = SampleContext {
            workload: args[0],
            mode: label,
            iteration,
            budget,
        };
        let disabled = require_completed(sandbox.try_run(args, &[]), &context);
        let marker_file = sandbox.marker_file(&format!("{label}-{iteration}"));
        let mut env = target.env.clone();
        env.extend(extra_env.iter().cloned());
        env.push((
            "SCE_TELEMETRY_TEST_LIFECYCLE_FILE",
            marker_file.to_string_lossy().into_owned(),
        ));
        let enabled = require_completed(sandbox.try_run(args, &env), &context);

        let baseline = expected.get_or_insert_with(|| disabled.observed.clone());
        assert_eq!(
            &disabled.observed, baseline,
            "{label} {args:?} disabled drift"
        );
        assert_eq!(
            &enabled.observed, baseline,
            "{label} {args:?} enabled drift"
        );

        let recorded = markers(&std::fs::read_to_string(&marker_file).unwrap());
        assert_eq!(recorded.len(), 4, "{recorded:?}");
        if iteration >= WARMUP_RUNS {
            measurement
                .overhead
                .push(enabled.elapsed.saturating_sub(disabled.elapsed));
            measurement.post_command.push(post_command(&recorded));
        }
    }
    measurement.overhead.sort();
    measurement.post_command.sort();
    measurement
}

fn assert_protocol(mode: Mode) {
    let sandbox = Sandbox::new();
    let target = target(mode);
    let bounds = mode.bounds();
    for args in WORKLOADS {
        let label = format!("{mode:?}-{}", args[0]);
        let measurement = measure(&sandbox, args, &target, &[], &label, FLUSH_BUDGET);
        let summary = format!(
            "{label}: overhead p50={:?} p95={:?} max={:?}; post-command p50={:?} p95={:?} max={:?}",
            percentile(&measurement.overhead, 50),
            percentile(&measurement.overhead, 95),
            measurement.overhead.last().unwrap(),
            percentile(&measurement.post_command, 50),
            percentile(&measurement.post_command, 95),
            measurement.post_command.last().unwrap(),
        );
        println!("{summary}");
        assert!(
            percentile(&measurement.overhead, 50) <= bounds.p50,
            "{summary}"
        );
        assert!(
            percentile(&measurement.overhead, 95) <= bounds.p95,
            "{summary}"
        );
        assert_strict_max(&measurement.overhead, bounds.max, "overhead", &summary);
        assert_strict_max(
            &measurement.post_command,
            FLUSH_BUDGET + POST_COMMAND_SLACK,
            "post-command",
            &summary,
        );
    }
}

#[test]
fn telemetry_shutdown_subprocess_reachable_receiver() {
    assert_protocol(Mode::Reachable);
}

#[test]
fn telemetry_shutdown_subprocess_connection_refused() {
    assert_protocol(Mode::Refused);
}

#[test]
fn telemetry_shutdown_subprocess_accept_but_never_respond() {
    assert_protocol(Mode::Blackhole);
}

#[test]
fn telemetry_shutdown_subprocess_non_routable_address() {
    assert_protocol(Mode::NonRoutable);
}

#[test]
fn telemetry_shutdown_subprocess_expired_budget_does_not_wait_for_a_stalled_exporter() {
    let sandbox = Sandbox::new();
    let blackhole = target(Mode::Blackhole);
    let budget = Duration::from_millis(200);
    let extra = [("SCE_TELEMETRY_FLUSH_TIMEOUT_MS", "200".to_string())];
    let measurement = measure(&sandbox, &["version"], &blackhole, &extra, "expiry", budget);
    let worst = *measurement.post_command.last().unwrap();
    println!("expiry: post-command max={worst:?}");
    assert_strict_max(
        &measurement.post_command,
        budget + POST_COMMAND_SLACK,
        "post-command",
        "expiry",
    );
    assert!(
        worst >= budget / 2,
        "the stalled exporter should have consumed the budget, not returned early: {worst:?}"
    );
    assert_strict_max(
        &measurement.overhead,
        budget + END_TO_END_MAX_SLACK,
        "overhead",
        "expiry",
    );
}

#[test]
fn telemetry_shutdown_measurement_rejects_hard_timeout() {
    let context = SampleContext {
        workload: "version",
        mode: "synthetic",
        iteration: 7,
        budget: FLUSH_BUDGET,
    };
    let panic_message = |result: std::thread::Result<()>| {
        let payload = result.expect_err("expected a failure");
        payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default()
    };

    let message = panic_message(std::panic::catch_unwind(|| {
        require_completed(
            Err("sce [\"version\"] hung past the 5s hard timeout\nTHREAD DUMP".to_string()),
            &context,
        );
    }));
    for needle in [
        "workload=version",
        "mode=synthetic",
        "iteration=7",
        "flush_budget=1s",
        "THREAD DUMP",
    ] {
        assert!(message.contains(needle), "{needle} missing from {message}");
    }

    let bound = Duration::from_millis(750);
    let mut one_outlier = vec![Duration::from_millis(10); 29];
    one_outlier.push(bound + Duration::from_millis(1));
    assert!(percentile(&one_outlier, 50) <= bound);
    assert!(percentile(&one_outlier, 95) <= bound);
    let message = panic_message(std::panic::catch_unwind(|| {
        assert_strict_max(&one_outlier, bound, "overhead", "synthetic");
    }));
    assert!(message.contains("maximum sample"), "{message}");

    let mut bounded = vec![Duration::from_millis(10); 29];
    bounded.push(bound);
    assert_strict_max(&bounded, bound, "overhead", "synthetic");
}
