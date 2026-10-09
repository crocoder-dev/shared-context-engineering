use std::future::Future;
use std::sync::mpsc::{sync_channel, Receiver, SendError};
use std::time::Duration;

use opentelemetry::trace::{Status, TracerProvider};
use opentelemetry::{KeyValue, Value};
use opentelemetry_otlp::{Protocol, RetryPolicy, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider, SpanData, SpanEvents, SpanExporter,
    SpanLinks,
};
use opentelemetry_sdk::Resource;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::Layer;

use crate::services::command_registry::CommandRegistry;
use crate::services::error::CliError;
use crate::services::observability::otel_policy::{
    OtelAttribute, OtelName, OtelRawValue, OtelValue, OTEL_TARGET,
};
use crate::services::observability::tracing_boundary::ScopedDispatch;
use crate::services::observability::traits::{NoopTelemetry, Telemetry};
use crate::services::parse::command_runtime::parse_runtime_command;

pub const SCE_TELEMETRY_ENV: &str = "SCE_TELEMETRY";
pub const STANDALONE_MODE: &str = "standalone";
const OTLP_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const OTLP_TRACES_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT";
pub const SERVICE_NAME: &str = "sce";
pub const FLUSH_TIMEOUT_ENV: &str = "SCE_TELEMETRY_FLUSH_TIMEOUT_MS";
pub const DEFAULT_FLUSH_BUDGET: Duration = Duration::from_secs(1);
pub const MAX_FLUSH_BUDGET: Duration = Duration::from_secs(5);
const EXPORT_TIMEOUT: Duration = Duration::from_millis(750);
const SHUTDOWN_THREAD_NAME: &str = "sce-otel-shutdown";
const INIT_THREAD_NAME: &str = "sce-otel-init";

#[cfg(feature = "telemetry-test-receiver")]
pub mod test_receiver;

pub mod lifecycle_markers;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatchSettings {
    pub queue_size: usize,
    pub batch_size: usize,
    pub scheduled_delay: Duration,
}

impl BatchSettings {
    pub const PRODUCTION: Self = Self {
        queue_size: 256,
        batch_size: 64,
        scheduled_delay: Duration::from_secs(2),
    };
}

pub fn resolve_flush_budget(env: &impl Fn(&str) -> Option<String>) -> Duration {
    let max_millis = u64::try_from(MAX_FLUSH_BUDGET.as_millis()).unwrap_or(u64::MAX);
    env(FLUSH_TIMEOUT_ENV)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| (1..=max_millis).contains(millis))
        .map_or(DEFAULT_FLUSH_BUDGET, Duration::from_millis)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryMode {
    Disabled,
    Standalone,
}

pub fn resolve_telemetry_mode(
    env: &impl Fn(&str) -> Option<String>,
    export_permitted_for_invocation: impl FnOnce() -> bool,
) -> TelemetryMode {
    if env(SCE_TELEMETRY_ENV).as_deref() != Some(STANDALONE_MODE) {
        return TelemetryMode::Disabled;
    }
    if export_permitted_for_invocation() {
        TelemetryMode::Standalone
    } else {
        TelemetryMode::Disabled
    }
}

#[derive(Debug)]
pub struct BoundaryBExporter<E> {
    inner: E,
}

impl<E> BoundaryBExporter<E> {
    pub fn new(inner: E) -> Self {
        Self { inner }
    }
}

fn raw_attribute_value(value: &Value) -> Option<OtelRawValue<'_>> {
    match value {
        Value::I64(number) => Some(OtelRawValue::Number(u64::try_from(*number).ok()?)),
        Value::String(text) => Some(OtelRawValue::Text(text.as_str())),
        _ => None,
    }
}

fn canonical_number(text: &str) -> Option<u64> {
    let number = text.parse::<u64>().ok()?;
    (number.to_string() == text).then_some(number)
}

fn admit_attribute(attribute: &KeyValue) -> Option<KeyValue> {
    let key = attribute.key.as_str();
    let raw = raw_attribute_value(&attribute.value)?;
    let admitted = OtelAttribute::admit(key, raw).or_else(|| match raw {
        OtelRawValue::Text(text) => {
            OtelAttribute::admit(key, OtelRawValue::Number(canonical_number(text)?))
        }
        OtelRawValue::Number(_) => None,
    })?;
    let value = match admitted.value() {
        OtelValue::Number(number) => Value::I64(i64::try_from(number).unwrap_or(i64::MAX)),
        OtelValue::Static(text) => Value::from(text),
    };
    Some(KeyValue::new(admitted.key(), value))
}

fn admit_span(mut span: SpanData) -> Option<SpanData> {
    OtelName::parse(span.name.as_ref())?;
    span.attributes = span.attributes.iter().filter_map(admit_attribute).collect();
    span.dropped_attributes_count = 0;
    span.events = SpanEvents::default();
    span.links = SpanLinks::default();
    if matches!(span.status, Status::Error { .. }) {
        span.status = Status::error("");
    }
    Some(span)
}

