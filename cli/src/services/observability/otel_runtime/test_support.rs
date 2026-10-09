use std::future::{ready, Future};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use opentelemetry::Value;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};

use super::{provider_with_exporter, scoped_dispatch};
use crate::services::observability::tracing_boundary::ScopedDispatch;

pub(crate) const BATCH_THREAD_NAME: &str = "OpenTelemetry.Traces.BatchProcessor";
const STALL_CEILING: Duration = Duration::from_mins(1);

pub(crate) type ExportThread = (Option<String>, ThreadId);

#[derive(Clone, Debug, Default)]
pub(crate) struct Stall {
    state: Arc<(Mutex<StallState>, Condvar)>,
}

#[derive(Debug, Default)]
struct StallState {
    entered: usize,
    released: bool,
}

impl Stall {
    pub(crate) fn block(&self) {
        let (lock, condvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.entered += 1;
        condvar.notify_all();
        let deadline = Instant::now() + STALL_CEILING;
        while !state.released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            state = condvar.wait_timeout(state, remaining).unwrap().0;
        }
    }

    pub(crate) fn wait_entered(&self, timeout: Duration) -> bool {
        let (lock, condvar) = &*self.state;
        let deadline = Instant::now() + timeout;
        let mut state = lock.lock().unwrap();
        while state.entered == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            state = condvar.wait_timeout(state, remaining).unwrap().0;
        }
        true
    }

    pub(crate) fn is_released(&self) -> bool {
        self.state.0.lock().unwrap().released
    }

    pub(crate) fn release(&self) {
        let (lock, condvar) = &*self.state;
        lock.lock().unwrap().released = true;
        condvar.notify_all();
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Capture {
    pub(crate) spans: Arc<Mutex<Vec<SpanData>>>,
    pub(crate) export_threads: Arc<Mutex<Vec<ExportThread>>>,
    pub(crate) shutdown_calls: Arc<AtomicUsize>,
    pub(crate) exports_after_shutdown: Arc<AtomicUsize>,
    pub(crate) shutdown_threads: Arc<Mutex<Vec<Option<String>>>>,
    pub(crate) export_stall: Option<Stall>,
    pub(crate) shutdown_stall: Option<Stall>,
}

impl Capture {
    pub(crate) fn spans(&self) -> Vec<SpanData> {
        self.spans.lock().unwrap().clone()
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.spans()
            .iter()
            .map(|span| span.name.to_string())
            .collect()
    }

    pub(crate) fn shutdown_calls(&self) -> usize {
        self.shutdown_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn exports_after_shutdown(&self) -> usize {
        self.exports_after_shutdown.load(Ordering::SeqCst)
    }
}

impl SpanExporter for Capture {
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        let current = std::thread::current();
        self.export_threads
            .lock()
            .unwrap()
            .push((current.name().map(str::to_string), current.id()));
        if self.shutdown_calls() > 0 {
            self.exports_after_shutdown.fetch_add(1, Ordering::SeqCst);
        }
        self.spans.lock().unwrap().extend(batch);
        if let Some(stall) = &self.export_stall {
            stall.block();
        }
        ready(Ok(()))
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        self.shutdown_threads
            .lock()
            .unwrap()
            .push(std::thread::current().name().map(str::to_string));
        self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(stall) = &self.shutdown_stall {
            stall.block();
        }
        Ok(())
    }
}

pub(crate) fn harness() -> (Capture, SdkTracerProvider, ScopedDispatch) {
    let capture = Capture::default();
    let provider = provider_with_exporter(capture.clone());
    let dispatch = scoped_dispatch(&provider);
    (capture, provider, dispatch)
}

pub(crate) fn attributes(span: &SpanData) -> Vec<(String, String)> {
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

pub(crate) fn span_named<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    spans
        .iter()
        .find(|span| span.name == name)
        .unwrap_or_else(|| panic!("span {name} exported"))
}

pub(crate) fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}
