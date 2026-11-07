//! Accept loop, per-connection state machine and the byte pump.
//!
//! One task per listener, one task per connection. A connection goes
//! header (10 s deadline) → grant → `mvp` → destination policy → per-sub limits
//! → upstream connect (8 s deadline) → `OK` → bidirectional copy until EOF,
//! idle timeout (300 s) or the daily quota runs out.

use std::io::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::dest;
use crate::grant::Verifier;
use crate::limits::{Limits, Reject, Slot};
use crate::proto;
use crate::proxy_v2;
use crate::util::{fmt_rfc3339, now_unix};
use crate::Code;

/// Per-direction copy buffer. Two 16 KiB buffers per connection is ~12.8 MB at
/// the 400-connection cap, which leaves the 1 GB OVM2 plenty of headroom.
const BUF: usize = 16 * 1024;

/// Everything a connection needs; cloned per accepted socket.
#[derive(Clone)]
pub struct Server {
    cfg: Config,
    verifier: Verifier,
    limits: Limits,
}

impl Server {
    /// Builds the server from configuration, with the pinned production key.
    pub fn new(cfg: Config) -> Self {
        let limits = Limits::new(
            cfg.per_sub_concurrency,
            cfg.per_sub_rate_per_min,
            cfg.per_sub_daily_bytes,
            cfg.per_ip_concurrency,
            cfg.global_max,
        );
        Server {
            cfg,
            verifier: Verifier::pinned(),
            limits,
        }
    }

    /// Test seam: same server with an injected verifying key (used by the
    /// end-to-end tests, which cannot sign with the production key).
    pub fn new_with_verifier(cfg: Config, verifier: Verifier) -> Self {
        let limits = Limits::new(
            cfg.per_sub_concurrency,
            cfg.per_sub_rate_per_min,
            cfg.per_sub_daily_bytes,
            cfg.per_ip_concurrency,
            cfg.global_max,
        );
        Server {
            cfg,
            verifier,
            limits,
        }
    }

    /// Binds one listener and serves it (used by the tests).
    pub async fn serve_on(self, listener: TcpListener) -> std::io::Result<()> {
        log_line(&format!(
            "time={} event=listen addr={} workers={} global_max={}",
            fmt_rfc3339(now_unix()),
            listener.local_addr()?,
            self.cfg.workers,
            self.cfg.global_max
        ));
        self.accept_loop(listener).await
    }

