use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use opentelemetry::trace::{SpanId, Status, TracerProvider};

use super::test_support::{
    attributes, harness, span_named, wait_until, Capture, Stall, BATCH_THREAD_NAME,
};
use super::{
    provider_with_exporter, provider_with_settings, scoped_dispatch, BatchSettings, OtelTelemetry,
    ShutdownOutcome, ShutdownWorker, SERVICE_NAME,
};
use crate::services::hooks::mutation_scope_lock::{AdapterLockSpec, AdvisoryLockError};
use crate::services::observability::otel_policy::OtelName;
use crate::services::observability::tracing_boundary::test_spans::{
    emit_raw_boundary_b_inputs, emit_target_matrix, in_otel_span, thread_default_is_none,
};
use crate::services::observability::tracing_boundary::{spawn_in_current_scope, ScopedDispatch};
use crate::services::observability::traits::Telemetry;

const SECRET: &str = "sk-live-SECRET-TOKEN-9f8e7d";
const STALL_PROBE_BUDGET: Duration = Duration::from_millis(300);
const SCHEDULING_SLACK: Duration = Duration::from_millis(700);
const LOCK_CONTENDER_BOUND: Duration = Duration::from_secs(2);
const EXPORT_START_TIMEOUT: Duration = Duration::from_secs(10);

fn run_sync<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn stalled_telemetry(
    export_stall: Option<Stall>,
    shutdown_stall: Option<Stall>,
    budget: Duration,
) -> (Capture, OtelTelemetry) {
    let capture = Capture {
        export_stall,
        shutdown_stall,
        ..Capture::default()
    };
    let provider = provider_with_exporter(capture.clone());
    (
        capture,
        OtelTelemetry::from_provider_with_budget(provider, budget),
    )
}

