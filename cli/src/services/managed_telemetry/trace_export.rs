use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use opentelemetry::logs::LoggerProvider as _;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{TraceContextExt as _, TracerProvider as _};
use opentelemetry::{Context, KeyValue};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{LogExporter, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::logs::{
    BatchConfigBuilder as LogBatchConfigBuilder, BatchLogProcessor, SdkLoggerProvider,
};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use tracing::instrument::WithSubscriber;
use tracing::{field, Dispatch, Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, Registry};

use super::log_export::{is_exportable_event, LogBridgeLayer};
use super::{ManagedBearerToken, ManagedOtlpEndpoints};
use crate::services::error::CliError;
use crate::services::observability::traits::Telemetry;

const SERVICE_NAME: &str = "sce";
const TRACER_NAME: &str = "sce.cli";
pub const COMMAND_SPAN_TARGET: &str = "sce.managed.trace";
const COMMAND_SPAN_NAME: &str = "sce.command";
const HOOK_SPAN_NAME: &str = "sce.hook";
pub const COMMAND_NAME_FIELD: &str = "sce.command.name";
const AGENT_CLAUDE: &str = "claude";
const HOOK_EVENT_FIELD: &str = "sce.hook.event";
const HOOK_TOOL_FIELD: &str = "sce.hook.tool";
const CLAUDE_SESSION_ID_FIELD: &str = "sce.claude.session_id";
const CLAUDE_TOOL_USE_ID_FIELD: &str = "sce.claude.tool_use_id";
const CLAUDE_PAYLOAD_FIELDS: [(&str, &str); 4] = [
    ("hook_event_name", HOOK_EVENT_FIELD),
    ("tool_name", HOOK_TOOL_FIELD),
    ("session_id", CLAUDE_SESSION_ID_FIELD),
    ("tool_use_id", CLAUDE_TOOL_USE_ID_FIELD),
];
const TRACEPARENT_ENV: &str = "TRACEPARENT";
const TRACESTATE_ENV: &str = "TRACESTATE";
const TRACEPARENT_HEADER: &str = "traceparent";
const TRACESTATE_HEADER: &str = "tracestate";
const OUTCOME_FIELD: &str = "sce.outcome";
const ERROR_CLASS_FIELD: &str = "sce.error.class";
const OUTCOME_SUCCESS: &str = "success";
const OUTCOME_FAILURE: &str = "failure";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const EXPORT_TIMEOUT: Duration = Duration::from_secs(3);
const BATCH_QUEUE_SIZE: usize = 64;
const BATCH_EXPORT_SIZE: usize = 32;
const SCHEDULED_DELAY: Duration = Duration::from_millis(500);
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
struct PinnedBearerClient {
    client: reqwest::Client,
    runtime: tokio::runtime::Handle,
    endpoint: &'static str,
    authorization: HeaderValue,
}

#[async_trait::async_trait]
impl HttpClient for PinnedBearerClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let content_type = request.headers().get(CONTENT_TYPE).cloned();
        let body = request.into_body();
        let mut builder = self
            .client
            .post(self.endpoint)
            .header(AUTHORIZATION, self.authorization.clone())
            .body(body);
        if let Some(content_type) = content_type {
            builder = builder.header(CONTENT_TYPE, content_type);
        }
        let status = self
            .runtime
            .spawn(async move { builder.send().await.map(|response| response.status()) })
            .await??;
        let response = Response::builder().status(status).body(Bytes::new())?;
        Ok(response)
    }
}

