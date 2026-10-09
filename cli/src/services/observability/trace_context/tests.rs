use std::time::Duration;

use opentelemetry::trace::{SpanId, TraceContextExt, TraceFlags, TraceId};
use opentelemetry_sdk::trace::SpanData;

use super::{extract_remote_parent, TRACEPARENT_ENV, TRACESTATE_ENV};
use crate::services::observability::otel_policy::CommandName;
use crate::services::observability::otel_runtime::test_support::Capture;
use crate::services::observability::otel_runtime::{
    provider_with_exporter, OtelTelemetry, ShutdownOutcome,
};
use crate::services::observability::tracing_boundary::CommandSpan;
use crate::services::observability::traits::Telemetry;

const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const PARENT_ID: &str = "00f067aa0ba902b7";

fn env_of(traceparent: Option<&str>, tracestate: Option<&str>) -> impl Fn(&str) -> Option<String> {
    let traceparent = traceparent.map(str::to_string);
    let tracestate = tracestate.map(str::to_string);
    move |key| match key {
        TRACEPARENT_ENV => traceparent.clone(),
        TRACESTATE_ENV => tracestate.clone(),
        _ => None,
    }
}

async fn command_span_with(traceparent: Option<&str>, tracestate: Option<&str>) -> Vec<SpanData> {
    let capture = Capture::default();
    let telemetry = OtelTelemetry::from_provider_with_budget(
        provider_with_exporter(capture.clone()),
        Duration::from_secs(5),
    );
    let env = env_of(traceparent, tracestate);
    telemetry
        .with_default_subscriber(&mut || async {
            let span =
                CommandSpan::start(CommandName::parse("version"), extract_remote_parent(&env));
            span.scope(async {
                tokio::task::yield_now().await;
            })
            .await;
            span.finish(None);
            Ok(String::new())
        })
        .await
        .unwrap();
    assert_eq!(telemetry.shutdown().await, ShutdownOutcome::Completed);
    capture.spans()
}

fn sampled_header() -> String {
    format!("00-{TRACE_ID}-{PARENT_ID}-01")
}

#[test]
fn trace_context_extracts_sampled_remote_parent() {
    let env = env_of(Some(&sampled_header()), None);
    let context = extract_remote_parent(&env).expect("valid context");
    let span = context.span();
    let span_context = span.span_context();
    assert_eq!(span_context.trace_id().to_string(), TRACE_ID);
    assert_eq!(span_context.span_id().to_string(), PARENT_ID);
    assert!(span_context.is_sampled());
    assert!(span_context.is_remote());
}

#[test]
fn trace_context_extracts_unsampled_remote_parent() {
    let header = format!("00-{TRACE_ID}-{PARENT_ID}-00");
    let context = extract_remote_parent(&env_of(Some(&header), None)).expect("valid context");
    assert_eq!(
        context.span().span_context().trace_flags(),
        TraceFlags::default()
    );
}

#[test]
fn trace_context_rejects_absent_and_malformed_values() {
    assert!(extract_remote_parent(&env_of(None, Some("vendor=value"))).is_none());
    let zero_trace = format!("00-{}-{PARENT_ID}-01", "0".repeat(32));
    let zero_span = format!("00-{TRACE_ID}-{}-01", "0".repeat(16));
    let upper = format!("00-{}-{PARENT_ID}-01", TRACE_ID.to_uppercase());
    for malformed in [
        "",
        "garbage",
        "00-short-short-01",
        "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
        zero_trace.as_str(),
        zero_span.as_str(),
        upper.as_str(),
    ] {
        assert!(
            extract_remote_parent(&env_of(Some(malformed), None)).is_none(),
            "{malformed}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn trace_context_valid_parent_sets_trace_id_parent_span_and_sampled_flag() {
    let spans = command_span_with(Some(&sampled_header()), Some("vendor=value")).await;
    assert_eq!(spans.len(), 1, "{spans:?}");
    let span = &spans[0];
    assert_eq!(span.span_context.trace_id().to_string(), TRACE_ID);
    assert_eq!(span.parent_span_id.to_string(), PARENT_ID);
    assert_ne!(span.span_context.span_id().to_string(), PARENT_ID);
    assert!(span.span_context.is_sampled());
    assert_eq!(span.span_context.trace_state().header(), "vendor=value");
}

#[tokio::test(flavor = "multi_thread")]
async fn trace_context_unsampled_parent_is_honored() {
    let header = format!("00-{TRACE_ID}-{PARENT_ID}-00");
    let spans = command_span_with(Some(&header), None).await;
    assert!(spans.is_empty(), "{spans:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn trace_context_malformed_or_absent_yields_independent_root() {
    for traceparent in [None, Some("garbage"), Some("00-short-short-01")] {
        let spans = command_span_with(traceparent, Some("vendor=value")).await;
        assert_eq!(spans.len(), 1, "{traceparent:?}");
        let span = &spans[0];
        assert_eq!(span.parent_span_id, SpanId::INVALID, "{traceparent:?}");
        assert_ne!(span.span_context.trace_id(), TraceId::INVALID);
        assert_ne!(span.span_context.trace_id().to_string(), TRACE_ID);
        assert!(span.span_context.trace_state().header().is_empty());
    }
}
