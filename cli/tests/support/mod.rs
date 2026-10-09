#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SCE: &str = env!("CARGO_BIN_EXE_sce");
pub const HARD_TIMEOUT: Duration = Duration::from_secs(5);
pub const NON_ROUTABLE_ENDPOINT: &str = "http://192.0.2.1:4318";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observed {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

pub struct Run {
    pub observed: Observed,
    pub elapsed: Duration,
}

pub struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn marker_file(&self, label: &str) -> PathBuf {
        self.dir.path().join(format!("{label}.lifecycle"))
    }

    pub fn run(&self, args: &[&str], envs: &[(&str, String)]) -> Run {
        self.try_run(args, envs)
            .unwrap_or_else(|dump| panic!("{dump}"))
    }

    pub fn try_run(&self, args: &[&str], envs: &[(&str, String)]) -> Result<Run, String> {
        let mut command = Command::new(SCE);
        command
            .args(args)
            .current_dir(self.path())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.path())
            .env("XDG_CONFIG_HOME", self.path().join("config"))
            .env("XDG_STATE_HOME", self.path().join("state"))
            .env("XDG_CACHE_HOME", self.path().join("cache"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in envs {
            command.env(key, value);
        }
        let started = Instant::now();
        let mut child = command.spawn().unwrap();
        let stdout = drain(child.stdout.take().unwrap());
        let stderr = drain(child.stderr.take().unwrap());
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > HARD_TIMEOUT {
                let dump = thread_dump(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "sce {args:?} hung past the {HARD_TIMEOUT:?} hard timeout\n{dump}"
                ));
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let elapsed = started.elapsed();
        Ok(Run {
            observed: Observed {
                stdout: stdout.join().unwrap(),
                stderr: stderr.join().unwrap(),
                code: status.code(),
            },
            elapsed,
        })
    }
}

fn thread_dump(pid: u32) -> String {
    let mut lines = Vec::new();
    if let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/task")) {
        for entry in entries.flatten() {
            let base = entry.path();
            let read = |name: &str| {
                std::fs::read_to_string(base.join(name))
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            let state = read("status")
                .lines()
                .find(|line| line.starts_with("State:"))
                .unwrap_or("")
                .to_string();
            lines.push(format!(
                "tid={} comm={} wchan={} {}",
                entry.file_name().to_string_lossy(),
                read("comm"),
                read("wchan"),
                state
            ));
        }
    }
    lines.join("\n")
}

fn drain(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stream.read_to_end(&mut buffer);
        String::from_utf8_lossy(&buffer).into_owned()
    })
}

#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub head: String,
    pub body: Vec<u8>,
}

pub struct Receiver {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    connections: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Receiver {
    pub fn start(respond: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let held: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        let thread = {
            let (requests, connections, stop) =
                (requests.clone(), connections.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok((stream, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    connections.fetch_add(1, Ordering::SeqCst);
                    if respond {
                        let requests = requests.clone();
                        std::thread::spawn(move || serve(stream, &requests));
                    } else {
                        held.lock().unwrap().push(stream);
                    }
                }
            })
        };
        Self {
            addr,
            requests,
            connections,
            stop,
            thread: Some(thread),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn wait_for_request(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if !self.requests().is_empty() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(mut stream: TcpStream, requests: &Mutex<Vec<CapturedRequest>>) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let Ok(read) = stream.read(&mut chunk) else {
            break;
        };
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    if head.starts_with("post /v1/traces") {
        requests.lock().unwrap().push(CapturedRequest {
            head,
            body: buffer[header_end..].to_vec(),
        });
    }
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );
}

pub fn closed_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

pub fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

pub fn contains(haystack: &[u8], needle: &str) -> bool {
    count_occurrences(haystack, needle.as_bytes()) > 0
}

pub fn standalone_env(endpoint: &str) -> Vec<(&'static str, String)> {
    vec![
        ("SCE_TELEMETRY", "standalone".to_string()),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.to_string()),
    ]
}