impl<E: SpanExporter> SpanExporter for BoundaryBExporter<E> {
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        let admitted: Vec<SpanData> = batch.into_iter().filter_map(admit_span).collect();
        async move {
            if admitted.is_empty() {
                return Ok(());
            }
            self.inner.export(admitted).await
        }
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

pub fn scoped_dispatch(provider: &SdkTracerProvider) -> ScopedDispatch {
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer(SERVICE_NAME))
        .with_location(false)
        .with_threads(false)
        .with_tracked_inactivity(false)
        .with_filter(filter_fn(|metadata| {
            metadata.is_span()
                && metadata.target() == OTEL_TARGET
                && OtelName::parse(metadata.name()).is_some()
        }));
    ScopedDispatch::from_layer(layer)
}

fn resource() -> Resource {
    Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", SERVICE_NAME),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        ])
        .build()
}

pub fn provider_with_exporter<E: SpanExporter + 'static>(exporter: E) -> SdkTracerProvider {
    provider_with_settings(exporter, BatchSettings::PRODUCTION)
}

pub fn provider_with_settings<E: SpanExporter + 'static>(
    exporter: E,
    settings: BatchSettings,
) -> SdkTracerProvider {
    let config = BatchConfigBuilder::default()
        .with_max_queue_size(settings.queue_size)
        .with_max_export_batch_size(settings.batch_size)
        .with_scheduled_delay(settings.scheduled_delay)
        .build();
    let processor = BatchSpanProcessor::builder(BoundaryBExporter::new(exporter))
        .with_batch_config(config)
        .build();
    SdkTracerProvider::builder()
        .with_resource(resource())
        .with_span_processor(processor)
        .build()
}

fn standalone_traces_endpoint(env: &impl Fn(&str) -> Option<String>) -> Option<String> {
    let configured = |key| env(key).filter(|value| !value.is_empty());
    configured(OTLP_TRACES_ENDPOINT_ENV).or_else(|| {
        configured(OTLP_ENDPOINT_ENV)
            .map(|base| format!("{}/v1/traces", base.trim_end_matches('/')))
    })
}

fn http_client_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .timeout(EXPORT_TIMEOUT)
        .connect_timeout(EXPORT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
}

fn http_client() -> anyhow::Result<reqwest::blocking::Client> {
    http_client_builder()
        .build()
        .or_else(|_| {
            http_client_builder()
                .tls_certs_only(std::iter::empty())
                .build()
        })
        .map_err(Into::into)
}

fn build_provider(traces_endpoint: Option<String>) -> anyhow::Result<SdkTracerProvider> {
    let builder = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_http_client(http_client()?)
        .with_protocol(Protocol::HttpBinary)
        .with_retry_policy(RetryPolicy::disabled())
        .with_timeout(EXPORT_TIMEOUT);
    let exporter = match traces_endpoint {
        Some(endpoint) => builder.with_endpoint(endpoint),
        None => builder,
    }
    .build()?;
    Ok(provider_with_exporter(exporter))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownOutcome {
    Completed,
    Incomplete,
    Unavailable,
}

struct Teardown {
    dispatch: ScopedDispatch,
    provider: SdkTracerProvider,
}

impl Teardown {
    /// Last-resort path when no shutdown thread can take ownership. Dropping
    /// the final `SdkTracerProvider` reference runs the SDK's synchronous
    /// shutdown, and `shutdown_with_timeout(Duration::ZERO)` is not guaranteed
    /// to return immediately, so neither may run on the caller. Forgetting
    /// skips both destructors: no SDK code runs, nothing is exported or
    /// written, and the provider, dispatch and batch worker stay allocated
    /// until the short-lived CLI process exits. Memory is not reclaimed before
    /// then; the tradeoff is accepted because this path needs thread creation
    /// or handoff to have failed.
    fn abandon(self) {
        std::mem::forget(self);
    }
}

struct ShutdownWorker {
    handoff: Receiver<Teardown>,
    done: tokio::sync::oneshot::Sender<ShutdownOutcome>,
    budget: Duration,
}

impl ShutdownWorker {
    fn run(self) {
        let outcome = match self.handoff.recv() {
            Ok(Teardown { dispatch, provider }) => {
                let outcome = match provider.shutdown_with_timeout(self.budget) {
                    Ok(()) => ShutdownOutcome::Completed,
                    Err(_) => ShutdownOutcome::Incomplete,
                };
                drop(dispatch);
                drop(provider);
                outcome
            }
            Err(_) => ShutdownOutcome::Unavailable,
        };
        let _ = self.done.send(outcome);
    }
}

fn spawn_shutdown_thread(worker: ShutdownWorker) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(SHUTDOWN_THREAD_NAME.to_string())
        .spawn(move || worker.run())
        .map(drop)
}

