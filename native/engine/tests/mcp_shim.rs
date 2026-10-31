//! MCP stdio shim (`src/engine/mcp.mjs`) protocol round-trip against a STUB
//! engine: this test owns the wire format (JSON-RPC framing, tools/list,
//! tools/call), so it must not depend on the real engine's routes.
//!
//! Everything is loopback: the fake engine is `test_support::MockServer` on an
//! ephemeral port and the token lives in a temp data dir. The shim itself runs
//! on a thread over in-memory pipes, so the tests speak the same
//! newline-delimited frames a real MCP client does.

use std::io::{self, BufRead, Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use espectral_engine::mcp::{self, ShimConfig};
use espectral_engine::test_support::{MockRequest, MockResponse, MockServer};
use serde_json::{json, Value};

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

// ---------------------------------------------------------------------------
// Stub engine
// ---------------------------------------------------------------------------

type ReplyFn = Box<dyn Fn(&MockRequest) -> MockResponse + Send + Sync>;

fn stub_reply(req: &MockRequest) -> MockResponse {
    let auth = req.header("authorization").unwrap_or_default();
    if auth != format!("Bearer {TOKEN}") {
        return MockResponse::json(&json!({ "error": { "code": "AGENT_AUTH", "message": "bad token" } }))
            .with_status(401);
    }
    let path = req.path.split('?').next().unwrap_or("");
    match path {
        "/api/agent/instances" => MockResponse::json(&json!({
            "instances": [{ "name": "agent-spike", "version": "26.3", "loader": "fabric", "capture_tools": true }]
        })),
        "/api/agent/launches" => MockResponse::json(&json!({
            "launches": [{ "key": "l1", "instance": "agent-spike", "running": true, "agent": true, "state": null }]
        })),
        "/api/agent/call" => MockResponse::json(&json!({
            "ok": true,
            "data": {
                "image_path": "/tmp/shot.png",
                "preview_path": "/tmp/shot.png",
                "width": 4,
                "height": 2,
                "preview": { "mime": "image/png", "base64": "iVBORw0KGgo=" }
            }
        })),
        "/api/agent/state" => MockResponse::json(&json!({
            "key": "l1", "instance": "agent-spike", "running": true, "state": { "phase": "in_world" }
        })),
        "/api/agent/launch" => MockResponse::json(&json!({ "key": "l2", "state": { "phase": "in_world" } })),
        _ => MockResponse::json(&json!({ "error": { "code": "NOT_FOUND", "message": "nope" } })).with_status(404),
    }
}

/// A stub engine: `MockServer` plus a swappable override reply.
struct Stub {
    server: MockServer,
    over: Arc<Mutex<Option<ReplyFn>>>,
}

impl Stub {
    async fn start() -> Self {
        let over: Arc<Mutex<Option<ReplyFn>>> = Arc::default();
        let o = over.clone();
        let server = MockServer::start(move |req: &MockRequest| {
            if let Some(f) = o.lock().unwrap().as_ref() {
                return f(req);
            }
            stub_reply(req)
        })
        .await;
        Self { server, over }
    }

    fn port(&self) -> u16 {
        self.server.host().rsplit(':').next().unwrap().parse().unwrap()
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }

    fn set_reply(&self, f: impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static) {
        *self.over.lock().unwrap() = Some(Box::new(f));
    }

    fn clear_reply(&self) {
        *self.over.lock().unwrap() = None;
    }

    fn seen(&self) -> Vec<MockRequest> {
        self.server.requests()
    }
}

// ---------------------------------------------------------------------------
// In-memory pipes + a shim running on a thread
// ---------------------------------------------------------------------------

/// Reader side of the client->shim pipe; EOF once the sender is dropped.
struct PipeReader {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl PipeReader {
    fn new(rx: Receiver<Vec<u8>>) -> Self {
        Self { rx, buf: Vec::new(), pos: 0 }
    }
}

impl BufRead for PipeReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos >= self.buf.len() {
            self.buf.clear();
            self.pos = 0;
            match self.rx.recv() {
                Ok(chunk) => self.buf = chunk,
                Err(_) => return Ok(&[]),
            }
        }
        Ok(&self.buf[self.pos..])
    }

    fn consume(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.buf.len());
    }
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let avail = self.fill_buf()?;
        if avail.is_empty() {
            return Ok(0);
        }
        let n = avail.len().min(out.len());
        out[..n].copy_from_slice(&avail[..n]);
        self.consume(n);
        Ok(n)
    }
}

