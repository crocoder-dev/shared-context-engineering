use std::future::Future;
use std::time::Duration;

use opentelemetry::logs::LoggerProvider as _;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{LogExporter, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::logs::{
    BatchConfigBuilder as LogBatchConfigBuilder, BatchLogProcessor, SdkLoggerProvider,
};
use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use tracing::instrument::WithSubscriber;
use tracing::{field, Dispatch, Instrument, Span};
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
pub const COMMAND_NAME_FIELD: &str = "sce.command.name";
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

#[derive(Debug)]
pub struct ManagedTraceTelemetry {
    provider: SdkTracerProvider,
    log_provider: SdkLoggerProvider,
    dispatch: Dispatch,
}

impl ManagedTraceTelemetry {
    pub fn new(
        bearer: &ManagedBearerToken,
        endpoints: ManagedOtlpEndpoints,
        runtime: tokio::runtime::Handle,
    ) -> anyhow::Result<Self> {
        let mut authorization = HeaderValue::from_str(&bearer.authorization_header_value())?;
        authorization.set_sensitive(true);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()?;
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
        })
    }

    pub async fn shutdown(self) {
        let provider = self.provider;
        let log_provider = self.log_provider;
        let task = tokio::task::spawn_blocking(move || {
            let _ = provider.shutdown_with_timeout(SHUTDOWN_TIMEOUT);
            let _ = log_provider.shutdown_with_timeout(SHUTDOWN_TIMEOUT);
        });
        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT + Duration::from_secs(1), task).await;
    }

    fn command_span(&self) -> Span {
        tracing::dispatcher::with_default(&self.dispatch, || {
            tracing::info_span!(
                target: COMMAND_SPAN_TARGET,
                COMMAND_SPAN_NAME,
                sce.command.name = field::Empty,
                sce.outcome = field::Empty,
                sce.error.class = field::Empty,
            )
        })
    }
}

impl Telemetry for ManagedTraceTelemetry {
    async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, CliError>>,
    {
        let span = self.command_span();
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

pub fn record_command_name(name: &str) {
    Span::current().record(COMMAND_NAME_FIELD, name);
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::services::managed_telemetry::{ManagedTelemetryMode, MANAGED_OTLP_DEFAULTS};

    struct CapturedRequest {
        head: String,
        body: Vec<u8>,
    }

    async fn capture_requests(
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

    fn loopback_telemetry(address: std::net::SocketAddr, token: &str) -> ManagedTraceTelemetry {
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
                },
            ),
            0,
        );
        let ManagedTelemetryMode::Managed { bearer, .. } = mode else {
            panic!("expected managed mode");
        };
        ManagedTraceTelemetry::new(&bearer, endpoints, tokio::runtime::Handle::current())
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
}