fn spawn_teardown<S>(
    teardown: Teardown,
    budget: Duration,
    spawn_worker: S,
) -> Option<tokio::sync::oneshot::Receiver<ShutdownOutcome>>
where
    S: FnOnce(ShutdownWorker) -> std::io::Result<()>,
{
    let (done, done_receiver) = tokio::sync::oneshot::channel();
    let (handoff_sender, handoff) = sync_channel::<Teardown>(1);
    let worker = ShutdownWorker {
        handoff,
        done,
        budget,
    };
    if spawn_worker(worker).is_err() {
        teardown.abandon();
        return None;
    }
    if let Err(SendError(teardown)) = handoff_sender.send(teardown) {
        teardown.abandon();
        return None;
    }
    Some(done_receiver)
}

pub struct OtelTelemetry {
    teardown: Option<Teardown>,
    flush_budget: Duration,
}

impl OtelTelemetry {
    pub fn from_provider_with_budget(provider: SdkTracerProvider, flush_budget: Duration) -> Self {
        Self {
            teardown: Some(Teardown {
                dispatch: scoped_dispatch(&provider),
                provider,
            }),
            flush_budget,
        }
    }

    pub fn standalone(env: &impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        Self::with_endpoint(standalone_traces_endpoint(env), resolve_flush_budget(env))
    }

    pub fn with_endpoint(
        traces_endpoint: Option<String>,
        flush_budget: Duration,
    ) -> anyhow::Result<Self> {
        let provider = std::thread::Builder::new()
            .name(INIT_THREAD_NAME.to_string())
            .spawn(move || build_provider(traces_endpoint))?
            .join()
            .map_err(|_| anyhow::anyhow!("telemetry initialization panicked"))??;
        Ok(Self::from_provider_with_budget(provider, flush_budget))
    }

    pub async fn shutdown(self) -> ShutdownOutcome {
        self.shutdown_with(spawn_shutdown_thread).await
    }

    async fn shutdown_with<S>(mut self, spawn_worker: S) -> ShutdownOutcome
    where
        S: FnOnce(ShutdownWorker) -> std::io::Result<()>,
    {
        let Some(teardown) = self.teardown.take() else {
            return ShutdownOutcome::Unavailable;
        };
        let budget = self.flush_budget;
        let Some(done) = spawn_teardown(teardown, budget, spawn_worker) else {
            return ShutdownOutcome::Unavailable;
        };
        match tokio::time::timeout(budget, done).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) | Err(_) => ShutdownOutcome::Incomplete,
        }
    }
}

impl OtelTelemetry {
    fn release_with<S>(&mut self, spawn_worker: S)
    where
        S: FnOnce(ShutdownWorker) -> std::io::Result<()>,
    {
        if let Some(teardown) = self.teardown.take() {
            drop(spawn_teardown(teardown, self.flush_budget, spawn_worker));
        }
    }
}

impl Drop for OtelTelemetry {
    fn drop(&mut self) {
        self.release_with(spawn_shutdown_thread);
    }
}

impl Telemetry for OtelTelemetry {
    async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, CliError>>,
    {
        match &self.teardown {
            Some(teardown) => teardown.dispatch.scope(action()).await,
            None => action().await,
        }
    }
}

pub enum RuntimeTelemetry {
    Noop(NoopTelemetry),
    Otel(OtelTelemetry),
}

impl RuntimeTelemetry {
    pub fn disabled() -> Self {
        Self::Noop(NoopTelemetry)
    }

    pub async fn shutdown(self) {
        if let Self::Otel(telemetry) = self {
            telemetry.shutdown().await;
        }
    }
}

impl Telemetry for RuntimeTelemetry {
    async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, CliError>>,
    {
        match self {
            Self::Noop(telemetry) => telemetry.with_default_subscriber(action).await,
            Self::Otel(telemetry) => telemetry.with_default_subscriber(action).await,
        }
    }
}

pub fn select_runtime_telemetry(
    args: &[String],
    registry: &CommandRegistry,
    env: &impl Fn(&str) -> Option<String>,
) -> RuntimeTelemetry {
    #[cfg(feature = "telemetry-test-receiver")]
    if let Some(config) = test_receiver::resolve(env) {
        return OtelTelemetry::with_endpoint(Some(config.traces_endpoint()), config.flush_budget)
            .map_or_else(|_| RuntimeTelemetry::disabled(), RuntimeTelemetry::Otel);
    }
    let mode = resolve_telemetry_mode(env, || {
        parse_runtime_command(args.iter().cloned(), registry, None)
            .is_ok_and(|command| !command.is_hook_invocation())
    });
    match mode {
        TelemetryMode::Disabled => RuntimeTelemetry::disabled(),
        TelemetryMode::Standalone => OtelTelemetry::standalone(env)
            .map_or_else(|_| RuntimeTelemetry::disabled(), RuntimeTelemetry::Otel),
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