/// shim->client pipe: appends to a shared buffer the test drains a line at a time.
struct OutBuf {
    text: Mutex<String>,
}

impl OutBuf {
    fn new() -> Self {
        Self { text: Mutex::new(String::new()) }
    }

    fn push(&self, chunk: &[u8]) {
        self.text.lock().unwrap().push_str(&String::from_utf8_lossy(chunk));
    }

    /// Take every complete line, leaving an unterminated tail in place.
    fn lines(&self) -> Vec<String> {
        let mut g = self.text.lock().unwrap();
        let mut out = Vec::new();
        while let Some(i) = g.find('\n') {
            let line = g[..i].trim().to_string();
            g.drain(..=i);
            if !line.is_empty() {
                out.push(line);
            }
        }
        out
    }
}

struct OutWriter(Arc<OutBuf>);

impl Write for OutWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.push(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A running shim process equivalent: stdin writer + collected frames.
struct Shim {
    stdin: Option<Sender<Vec<u8>>>,
    out: Arc<OutBuf>,
    seen: Arc<Mutex<Vec<Value>>>,
    join: Option<std::thread::JoinHandle<io::Result<()>>>,
}

impl Shim {
    fn start(cfg: ShimConfig) -> Self {
        let (tx, rx) = channel::<Vec<u8>>();
        let out = Arc::new(OutBuf::new());
        let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
        let writer = OutWriter(out.clone());
        let join = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            mcp::serve(PipeReader::new(rx), writer, &cfg, &rt)
        });
        Self { stdin: Some(tx), out, seen, join: Some(join) }
    }

    /// Write a raw frame (no newline is added).
    fn send_raw(&self, s: &str) {
        self.stdin.as_ref().expect("stdin open").send(s.as_bytes().to_vec()).unwrap();
    }

    fn send_frame(&self, v: &Value) {
        self.send_raw(&format!("{v}\n"));
    }

    /// Move every complete output line into `seen`.
    fn pump(&self) {
        let mut seen = self.seen.lock().unwrap();
        for line in self.out.lines() {
            // A non-protocol byte on stdout is a bug: fail loudly, like the JS test.
            let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("non-protocol stdout: {line:?}: {e}"));
            seen.push(v);
        }
    }

    /// Wait for the response frame whose `id` matches, removing it.
    fn wait_id(&self, id: &Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.pump();
            {
                let mut seen = self.seen.lock().unwrap();
                if let Some(i) = seen.iter().position(|f| f.get("id") == Some(id)) {
                    return seen.remove(i);
                }
            }
            if Instant::now() >= deadline {
                panic!("no reply for id {id}; stdout: {:?}", self.out.text.lock().unwrap());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn rpc(&self, id: Value, method: &str, params: Option<Value>) -> Value {
        let mut frame = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(p) = params {
            frame["params"] = p;
        }
        self.send_frame(&frame);
        self.wait_id(&id)
    }

    fn notify(&self, method: &str) {
        self.send_frame(&json!({ "jsonrpc": "2.0", "method": method }));
    }

    /// Frames collected so far (drains `seen`).
    fn take_frames(&self) -> Vec<Value> {
        self.pump();
        std::mem::take(&mut *self.seen.lock().unwrap())
    }

    /// Close stdin, run the shim to completion and return its exit status.
    fn close(&mut self) -> io::Result<()> {
        drop(self.stdin.take());
        let join = self.join.take().expect("joined once");
        let r = join.join().expect("shim thread panicked");
        self.pump();
        r
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn token_dir(token: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("espectral-mcp-test-").tempdir().unwrap();
    std::fs::write(mcp::token_path_for(dir.path()), format!("{token}\n")).unwrap();
    dir
}

fn cfg_for(stub: &Stub, dir: &tempfile::TempDir) -> ShimConfig {
    ShimConfig::new(stub.base(), dir.path())
}

fn call(shim: &Shim, id: i64, name: &str, args: Value) -> Value {
    shim.rpc(json!(id), "tools/call", Some(json!({ "name": name, "arguments": args })))
}

/// The text content of a `tools/call` result.
fn text_of(res: &Value) -> String {
    res["result"]["content"][0]["text"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// 1. round-trip
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_tools_list_and_call_round_trip() {
    let stub = Stub::start().await;
    let dir = token_dir(TOKEN);
    let mut shim = Shim::start(cfg_for(&stub, &dir));

    let init = shim.rpc(
        json!(1),
        "initialize",
        Some(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" },
        })),
    );
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18", "echoes a supported protocol version");
    assert_eq!(init["result"]["capabilities"]["tools"]["listChanged"], false);
    assert_eq!(init["result"]["serverInfo"]["name"], "espectral-client");
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.to_lowercase().contains("untrusted"), "instructions warn about untrusted game text");

    shim.notify("notifications/initialized");

    let ping = shim.rpc(json!(2), "ping", Some(json!({})));
    assert_eq!(ping["result"], json!({}));

    let list = shim.rpc(json!(3), "tools/list", Some(json!({})));
    let tools = list["result"]["tools"].as_array().unwrap();
    for expected in [
        "list_instances",
        "list_launches",
        "launch",
        "stop",
        "state",
        "logs",
        "screenshot",
        "screen_tree",
        "click",
        "key",
        "type",
        "chat",
        "command",
        "disconnect",
        "hud",
        "shaders",
        "record_start",
        "record_stop",
        "replays",
        "clip_render",
        "clip_status",
    ] {
        assert!(tools.iter().any(|t| t["name"] == expected), "tool {expected} is listed");
    }
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(tool["description"].as_str().unwrap().len() > 20, "{name} has a real description");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name} has an object schema");
    }
    let shot = tools.iter().find(|t| t["name"] == "screenshot").unwrap();
    assert!(shot["description"].as_str().unwrap().contains("TIER"));

    // Engine-level tool.
    let instances = call(&shim, 4, "list_instances", json!({}));
    assert_eq!(instances["result"]["isError"], false);
    let payload: Value = serde_json::from_str(&text_of(&instances)).unwrap();
    assert_eq!(payload["instances"][0]["capture_tools"], true);
    let seen = stub.seen();
    let seek = seen.iter().find(|r| r.path == "/api/agent/instances").expect("instances hit");
    assert_eq!(seek.header("authorization").unwrap(), format!("Bearer {TOKEN}"));
    assert_eq!(seek.header("x-espectral-client").unwrap(), "1");
    assert_eq!(seek.method, "GET");
    assert_eq!(seek.header("content-type"), None, "engine-only GETs send no body");

    // Engine-level launch with join/window: the body must be forwarded verbatim.
    let launched = call(
        &shim,
        5,
        "launch",
        json!({
            "instance": "agent-spike",
            "join": { "world": "Mundo" },
            "window": { "width": 1080, "height": 1920 },
            "wait_for": "in_world",
        }),
    );
    assert_eq!(launched["result"]["isError"], false);
    let launch_seen = stub.seen().into_iter().find(|r| r.path == "/api/agent/launch").expect("launch hit");
    assert_eq!(
        launch_seen.body_json().unwrap(),
        json!({
            "instance": "agent-spike",
            "join": { "world": "Mundo" },
            "window": { "width": 1080, "height": 1920 },
            "wait_for": "in_world",
        })
    );

    // Mod tool: args minus `key` go to the engine as call.args.
    let shot_call = call(&shim, 6, "screenshot", json!({ "key": "l1", "width": 1920, "height": 1080, "hud": false }));
    assert_eq!(shot_call["result"]["isError"], false);
    let kinds: Vec<&str> =
        shot_call["result"]["content"].as_array().unwrap().iter().map(|c| c["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["image", "text"], "image content then the JSON text content");
    let image = &shot_call["result"]["content"][0];
    assert_eq!(image["mimeType"], "image/png");
    assert_eq!(image["data"], "iVBORw0KGgo=");
    let text: Value = serde_json::from_str(shot_call["result"]["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["image_path"], "/tmp/shot.png");
    assert!(text.get("preview").is_none(), "the inline base64 is not repeated in the text");
    let call_seen = stub.seen().into_iter().rfind(|r| r.path == "/api/agent/call").expect("call hit");
    assert_eq!(call_seen.header("x-espectral-client").unwrap(), "1");
    assert_eq!(call_seen.header("content-type").unwrap(), "application/json");
    assert_eq!(
        call_seen.body_json().unwrap(),
        json!({
            "key": "l1",
            "tool": "screenshot",
            "args": { "width": 1920, "height": 1080, "hud": false },
            "timeout_s": 60,
        })
    );

    // `key` tool maps the MCP `keyName` argument to the mod's `key` arg.
    call(&shim, 7, "key", json!({ "key": "l1", "keyName": "e", "action": "press" }));
    let key_call = stub.seen().into_iter().rfind(|r| r.path == "/api/agent/call").unwrap();
    assert_eq!(key_call.body_json().unwrap()["args"], json!({ "key": "e", "action": "press" }));

    shim.close().unwrap();
}

// ---------------------------------------------------------------------------
// 2. engine errors surface as actionable isError content
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_errors_come_back_as_is_error_content() {
    let stub = Stub::start().await;
    let dir = token_dir(TOKEN);
    let mut shim = Shim::start(cfg_for(&stub, &dir));
    shim.rpc(json!(1), "initialize", Some(json!({ "protocolVersion": "2024-11-05" })));

    stub.set_reply(|_| {
        MockResponse::json(&json!({ "error": { "code": "AGENT_DISABLED", "message": "off" } })).with_status(403)
    });
    let res = call(&shim, 2, "list_instances", json!({}));
    assert_eq!(res["result"]["isError"], true);
    let text = text_of(&res);
    assert!(text.contains("AGENT_DISABLED"), "{text}");
    assert!(text.to_lowercase().contains("disabled"), "{text}");
    stub.clear_reply();

    // A tool the engine refuses (tier) surfaces the engine's code.
    stub.set_reply(|_| {
        MockResponse::json(&json!({ "error": { "code": "TIER_REFUSED", "message": "act tier refused here" } }))
            .with_status(502)
    });
    let res = call(&shim, 3, "chat", json!({ "key": "l1", "text": "hi" }));
    assert_eq!(res["result"]["isError"], true);
    assert!(text_of(&res).contains("TIER_REFUSED"));
    stub.clear_reply();

    shim.close().unwrap();
}

// ---------------------------------------------------------------------------
// 3. no token / malformed token / engine down are distinct
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_token_bad_token_and_engine_down_are_distinct() {
    let stub = Stub::start().await;

    // No token file at all: agent access is off.
    let empty = tempfile::Builder::new().prefix("espectral-mcp-empty-").tempdir().unwrap();
    let mut shim = Shim::start(ShimConfig::new(stub.base(), empty.path()));
    shim.rpc(json!(1), "initialize", Some(json!({})));
    let res = call(&shim, 2, "list_instances", json!({}));
    assert_eq!(res["result"]["isError"], true);
    let text = text_of(&res);
    assert!(text.contains("NO_TOKEN"), "{text}");
    assert!(text.contains("Agent access"), "{text}");
    shim.close().unwrap();

    // A token file that exists but is garbage is a different problem (rotate),
    // not "agent access is off" — and the token itself is never printed.
    let bad = tempfile::Builder::new().prefix("espectral-mcp-badtoken-").tempdir().unwrap();
    std::fs::write(mcp::token_path_for(bad.path()), "not-a-token\n").unwrap();
    let mut shim = Shim::start(ShimConfig::new(stub.base(), bad.path()));
    shim.rpc(json!(1), "initialize", Some(json!({})));
    let res = call(&shim, 2, "list_instances", json!({}));
    assert_eq!(res["result"]["isError"], true);
    let text = text_of(&res);
    assert!(text.contains("malformed"), "{text}");
    assert!(text.contains("rotate"), "{text}");
    assert!(!text.contains("not-a-token"), "the file contents are not echoed");
    shim.close().unwrap();

    // Token present, engine down.
    let dir = token_dir(TOKEN);
    let mut shim = Shim::start(ShimConfig::new("http://127.0.0.1:1", dir.path()));
    shim.rpc(json!(1), "initialize", Some(json!({})));
    let res = call(&shim, 2, "list_instances", json!({}));
    assert_eq!(res["result"]["isError"], true);
    let text = text_of(&res);
    assert!(text.contains("ENGINE_DOWN"), "{text}");
    assert!(text.contains("Is Espectral Client running"), "{text}");
    shim.close().unwrap();
}

// ---------------------------------------------------------------------------
// 4. malformed frames and unknown methods keep the stream healthy
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_frames_do_not_kill_the_stream() {
    let stub = Stub::start().await;
    let dir = token_dir(TOKEN);
    let mut shim = Shim::start(cfg_for(&stub, &dir));
    shim.rpc(json!(1), "initialize", Some(json!({})));

    shim.send_raw("{not json}\n");
    let bad = shim.rpc(json!(2), "resources/list", Some(json!({})));
    assert_eq!(bad["error"]["code"], -32601);

    // The parse error for the garbage frame came back too, with id null.
    let frames = shim.take_frames();
    let parse_error = frames.iter().find(|f| f["error"]["code"] == -32700).expect("parse error frame");
    assert_eq!(parse_error["id"], Value::Null);

    // Still alive and speaking protocol after the garbage frame.
    let list = shim.rpc(json!(3), "tools/list", Some(json!({})));
    assert!(!list["result"]["tools"].as_array().unwrap().is_empty());
    shim.close().unwrap();
}

// ---------------------------------------------------------------------------
// 5. the token is re-read on every call
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_is_re_read_per_call_so_rotation_takes_effect() {
    let stub = Stub::start().await;
    let dir = token_dir(TOKEN);
    let mut shim = Shim::start(cfg_for(&stub, &dir));
    shim.rpc(json!(1), "initialize", Some(json!({})));

    let res = call(&shim, 2, "list_instances", json!({}));
    assert_eq!(res["result"]["isError"], false);

    // Rotate behind the shim's back: the next call must use the new token.
    std::fs::write(mcp::token_path_for(dir.path()), format!("{OTHER_TOKEN}\n")).unwrap();
    let res = call(&shim, 3, "list_instances", json!({}));
    assert_eq!(res["result"]["isError"], true);
    assert!(text_of(&res).contains("AGENT_AUTH"), "{}", text_of(&res));
    let used = stub.seen().last().unwrap().header("authorization").unwrap();
    assert_eq!(used, format!("Bearer {OTHER_TOKEN}"));

    shim.close().unwrap();
}

// ---------------------------------------------------------------------------
// 6. JSON-RPC 2.0 semantics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn jsonrpc_semantics_id_echo_notifications_and_unknowns() {
    let dir = token_dir(TOKEN);
    let cfg = ShimConfig::new("http://127.0.0.1:1", dir.path());

    // Ids are echoed verbatim: strings, 0 and integers beyond 32 bits included.
    let r = mcp::handle_message(&cfg, &json!({ "jsonrpc": "2.0", "id": "abc", "method": "ping" })).await.unwrap();
    assert_eq!(r, json!({ "jsonrpc": "2.0", "id": "abc", "result": {} }));
    let r = mcp::handle_message(&cfg, &json!({ "jsonrpc": "2.0", "id": 0, "method": "ping" })).await.unwrap();
    assert_eq!(r["id"], 0);
    let r = mcp::handle_message(&cfg, &json!({ "jsonrpc": "2.0", "id": 9007199254740991i64, "method": "ping" }))
        .await
        .unwrap();
    assert_eq!(r["id"], 9007199254740991i64);

    // Notifications (no `id`) are NEVER answered — not even with an error, and
    // not even when they name a request method.
    for msg in [
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "method": "tools/call", "params": { "name": "list_instances" } }),
        json!({ "jsonrpc": "2.0", "method": "no/such/notification" }),
    ] {
        assert_eq!(mcp::handle_message(&cfg, &msg).await, None, "notification {msg} answered");
    }

    // `id: null` is a real request (answered with id null).
    let r = mcp::handle_message(&cfg, &json!({ "jsonrpc": "2.0", "id": null, "method": "ping" })).await.unwrap();
    assert_eq!(r["id"], Value::Null);
    assert_eq!(r["result"], json!({}));

    // Unknown tool = protocol error (-32602), never a successful tool result.
    let r = mcp::handle_message(
        &cfg,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "nope" } }),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
    assert_eq!(r["result"], Value::Null);
    assert_eq!(r["id"], 1);

    // Non-object `arguments` is a malformed request, not a tool failure.
    let r = mcp::handle_message(
        &cfg,
        &json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": "list_instances", "arguments": [] } }),
    )
    .await
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);

    // Unknown method and malformed requests.
    let r = mcp::handle_message(&cfg, &json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/list" })).await.unwrap();
    assert_eq!(r["error"]["code"], -32601);
    let r = mcp::handle_message(&cfg, &json!({ "jsonrpc": "2.0", "id": 3 })).await.unwrap();
    assert_eq!(r["error"]["code"], -32600);
    assert_eq!(r["id"], 3);
    let r = mcp::handle_message(&cfg, &json!([1, 2])).await.unwrap();
    assert_eq!(r, json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": "invalid request" } }));

    // initialize result shape.
    let init = mcp::handle_message(
        &cfg,
        &json!({ "jsonrpc": "2.0", "id": 4, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }),
    )
    .await
    .unwrap();
    assert_eq!(init["id"], 4);
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["capabilities"], json!({ "tools": { "listChanged": false } }));
    assert_eq!(init["result"]["serverInfo"]["name"], "espectral-client");
    assert!(init["result"]["serverInfo"]["version"].is_string());

    // An unsupported protocol version falls back to the newest we speak.
    let init = mcp::handle_message(
        &cfg,
        &json!({ "jsonrpc": "2.0", "id": 5, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } }),
    )
    .await
    .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
}