    /// Binds every configured address and serves until the process dies.
    pub async fn run(self) -> std::io::Result<()> {
        let mut listeners = Vec::new();
        for addr in &self.cfg.listen {
            listeners.push(TcpListener::bind(addr).await?);
        }

        if self.cfg.allow_loopback_destinations {
            log_line(&format!(
                "time={} event=warning detail=loopback-destinations-enabled \
                 (test only: 127/8 and ::1 are reachable; never use in production)",
                fmt_rfc3339(now_unix())
            ));
        }

        // Prune the limit maps once a minute so they stay proportional to
        // recent traffic rather than to uptime. Detached: it lives as long as
        // the process and is never awaited.
        let limits = self.limits.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                limits.prune();
            }
        });

        let mut tasks = Vec::new();
        for listener in listeners {
            let server = self.clone();
            tasks.push(tokio::spawn(async move { server.serve_on(listener).await }));
        }
        for t in tasks {
            if let Err(e) = t.await {
                log_line(&format!("event=listen-task-error err={e}"));
            }
        }
        Ok(())
    }

    async fn accept_loop(self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    // Minecraft is latency-sensitive: no Nagle on either side.
                    let _ = stream.set_nodelay(true);
                    match self.limits.accept(peer.ip()) {
                        Ok(slot) => {
                            let server = self.clone();
                            tokio::spawn(
                                async move { server.connection(stream, peer, slot).await },
                            );
                        }
                        Err(reject) => {
                            let code = match reject {
                                Reject::Busy => Code::Busy,
                                Reject::Limit => Code::Limit,
                            };
                            tokio::spawn(
                                async move { reject_connection(stream, peer, code).await },
                            );
                        }
                    }
                }
                Err(e) => {
                    // EMFILE and friends: back off instead of spinning.
                    log_line(&format!("event=accept-error err={e}"));
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    /// Writes `ERR <code>` (best effort), half-closes and logs the connection.
    async fn fail(&self, client: &mut TcpStream, log: ConnLog, src: IpAddr, code: Code) -> Code {
        debug_assert!(code.is_client_error());
        let line = err_line(code);
        let _ = tokio::time::timeout(Duration::from_secs(2), send_line(client, &line)).await;
        let _ = client.shutdown().await;
        log.finish(src, code)
    }

    /// One connection, from the first byte to the end-of-connection log line.
    async fn connection(self, mut client: TcpStream, peer: SocketAddr, mut slot: Slot) -> Code {
        let mut log = ConnLog::new();
        let src = peer.ip();

        // 1. Handshake line, 10 s.
        let (req, pipelined) =
            match tokio::time::timeout(self.cfg.header_timeout, proto::read_header(&mut client))
                .await
            {
                Ok(Ok(v)) => v,
                Ok(Err(code)) => return self.fail(&mut client, log, src, code).await,
                Err(_) => return self.fail(&mut client, log, src, Code::Proto).await,
            };
        log.dest = Some(req.dest);

        // 2. Grant.
        let claims = match self.verifier.verify(&req.grant) {
            Ok(c) => c,
            Err(code) => return self.fail(&mut client, log, src, code).await,
        };
        log.sub = Some(claims.sub_short().to_owned());
        if !claims.is_mvp() {
            return self.fail(&mut client, log, src, Code::NotMvp).await;
        }

        // 3. Destination policy (IP literal only; the relay never resolves).
        if !dest::allowed_with(
            req.dest.ip(),
            req.dest.port(),
            self.cfg.allow_loopback_destinations,
        ) {
            return self.fail(&mut client, log, src, Code::Dest).await;
        }

        // 4. Per-sub concurrency, rate and quota.
        if self.limits.attach_sub(&mut slot, &claims.sub).is_err() {
            return self.fail(&mut client, log, src, Code::Limit).await;
        }

        // 5. Upstream, 8 s.
        let mut upstream = match tokio::time::timeout(
            self.cfg.connect_timeout,
            TcpStream::connect(req.dest),
        )
        .await
        {
            Ok(Ok(s)) => s,
            _ => return self.fail(&mut client, log, src, Code::Connect).await,
        };
        let _ = upstream.set_nodelay(true);

        // 5b. A destination behind a PROXY-aware front (`--proxy-v2`) needs the
        // player's real address before any Minecraft byte, or its per-IP limits
        // and bans would see the relay. The header is the relay's own bytes, so
        // it is neither charged to the quota nor counted in the log.
        if self.cfg.proxy_v2.contains(&proxy_v2::canonical(req.dest)) {
            let header = proxy_v2::encode(peer, req.dest);
            if upstream.write_all(&header).await.is_err() {
                return self.fail(&mut client, log, src, Code::Connect).await;
            }
        }

        // 6. Charge bytes the client pipelined right behind the header before
        // promising `OK` — a client that already overran its quota must get
        // `ERR limit`, not an `OK` followed by a dropped socket.
        let pipelined_len = pipelined.len() as u64;
        if pipelined_len > 0 && !slot.charge(pipelined_len) {
            return self.fail(&mut client, log, src, Code::Limit).await;
        }

        // 7. `OK`; from the next byte the pipe is transparent.
        if send_line(&mut client, "OK\n").await.is_err() {
            return log.finish(src, Code::Error);
        }
        if pipelined_len > 0 {
            if upstream.write_all(&pipelined).await.is_err() {
                return log.finish(src, Code::Error);
            }
            log.up += pipelined_len;
        }

        let (up, down, code) = self.pump(client, upstream, &slot).await;
        log.up += up;
        log.down += down;
        log.finish(src, code)
    }

    /// Transparent bidirectional copy with idle tracking and quota charging.
    /// Both directions run as two futures in this task: no extra tasks, and
    /// dropping them closes the sockets.
    async fn pump(&self, client: TcpStream, upstream: TcpStream, slot: &Slot) -> (u64, u64, Code) {
        let (mut cr, mut cw) = client.into_split();
        let (mut ur, mut uw) = upstream.into_split();
        let epoch = Instant::now();
        let last_activity = AtomicU64::new(0);
        let up = AtomicU64::new(0);
        let down = AtomicU64::new(0);

        let c2u = copy_dir(&mut cr, &mut uw, slot, &last_activity, &up, epoch);
        let u2c = copy_dir(&mut ur, &mut cw, slot, &last_activity, &down, epoch);
        tokio::pin!(c2u, u2c);

        let (mut a_done, mut b_done) = (false, false);
        let mut a = Code::Closed;
        let mut b = Code::Closed;
        let mut timed_out = false;
        loop {
            let idle_ms = last_activity.load(Ordering::Relaxed);
            let now_ms = epoch.elapsed().as_millis() as u64;
            let idle = self.cfg.idle_timeout.as_millis() as u64;
            if now_ms.saturating_sub(idle_ms) >= idle {
                timed_out = true;
                break;
            }
            let left = Duration::from_millis(idle - (now_ms - idle_ms));
            tokio::select! {
                r = &mut c2u, if !a_done => { a = r; a_done = true; }
                r = &mut u2c, if !b_done => { b = r; b_done = true; }
                _ = tokio::time::sleep(left) => {}
            }
            // A hard failure (quota breach, socket error) ends the session at
            // once; a clean EOF in one direction only half-closes it.
            let fatal = |c: Code| matches!(c, Code::Limit | Code::Error);
            if (a_done && fatal(a)) || (b_done && fatal(b)) {
                break;
            }
            if a_done && b_done {
                break;
            }
        }

        let code = if timed_out { Code::Idle } else { merge(a, b) };
        (
            up.load(Ordering::Relaxed),
            down.load(Ordering::Relaxed),
            code,
        )
    }
}

