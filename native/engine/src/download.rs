//! Port of `src/engine/download.mjs` — the engine HTTP client.
//!
//! - [`HttpClient::fetch_text`] / [`HttpClient::fetch_json`] / [`HttpClient::fetch_bytes`]:
//!   small payloads, 3 attempts with 500/1500 ms backoff on network errors and
//!   429/502/503/504; errors name the host and the network reason.
//! - [`HttpClient::download_file`]: SHA-1 (or size) verified download into
//!   `<dest>.part`, `Range` resume, per-chunk stall watchdog, atomic rename,
//!   per-destination lock, and identity preservation (a hashless re-download
//!   of byte-identical content keeps the original file and its mtime — the
//!   JEP 483 AOT cache stamps classpath mtimes).
//! - [`HttpClient::download_all`]: bounded-concurrency batch with per-item
//!   `mod-progress` / `asset-progress` events (start/done/error) and an
//!   aggregate error when any item fails.
//! - Host mirrors: `meta.fabricmc.net` -> `meta2.fabricmc.net`,
//!   `maven.fabricmc.net` -> `maven2.fabricmc.net` (Cloudflare blocks during
//!   LaLiga matches). An outage (network error / 5xx — never a 4xx nor a
//!   sha1 mismatch) falls through to the mirror, which is then preferred for
//!   10 minutes.
//!
//! rustls only (reqwest built without native-tls). System proxies
//! (`HTTPS_PROXY`…) are honoured unless [`HttpOptions::use_system_proxy`] is off.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;

use crate::config::ConfigStore;
use crate::error::{EngineError, EngineResult, HttpError, HttpErrorKind};
use crate::events::{EventBus, ProgressChannel, ProgressEvent};

/// User-Agent sent on every request (JS `USER_AGENT`).
pub const USER_AGENT: &str = "EspectralClient/1.1.1";

/// Default host -> mirror map.
pub const DEFAULT_MIRRORS: &[(&str, &str)] =
    &[("meta.fabricmc.net", "meta2.fabricmc.net"), ("maven.fabricmc.net", "maven2.fabricmc.net")];

const RETRYABLE_STATUS: [u16; 4] = [429, 502, 503, 504];

/// Tunables (defaults = the JS constants). Tests shrink the backoff and point
/// the mirror map at loopback servers.
#[derive(Debug, Clone)]
pub struct HttpOptions {
    /// `host[:port]` -> `host[:port]`.
    pub mirrors: Vec<(String, String)>,
    /// Sleep before attempt 2, 3, … of fetch_text (JS [500, 1500] ms).
    pub retry_backoff: Vec<Duration>,
    /// fetch_text attempts (JS 3).
    pub fetch_attempts: u32,
    /// download_file attempts on 416 / stall / interrupted body (JS 5).
    pub download_attempts: u32,
    /// How long a host's mirror stays preferred after an outage (JS 10 min).
    pub mirror_sticky: Duration,
    pub user_agent: String,
    /// Honour HTTP(S)_PROXY / NO_PROXY.
    pub use_system_proxy: bool,
}

impl Default for HttpOptions {
    fn default() -> Self {
        HttpOptions {
            mirrors: DEFAULT_MIRRORS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            retry_backoff: vec![Duration::from_millis(500), Duration::from_millis(1500)],
            fetch_attempts: 3,
            download_attempts: 5,
            mirror_sticky: Duration::from_secs(10 * 60),
            user_agent: USER_AGENT.to_string(),
            use_system_proxy: true,
        }
    }
}

/// Options for small fetches.
#[derive(Debug, Clone)]
pub struct FetchOptions {
    pub headers: Vec<(String, String)>,
    /// Applies to the response headers and again to the body (JS 60 s).
    pub timeout: Duration,
}

impl Default for FetchOptions {
    fn default() -> Self {
        FetchOptions { headers: vec![], timeout: Duration::from_secs(60) }
    }
}

/// Byte-progress callback: `(received, total)`.
pub type ProgressFn = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// Options for [`HttpClient::download_file`].
#[derive(Clone, Default)]
pub struct DownloadOptions {
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub headers: Vec<(String, String)>,
    /// Header timeout AND per-chunk stall timeout (JS 120 s). None = default.
    pub timeout: Option<Duration>,
    pub on_progress: Option<ProgressFn>,
}

impl std::fmt::Debug for DownloadOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadOptions").field("sha1", &self.sha1).field("size", &self.size).finish()
    }
}

/// Progress event an item of a batch reports on (download.mjs `eventName` + `context`).
#[derive(Debug, Clone)]
pub struct ItemEvent {
    pub channel: ProgressChannel,
    /// Spread over the base payload (`{ instance, phase, index, total, ...context }`).
    pub context: ProgressEvent,
}