// ---------------------------------------------------------------------------
// 7. a 200 {ok:false} envelope is a refusal, not an empty success
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ok_false_envelope_is_a_refusal() {
    let stub = Stub::start().await;
    let dir = token_dir(TOKEN);
    let mut shim = Shim::start(cfg_for(&stub, &dir));
    shim.rpc(json!(1), "initialize", Some(json!({})));

    stub.set_reply(|_| {
        MockResponse::json(&json!({ "ok": false, "error": { "code": "BUSY", "message": "renderer busy" } }))
    });
    let res = call(&shim, 2, "screenshot", json!({ "key": "l1" }));
    assert_eq!(res["result"]["isError"], true);
    assert_eq!(text_of(&res), "BUSY: renderer busy");
    stub.clear_reply();

    shim.close().unwrap();
}

// ---------------------------------------------------------------------------
// 8. CRLF frames and notifications
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crlf_frames_and_notifications_keep_the_stream_healthy() {
    let stub = Stub::start().await;
    let dir = token_dir(TOKEN);
    let mut shim = Shim::start(cfg_for(&stub, &dir));
    shim.rpc(json!(1), "initialize", Some(json!({})));
    let before = shim.take_frames().len();

    // CRLF line ending, then a notification (no id -> no reply at all).
    shim.send_raw(
        "{\"jsonrpc\":\"2.0\",\"id\":\"crlf\",\"method\":\"ping\"}\r\n\
         {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\r\n",
    );
    let sentinel = shim.rpc(json!("sentinel"), "ping", Some(json!({})));
    let got: Vec<Value> = shim.take_frames().into_iter().filter(|f| f["id"] != sentinel["id"]).collect();
    let ids: Vec<&Value> = got.iter().map(|f| &f["id"]).collect();
    assert_eq!(
        ids,
        [&json!("crlf")],
        "before={before}: the CRLF request was answered and the notification produced no frame"
    );
    assert_eq!(got[0]["result"], json!({}));

    shim.close().unwrap();
}