fn is_command_span(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.is_span() && metadata.target() == COMMAND_SPAN_TARGET
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedSpanKind {
    Command,
    ClaudeHook,
}

#[derive(Debug)]
pub struct ManagedTraceTelemetry {
    provider: SdkTracerProvider,
    log_provider: SdkLoggerProvider,
    dispatch: Dispatch,
    span_kind: ManagedSpanKind,
    parent_context: Option<Context>,
}

impl ManagedTraceTelemetry {
    pub fn new(
        bearer: &ManagedBearerToken,
        endpoints: ManagedOtlpEndpoints,
        runtime: tokio::runtime::Handle,
        span_kind: ManagedSpanKind,
    ) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        let mut telemetry = Self::with_client(bearer, endpoints, runtime, client, span_kind)?;
        if span_kind == ManagedSpanKind::ClaudeHook {
            telemetry.parent_context = extract_parent_context(
                std::env::var(TRACEPARENT_ENV).ok(),
                std::env::var(TRACESTATE_ENV).ok(),
            );
        }
        Ok(telemetry)
    }

    fn with_client(
        bearer: &ManagedBearerToken,
        endpoints: ManagedOtlpEndpoints,
        runtime: tokio::runtime::Handle,
        client: reqwest::Client,
        span_kind: ManagedSpanKind,
    ) -> anyhow::Result<Self> {
        let mut authorization = HeaderValue::from_str(&bearer.authorization_header_value())?;
        authorization.set_sensitive(true);
        let http_client = PinnedBearerClient {
            client: client.clone(),
            runtime: runtime.clone(),
            endpoint: endpoints.traces_endpoint,
            authorization: authorization.clone(),
        };
        let log_http_client = PinnedBearerClient {
            client,
            runtime,
            endpoint: endpoints.logs_endpoint,
            authorization,
        };
        let exporter = SpanExporter::builder()
            .with_http()
            .with_http_client(http_client)
            .with_endpoint(endpoints.traces_endpoint)
            .with_timeout(EXPORT_TIMEOUT)
            .build()?;
        let batch_config = BatchConfigBuilder::default()
            .with_max_queue_size(BATCH_QUEUE_SIZE)
            .with_max_export_batch_size(BATCH_EXPORT_SIZE)
            .with_scheduled_delay(SCHEDULED_DELAY)
            .build();
        let processor = BatchSpanProcessor::builder(exporter)
            .with_batch_config(batch_config)
            .build();
        let resource = Resource::builder_empty()
            .with_attributes([
                KeyValue::new("service.name", SERVICE_NAME),
                KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            ])
            .build();
        let resource_for_logs = resource.clone();
        let provider = SdkTracerProvider::builder()
            .with_resource(resource)
            .with_span_processor(processor)
            .build();
        let log_exporter = LogExporter::builder()
            .with_http()
            .with_http_client(log_http_client)
            .with_endpoint(endpoints.logs_endpoint)
            .with_timeout(EXPORT_TIMEOUT)
            .build()?;
        let log_batch_config = LogBatchConfigBuilder::default()
            .with_max_queue_size(BATCH_QUEUE_SIZE)
            .with_max_export_batch_size(BATCH_EXPORT_SIZE)
            .with_scheduled_delay(SCHEDULED_DELAY)
            .build();
        let log_provider = SdkLoggerProvider::builder()
            .with_resource(resource_for_logs)
            .with_log_processor(
                BatchLogProcessor::builder(log_exporter)
                    .with_batch_config(log_batch_config)
                    .build(),
            )
            .build();
        let span_layer = tracing_opentelemetry::layer()
            .with_tracer(provider.tracer(TRACER_NAME))
            .with_filter(filter_fn(
                is_command_span as fn(&tracing::Metadata<'_>) -> bool,
            ));
        let log_layer = LogBridgeLayer::new(log_provider.logger(TRACER_NAME)).with_filter(
            filter_fn(is_exportable_event as fn(&tracing::Metadata<'_>) -> bool),
        );
        let dispatch = Dispatch::new(Registry::default().with(span_layer).with(log_layer));
        Ok(Self {
            provider,
            log_provider,
            dispatch,
            span_kind,
            parent_context: None,
        })
    }

    pub async fn shutdown(self) {
        let deadline = tokio::time::Instant::now() + SHUTDOWN_TIMEOUT;
        let Self {
            provider,
            log_provider,
            dispatch,
            ..
        } = self;
        drop(dispatch);
        let traces =
            tokio::task::spawn_blocking(move || provider.shutdown_with_timeout(SHUTDOWN_TIMEOUT));
        let logs = tokio::task::spawn_blocking(move || {
            log_provider.shutdown_with_timeout(SHUTDOWN_TIMEOUT)
        });
        let _ = tokio::time::timeout_at(deadline, async {
            let _ = traces.await;
            let _ = logs.await;
        })
        .await;
    }

    fn root_span(&self) -> Span {
        let span = self.unparented_root_span();
        if let Some(parent_context) = &self.parent_context {
            let _ = span.set_parent(parent_context.clone());
        }
        span
    }

    fn unparented_root_span(&self) -> Span {
        tracing::dispatcher::with_default(&self.dispatch, || match self.span_kind {
            ManagedSpanKind::Command => tracing::info_span!(
                target: COMMAND_SPAN_TARGET,
                COMMAND_SPAN_NAME,
                sce.command.name = field::Empty,
                sce.outcome = field::Empty,
                sce.error.class = field::Empty,
            ),
            ManagedSpanKind::ClaudeHook => tracing::info_span!(
                target: COMMAND_SPAN_TARGET,
                HOOK_SPAN_NAME,
                sce.agent = AGENT_CLAUDE,
                sce.hook.event = field::Empty,
                sce.hook.tool = field::Empty,
                sce.claude.session_id = field::Empty,
                sce.claude.tool_use_id = field::Empty,
                sce.outcome = field::Empty,
                sce.error.class = field::Empty,
            ),
        })
    }
}

impl Telemetry for ManagedTraceTelemetry {
    async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, CliError>>,
    {
        let span = self.root_span();
        let result = action()
            .instrument(span.clone())
            .with_subscriber(self.dispatch.clone())
            .await;
        match &result {
            Ok(_) => {
                span.record(OUTCOME_FIELD, OUTCOME_SUCCESS);
            }
            Err(error) => {
                span.record(OUTCOME_FIELD, OUTCOME_FAILURE);
                span.record(ERROR_CLASS_FIELD, error.code());
            }
        }
        drop(span);
        result
    }
}

fn extract_parent_context(
    traceparent: Option<String>,
    tracestate: Option<String>,
) -> Option<Context> {
    let traceparent = traceparent?;
    let mut carrier = HashMap::from([(TRACEPARENT_HEADER.to_string(), traceparent)]);
    if let Some(tracestate) = tracestate {
        carrier.insert(TRACESTATE_HEADER.to_string(), tracestate);
    }
    let context = TraceContextPropagator::new().extract(&carrier);
    context.span().span_context().is_valid().then_some(context)
}

pub fn record_command_name(name: &str) {
    Span::current().record(COMMAND_NAME_FIELD, name);
}

pub fn record_claude_hook_payload(stdin_payload: &str) {
    let span = Span::current();
    if span.is_disabled() {
        return;
    }
    let Ok(serde_json::Value::Object(payload)) = serde_json::from_str(stdin_payload) else {
        return;
    };
    for (payload_key, span_field) in CLAUDE_PAYLOAD_FIELDS {
        if let Some(value) = payload
            .get(payload_key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        {
            span.record(span_field, value);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::services::managed_telemetry::{ManagedTelemetryMode, MANAGED_OTLP_DEFAULTS};

    pub(crate) struct CapturedRequest {
        pub(crate) head: String,
        pub(crate) body: Vec<u8>,
    }

    pub(crate) async fn capture_requests(
        listener: TcpListener,
        status_line: &'static str,
        expected: usize,
    ) -> Vec<CapturedRequest> {
        let mut captured = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while captured.len() < expected {
            let Ok(request) =
                tokio::time::timeout_at(deadline, capture_one_request(&listener, status_line))
                    .await
            else {
                break;
            };
            captured.push(request);
        }
        captured
    }

    async fn capture_one_request(
        listener: &TcpListener,
        status_line: &'static str,
    ) -> CapturedRequest {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.expect("read");
            assert!(read > 0, "connection closed before headers");
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let content_length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while buffer.len() < header_end + content_length {
            let read = stream.read(&mut chunk).await.expect("read body");
            assert!(read > 0, "connection closed before body");
            buffer.extend_from_slice(&chunk[..read]);
        }
        let response = format!("{status_line}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        stream
            .write_all(response.as_bytes())
            .await
            .expect("respond");
        CapturedRequest {
            head,
            body: buffer[header_end..].to_vec(),
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct DecodedSpan {
        pub(crate) name: String,
        pub(crate) trace_id: Vec<u8>,
        pub(crate) span_id: Vec<u8>,
        pub(crate) parent_span_id: Vec<u8>,
        pub(crate) attributes: Vec<(String, String)>,
        pub(crate) resource_attributes: Vec<(String, String)>,
    }

    impl DecodedSpan {
        pub(crate) fn sce_attributes(&self) -> Vec<(String, String)> {
            let mut attributes = self
                .attributes
                .iter()
                .filter(|(key, _)| key.starts_with("sce."))
                .cloned()
                .collect::<Vec<_>>();
            attributes.sort();
            attributes
        }
    }

    fn read_varint(bytes: &[u8], position: &mut usize) -> u64 {
        let mut value = 0_u64;
        let mut shift = 0;
        loop {
            let byte = bytes[*position];
            *position += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    }

    fn length_delimited_fields(bytes: &[u8]) -> Vec<(u64, &[u8])> {
        let mut fields = Vec::new();
        let mut position = 0;
        while position < bytes.len() {
            let key = read_varint(bytes, &mut position);
            match key & 0x7 {
                0 => {
                    read_varint(bytes, &mut position);
                }
                1 => position += 8,
                2 => {
                    let length = usize::try_from(read_varint(bytes, &mut position)).unwrap();
                    fields.push((key >> 3, &bytes[position..position + length]));
                    position += length;
                }
                5 => position += 4,
                wire => panic!("unsupported protobuf wire type {wire}"),
            }
        }
        fields
    }

    fn fields_numbered(bytes: &[u8], number: u64) -> impl Iterator<Item = &[u8]> {
        length_delimited_fields(bytes)
            .into_iter()
            .filter(move |(field, _)| *field == number)
            .map(|(_, value)| value)
    }

    fn decode_string_attribute(bytes: &[u8]) -> Option<(String, String)> {
        let key = fields_numbered(bytes, 1).next()?;
        let value = fields_numbered(bytes, 2).next()?;
        let string = fields_numbered(value, 1).next()?;
        Some((
            String::from_utf8_lossy(key).to_string(),
            String::from_utf8_lossy(string).to_string(),
        ))
    }

    pub(crate) fn decode_spans(body: &[u8]) -> Vec<DecodedSpan> {
        let mut spans = Vec::new();
        for resource_spans in fields_numbered(body, 1) {
            let resource_attributes = fields_numbered(resource_spans, 1)
                .flat_map(|resource| fields_numbered(resource, 1))
                .filter_map(decode_string_attribute)
                .collect::<Vec<_>>();
            for scope_spans in fields_numbered(resource_spans, 2) {
                for span in fields_numbered(scope_spans, 2) {
                    let mut decoded = DecodedSpan {
                        resource_attributes: resource_attributes.clone(),
                        ..DecodedSpan::default()
                    };
                    for (field, value) in length_delimited_fields(span) {
                        match field {
                            1 => decoded.trace_id = value.to_vec(),
                            2 => decoded.span_id = value.to_vec(),
                            4 => decoded.parent_span_id = value.to_vec(),
                            5 => decoded.name = String::from_utf8_lossy(value).to_string(),
                            9 => decoded.attributes.extend(decode_string_attribute(value)),
                            _ => {}
                        }
                    }
                    spans.push(decoded);
                }
            }
        }
        spans
    }

    pub(crate) fn trace_spans(requests: &[CapturedRequest]) -> Vec<DecodedSpan> {
        requests
            .iter()
            .filter(|request| {
                request
                    .head
                    .to_ascii_lowercase()
                    .starts_with("post /v1/traces ")
            })
            .flat_map(|request| decode_spans(&request.body))
            .collect()
    }

    pub(crate) fn loopback_telemetry(
        address: std::net::SocketAddr,
        token: &str,
    ) -> ManagedTraceTelemetry {
        loopback_telemetry_of_kind(address, token, ManagedSpanKind::Command)
    }

    pub(crate) fn loopback_hook_telemetry(
        address: std::net::SocketAddr,
        token: &str,
    ) -> ManagedTraceTelemetry {
        loopback_telemetry_of_kind(address, token, ManagedSpanKind::ClaudeHook)
    }

    fn loopback_hook_telemetry_with_traceparent(
        address: std::net::SocketAddr,
        traceparent: Option<&str>,
    ) -> ManagedTraceTelemetry {
        let mut telemetry = loopback_hook_telemetry(address, "bao-token-value");
        telemetry.parent_context = extract_parent_context(
            traceparent.map(str::to_string),
            Some("vendor=opaque".to_string()),
        );
        telemetry
    }

    const UPSTREAM_TRACE_ID: [u8; 16] = [
        0x4b, 0xf9, 0x2f, 0x35, 0x77, 0xb3, 0x4d, 0xa6, 0xa3, 0xce, 0x92, 0x9d, 0x0e, 0x0e, 0x47,
        0x36,
    ];
    const UPSTREAM_SPAN_ID: [u8; 8] = [0x00, 0xf0, 0x67, 0xaa, 0x0b, 0xa9, 0x02, 0xb7];
    const UPSTREAM_TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    async fn export_hook_spans(
        traceparent: Option<&str>,
        action: impl Future<Output = ()> + Send + 'static,
    ) -> Vec<DecodedSpan> {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let receiver = tokio::spawn(capture_requests(listener, "HTTP/1.1 200 OK", 1));
        let telemetry = loopback_hook_telemetry_with_traceparent(address, traceparent);
        let mut action = Some(action);
        let result = telemetry
            .with_default_subscriber(&mut || {
                let action = action.take().expect("single invocation");
                async move {
                    action.await;
                    Ok(String::new())
                }
            })
            .await;
        assert_eq!(result.expect("hook result"), "");
        telemetry.shutdown().await;
        trace_spans(&receiver.await.expect("receiver join"))
    }

    fn loopback_telemetry_of_kind(
        address: std::net::SocketAddr,
        token: &str,
        span_kind: ManagedSpanKind,
    ) -> ManagedTraceTelemetry {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .tls_certs_only(Vec::new())
            .build()
            .expect("test http client");
        loopback_telemetry_with_client(address, token, client, span_kind)
    }

    fn loopback_telemetry_with_client(
        address: std::net::SocketAddr,
        token: &str,
        client: reqwest::Client,
        span_kind: ManagedSpanKind,
    ) -> ManagedTraceTelemetry {
        let traces_endpoint: &'static str =
            Box::leak(format!("http://{address}/v1/traces").into_boxed_str());
        let logs_endpoint: &'static str =
            Box::leak(format!("http://{address}/v1/logs").into_boxed_str());
        let endpoints = ManagedOtlpEndpoints {
            traces_endpoint,
            logs_endpoint,
            ..MANAGED_OTLP_DEFAULTS
        };
        let mode = crate::services::managed_telemetry::select_mode(
            Some(
                crate::services::telemetry_credential_storage::TelemetryCredential {
                    token: token.to_string(),
                    accessor: "local-accessor-secret".to_string(),
                    expires_at_unix_seconds: u64::MAX,
                    identity: crate::services::telemetry_credential_storage::TelemetryIdentity {
                        user_id: "user_a".to_string(),
                        workspace_id: "workspace_a".to_string(),
                    },
                },
            ),
            0,
        );
        let ManagedTelemetryMode::Managed { bearer, .. } = mode else {
            panic!("expected managed mode");
        };
        ManagedTraceTelemetry::with_client(
            &bearer,
            endpoints,
            tokio::runtime::Handle::current(),
            client,
            span_kind,
        )
        .expect("telemetry")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn managed_trace_export_posts_allowlisted_span_with_bearer_only() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let receiver = tokio::spawn(capture_requests(listener, "HTTP/1.1 200 OK", 2));
        let telemetry = loopback_telemetry(address, "bao-token-value");

        let result = telemetry
            .with_default_subscriber(&mut || async {
                record_command_name("config");
                let fields_json = r#"{"command":"config","argv":"--token secret-arg","repo":"/home/u/repo","error_source":"Bearer leak-value"}"#;
                tracing::info!(target: "sce", event_id = "sce.test.event", event_message = "Starting command", fields = %fields_json);
                tokio::task::yield_now().await;
                Ok("payload".to_string())
            })
            .await;
        assert_eq!(result.expect("command result"), "payload");
        telemetry.shutdown().await;

        let requests = receiver.await.expect("receiver join");
        assert_eq!(
            requests.len(),
            2,
            "{:?}",
            requests
                .iter()
                .map(|r| r.head.lines().next().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        );
        for request in &requests {
            let head = request.head.to_ascii_lowercase();
            assert!(head.contains("authorization: bearer bao-token-value"));
            assert!(head.contains("application/x-protobuf"));
            assert!(!head.contains("local-accessor-secret"));
        }
        let trace = requests
            .iter()
            .find(|request| {
                request
                    .head
                    .to_ascii_lowercase()
                    .starts_with("post /v1/traces ")
            })
            .expect("trace request");
        let log = requests
            .iter()
            .find(|request| {
                request
                    .head
                    .to_ascii_lowercase()
                    .starts_with("post /v1/logs ")
            })
            .expect("log request");
        let log_body = String::from_utf8_lossy(&log.body).to_string();
        for expected in [
            "sce.test.event",
            "Starting command",
            "config",
            "service.name",
        ] {
            assert!(log_body.contains(expected), "missing log {expected}");
        }
        for leaked in [
            "secret-arg",
            "/home/u/repo",
            "leak-value",
            "bao-token-value",
            "local-accessor-secret",
        ] {
            assert!(!log_body.contains(leaked), "leaked {leaked}");
        }
        let body = String::from_utf8_lossy(&trace.body).to_string();
        for expected in [
            "sce.command",
            "sce.command.name",
            "config",
            "service.name",
            "success",
        ] {
            assert!(body.contains(expected), "missing {expected}");
        }
        assert!(!body.contains("local-accessor-secret"));
        assert!(!body.contains("bao-token-value"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claude_hook_export_posts_one_hook_span_with_correlation_ids_only() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let receiver = tokio::spawn(capture_requests(listener, "HTTP/1.1 200 OK", 1));
        let telemetry = loopback_hook_telemetry(address, "bao-token-value");
        let payload = r#"{"session_id":"session-7d07","transcript_path":"/home/u/private-repo/transcript.jsonl","cwd":"/home/u/private-repo","prompt":"PROMPT_TEXT_VALUE","hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"/home/u/private-repo/src/secret.rs","content":"FILE_CONTENT_VALUE Bearer leak-value"},"tool_response":{"success":true,"output":"TOOL_RESPONSE_VALUE"},"tool_use_id":"toolu_01KN1s"}"#;

        let result = telemetry
            .with_default_subscriber(&mut || async {
                record_command_name("hooks");
                record_claude_hook_payload(payload);
                tokio::task::yield_now().await;
                Ok(String::new())
            })
            .await;
        assert_eq!(result.expect("hook result"), "");
        telemetry.shutdown().await;

        let requests = receiver.await.expect("receiver join");
        let spans = trace_spans(&requests);
        assert_eq!(spans.len(), 1, "{spans:?}");
        let span = &spans[0];
        assert_eq!(span.name, "sce.hook");
        assert!(span.parent_span_id.is_empty());
        assert!(span
            .resource_attributes
            .contains(&("service.name".to_string(), "sce".to_string())));
        let expected = [
            ("sce.agent", "claude"),
            ("sce.claude.session_id", "session-7d07"),
            ("sce.claude.tool_use_id", "toolu_01KN1s"),
            ("sce.hook.event", "PostToolUse"),
            ("sce.hook.tool", "Write"),
            ("sce.outcome", "success"),
        ]
        .map(|(key, value)| (key.to_string(), value.to_string()));
        assert_eq!(span.sce_attributes(), expected);
        for request in &requests {
            let body = String::from_utf8_lossy(&request.body).to_string();
            for leaked in [
                "private-repo",
                "PROMPT_TEXT_VALUE",
                "FILE_CONTENT_VALUE",
                "TOOL_RESPONSE_VALUE",
                "leak-value",
                "bao-token-value",
                "local-accessor-secret",
                "sce.command",
            ] {
                assert!(!body.contains(leaked), "leaked {leaked}");
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claude_hook_span_joins_valid_upstream_trace_with_fresh_span_id() {
        let spans = export_hook_spans(Some(UPSTREAM_TRACEPARENT), async {
            tokio::task::yield_now().await;
        })
        .await;
        assert_eq!(spans.len(), 1, "{spans:?}");
        let span = &spans[0];
        assert_eq!(span.name, "sce.hook");
        assert_eq!(span.trace_id, UPSTREAM_TRACE_ID);
        assert_eq!(span.parent_span_id, UPSTREAM_SPAN_ID);
        assert_eq!(span.span_id.len(), 8);
        assert_ne!(span.span_id, UPSTREAM_SPAN_ID);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claude_hook_span_starts_independent_root_without_usable_traceparent() {
        for traceparent in [
            None,
            Some(""),
            Some("not-a-traceparent"),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7"),
            Some("00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01"),
            Some("ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
            Some("00-00000000000000000000000000000000-00f067aa0ba902b7-01"),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01"),
        ] {
            let spans = export_hook_spans(traceparent, async {}).await;
            assert_eq!(spans.len(), 1, "{traceparent:?}: {spans:?}");
            let span = &spans[0];
            assert_eq!(span.name, "sce.hook");
            assert!(span.parent_span_id.is_empty(), "{traceparent:?}");
            assert_eq!(span.trace_id.len(), 16, "{traceparent:?}");
            assert_ne!(span.trace_id, UPSTREAM_TRACE_ID, "{traceparent:?}");
            assert!(
                span.trace_id.iter().any(|byte| *byte != 0),
                "{traceparent:?}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn claude_hook_child_spans_inherit_upstream_trace_across_polls_and_workers() {
        let spans = export_hook_spans(Some(UPSTREAM_TRACEPARENT), async {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
            let child = tracing::info_span!(target: COMMAND_SPAN_TARGET, "sce.test.child");
            async {
                tokio::task::yield_now().await;
                tokio::spawn(
                    async {
                        tokio::task::yield_now().await;
                        let grandchild =
                            tracing::info_span!(target: COMMAND_SPAN_TARGET, "sce.test.grandchild");
                        async {
                            for _ in 0..4 {
                                tokio::task::yield_now().await;
                            }
                        }
                        .instrument(grandchild)
                        .await;
                    }
                    .in_current_span()
                    .with_current_subscriber(),
                )
                .await
                .expect("spawned child join");
                tokio::task::yield_now().await;
            }
            .instrument(child)
            .await;
        })
        .await;
        assert_eq!(spans.len(), 3, "{spans:?}");
        let by_name = |name: &str| {
            spans
                .iter()
                .find(|span| span.name == name)
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        let hook = by_name("sce.hook");
        let child = by_name("sce.test.child");
        let grandchild = by_name("sce.test.grandchild");
        for span in [hook, child, grandchild] {
            assert_eq!(span.trace_id, UPSTREAM_TRACE_ID, "{}", span.name);
        }
        assert_eq!(hook.parent_span_id, UPSTREAM_SPAN_ID);
        assert_eq!(child.parent_span_id, hook.span_id);
        assert_eq!(grandchild.parent_span_id, child.span_id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn managed_trace_export_does_not_follow_redirects_or_alter_results() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let receiver = tokio::spawn(capture_requests(
            listener,
            "HTTP/1.1 307 Temporary Redirect\r\nlocation: http://127.0.0.1:1/steal",
            1,
        ));
        let telemetry = loopback_telemetry(address, "bao-token-value");

        let result = telemetry
            .with_default_subscriber(&mut || async {
                Err(CliError::runtime(anyhow::anyhow!("boom")))
            })
            .await;
        assert!(result.is_err());
        telemetry.shutdown().await;

        let requests = receiver.await.expect("receiver join");
        assert!(!requests.is_empty());
        for request in &requests {
            let head = request.head.to_ascii_lowercase();
            assert!(head.starts_with("post /v1/traces ") || head.starts_with("post /v1/logs "));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreachable_collector_does_not_block_command_or_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        drop(listener);
        let telemetry = loopback_telemetry(address, "bao-token-value");

        let result = telemetry
            .with_default_subscriber(&mut || async { Ok("ok".to_string()) })
            .await;
        assert_eq!(result.expect("result"), "ok");
        let started = std::time::Instant::now();
        telemetry.shutdown().await;
        assert!(started.elapsed() < Duration::from_secs(6));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ambient_otlp_environment_cannot_redirect_or_inject_headers() {
        let alternate = TcpListener::bind("127.0.0.1:0").await.expect("bind alt");
        let alternate_address = alternate.local_addr().expect("alt addr");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        std::env::set_var(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            format!("http://{alternate_address}"),
        );
        std::env::set_var(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            format!("http://{alternate_address}/v1/traces"),
        );
        std::env::set_var("OTEL_EXPORTER_OTLP_HEADERS", "x-ambient-leak=1");
        std::env::set_var("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc");
        let receiver = tokio::spawn(capture_requests(listener, "HTTP/1.1 200 OK", 2));
        let telemetry = loopback_telemetry(address, "bao-token-value");

        let result = telemetry
            .with_default_subscriber(&mut || async {
                record_command_name("config");
                tracing::info!(target: "sce", event_id = "sce.test.event", event_message = "Starting command");
                Ok("payload".to_string())
            })
            .await;
        assert_eq!(result.expect("command result"), "payload");
        telemetry.shutdown().await;
        for name in [
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            "OTEL_EXPORTER_OTLP_HEADERS",
            "OTEL_EXPORTER_OTLP_PROTOCOL",
        ] {
            std::env::remove_var(name);
        }

        let requests = receiver.await.expect("receiver join");
        assert_eq!(requests.len(), 2);
        for request in &requests {
            let head = request.head.to_ascii_lowercase();
            assert!(head.contains("authorization: bearer bao-token-value"));
            assert!(!head.contains("x-ambient-leak"));
            assert!(head.contains("application/x-protobuf"));
        }
        let unexpected = tokio::time::timeout(Duration::from_millis(300), alternate.accept()).await;
        assert!(unexpected.is_err(), "alternate receiver was contacted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stalled_collector_does_not_block_command_and_shutdown_is_bounded() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let holder = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        let telemetry = loopback_telemetry(address, "bao-token-value");

        let command_started = std::time::Instant::now();
        let result = telemetry
            .with_default_subscriber(&mut || async { Ok("ok".to_string()) })
            .await;
        assert_eq!(result.expect("result"), "ok");
        assert!(command_started.elapsed() < Duration::from_secs(1));
        let started = std::time::Instant::now();
        telemetry.shutdown().await;
        assert!(started.elapsed() < Duration::from_secs(6));
        holder.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_commands_keep_results_and_export_isolated() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let receiver = tokio::spawn(capture_requests(listener, "HTTP/1.1 200 OK", 1));
        let telemetry = std::sync::Arc::new(loopback_telemetry(address, "bao-token-value"));

        let first = {
            let telemetry = telemetry.clone();
            tokio::spawn(async move {
                telemetry
                    .with_default_subscriber(&mut || async {
                        record_command_name("first");
                        tokio::task::yield_now().await;
                        Ok("one".to_string())
                    })
                    .await
            })
        };
        let second = {
            let telemetry = telemetry.clone();
            tokio::spawn(async move {
                telemetry
                    .with_default_subscriber(&mut || async {
                        record_command_name("second");
                        tokio::task::yield_now().await;
                        Err(CliError::runtime(anyhow::anyhow!("boom")))
                    })
                    .await
            })
        };
        assert_eq!(first.await.expect("first join").expect("first"), "one");
        assert!(second.await.expect("second join").is_err());
        std::sync::Arc::try_unwrap(telemetry)
            .expect("sole owner")
            .shutdown()
            .await;

        let requests = receiver.await.expect("receiver join");
        let trace = requests
            .iter()
            .find(|request| {
                request
                    .head
                    .to_ascii_lowercase()
                    .starts_with("post /v1/traces ")
            })
            .expect("trace request");
        let body = String::from_utf8_lossy(&trace.body).to_string();
        assert!(body.contains("first"));
        assert!(body.contains("second"));
        assert!(body.contains("success"));
        assert!(body.contains("failure"));
    }

    #[test]
    fn stalled_exporter_shutdown_cannot_extend_process_lifetime() {
        let (elapsed, _) = crate::app::tests::run_fault_child(
            "services::managed_telemetry::trace_export::tests::isolated_stalled_exporter_shutdown",
        );
        let bound = SHUTDOWN_TIMEOUT + Duration::from_secs(2);
        assert!(elapsed < bound, "child exited after {elapsed:?}");
    }

    #[test]
    fn isolated_stalled_exporter_shutdown() {
        if !crate::app::tests::is_fault_child() {
            return;
        }
        let runtime = crate::app::build_runtime().expect("runtime");
        runtime.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let address = listener.local_addr().expect("addr");
            tokio::spawn(async move {
                let mut held = Vec::new();
                while let Ok((stream, _)) = listener.accept().await {
                    held.push(stream);
                }
            });
            let unbounded_client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .tls_certs_only(Vec::new())
                .build()
                .expect("test http client");
            let telemetry =
                loopback_telemetry_with_client(
                    address,
                    "bao-token-value",
                    unbounded_client,
                    ManagedSpanKind::Command,
                );
            let result = telemetry
                .with_default_subscriber(&mut || async {
                    record_command_name("config");
                    tracing::error!(target: "sce", event_id = "sce.test.event", event_message = "failed");
                    Err(CliError::runtime(anyhow::anyhow!("boom")))
                })
                .await;
            assert!(result.is_err());
            let started = std::time::Instant::now();
            telemetry.shutdown().await;
            assert!(started.elapsed() < SHUTDOWN_TIMEOUT + Duration::from_millis(500));
        });
        crate::app::finish_runtime(runtime);
    }
}