/// One entry of a [`HttpClient::download_all`] batch.
#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub event: Option<ItemEvent>,
}

impl DownloadItem {
    pub fn new(url: impl Into<String>, dest: impl Into<PathBuf>) -> Self {
        DownloadItem { url: url.into(), dest: dest.into(), sha1: None, size: None, headers: vec![], event: None }
    }
    pub fn sha1(mut self, sha1: Option<String>) -> Self {
        self.sha1 = sha1;
        self
    }
    pub fn size(mut self, size: Option<u64>) -> Self {
        self.size = size;
        self
    }
    pub fn event(mut self, channel: ProgressChannel, context: ProgressEvent) -> Self {
        self.event = Some(ItemEvent { channel, context });
        self
    }
}

/// Per-item completion passed to [`DownloadAllOptions::on_item`].
#[derive(Debug, Clone)]
pub struct ItemOutcome {
    pub index: usize,
    pub ok: bool,
    pub dest: PathBuf,
    pub error: Option<String>,
    pub item: DownloadItem,
}

pub type ItemFn = Arc<dyn Fn(&ItemOutcome) + Send + Sync>;

#[derive(Clone, Default)]
pub struct DownloadAllOptions {
    /// None = config `download_concurrency` (default 6).
    pub concurrency: Option<usize>,
    pub on_item: Option<ItemFn>,
}

/// One failed item of a batch.
#[derive(Debug, Clone)]
pub struct DownloadFailure {
    pub url: String,
    pub dest: PathBuf,
    pub error: String,
}

/// `downloadAll: N/M item(s) failed` with the per-item details.
#[derive(Debug, Clone, thiserror::Error)]
#[error("downloadAll: {}/{total} item(s) failed", failures.len())]
pub struct DownloadAllError {
    pub total: usize,
    pub failures: Vec<DownloadFailure>,
    /// Per item, in input order: Ok(dest) or Err(message).
    pub results: Vec<Result<PathBuf, String>>,
}

impl From<DownloadAllError> for EngineError {
    fn from(e: DownloadAllError) -> Self {
        EngineError::Other(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Hash helpers
// ---------------------------------------------------------------------------

/// Lowercase hex SHA-1 of bytes.
pub fn sha1_hex(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

/// Lowercase hex SHA-1 of a file (streamed; blocking — see [`sha1_file`]).
pub fn sha1_file_blocking(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Lowercase hex SHA-1 of a file, off the async runtime.
pub async fn sha1_file(path: &Path) -> std::io::Result<String> {
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || sha1_file_blocking(&p)).await.map_err(std::io::Error::other)?
}

async fn sha1_file_safe(path: &Path) -> Option<String> {
    sha1_file(path).await.ok()
}

async fn file_size(path: &Path) -> Option<u64> {
    tokio::fs::metadata(path).await.ok().map(|m| m.len())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// `host[:port]` of a URL (JS `new URL(u).host`).
pub fn url_host(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => match (u.host_str(), u.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => url.to_string(),
        },
        Err(_) => url.to_string(),
    }
}

/// Best JS-like network reason for a reqwest error (`ECONNRESET`, `ECONNREFUSED`, …).
fn network_reason(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "request timeout".into();
    }
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(err);
    let mut last = err.to_string();
    while let Some(s) = src {
        if let Some(io) = s.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind as K;
            return match io.kind() {
                K::ConnectionRefused => "ECONNREFUSED".into(),
                K::ConnectionReset => "ECONNRESET".into(),
                K::ConnectionAborted => "ECONNABORTED".into(),
                K::TimedOut => "ETIMEDOUT".into(),
                K::UnexpectedEof => "ECONNRESET".into(),
                _ => io.to_string(),
            };
        }
        last = s.to_string();
        src = s.source();
    }
    last
}

fn engine_is_outage(e: &EngineError) -> bool {
    match e {
        EngineError::Http(h) => h.is_outage(),
        EngineError::Api { status, .. } => *status >= 500,
        EngineError::NotImplemented(_) => false,
        _ => true,
    }
}

fn http_err(kind: HttpErrorKind, url: &str, message: String) -> EngineError {
    EngineError::Http(HttpError { kind, url: url.to_string(), message })
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

struct Inner {
    client: reqwest::Client,
    opts: HttpOptions,
    mirrors: HashMap<String, String>,
    prefer_mirror_until: Mutex<HashMap<String, Instant>>,
    dest_locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    events: EventBus,
    config: Option<Arc<ConfigStore>>,
}

/// The engine's HTTP client (cheap to clone).
#[derive(Clone)]
pub struct HttpClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient").field("opts", &self.inner.opts).finish()
    }
}

impl HttpClient {
    pub fn new(opts: HttpOptions, events: EventBus, config: Option<Arc<ConfigStore>>) -> EngineResult<Self> {
        let mut b = reqwest::Client::builder()
            .user_agent(opts.user_agent.clone())
            .redirect(reqwest::redirect::Policy::limited(10))
            .connect_timeout(Duration::from_secs(30));
        if !opts.use_system_proxy {
            b = b.no_proxy();
        }
        let client = b.build().map_err(|e| EngineError::other(format!("http client init failed: {e}")))?;
        let mirrors = opts.mirrors.iter().cloned().collect();
        Ok(HttpClient {
            inner: Arc::new(Inner {
                client,
                opts,
                mirrors,
                prefer_mirror_until: Mutex::new(HashMap::new()),
                dest_locks: Mutex::new(HashMap::new()),
                events,
                config,
            }),
        })
    }

