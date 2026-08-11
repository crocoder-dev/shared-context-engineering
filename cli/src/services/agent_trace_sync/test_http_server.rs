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
