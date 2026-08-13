//! A bespoke, in-repo, test-only HTTP/1.1 server used to exercise
//! `AuthenticatedControlPlaneClient` (and later the sync engine and CLI
//! wiring) against canned responses and captured requests, without adding a
//! `wiremock`/`httptest`-style dev-dependency to a crate that currently
//! declares none.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct CannedResponse {
    pub status: u16,
    pub body: String,
}

impl CannedResponse {
    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }
}

/// A single-threaded, sequential test HTTP server: each accepted connection
/// is answered with the next queued [`CannedResponse`] (or a `500` marker
/// response when the queue is empty), and every parsed request is recorded
/// for assertions.
pub struct TestHttpServer {
    pub base_url: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    responses: Arc<Mutex<VecDeque<CannedResponse>>>,
}

impl TestHttpServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let addr = listener.local_addr().expect("read test http server addr");

        let requests: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let responses: Arc<Mutex<VecDeque<CannedResponse>>> = Arc::new(Mutex::new(VecDeque::new()));

        let thread_requests = Arc::clone(&requests);
        let thread_responses = Arc::clone(&responses);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                handle_connection(stream, &thread_requests, &thread_responses);
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            requests,
            responses,
        }
    }

    pub fn queue_response(&self, response: CannedResponse) {
        self.responses
            .lock()
            .expect("test http server responses lock")
            .push_back(response);
    }

    pub fn captured_requests(&self) -> Vec<CapturedRequest> {
        self.requests
            .lock()
            .expect("test http server requests lock")
            .clone()
    }

    pub fn call_count(&self) -> usize {
        self.requests
            .lock()
            .expect("test http server requests lock")
            .len()
    }
}

/// A concurrent test server for sync regressions. State responses are queued
/// independently from dynamically selected batch responses, allowing each
/// stream's expected cursor to determine the response it receives.
pub struct ConcurrentBatchTestServer {
    pub base_url: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    state_response: Arc<Mutex<Option<CannedResponse>>>,
    batch_responses: Arc<Mutex<HashMap<(String, i64), CannedResponse>>>,
    metrics: Arc<Mutex<ConcurrentBatchMetrics>>,
}

#[derive(Clone, Debug, Default)]
struct ConcurrentBatchMetrics {
    state_requests: usize,
    in_flight: usize,
    max_in_flight: usize,
    in_flight_by_stream: HashMap<String, usize>,
    max_in_flight_by_stream: HashMap<String, usize>,
    expected_cursors_by_stream: HashMap<String, Vec<i64>>,
}

impl ConcurrentBatchTestServer {
    pub fn start(batch_delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind concurrent test http server");
        let addr = listener
            .local_addr()
            .expect("read concurrent test http server addr");

        let requests: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let state_response = Arc::new(Mutex::new(None));
        let batch_responses = Arc::new(Mutex::new(HashMap::new()));
        let metrics = Arc::new(Mutex::new(ConcurrentBatchMetrics::default()));

        let thread_requests = Arc::clone(&requests);
        let thread_state_response = Arc::clone(&state_response);
        let thread_batch_responses = Arc::clone(&batch_responses);
        let thread_metrics = Arc::clone(&metrics);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let requests = Arc::clone(&thread_requests);
                let state_response = Arc::clone(&thread_state_response);
                let batch_responses = Arc::clone(&thread_batch_responses);
                let metrics = Arc::clone(&thread_metrics);
                thread::spawn(move || {
                    handle_concurrent_connection(
                        stream,
                        &requests,
                        &state_response,
                        &batch_responses,
                        &metrics,
                        batch_delay,
                    );
                });
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            requests,
            state_response,
            batch_responses,
            metrics,
        }
    }

    pub fn queue_state_response(&self, response: CannedResponse) {
        *self
            .state_response
            .lock()
            .expect("concurrent test http server state response lock") = Some(response);
    }

    pub fn queue_batch_response(
        &self,
        stream: impl Into<String>,
        expected_cursor: i64,
        response: CannedResponse,
    ) {
        self.batch_responses
            .lock()
            .expect("concurrent test http server batch responses lock")
            .insert((stream.into(), expected_cursor), response);
    }

    pub fn captured_requests(&self) -> Vec<CapturedRequest> {
        self.requests
            .lock()
            .expect("concurrent test http server requests lock")
            .clone()
    }

    pub fn state_request_count(&self) -> usize {
        self.metrics
            .lock()
            .expect("concurrent test http server metrics lock")
            .state_requests
    }

    pub fn max_in_flight(&self) -> usize {
        self.metrics
            .lock()
            .expect("concurrent test http server metrics lock")
            .max_in_flight
    }

    pub fn max_in_flight_for(&self, stream: &str) -> usize {
        self.metrics
            .lock()
            .expect("concurrent test http server metrics lock")
            .max_in_flight_by_stream
            .get(stream)
            .copied()
            .unwrap_or(0)
    }

    pub fn expected_cursors_for(&self, stream: &str) -> Vec<i64> {
        self.metrics
            .lock()
            .expect("concurrent test http server metrics lock")
            .expected_cursors_by_stream
            .get(stream)
            .cloned()
            .unwrap_or_default()
    }
}