// ---------------------------------------------------------------------------
// 9. stdin close flushes the last frame and any in-flight reply
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdin_close_flushes_last_frame_and_in_flight_reply() {
    let stub = Stub::start().await;
    let dir = token_dir(TOKEN);

    // A final frame with no trailing newline must still be answered.
    let mut shim = Shim::start(cfg_for(&stub, &dir));
    shim.rpc(json!(1), "initialize", Some(json!({})));
    shim.take_frames();
    shim.send_raw("{\"jsonrpc\":\"2.0\",\"id\":\"last\",\"method\":\"ping\"}");
    shim.close().expect("stdin close exits cleanly");
    let flushed = shim.take_frames();
    let ids: Vec<&Value> = flushed.iter().map(|f| &f["id"]).collect();
    assert_eq!(ids, [&json!("last")]);
    assert_eq!(flushed[0]["result"], json!({}));

    // ...and a tool call still waiting on the engine is not dropped by the exit.
    let mut slow = Shim::start(cfg_for(&stub, &dir));
    slow.rpc(json!(1), "initialize", Some(json!({})));
    stub.set_reply(|_| {
        MockResponse::json(&json!({ "instances": [{ "name": "slow" }] })).with_delay(Duration::from_millis(250))
    });
    slow.send_frame(&json!({
        "jsonrpc": "2.0", "id": "slow1", "method": "tools/call", "params": { "name": "list_instances" }
    }));
    slow.close().unwrap();
    stub.clear_reply();
    let frames = slow.take_frames();
    let reply = frames.iter().find(|f| f["id"] == "slow1").expect("the in-flight reply was delivered before exit");
    assert_eq!(reply["result"]["isError"], false);
    let payload: Value = serde_json::from_str(text_of(reply).as_str()).unwrap();
    assert_eq!(payload["instances"][0]["name"], "slow");
}