    /// The underlying reqwest client (for POSTs etc. — msauth, identity, discord).
    /// Requests built from it still get the engine User-Agent.
    pub fn client(&self) -> &reqwest::Client {
        &self.inner.client
    }

    pub fn options(&self) -> &HttpOptions {
        &self.inner.opts
    }

    /// Forget learned mirror preferences (JS resetMirrorPreference).
    pub fn reset_mirror_preference(&self) {
        self.inner.prefer_mirror_until.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    /// Same URL on the host's mirror, or None.
    pub fn mirror_url(&self, url: &str) -> Option<String> {
        let mut u = url::Url::parse(url).ok()?;
        let host = url_host(url);
        let mirror = self.inner.mirrors.get(&host)?;
        let (mh, mp) = match mirror.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
                (h.to_string(), p.parse::<u16>().ok())
            }
            _ => (mirror.clone(), None),
        };
        u.set_host(Some(&mh)).ok()?;
        u.set_port(mp).ok()?;
        Some(u.to_string())
    }

    fn default_concurrency(&self) -> usize {
        self.inner.config.as_ref().map(|c| c.download_concurrency()).unwrap_or(6)
    }

    /// Run `attempt(url, fail_fast)` on the primary, falling back to the mirror on an outage.
    async fn via_mirror<T, F, Fut>(&self, url: &str, attempt: F) -> EngineResult<T>
    where
        F: Fn(String, bool) -> Fut,
        Fut: std::future::Future<Output = EngineResult<T>>,
    {
        let alt = match self.mirror_url(url) {
            Some(a) => a,
            None => return attempt(url.to_string(), false).await,
        };
        let host = url_host(url);
        let mirror_host = url_host(&alt);
        let prefer = {
            let m = self.inner.prefer_mirror_until.lock().unwrap_or_else(|p| p.into_inner());
            m.get(&host).map(|t| *t > Instant::now()).unwrap_or(false)
        };
        let (first, second) = if prefer { (alt.clone(), url.to_string()) } else { (url.to_string(), alt.clone()) };
        let err = match attempt(first.clone(), true).await {
            Ok(v) => return Ok(v),
            Err(e) => e,
        };
        if !engine_is_outage(&err) {
            return Err(err);
        }
        {
            let mut m = self.inner.prefer_mirror_until.lock().unwrap_or_else(|p| p.into_inner());
            if first == url {
                m.insert(host.clone(), Instant::now() + self.inner.opts.mirror_sticky);
                tracing::warn!("[download] {host} unreachable ({err}) — using {mirror_host}");
            } else {
                m.remove(&host);
            }
        }
        match attempt(second, false).await {
            Ok(v) => Ok(v),
            Err(err2) => {
                if !engine_is_outage(&err2) {
                    return Err(err2);
                }
                let kind = match &err2 {
                    EngineError::Http(h) => h.kind.clone(),
                    _ => HttpErrorKind::Network,
                };
                Err(http_err(kind, url, format!("could not reach {host} nor its mirror {mirror_host}: {err}; {err2}")))
            }
        }
    }

    // ---- small fetches ----------------------------------------------------

    /// GET a small text payload (retries + mirror fallback).
    pub async fn fetch_text(&self, url: &str, opts: &FetchOptions) -> EngineResult<String> {
        let bytes = self.fetch_bytes(url, opts).await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// GET + parse JSON (`invalid JSON from <url>` on a bad body).
    pub async fn fetch_json<T: DeserializeOwned>(&self, url: &str, opts: &FetchOptions) -> EngineResult<T> {
        let bytes = self.fetch_bytes(url, opts).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| http_err(HttpErrorKind::InvalidJson, url, format!("invalid JSON from {url}: {e}")))
    }

    /// GET a small binary payload (same semantics as fetch_text).
    pub async fn fetch_bytes(&self, url: &str, opts: &FetchOptions) -> EngineResult<Vec<u8>> {
        let attempts = self.inner.opts.fetch_attempts.max(1);
        self.via_mirror(url, |u, fail_fast| async move {
            self.fetch_once(&u, opts, if fail_fast { 1 } else { attempts }).await
        })
        .await
    }

