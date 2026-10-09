use std::future::{ready, Future};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use opentelemetry::trace::{SpanId, Status, TracerProvider};
use opentelemetry::Value;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};

use super::{provider_with_exporter, scoped_dispatch, SERVICE_NAME};
use crate::services::observability::otel_policy::OtelName;
use crate::services::observability::tracing_boundary::test_spans::{
    emit_raw_boundary_b_inputs, in_otel_span, thread_default_is_none,
};
use crate::services::observability::tracing_boundary::{spawn_in_current_scope, ScopedDispatch};

const SECRET: &str = "sk-live-SECRET-TOKEN-9f8e7d";
const BATCH_THREAD_NAME: &str = "OpenTelemetry.Traces.BatchProcessor";

type ExportThread = (Option<String>, ThreadId);

#[derive(Clone, Debug, Default)]
struct Capture {
    spans: Arc<Mutex<Vec<SpanData>>>,
    export_threads: Arc<Mutex<Vec<ExportThread>>>,
}

impl Capture {
    fn spans(&self) -> Vec<SpanData> {
        self.spans.lock().unwrap().clone()
    }

    fn names(&self) -> Vec<String> {
        self.spans()
            .iter()
            .map(|span| span.name.to_string())
            .collect()
    }
}

impl SpanExporter for Capture {
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        let current = std::thread::current();
        self.export_threads
            .lock()
            .unwrap()
            .push((current.name().map(str::to_string), current.id()));
        self.spans.lock().unwrap().extend(batch);
        ready(Ok(()))
    }
}

fn harness() -> (Capture, SdkTracerProvider, ScopedDispatch) {
    let capture = Capture::default();
    let provider = provider_with_exporter(capture.clone());
    let dispatch = scoped_dispatch(&provider);
    (capture, provider, dispatch)
}

fn attributes(span: &SpanData) -> Vec<(String, String)> {
    let mut attributes: Vec<(String, String)> = span
        .attributes
        .iter()
        .map(|attribute| {
            let value = match &attribute.value {
                Value::I64(number) => number.to_string(),
                Value::String(text) => text.to_string(),
                other => format!("{other:?}"),
            };
            (attribute.key.to_string(), value)
        })
        .collect();
    attributes.sort();
    attributes
}

fn span_named<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    spans
        .iter()
        .find(|span| span.name == name)
        .unwrap_or_else(|| panic!("span {name} exported"))
}

fn run_sync<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
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
async fn telemetry_async_scope_survives_worker_thread_migration() {
    let (capture, provider, dispatch) = harness();
    let task = tokio::spawn(dispatch.scope(async {
        let mut threads = std::collections::HashSet::new();
        for _ in 0..64 {
            in_otel_span(OtelName::Sync, async {
                tokio::task::yield_now().await;
            })
            .await;
            threads.insert(std::thread::current().id());
            tokio::task::yield_now().await;
        }
        threads.len()
    }));
    let threads_seen = task.await.unwrap();
    provider.force_flush().unwrap();

    assert!(threads_seen >= 1);
    assert_eq!(capture.names(), vec!["sce.sync"; 64]);
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