// ---------------------------------------------------------------------------
// 10. deadlines and error text
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fetch_deadlines_and_error_text() {
    let dir = token_dir(TOKEN);

    // Headers + a partial body, then never finishes: must not hang the call.
    let stalling = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stall_port = stalling.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = stalling.accept().await else { return };
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n{\"instances\":").await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            });
        }
    });
    let cfg = ShimConfig::new(format!("http://127.0.0.1:{stall_port}"), dir.path());
    let t0 = Instant::now();
    let err = mcp::engine_fetch(&cfg, "GET", "/api/agent/instances", None, Duration::from_millis(150))
        .await
        .expect_err("stalled body must time out");
    assert!(err.code == "TIMEOUT" || err.code == "ENGINE_DOWN", "{err}");
    assert!(t0.elapsed() < Duration::from_secs(5), "a stalled body does not hang the call");

    // An error reply with an empty body still says something.
    let empty = MockServer::start(|_| MockResponse::bytes(Vec::new()).with_status(503)).await;
    let port: u16 = empty.host().rsplit(':').next().unwrap().parse().unwrap();
    let cfg = ShimConfig::new(format!("http://127.0.0.1:{port}"), dir.path());
    let err =
        mcp::engine_fetch(&cfg, "GET", "/api/agent/instances", None, Duration::from_secs(5)).await.expect_err("503");
    assert!(err.message.contains("HTTP 503"), "{}", err.message);

    // A 401 whose body echoes the presented token must not leak it back.
    let leak = MockServer::start(|req: &MockRequest| {
        let auth = req.header("authorization").unwrap_or_default();
        MockResponse::json(&json!({ "error": { "code": "AGENT_AUTH", "message": format!("bad token {auth}") } }))
            .with_status(401)
    })
    .await;
    let port: u16 = leak.host().rsplit(':').next().unwrap().parse().unwrap();
    let cfg = ShimConfig::new(format!("http://127.0.0.1:{port}"), dir.path());
    let err =
        mcp::engine_fetch(&cfg, "GET", "/api/agent/instances", None, Duration::from_secs(5)).await.expect_err("401");
    assert_eq!(err.code, "AGENT_AUTH");
    assert!(!err.message.contains(TOKEN), "the token is never echoed back in an error message");
    assert!(err.message.contains("rotate"), "{}", err.message);
}