    async fn backoff(&self, attempt: u32) {
        let b = &self.inner.opts.retry_backoff;
        let d = b.get(attempt as usize - 1).or(b.last()).copied().unwrap_or(Duration::ZERO);
        tokio::time::sleep(d).await;
    }

    async fn fetch_once(&self, url: &str, opts: &FetchOptions, max_attempts: u32) -> EngineResult<Vec<u8>> {
        let host = url_host(url);
        let mut attempt = 1u32;
        loop {
            let mut req = self.inner.client.get(url);
            for (k, v) in &opts.headers {
                req = req.header(k, v);
            }
            let sent = tokio::time::timeout(opts.timeout, req.send()).await;
            let res = match sent {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    if attempt < max_attempts {
                        self.backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    let reason = network_reason(&e);
                    let kind = if e.is_timeout() { HttpErrorKind::Timeout } else { HttpErrorKind::Network };
                    return Err(http_err(
                        kind,
                        url,
                        format!("could not reach {host} ({reason}) after {attempt} attempts"),
                    ));
                }
                Err(_) => {
                    if attempt < max_attempts {
                        self.backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(http_err(
                        HttpErrorKind::Timeout,
                        url,
                        format!("could not reach {host} (request timeout) after {attempt} attempts"),
                    ));
                }
            };
            let status = res.status().as_u16();
            if !res.status().is_success() {
                drop(res);
                if RETRYABLE_STATUS.contains(&status) && attempt < max_attempts {
                    self.backoff(attempt).await;
                    attempt += 1;
                    continue;
                }
                return Err(http_err(HttpErrorKind::Status(status), url, format!("GET {url} -> HTTP {status}")));
            }
            return match tokio::time::timeout(opts.timeout, res.bytes()).await {
                Ok(Ok(b)) => Ok(b.to_vec()),
                Ok(Err(e)) => Err(http_err(
                    HttpErrorKind::Interrupted,
                    url,
                    format!("could not read {host} ({})", network_reason(&e)),
                )),
                Err(_) => {
                    Err(http_err(HttpErrorKind::Timeout, url, format!("could not read {host} (request timeout)")))
                }
            };
        }
    }

    // ---- file downloads ---------------------------------------------------

    fn dest_lock(&self, dest: &Path) -> Arc<tokio::sync::Mutex<()>> {
        let mut m = self.inner.dest_locks.lock().unwrap_or_else(|p| p.into_inner());
        m.entry(dest.to_path_buf()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
    }

    fn release_dest_lock(&self, dest: &Path, lock: Arc<tokio::sync::Mutex<()>>) {
        let mut m = self.inner.dest_locks.lock().unwrap_or_else(|p| p.into_inner());
        // map + `lock` = 2 -> nobody else waits on it.
        if Arc::strong_count(&lock) <= 2 {
            m.remove(dest);
        }
    }

    /// Verified download of `url` into `dest` (see module docs). Concurrent
    /// calls for the same `dest` are serialized.
    pub async fn download_file(&self, url: &str, dest: &Path, opts: &DownloadOptions) -> EngineResult<PathBuf> {
        let lock = self.dest_lock(dest);
        let r = {
            let _g = lock.lock().await;
            self.via_mirror(url, |u, _| async move { self.download_inner(&u, dest, opts).await }).await
        };
        self.release_dest_lock(dest, lock);
        r
    }

    async fn finalize_part(
        &self,
        part: &Path,
        dest: &Path,
        sha1: Option<&str>,
        size: Option<u64>,
    ) -> EngineResult<PathBuf> {
        if sha1.is_none() {
            let (fresh, current) = tokio::join!(sha1_file_safe(part), sha1_file_safe(dest));
            if fresh.is_some() && fresh == current {
                let _ = tokio::fs::remove_file(part).await;
                return Ok(dest.to_path_buf());
            }
        }
        if tokio::fs::rename(part, dest).await.is_err() {
            if let Some(s) = sha1 {
                if sha1_file_safe(dest).await.as_deref() == Some(s) {
                    return Ok(dest.to_path_buf());
                }
            } else if size.is_some() && file_size(dest).await == size {
                return Ok(dest.to_path_buf());
            }
            return Err(EngineError::other(format!(
                "rename failed for {} and no verified file in place",
                dest.display()
            )));
        }
        Ok(dest.to_path_buf())
    }

    async fn download_inner(&self, url: &str, dest: &Path, opts: &DownloadOptions) -> EngineResult<PathBuf> {
        let sha1 = opts.sha1.as_deref().filter(|s| !s.is_empty());
        let size = opts.size;
        let timeout = opts.timeout.unwrap_or(Duration::from_secs(120));
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Already present and verified -> nothing to do.
        if let Some(s) = sha1 {
            if sha1_file_safe(dest).await.as_deref() == Some(s) {
                return Ok(dest.to_path_buf());
            }
        } else if size.is_some() && file_size(dest).await == size {
            return Ok(dest.to_path_buf());
        }
        let part = part_path(dest);
        // A completed .part that verifies is just a finished download.
        if let Some(s) = sha1 {
            if file_size(&part).await.unwrap_or(0) > 0 && sha1_file_safe(&part).await.as_deref() == Some(s) {
                return self.finalize_part(&part, dest, sha1, size).await;
            }
        }
        let host = url_host(url);
        for _attempt in 0..self.inner.opts.download_attempts.max(1) {
            let part_size = file_size(&part).await.unwrap_or(0);
            let mut req = self.inner.client.get(url);
            for (k, v) in &opts.headers {
                req = req.header(k, v);
            }
            if part_size > 0 {
                req = req.header(reqwest::header::RANGE, format!("bytes={part_size}-"));
            }
            let res = match tokio::time::timeout(timeout, req.send()).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    let kind = if e.is_timeout() { HttpErrorKind::Timeout } else { HttpErrorKind::Network };
                    return Err(http_err(kind, url, format!("could not reach {host} ({})", network_reason(&e))));
                }
                Err(_) => {
                    return Err(http_err(
                        HttpErrorKind::Timeout,
                        url,
                        format!("could not reach {host} (download timeout)"),
                    ))
                }
            };
            let status = res.status().as_u16();
            if status == 416 {
                drop(res);
                if let Some(s) = sha1 {
                    if sha1_file_safe(&part).await.as_deref() == Some(s) {
                        return self.finalize_part(&part, dest, sha1, size).await;
                    }
                } else if size.is_some() && file_size(&part).await == size {
                    return self.finalize_part(&part, dest, sha1, size).await;
                }
                let _ = tokio::fs::remove_file(&part).await;
                continue;
            }
            if !res.status().is_success() {
                return Err(http_err(HttpErrorKind::Status(status), url, format!("GET {url} -> HTTP {status}")));
            }
            let append = status == 206 && part_size > 0;
            let hinted = res.content_length().unwrap_or(0) + if append { part_size } else { 0 };
            let total = size.or(if hinted > 0 { Some(hinted) } else { None });
            let mut received = if append { part_size } else { 0 };
            let mut out = if append {
                tokio::fs::OpenOptions::new().append(true).open(&part).await?
            } else {
                tokio::fs::File::create(&part).await?
            };
            let mut stream = res.bytes_stream();
            let mut interrupted = false;
            loop {
                match tokio::time::timeout(timeout, stream.next()).await {
                    Err(_) => {
                        // stalled (no data for `timeout`) — retry from the .part
                        interrupted = true;
                        break;
                    }
                    Ok(None) => break,
                    Ok(Some(Err(e))) => {
                        tracing::debug!("[download] {url} interrupted: {}", network_reason(&e));
                        interrupted = true;
                        break;
                    }
                    Ok(Some(Ok(chunk))) => {
                        out.write_all(&chunk).await?;
                        received += chunk.len() as u64;
                        if let Some(cb) = &opts.on_progress {
                            cb(received, total);
                        }
                    }
                }
            }
            out.flush().await?;
            drop(out);
            if interrupted {
                continue;
            }
            if let Some(s) = sha1 {
                let actual = sha1_file_safe(&part).await;
                if actual.as_deref() != Some(s) {
                    let _ = tokio::fs::remove_file(&part).await;
                    return Err(http_err(
                        HttpErrorKind::Sha1Mismatch,
                        url,
                        format!(
                            "sha1 mismatch for {url}: expected {s}, got {}",
                            actual.unwrap_or_else(|| "null".into())
                        ),
                    ));
                }
            } else if let Some(sz) = size {
                let actual = file_size(&part).await;
                if actual != Some(sz) {
                    let _ = tokio::fs::remove_file(&part).await;
                    return Err(http_err(
                        HttpErrorKind::SizeMismatch,
                        url,
                        format!(
                            "size mismatch for {url}: expected {sz}, got {}",
                            actual.map(|a| a as i64).unwrap_or(-1)
                        ),
                    ));
                }
            }
            return self.finalize_part(&part, dest, sha1, size).await;
        }
        Err(http_err(HttpErrorKind::Interrupted, url, format!("download failed after retries: {url}")))
    }

