//! Test helpers shared by every cluster (unit tests and `engine/tests/*.rs`).
//!
//! - [`fixture`]: path into the repo's `tests/fixtures` (shared with the JS tests).
//! - [`MockServer`]: a tiny loopback HTTP/1.1 server. Point engine URLs at it
//!   (or map a real host to it with `HttpOptions::mirrors`) — tests must never
//!   touch the real network.
//! - [`temp_engine`]: an [`Engine`](crate::Engine) on a fresh temp data dir
//!   with system proxies disabled and millisecond retry backoff.
//!
//! Compiled into the library (doc-hidden) so integration tests can use it.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// `<repo>/tests/fixtures/<name>`.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("tests").join("fixtures").join(name)
}

/// An engine on a temp data dir (kept alive by the returned guard).
pub fn temp_engine() -> (tempfile::TempDir, crate::Engine) {
    let dir = tempfile::Builder::new().prefix("espectral-engine-test-").tempdir().expect("tempdir");
    let engine = crate::Engine::new(test_options(dir.path().to_path_buf())).expect("engine");
    (dir, engine)
}

/// EngineOptions for tests: given data dir, no system proxy, fast retries.
pub fn test_options(data_dir: PathBuf) -> crate::EngineOptions {
    crate::EngineOptions {
        data_dir: Some(data_dir),
        http: crate::download::HttpOptions {
            retry_backoff: vec![Duration::from_millis(1), Duration::from_millis(1)],
            use_system_proxy: false,
            ..Default::default()
        },
        run_onboarding: false,
        start_background_tasks: false,
    }
}

#[derive(Debug, Clone)]
pub struct MockRequest {
    pub method: String,
    /// Path + query, e.g. `/v2/versions/loader?x=1`.
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MockRequest {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }
    pub fn body_json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

#[derive(Debug, Clone)]
pub struct MockResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Close the socket without answering (network error on the client).
    pub drop: bool,
    /// Sleep before answering.
    pub delay: Option<Duration>,
}

impl MockResponse {
    pub fn ok(body: &str) -> Self {
        Self::bytes(body.as_bytes().to_vec())
    }
    pub fn bytes(body: Vec<u8>) -> Self {
        MockResponse { status: 200, headers: vec![], body, drop: false, delay: None }
    }
    pub fn json(v: &serde_json::Value) -> Self {
        Self::ok(&v.to_string()).with_header("content-type", "application/json")
    }
    pub fn status(status: u16) -> Self {
        MockResponse { status, headers: vec![], body: format!("HTTP {status}").into_bytes(), drop: false, delay: None }
    }
    pub fn drop_connection() -> Self {
        MockResponse { status: 0, headers: vec![], body: vec![], drop: true, delay: None }
    }
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
    pub fn with_header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
    pub fn with_delay(mut self, d: Duration) -> Self {
        self.delay = Some(d);
        self
    }
}

type Handler = Arc<dyn Fn(usize, &MockRequest) -> MockResponse + Send + Sync>;

/// Loopback HTTP server; stops when dropped.
pub struct MockServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<MockRequest>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<MockRequest> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 1 << 20 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split(' ');
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < len {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some(MockRequest { method, path, headers, body })
}

impl MockServer {
    /// Answer every request with `handler(&req)`.
    pub async fn start(handler: impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static) -> Self {
        Self::start_seq(move |_, r| handler(r)).await
    }

    /// Answer with `handler(n, &req)`, n = 0-based request counter.
    pub async fn start_seq(handler: impl Fn(usize, &MockRequest) -> MockResponse + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
        let addr = listener.local_addr().expect("addr");
        let requests: Arc<Mutex<Vec<MockRequest>>> = Arc::default();
        let handler: Handler = Arc::new(handler);
        let reqs = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let handler = handler.clone();
                let reqs = reqs.clone();
                tokio::spawn(async move {
                    let Some(req) = read_request(&mut sock).await else { return };
                    let n = {
                        let mut g = reqs.lock().unwrap();
                        g.push(req.clone());
                        g.len() - 1
                    };
                    let resp = handler(n, &req);
                    if let Some(d) = resp.delay {
                        tokio::time::sleep(d).await;
                    }
                    if resp.drop {
                        return; // socket dropped without a response
                    }
                    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status));
                    head.push_str(&format!("content-length: {}\r\nconnection: close\r\n", resp.body.len()));
                    for (k, v) in &resp.headers {
                        head.push_str(&format!("{k}: {v}\r\n"));
                    }
                    head.push_str("\r\n");
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&resp.body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        MockServer { addr, requests, task }
    }

    /// `127.0.0.1:<port>` (what `url_host` / mirror maps use).
    pub fn host(&self) -> String {
        self.addr.to_string()
    }

    /// `http://127.0.0.1:<port><path>`.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn hits(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub fn requests(&self) -> Vec<MockRequest> {
        self.requests.lock().unwrap().clone()
    }
}