async fn emit_command_scope(telemetry: &OtelTelemetry) {
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
#[test]
fn tracing_boundary_b_exporter_layer_exports_only_allowlisted_spans_and_attributes() {
    let (capture, provider, dispatch) = harness();
    run_sync(dispatch.scope(async { emit_raw_boundary_b_inputs(SECRET) }));
    provider.force_flush().unwrap();

    let spans = capture.spans();
    assert_eq!(capture.names(), vec!["sce.command"]);
    let span = &spans[0];
    assert_eq!(
        attributes(span),
        vec![
            ("sce.duration_ms".to_string(), "5".to_string()),
            ("sce.error.category".to_string(), "parse".to_string()),
            ("sce.outcome".to_string(), "success".to_string()),
        ]
    );
    assert!(span.events.is_empty());
    assert!(span.links.is_empty());
    assert_eq!(span.status, Status::error(""));
    let rendered = format!("{spans:?}");
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(!rendered.contains("exception"), "{rendered}");
    assert!(!rendered.contains("password"), "{rendered}");
}

#[test]
fn tracing_boundary_b_exporter_wrapper_enforces_policy_without_layer_filter() {
    let capture = Capture::default();
    let provider = provider_with_exporter(capture.clone());
    let layer = tracing_opentelemetry::layer().with_tracer(provider.tracer(SERVICE_NAME));
    let dispatch = ScopedDispatch::from_layer(layer);
    run_sync(dispatch.scope(async { emit_raw_boundary_b_inputs(SECRET) }));
    provider.force_flush().unwrap();

    let spans = capture.spans();
    assert!(!spans.is_empty());
    for span in &spans {
        assert!(OtelName::parse(&span.name).is_some(), "{}", span.name);
        assert!(span.events.is_empty());
        assert!(span.links.is_empty());
        assert!(matches!(span.status, Status::Unset | Status::Error { .. }));
        if let Status::Error { description } = &span.status {
            assert!(description.is_empty());
        }
        for (key, value) in attributes(span) {
            assert!(
                crate::services::observability::otel_policy::OtelAttribute::KEYS
                    .contains(&key.as_str()),
                "{key}"
            );
            assert_ne!(value, SECRET);
        }
    }
    let rendered = format!("{spans:?}");
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(!rendered.contains("sce.arbitrary"), "{rendered}");
    assert!(!rendered.contains("otel.name"), "{rendered}");
}

#[test]
fn tracing_boundary_b_layer_filter_excludes_sce_target_spans() {
    let (capture, provider, dispatch) = harness();
    run_sync(dispatch.scope(async { emit_raw_boundary_b_inputs(SECRET) }));
    provider.force_flush().unwrap();
    assert_eq!(capture.spans().len(), 1);
    assert_eq!(
        attributes(&capture.spans()[0])
            .iter()
            .filter(|(key, _)| key == "sce.outcome")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_async_scope_is_active_on_every_poll_with_nested_parents() {
    let (capture, provider, dispatch) = harness();
    dispatch
        .scope(async {
            in_otel_span(OtelName::Command, async {
                for _ in 0..3 {
                    tokio::task::yield_now().await;
                }
                in_otel_span(OtelName::Reconciliation, async {
                    tokio::task::yield_now().await;
                    in_otel_span(OtelName::GitSnapshot, async {
                        tokio::task::yield_now().await;
                    })
                    .await;
                })
                .await;
            })
            .await;
        })
        .await;
    provider.force_flush().unwrap();

    let spans = capture.spans();
    assert_eq!(spans.len(), 3);
    let command = span_named(&spans, "sce.command");
    let reconciliation = span_named(&spans, "sce.reconciliation");
    let snapshot = span_named(&spans, "sce.git.snapshot");
    assert_eq!(command.parent_span_id, SpanId::INVALID);
    assert_eq!(
        reconciliation.parent_span_id,
        command.span_context.span_id()
    );
    assert_eq!(
        snapshot.parent_span_id,
        reconciliation.span_context.span_id()
    );
    assert_eq!(
        command.span_context.trace_id(),
        snapshot.span_context.trace_id()
    );
    assert!(thread_default_is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_async_scope_isolates_concurrent_commands() {
    let (capture_a, provider_a, dispatch_a) = harness();
    let (capture_b, provider_b, dispatch_b) = harness();
    let mut tasks = Vec::new();
    for index in 0..16 {
        let (dispatch, name) = if index % 2 == 0 {
            (dispatch_a.clone(), OtelName::Command)
        } else {
            (dispatch_b.clone(), OtelName::Reconciliation)
        };
        tasks.push(tokio::spawn(dispatch.scope(async move {
            for _ in 0..4 {
                in_otel_span(name, async {
                    tokio::task::yield_now().await;
                })
                .await;
            }
        })));
    }
    for task in tasks {
        task.await.unwrap();
    }
    provider_a.force_flush().unwrap();
    provider_b.force_flush().unwrap();

    assert_eq!(capture_a.names(), vec!["sce.command"; 32]);
    assert_eq!(capture_b.names(), vec!["sce.reconciliation"; 32]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_async_scope_reaches_spawned_tasks_only_through_the_helper() {
    let (capture, provider, dispatch) = harness();
    dispatch
        .scope(async {
            in_otel_span(OtelName::Command, async {
                spawn_in_current_scope(async {
                    in_otel_span(OtelName::GitSnapshot, async {
                        tokio::task::yield_now().await;
                    })
                    .await;
                })
                .await
                .unwrap();
                tokio::spawn(async {
                    in_otel_span(OtelName::DbOperation, async {}).await;
                })
                .await
                .unwrap();
            })
            .await;
        })
        .await;
    provider.force_flush().unwrap();

    let spans = capture.spans();
    assert_eq!(spans.len(), 2, "{:?}", capture.names());
    let command = span_named(&spans, "sce.command");
    let snapshot = span_named(&spans, "sce.git.snapshot");
    assert_eq!(snapshot.parent_span_id, command.span_context.span_id());
    assert!(!capture.names().contains(&"sce.db.operation".to_string()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telemetry_async_scope_exports_from_dedicated_thread_without_global_subscriber() {
    let (capture, provider, dispatch) = harness();
    let command_thread = std::thread::current().id();
    dispatch
        .scope(async {
            in_otel_span(OtelName::Command, async {
                tokio::task::yield_now().await;
            })
            .await;
        })
        .await;
    provider.force_flush().unwrap();

    let threads = capture.export_threads.lock().unwrap().clone();
    assert!(!threads.is_empty());
    for (name, id) in threads {
        assert_eq!(name.as_deref(), Some(BATCH_THREAD_NAME));
        assert_ne!(id, command_thread);
    }
    assert!(thread_default_is_none());
}

#[test]
fn tracing_boundary_b_production_subscriber_rejects_wrong_target() {
    let (capture, provider, dispatch) = harness();
    run_sync(dispatch.scope(async { emit_target_matrix() }));
    provider.force_flush().unwrap();

    let spans = capture.spans();
    assert_eq!(capture.names(), vec!["sce.command"], "{spans:?}");
    let span = &spans[0];
    assert_eq!(
        attributes(span),
        vec![("sce.outcome".to_string(), "success".to_string())]
    );
    assert!(span.events.is_empty());
    assert!(span.links.is_empty());
    let rendered = format!("{spans:?}");
    for rejected in ["failure", "timeout", "cancelled", "sce.not_enumerated"] {
        assert!(!rendered.contains(rejected), "{rejected}: {rendered}");
    }
    assert!(!rendered.contains("sce.app.start"), "{rendered}");
}

#[test]
fn tracing_boundary_b_exporter_wrapper_cannot_observe_the_tracing_target() {
    let capture = Capture::default();
    let provider = provider_with_exporter(capture.clone());
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer(SERVICE_NAME))
        .with_location(false)
        .with_threads(false);
    let dispatch = ScopedDispatch::from_layer(layer);
    run_sync(dispatch.scope(async { emit_target_matrix() }));
    provider.force_flush().unwrap();

    let spans = capture.spans();
    let outcomes: Vec<String> = spans
        .iter()
        .flat_map(attributes)
        .filter(|(key, _)| key == "sce.outcome")
        .map(|(_, value)| value)
        .collect();
    assert!(
        outcomes.contains(&"failure".to_string()) && outcomes.contains(&"timeout".to_string()),
        "wrong-target spans reach the wrapper because SpanData carries no tracing target: {outcomes:?}"
    );
    assert!(!spans.iter().any(|span| span.name == "sce.not_enumerated"));
    for span in &spans {
        assert!(
            attributes(span).iter().all(|(key, _)| key != "target"),
            "no target attribute is fabricated"
        );
    }
}

#[test]
fn telemetry_async_scope_survives_worker_thread_migration() {
    type Handoff = Pin<Box<dyn Future<Output = ()> + Send>>;

    let (capture, provider, dispatch) = harness();
    let observed: Arc<Mutex<Vec<(ThreadId, bool)>>> = Arc::default();

    let future: Handoff = Box::pin(dispatch.scope({
        let observed = observed.clone();
        async move {
            let record = |observed: &Mutex<Vec<(ThreadId, bool)>>| {
                observed
                    .lock()
                    .unwrap()
                    .push((std::thread::current().id(), !thread_default_is_none()));
            };
            in_otel_span(OtelName::Command, async {
                in_otel_span(OtelName::Reconciliation, async {
                    record(&observed);
                })
                .await;
                let mut suspended = false;
                std::future::poll_fn(|_| {
                    if suspended {
                        Poll::Ready(())
                    } else {
                        suspended = true;
                        Poll::Pending
                    }
                })
                .await;
                in_otel_span(OtelName::GitSnapshot, async {
                    record(&observed);
                })
                .await;
            })
            .await;
        }
    }));

    let (handoff_sender, handoff_receiver) = mpsc::channel::<Handoff>();
    let thread_a = std::thread::Builder::new()
        .name("migration-a".to_string())
        .spawn(move || {
            let mut future = future;
            let mut cx = Context::from_waker(Waker::noop());
            let first = future.as_mut().poll(&mut cx);
            let id = std::thread::current().id();
            let clean = thread_default_is_none();
            handoff_sender.send(future).unwrap();
            (first.is_pending(), id, clean)
        })
        .unwrap();
    let thread_b = std::thread::Builder::new()
        .name("migration-b".to_string())
        .spawn(move || {
            let mut future = handoff_receiver.recv().unwrap();
            let mut cx = Context::from_waker(Waker::noop());
            let second = future.as_mut().poll(&mut cx);
            (
                second.is_ready(),
                std::thread::current().id(),
                thread_default_is_none(),
            )
        })
        .unwrap();
    let (first_pending, id_a, clean_a) = thread_a.join().unwrap();
    let (second_ready, id_b, clean_b) = thread_b.join().unwrap();
    provider.force_flush().unwrap();

    assert!(first_pending, "first poll must suspend on thread A");
    assert!(second_ready, "second poll must complete on thread B");
    assert_ne!(id_a, id_b);
    assert!(clean_a && clean_b && thread_default_is_none());
    assert_eq!(
        *observed.lock().unwrap(),
        vec![(id_a, true), (id_b, true)],
        "the subscriber was active on each polling thread"
    );

    let spans = capture.spans();
    assert_eq!(spans.len(), 3, "{:?}", capture.names());
    let command = span_named(&spans, "sce.command");
    let first_child = span_named(&spans, "sce.reconciliation");
    let second_child = span_named(&spans, "sce.git.snapshot");
    assert_eq!(command.parent_span_id, SpanId::INVALID);
    assert_eq!(first_child.parent_span_id, command.span_context.span_id());
    assert_eq!(second_child.parent_span_id, command.span_context.span_id());
    for child in [first_child, second_child] {
        assert_eq!(
            child.span_context.trace_id(),
            command.span_context.trace_id()
        );
    }
}

#[test]
fn telemetry_shutdown_exactly_once() {
    run_sync(async {
        let (capture, telemetry) = stalled_telemetry(None, None, Duration::from_secs(5));
        emit_command_scope(&telemetry).await;
        assert_eq!(capture.shutdown_calls(), 0);

        let outcome = telemetry.shutdown().await;
        assert_eq!(outcome, ShutdownOutcome::Completed);
        assert_eq!(capture.shutdown_calls(), 1);
        assert_eq!(capture.names(), vec!["sce.command"]);
        assert_eq!(capture.exports_after_shutdown(), 0);
        let threads = capture.shutdown_threads.lock().unwrap().clone();
        assert_eq!(threads, vec![Some(BATCH_THREAD_NAME.to_string())]);
    });
}

#[test]
fn telemetry_shutdown_exactly_once_when_clones_outlive_the_handle() {
    run_sync(async {
        let capture = Capture::default();
        let provider = provider_with_exporter(capture.clone());
        let retained_provider = provider.clone();
        let retained_dispatch = scoped_dispatch(&provider);
        let telemetry = OtelTelemetry::from_provider_with_budget(provider, Duration::from_secs(5));
        emit_command_scope(&telemetry).await;

        assert_eq!(telemetry.shutdown().await, ShutdownOutcome::Completed);
        drop(retained_dispatch);
        drop(retained_provider);
        assert_eq!(capture.shutdown_calls(), 1);
        assert_eq!(capture.exports_after_shutdown(), 0);
        assert_eq!(capture.names(), vec!["sce.command"]);
    });
}

#[test]
fn telemetry_shutdown_exactly_once_when_dropped_without_explicit_shutdown() {
    let stall = Stall::default();
    let (capture, telemetry) = stalled_telemetry(None, Some(stall.clone()), Duration::from_secs(5));
    run_sync(emit_command_scope(&telemetry));

    let started = Instant::now();
    drop(telemetry);
    assert!(
        started.elapsed() < SCHEDULING_SLACK,
        "dropping the handle must not run the blocking teardown on the caller"
    );
    assert!(stall.wait_entered(EXPORT_START_TIMEOUT));
    assert_eq!(capture.shutdown_calls(), 1);
    stall.release();
    assert!(wait_until(EXPORT_START_TIMEOUT, || capture.names()
        == vec!["sce.command"]));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(capture.shutdown_calls(), 1);
    assert_eq!(capture.exports_after_shutdown(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn telemetry_shutdown_stalled_exporter_does_not_hold_tokio_worker() {
    let export_stall = Stall::default();
    let shutdown_stall = Stall::default();
    let (capture, telemetry) = stalled_telemetry(
        Some(export_stall.clone()),
        Some(shutdown_stall.clone()),
        STALL_PROBE_BUDGET,
    );
    emit_command_scope(&telemetry).await;

    let ticks = Arc::new(AtomicUsize::new(0));
    let max_gap_micros = Arc::new(AtomicUsize::new(0));
    let ticker = tokio::spawn({
        let ticks = ticks.clone();
        let max_gap_micros = max_gap_micros.clone();
        async move {
            let mut last = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                let now = Instant::now();
                let gap =
                    usize::try_from(now.duration_since(last).as_micros()).unwrap_or(usize::MAX);
                max_gap_micros.fetch_max(gap, Ordering::SeqCst);
                last = now;
                ticks.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let started = Instant::now();
    let outcome = tokio::spawn(async move { telemetry.shutdown().await })
        .await
        .unwrap();
    let elapsed = started.elapsed();
    ticker.abort();

    assert_eq!(outcome, ShutdownOutcome::Incomplete);
    assert!(elapsed >= STALL_PROBE_BUDGET, "{elapsed:?}");
    assert!(
        elapsed < STALL_PROBE_BUDGET + SCHEDULING_SLACK,
        "{elapsed:?}"
    );
    assert!(
        ticks.load(Ordering::SeqCst) >= 10,
        "the only Tokio worker kept polling other tasks while the exporter was stalled"
    );
    let max_gap = Duration::from_micros(max_gap_micros.load(Ordering::SeqCst) as u64);
    assert!(
        max_gap < STALL_PROBE_BUDGET / 2,
        "the only Tokio worker was starved for {max_gap:?} while the exporter was stalled"
    );
    assert!(export_stall.wait_entered(EXPORT_START_TIMEOUT));
    let threads = capture.shutdown_threads.lock().unwrap().clone();
    assert!(threads
        .iter()
        .flatten()
        .all(|name| !name.starts_with("tokio")));
    export_stall.release();
    shutdown_stall.release();
}

#[test]
fn telemetry_exporter_cannot_block_locks() {
    let dir = tempfile::tempdir().unwrap();
    let lock_dir = dir.path().to_path_buf();
    let spec = AdapterLockSpec::state("telemetry-exporter-isolation.lock");

    let locked_operation = |lock_dir: std::path::PathBuf| async move {
        let held = spec.acquire_async(&lock_dir).await.unwrap();
        let contended = spec
            .acquire_with_timeout_async(&lock_dir, Duration::from_millis(150))
            .await;
        assert!(matches!(contended, Err(AdvisoryLockError::TimedOut { .. })));
        std::fs::write(lock_dir.join("state.txt"), "committed").unwrap();
        drop(held);
        let started = Instant::now();
        let second_holder = spec
            .acquire_with_timeout_async(&lock_dir, LOCK_CONTENDER_BOUND)
            .await
            .unwrap();
        let waited = started.elapsed();
        drop(second_holder);
        let state_path = lock_dir.join("state.txt");
        let value = spec
            .run_locked_blocking(&lock_dir, move || {
                Ok(std::fs::read_to_string(state_path).ok())
            })
            .await
            .unwrap();
        (waited, value)
    };

    let baseline_dir = tempfile::tempdir().unwrap();
    let baseline = run_sync(async {
        let (waited, _) = locked_operation(baseline_dir.path().to_path_buf()).await;
        let state = std::fs::read_to_string(baseline_dir.path().join("state.txt")).unwrap();
        (waited < LOCK_CONTENDER_BOUND, state)
    });

    let export_stall = Stall::default();
    let capture = Capture {
        export_stall: Some(export_stall.clone()),
        shutdown_stall: Some(export_stall.clone()),
        ..Capture::default()
    };
    let provider = provider_with_settings(
        capture.clone(),
        BatchSettings {
            queue_size: 8,
            batch_size: 1,
            scheduled_delay: Duration::from_millis(10),
        },
    );
    let telemetry = OtelTelemetry::from_provider_with_budget(provider, STALL_PROBE_BUDGET);
    let enabled = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let result = telemetry
                .with_default_subscriber(&mut || {
                    let lock_dir = lock_dir.clone();
                    let export_stall = export_stall.clone();
                    async move {
                        in_otel_span(OtelName::Command, async {
                            in_otel_span(OtelName::WorktreeLock, async {}).await;
                            assert!(
                                export_stall.wait_entered(EXPORT_START_TIMEOUT),
                                "background export must have started and be stalled"
                            );
                            let (waited, _) = locked_operation(lock_dir.clone()).await;
                            assert!(!export_stall.is_released());
                            let state =
                                std::fs::read_to_string(lock_dir.join("state.txt")).unwrap();
                            Ok(format!("{}|{state}", waited < LOCK_CONTENDER_BOUND))
                        })
                        .await
                    }
                })
                .await
                .unwrap();
            let outcome = telemetry.shutdown().await;
            (result, outcome)
        });

    assert_eq!(baseline, (true, "committed".to_string()));
    assert_eq!(enabled.0, "true|committed");
    assert_eq!(enabled.1, ShutdownOutcome::Incomplete);
    export_stall.release();
}

fn failing_spawn(_: ShutdownWorker) -> std::io::Result<()> {
    Err(std::io::Error::other("injected shutdown thread failure"))
}

fn assert_exporter_shutdown_never_entered(capture: &Capture, stall: &Stall) {
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(capture.shutdown_calls(), 0);
    assert!(!stall.wait_entered(Duration::from_millis(50)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn telemetry_shutdown_thread_spawn_failure_is_nonblocking() {
    let stall = Stall::default();
    let (capture, telemetry) = stalled_telemetry(None, Some(stall.clone()), Duration::from_secs(5));
    emit_command_scope(&telemetry).await;

    let ticks = Arc::new(AtomicUsize::new(0));
    let ticker = tokio::spawn({
        let ticks = ticks.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                ticks.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let started = Instant::now();
    let outcome = tokio::spawn(telemetry.shutdown_with(failing_spawn))
        .await
        .unwrap();
    let elapsed = started.elapsed();
    tokio::time::sleep(Duration::from_millis(100)).await;
    ticker.abort();

    assert_eq!(outcome, ShutdownOutcome::Unavailable);
    assert!(elapsed < SCHEDULING_SLACK, "{elapsed:?}");
    assert!(ticks.load(Ordering::SeqCst) >= 5);
    assert_exporter_shutdown_never_entered(&capture, &stall);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn telemetry_shutdown_handoff_failure_is_nonblocking() {
    let stall = Stall::default();
    let (capture, telemetry) = stalled_telemetry(None, Some(stall.clone()), Duration::from_secs(5));
    emit_command_scope(&telemetry).await;

    let started = Instant::now();
    let outcome = tokio::spawn(telemetry.shutdown_with(|worker| {
        drop(worker);
        Ok(())
    }))
    .await
    .unwrap();

    assert_eq!(outcome, ShutdownOutcome::Unavailable);
    assert!(started.elapsed() < SCHEDULING_SLACK);
    assert_exporter_shutdown_never_entered(&capture, &stall);
}

#[test]
fn telemetry_drop_without_shutdown_spawn_failure_is_nonblocking() {
    let stall = Stall::default();
    let (capture, mut telemetry) =
        stalled_telemetry(None, Some(stall.clone()), Duration::from_secs(5));
    run_sync(emit_command_scope(&telemetry));

    let started = Instant::now();
    telemetry.release_with(failing_spawn);
    drop(telemetry);
    assert!(started.elapsed() < SCHEDULING_SLACK);
    assert_exporter_shutdown_never_entered(&capture, &stall);
}