    fn emit_item(&self, ev: &ItemEvent, base: Value, tail: Value) {
        let mut m: Map<String, Value> = base.as_object().cloned().unwrap_or_default();
        if let Ok(Value::Object(ctx)) = serde_json::to_value(&ev.context) {
            for (k, v) in ctx {
                m.insert(k, v);
            }
        }
        if let Value::Object(t) = tail {
            for (k, v) in t {
                m.insert(k, v);
            }
        }
        if let Ok(p) = serde_json::from_value::<ProgressEvent>(Value::Object(m)) {
            self.inner.events.emit(ev.channel.wrap(p));
        }
    }

    /// Concurrent batch download (JS downloadAll). Ok = every dest, in order.
    pub async fn download_all(
        &self,
        items: Vec<DownloadItem>,
        opts: DownloadAllOptions,
    ) -> Result<Vec<PathBuf>, DownloadAllError> {
        use serde_json::json;
        let total = items.len();
        let concurrency = opts.concurrency.unwrap_or_else(|| self.default_concurrency()).max(1);
        let on_item = opts.on_item.clone();
        let results: Vec<(usize, Result<PathBuf, String>, DownloadItem)> =
            futures_util::stream::iter(items.into_iter().enumerate())
                .map(|(i, item)| {
                    let on_item = on_item.clone();
                    async move {
                        let instance = item.event.as_ref().and_then(|e| e.context.instance.clone());
                        if let Some(ev) = &item.event {
                            self.emit_item(
                                ev,
                                json!({ "instance": instance, "phase": "start", "index": i, "total": total }),
                                Value::Null,
                            );
                        }
                        let dl = DownloadOptions {
                            sha1: item.sha1.clone(),
                            size: item.size,
                            headers: item.headers.clone(),
                            timeout: None,
                            on_progress: None,
                        };
                        let r = self.download_file(&item.url, &item.dest, &dl).await.map_err(|e| e.to_string());
                        match &r {
                            Ok(dest) => {
                                if let Some(ev) = &item.event {
                                    self.emit_item(
                                        ev,
                                        json!({ "instance": instance, "phase": "done", "index": i, "total": total }),
                                        json!({ "dest": dest.to_string_lossy(), "sha1": item.sha1 }),
                                    );
                                }
                            }
                            Err(msg) => {
                                if let Some(ev) = &item.event {
                                    self.emit_item(
                                        ev,
                                        json!({ "instance": instance, "phase": "error", "index": i, "total": total }),
                                        json!({ "error": msg, "message": msg }),
                                    );
                                }
                            }
                        }
                        if let Some(cb) = &on_item {
                            cb(&ItemOutcome {
                                index: i,
                                ok: r.is_ok(),
                                dest: r.as_ref().cloned().unwrap_or_else(|_| item.dest.clone()),
                                error: r.as_ref().err().cloned(),
                                item: item.clone(),
                            });
                        }
                        (i, r, item)
                    }
                })
                .buffer_unordered(concurrency)
                .collect()
                .await;
        let mut ordered: Vec<Option<(Result<PathBuf, String>, DownloadItem)>> = (0..total).map(|_| None).collect();
        for (i, r, item) in results {
            ordered[i] = Some((r, item));
        }
        let mut failures = vec![];
        let mut out = Vec::with_capacity(total);
        for (r, item) in ordered.into_iter().flatten() {
            if let Err(e) = &r {
                failures.push(DownloadFailure { url: item.url.clone(), dest: item.dest.clone(), error: e.clone() });
            }
            out.push(r);
        }
        if !failures.is_empty() {
            return Err(DownloadAllError { total, failures, results: out });
        }
        Ok(out.into_iter().map(|r| r.expect("checked")).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EngineEvent;
    use crate::test_support::{MockResponse, MockServer};

    fn client(mirrors: Vec<(String, String)>) -> (HttpClient, EventBus) {
        let bus = EventBus::new();
        let opts = HttpOptions {
            mirrors,
            retry_backoff: vec![Duration::from_millis(1), Duration::from_millis(1)],
            use_system_proxy: false,
            ..Default::default()
        };
        (HttpClient::new(opts, bus.clone(), None).unwrap(), bus)
    }

    fn set_old_mtime(p: &Path) {
        let t = std::time::SystemTime::now() - Duration::from_secs(300);
        let f = std::fs::File::options().write(true).open(p).unwrap();
        f.set_modified(t).unwrap();
    }

    #[tokio::test]
    async fn hashless_redownload_keeps_identical_file_and_mtime() {
        let srv = MockServer::start(|_| MockResponse::ok("identical-bytes")).await;
        let (c, _) = client(vec![]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("intermediary-0.0.0.jar");
        std::fs::write(&dest, "identical-bytes").unwrap();
        set_old_mtime(&dest);
        let before = std::fs::metadata(&dest).unwrap().modified().unwrap();
        c.download_file(&srv.url("/intermediary.jar"), &dest, &DownloadOptions::default()).await.unwrap();
        assert_eq!(std::fs::metadata(&dest).unwrap().modified().unwrap(), before, "mtime must survive");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "identical-bytes");
        assert!(!part_path(&dest).exists());
    }

    #[tokio::test]
    async fn hashless_redownload_replaces_different_bytes() {
        let srv = MockServer::start(|_| MockResponse::ok("new-bytes-longer")).await;
        let (c, _) = client(vec![]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("fabric-loader.jar");
        std::fs::write(&dest, "old-bytes").unwrap();
        set_old_mtime(&dest);
        let before = std::fs::metadata(&dest).unwrap().modified().unwrap();
        c.download_file(&srv.url("/fabric-loader.jar"), &dest, &DownloadOptions::default()).await.unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new-bytes-longer");
        assert!(std::fs::metadata(&dest).unwrap().modified().unwrap() > before);
        assert!(!part_path(&dest).exists());
    }

    #[tokio::test]
    async fn matching_sha1_issues_no_request() {
        let srv = MockServer::start(|_| MockResponse::status(500)).await;
        let (c, _) = client(vec![]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("asm.jar");
        std::fs::write(&dest, "hashed").unwrap();
        let opts =
            DownloadOptions { sha1: Some("6318553899daae2941718c02508aeee938af1a1c".into()), ..Default::default() };
        c.download_file(&srv.url("/asm.jar"), &dest, &opts).await.unwrap();
        assert_eq!(srv.hits(), 0);
    }

    #[tokio::test]
    async fn sha1_mismatch_is_reported_and_part_removed() {
        let srv = MockServer::start(|_| MockResponse::ok("wrong")).await;
        let (c, _) = client(vec![]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("x.jar");
        let opts =
            DownloadOptions { sha1: Some("6318553899daae2941718c02508aeee938af1a1c".into()), ..Default::default() };
        let err = c.download_file(&srv.url("/x.jar"), &dest, &opts).await.unwrap_err();
        assert!(err.has_code("sha1_mismatch"), "{err}");
        assert!(err.to_string().starts_with("sha1 mismatch for "));
        assert!(!part_path(&dest).exists() && !dest.exists());
    }

    #[tokio::test]
    async fn resumes_a_partial_download_with_range() {
        let body = "0123456789abcdef";
        let srv = MockServer::start(move |req| match req.header("range") {
            Some(r) => {
                let from: usize = r.trim_start_matches("bytes=").trim_end_matches('-').parse().unwrap();
                MockResponse::ok(&body[from..]).with_status(206)
            }
            None => MockResponse::ok(body),
        })
        .await;
        let (c, _) = client(vec![]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("big.bin");
        std::fs::write(part_path(&dest), "01234").unwrap();
        let opts = DownloadOptions { sha1: Some(sha1_hex(body.as_bytes())), ..Default::default() };
        c.download_file(&srv.url("/big.bin"), &dest, &opts).await.unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), body);
        assert_eq!(srv.requests()[0].header("range").as_deref(), Some("bytes=5-"));
    }

    #[tokio::test]
    async fn fetch_json_survives_a_transient_reset() {
        let srv = MockServer::start_seq(|n, _| {
            if n == 0 {
                MockResponse::drop_connection()
            } else {
                MockResponse::ok(r#"{"ok":true}"#)
            }
        })
        .await;
        let (c, _) = client(vec![]);
        let v: Value = c.fetch_json(&srv.url("/m.json"), &FetchOptions::default()).await.unwrap();
        assert_eq!(v, serde_json::json!({ "ok": true }));
        assert_eq!(srv.hits(), 2);
    }

    #[tokio::test]
    async fn fetch_names_host_and_reason_after_all_attempts() {
        let srv = MockServer::start(|_| MockResponse::drop_connection()).await;
        let (c, _) = client(vec![]);
        let err = c.fetch_text(&srv.url("/v1/x.json"), &FetchOptions::default()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&srv.host()), "{msg}");
        assert!(msg.contains("after 3 attempts"), "{msg}");
        assert_ne!(msg, "fetch failed");
        assert_eq!(srv.hits(), 3);
    }

    #[tokio::test]
    async fn fetch_does_not_retry_a_404_but_retries_503() {
        let srv = MockServer::start(|_| MockResponse::status(404)).await;
        let (c, _) = client(vec![]);
        let err = c.fetch_text(&srv.url("/9.9.json"), &FetchOptions::default()).await.unwrap_err();
        assert_eq!(err.status(), 404);
        assert_eq!(srv.hits(), 1);
        let srv = MockServer::start(|_| MockResponse::status(503)).await;
        let err = c.fetch_text(&srv.url("/x"), &FetchOptions::default()).await.unwrap_err();
        assert_eq!(err.status(), 503);
        assert_eq!(srv.hits(), 3);
    }

    #[tokio::test]
    async fn mirror_fallback_on_outage_and_sticks() {
        let primary = MockServer::start(|_| MockResponse::drop_connection()).await;
        let mirror = MockServer::start(|req| MockResponse::ok(&format!(r#"{{"path":"{}"}}"#, req.path))).await;
        let (c, _) = client(vec![(primary.host(), mirror.host())]);
        let v: Value = c.fetch_json(&primary.url("/v2/versions/loader/26.2"), &FetchOptions::default()).await.unwrap();
        assert_eq!(v["path"], "/v2/versions/loader/26.2");
        assert_eq!((primary.hits(), mirror.hits()), (1, 1), "one fast primary try, then the mirror");
        let _: Value = c.fetch_json(&primary.url("/v2/versions/loader"), &FetchOptions::default()).await.unwrap();
        assert_eq!((primary.hits(), mirror.hits()), (1, 2), "mirror preferred now");
        c.reset_mirror_preference();
        let _: Value = c.fetch_json(&primary.url("/v2/x"), &FetchOptions::default()).await.unwrap();
        assert_eq!(primary.hits(), 2);
    }

    #[tokio::test]
    async fn a_4xx_is_a_real_answer_not_an_outage() {
        let primary = MockServer::start(|_| MockResponse::status(400)).await;
        let mirror = MockServer::start(|_| MockResponse::ok("{}")).await;
        let (c, _) = client(vec![(primary.host(), mirror.host())]);
        let err = c.fetch_text(&primary.url("/v2/versions/loader/1.0"), &FetchOptions::default()).await.unwrap_err();
        assert_eq!(err.status(), 400);
        assert_eq!(mirror.hits(), 0);
    }

    #[tokio::test]
    async fn both_hosts_down_names_both() {
        let primary = MockServer::start(|_| MockResponse::drop_connection()).await;
        let mirror = MockServer::start(|_| MockResponse::drop_connection()).await;
        let (c, _) = client(vec![(primary.host(), mirror.host())]);
        let err = c.fetch_text(&primary.url("/v2/versions/loader"), &FetchOptions::default()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&format!("could not reach {} nor its mirror {}", primary.host(), mirror.host())), "{msg}");
    }

    #[tokio::test]
    async fn jar_downloads_from_the_mirror() {
        let primary = MockServer::start(|_| MockResponse::drop_connection()).await;
        let mirror = MockServer::start(|_| MockResponse::ok("loader-bytes")).await;
        let (c, _) = client(vec![(primary.host(), mirror.host())]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("fabric-loader-0.20.1.jar");
        c.download_file(&primary.url("/net/fabricmc/fabric-loader-0.20.1.jar"), &dest, &DownloadOptions::default())
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "loader-bytes");
    }

    #[tokio::test]
    async fn download_all_emits_progress_and_aggregates_failures() {
        let srv = MockServer::start(|req| {
            if req.path.ends_with("bad") {
                MockResponse::status(404)
            } else {
                MockResponse::ok("x")
            }
        })
        .await;
        let (c, bus) = client(vec![]);
        let mut rx = bus.subscribe();
        let d = tempfile::tempdir().unwrap();
        let ctx = |f: &str| ProgressEvent::for_instance("inst").filename(f).kind("library");
        let items = vec![
            DownloadItem::new(srv.url("/a"), d.path().join("a")).event(ProgressChannel::Mod, ctx("a")),
            DownloadItem::new(srv.url("/bad"), d.path().join("b")).event(ProgressChannel::Mod, ctx("b")),
        ];
        let seen = Arc::new(Mutex::new(vec![]));
        let seen2 = seen.clone();
        let err = c
            .download_all(
                items,
                DownloadAllOptions {
                    concurrency: Some(2),
                    on_item: Some(Arc::new(move |o| seen2.lock().unwrap().push((o.index, o.ok)))),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "downloadAll: 1/2 item(s) failed");
        assert!(err.results[0].is_ok() && err.results[1].is_err());
        let mut s = seen.lock().unwrap().clone();
        s.sort();
        assert_eq!(s, vec![(0, true), (1, false)]);
        let mut phases = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let EngineEvent::ModProgress(p) = ev {
                assert_eq!(p.instance.as_deref(), Some("inst"));
                assert_eq!(p.total, Some(2));
                if p.phase.as_deref() == Some("done") {
                    assert_eq!(p.sha1, Some(Value::Null));
                    assert!(p.dest.is_some());
                }
                phases.push(p.phase.unwrap());
            }
        }
        phases.sort();
        assert_eq!(phases, ["done", "error", "start", "start"]);
    }

    #[test]
    fn mirror_url_rewrites_host_only() {
        let (c, _) = client(DEFAULT_MIRRORS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect());
        assert_eq!(
            c.mirror_url("https://meta.fabricmc.net/v2/versions/loader?x=1").as_deref(),
            Some("https://meta2.fabricmc.net/v2/versions/loader?x=1")
        );
        assert_eq!(c.mirror_url("https://piston-meta.mojang.com/x"), None);
    }
}