fn merge(a: Code, b: Code) -> Code {
    // A quota breach wins; then a socket error; otherwise a clean close.
    match (a, b) {
        (Code::Limit, _) | (_, Code::Limit) => Code::Limit,
        (Code::Error, _) | (_, Code::Error) => Code::Error,
        _ => Code::Closed,
    }
}

/// Copies one direction until EOF/error. Every chunk is charged to the sub's
/// daily quota *before* it is forwarded; forwarding also refreshes the idle
/// timer. Charging per read (rather than batching) keeps the quota exact —
/// one uncontended mutex and a hash lookup per 16 KiB costs nothing next to
/// the syscalls it guards.
async fn copy_dir<R, W>(
    read: &mut R,
    write: &mut W,
    slot: &Slot,
    last_activity: &AtomicU64,
    counter: &AtomicU64,
    epoch: Instant,
) -> Code
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; BUF];
    loop {
        let n = match read.read(&mut buf).await {
            Ok(0) => {
                let _ = write.shutdown().await;
                return Code::Closed;
            }
            Ok(n) => n,
            Err(_) => return Code::Error,
        };
        // Charge before forwarding: a chunk the sub cannot afford must never
        // reach the upstream.
        if !slot.charge(n as u64) {
            return Code::Limit;
        }
        if write.write_all(&buf[..n]).await.is_err() {
            return Code::Error;
        }
        counter.fetch_add(n as u64, Ordering::Relaxed);
        last_activity.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

async fn reject_connection(mut stream: TcpStream, peer: SocketAddr, code: Code) {
    let log = ConnLog::new();
    let line = err_line(code);
    let _ = tokio::time::timeout(Duration::from_secs(2), send_line(&mut stream, &line)).await;
    // Half-close so the ERR line is flushed rather than lost to an RST.
    let _ = stream.shutdown().await;
    log.finish(peer.ip(), code);
}

fn err_line(code: Code) -> String {
    format!("ERR {}\n", code.as_str())
}

async fn send_line(stream: &mut TcpStream, line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes()).await?;
    stream.flush().await
}

/// One end-of-connection log line. Never contains payload bytes, the grant or
/// the full subject — only the first 8 hex of `sub`.
struct ConnLog {
    started: Instant,
    sub: Option<String>,
    dest: Option<SocketAddr>,
    up: u64,
    down: u64,
}

impl ConnLog {
    fn new() -> Self {
        ConnLog {
            started: Instant::now(),
            sub: None,
            dest: None,
            up: 0,
            down: 0,
        }
    }

    fn finish(self, src: IpAddr, code: Code) -> Code {
        log_line(&format!(
            "time={} sub={} src={} dest={} up={} down={} dur={:.2}s result={}",
            fmt_rfc3339(now_unix()),
            self.sub.as_deref().unwrap_or("-"),
            src,
            self.dest
                .map(|d| d.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            self.up,
            self.down,
            self.started.elapsed().as_secs_f64(),
            code.as_str()
        ));
        code
    }
}

/// Single-line stdout logger (journald wants one record per line).
fn log_line(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn err_lines_match_protocol() {
        assert_eq!(err_line(Code::Proto), "ERR proto\n");
        assert_eq!(err_line(Code::Auth), "ERR auth\n");
        assert_eq!(err_line(Code::Expired), "ERR expired\n");
        assert_eq!(err_line(Code::NotMvp), "ERR notmvp\n");
        assert_eq!(err_line(Code::Dest), "ERR dest\n");
        assert_eq!(err_line(Code::Limit), "ERR limit\n");
        assert_eq!(err_line(Code::Connect), "ERR connect\n");
        assert_eq!(err_line(Code::Busy), "ERR busy\n");
    }

    #[test]
    fn merge_prefers_limit_then_error() {
        assert_eq!(merge(Code::Closed, Code::Limit), Code::Limit);
        assert_eq!(merge(Code::Error, Code::Closed), Code::Error);
        assert_eq!(merge(Code::Closed, Code::Closed), Code::Closed);
    }
}
