use std::future::Future;
use std::time::Duration;

use opentelemetry::trace::{Status, TracerProvider};
use opentelemetry::{KeyValue, Value};
use opentelemetry_otlp::{Protocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanEvents, SpanExporter, SpanLinks};
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
const EXPORT_TIMEOUT: Duration = Duration::from_millis(750);

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
    SdkTracerProvider::builder()
        .with_resource(resource())
        .with_batch_exporter(BoundaryBExporter::new(exporter))
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

fn build_standalone_provider(endpoint: Option<String>) -> anyhow::Result<SdkTracerProvider> {
    let builder = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_http_client(http_client()?)
        .with_protocol(Protocol::HttpBinary)
        .with_timeout(EXPORT_TIMEOUT);
    let exporter = match endpoint {
        Some(endpoint) => builder.with_endpoint(endpoint),
        None => builder,
    }
    .build()?;
    Ok(provider_with_exporter(exporter))
}

pub struct OtelTelemetry {
    dispatch: ScopedDispatch,
    _provider: SdkTracerProvider,
}

impl OtelTelemetry {
    pub fn from_provider(provider: SdkTracerProvider) -> Self {
        Self {
            dispatch: scoped_dispatch(&provider),
            _provider: provider,
        }
    }

    pub fn standalone(env: &impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let endpoint = standalone_traces_endpoint(env);
        let provider = std::thread::Builder::new()
            .name("sce-otel-init".to_string())
            .spawn(move || build_standalone_provider(endpoint))?
            .join()
            .map_err(|_| anyhow::anyhow!("telemetry initialization panicked"))??;
        Ok(Self::from_provider(provider))
    }
}

impl Telemetry for OtelTelemetry {
    async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, CliError>>,
    {
        self.dispatch.scope(action()).await
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
mod tests;