fn handle_connection(
    stream: std::net::TcpStream,
    requests: &Arc<Mutex<Vec<CapturedRequest>>>,
    responses: &Arc<Mutex<VecDeque<CannedResponse>>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone test http stream"));

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.insert(name, value);
        }
    }

    let mut body_bytes = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body_bytes).is_err() {
        return;
    }
    let body = String::from_utf8_lossy(&body_bytes).to_string();

    requests
        .lock()
        .expect("test http server requests lock")
        .push(CapturedRequest {
            method,
            path,
            headers,
            body,
        });

    let canned = responses
        .lock()
        .expect("test http server responses lock")
        .pop_front()
        .unwrap_or_else(|| CannedResponse {
            status: 500,
            body: r#"{"error":"no canned response queued"}"#.to_string(),
        });

    let mut stream = stream;
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        canned.status,
        status_reason(canned.status),
        canned.body.len(),
        canned.body,
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn handle_concurrent_connection(
    stream: std::net::TcpStream,
    requests: &Arc<Mutex<Vec<CapturedRequest>>>,
    state_response: &Arc<Mutex<Option<CannedResponse>>>,
    batch_responses: &Arc<Mutex<HashMap<(String, i64), CannedResponse>>>,
    metrics: &Arc<Mutex<ConcurrentBatchMetrics>>,
    batch_delay: Duration,
) {
    let Some(request) = read_request(&stream) else {
        return;
    };
    requests
        .lock()
        .expect("concurrent test http server requests lock")
        .push(request.clone());

    let response = match request.path.as_str() {
        "/agent-trace/ingestion/state" => {
            metrics
                .lock()
                .expect("concurrent test http server metrics lock")
                .state_requests += 1;
            state_response
                .lock()
                .expect("concurrent test http server state response lock")
                .clone()
                .unwrap_or_else(|| CannedResponse::text(500, "no state response queued"))
        }
        "/agent-trace/ingestion/batch" => {
            let body: serde_json::Value =
                serde_json::from_str(&request.body).unwrap_or_else(|_| serde_json::json!({}));
            let stream = body
                .get("stream")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let expected_cursor = body
                .get("expectedCursor")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(i64::MIN);

            {
                let mut metrics = metrics
                    .lock()
                    .expect("concurrent test http server metrics lock");
                metrics.in_flight += 1;
                metrics.max_in_flight = metrics.max_in_flight.max(metrics.in_flight);
                let stream_in_flight = {
                    let count = metrics
                        .in_flight_by_stream
                        .entry(stream.clone())
                        .or_default();
                    *count += 1;
                    *count
                };
                let stream_max_in_flight = metrics
                    .max_in_flight_by_stream
                    .entry(stream.clone())
                    .or_default();
                *stream_max_in_flight = (*stream_max_in_flight).max(stream_in_flight);
                metrics
                    .expected_cursors_by_stream
                    .entry(stream.clone())
                    .or_default()
                    .push(expected_cursor);
            }

            thread::sleep(batch_delay);
            let response = batch_responses
                .lock()
                .expect("concurrent test http server batch responses lock")
                .get(&(stream.clone(), expected_cursor))
                .cloned()
                .unwrap_or_else(|| CannedResponse::text(500, "no batch response queued"));
            let mut metrics = metrics
                .lock()
                .expect("concurrent test http server metrics lock");
            metrics.in_flight -= 1;
            if let Some(stream_in_flight) = metrics.in_flight_by_stream.get_mut(&stream) {
                *stream_in_flight -= 1;
            }
            drop(metrics);
            response
        }
        _ => CannedResponse::text(404, "unknown test route"),
    };

    write_response(stream, &response);
}

fn read_request(stream: &std::net::TcpStream) -> Option<CapturedRequest> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return None;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.insert(name, value);
        }
    }

    let mut body_bytes = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body_bytes).is_err() {
        return None;
    }

    Some(CapturedRequest {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body_bytes).to_string(),
    })
}

fn write_response(mut stream: std::net::TcpStream, canned: &CannedResponse) {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        canned.status,
        status_reason(canned.status),
        canned.body.len(),
        canned.body,
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        409 => "Conflict",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}
