//! End-to-end tests: a real relay listener on loopback, a real echo upstream
//! and real sockets. Covers OK + byte round-trip, every `ERR` path, the
//! per-sub concurrency limit, and the PROXY v2 header for listed upstreams.

mod common;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::time::Duration;

use common::{mint, sub, GrantSpec};
use ruta_relay::config::Config;
use ruta_relay::grant::Verifier;
use ruta_relay::server::Server;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Starts an echo upstream on 127.0.0.1:0 and returns its address.
async fn echo_upstream() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    match s.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if s.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
    addr
}

/// Test config: loopback destinations, generous limits unless overridden.
fn test_config() -> Config {
    Config {
        allow_loopback_destinations: true,
        per_sub_concurrency: 4,
        per_sub_rate_per_min: 30,
        per_sub_daily_bytes: 8 * 1024 * 1024 * 1024,
        per_ip_concurrency: 64,
        global_max: 64,
        header_timeout: Duration::from_secs(5),
        connect_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_secs(30),
        ..Config::default()
    }
}

/// Boots a relay on an ephemeral loopback port with the test key.
async fn start_relay(cfg: Config) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let verifier = Verifier::with_key(common::signing_key().verifying_key());
    let server = Server::new_with_verifier(cfg, verifier);
    tokio::spawn(async move {
        let _ = server.serve_on(listener).await;
    });
    addr
}

async fn connect(relay: SocketAddr) -> TcpStream {
    TcpStream::connect(relay).await.unwrap()
}

/// Sends a raw handshake line and returns the first reply line.
async fn handshake_line(relay: SocketAddr, line: &[u8]) -> String {
    let mut s = connect(relay).await;
    s.write_all(line).await.unwrap();
    read_line(&mut s).await
}

async fn read_line(s: &mut TcpStream) -> String {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    while let Ok(Ok(1)) = tokio::time::timeout(Duration::from_secs(5), s.read(&mut b)).await {
        if b[0] == b'\n' {
            break;
        }
        out.push(b[0]);
    }
    String::from_utf8_lossy(&out).to_string()
}

#[tokio::test]
async fn ok_then_round_trips_bytes_both_ways() {
    let upstream = echo_upstream().await;
    let relay = start_relay(test_config()).await;
    let grant = mint(&GrantSpec::mvp(&sub(1), common::now()));

    let mut s = connect(relay).await;
    let line = format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port());
    s.write_all(line.as_bytes()).await.unwrap();
    assert_eq!(read_line(&mut s).await, "OK");

    // Minecraft handshake bytes: including a NUL and a high byte, to prove the
    // pipe is transparent rather than line-oriented.
    let payload: &[u8] = &[0x10, 0x00, 0x2f, 0xff, b'h', b'i', 0x00];
    s.write_all(payload).await.unwrap();
    let mut got = vec![0u8; payload.len()];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut got))
        .await
        .expect("echo timed out")
        .expect("echo failed");
    assert_eq!(got, payload);

    // A larger burst across several reads.
    let big = vec![0xa5u8; 100_000];
    s.write_all(&big).await.unwrap();
    let mut back = vec![0u8; big.len()];
    tokio::time::timeout(Duration::from_secs(10), s.read_exact(&mut back))
        .await
        .expect("bulk echo timed out")
        .expect("bulk echo failed");
    assert_eq!(back, big);
    drop(s);
}

#[tokio::test]
async fn pipelined_bytes_behind_header_are_forwarded() {
    let upstream = echo_upstream().await;
    let relay = start_relay(test_config()).await;
    let grant = mint(&GrantSpec::mvp(&sub(2), common::now()));

    let mut s = connect(relay).await;
    let mut line = format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port()).into_bytes();
    // The client writes the Minecraft handshake in the same syscall.
    line.extend_from_slice(b"\x01\x02\x03");
    s.write_all(&line).await.unwrap();
    assert_eq!(read_line(&mut s).await, "OK");
    let mut got = [0u8; 3];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut got))
        .await
        .expect("echo timed out")
        .unwrap();
    assert_eq!(got, [1u8, 2, 3]);
}

#[tokio::test]
async fn err_proto_on_garbage() {
    let relay = start_relay(test_config()).await;
    assert_eq!(
        handshake_line(relay, b"HELLO world 1.2.3.4 25565\n").await,
        "ERR proto"
    );
    assert_eq!(handshake_line(relay, b"\n").await, "ERR proto");
    assert_eq!(
        handshake_line(relay, b"ERT1  a.b.c 1.2.3.4 25565\n").await,
        "ERR proto"
    );
    // CRLF is not accepted.
    let grant = mint(&GrantSpec::mvp(&sub(3), common::now()));
    let line = format!("ERT1 {grant} 1.2.3.4 25565\r\n");
    assert_eq!(handshake_line(relay, line.as_bytes()).await, "ERR proto");
}

#[tokio::test]
async fn err_auth_on_bad_signature_and_wrong_claims() {
    let relay = start_relay(test_config()).await;
    let good = mint(&GrantSpec::mvp(&sub(4), common::now()));

    // Flip the last character of the signature.
    let mut bad_sig = good.clone();
    let last = bad_sig.pop().unwrap();
    bad_sig.push(if last == 'A' { 'B' } else { 'A' });
    assert_eq!(
        handshake_line(relay, format!("ERT1 {bad_sig} 1.2.3.4 25565\n").as_bytes()).await,
        "ERR auth"
    );

    for (name, spec) in [
        (
            "aud",
            GrantSpec {
                aud: "other-client",
                ..GrantSpec::mvp(&sub(5), common::now())
            },
        ),
        (
            "iss",
            GrantSpec {
                iss: "https://evil.example",
                ..GrantSpec::mvp(&sub(5), common::now())
            },
        ),
        (
            "typ",
            GrantSpec {
                typ: "JWT",
                ..GrantSpec::mvp(&sub(5), common::now())
            },
        ),
        (
            "kid",
            GrantSpec {
                kid: "eid2",
                ..GrantSpec::mvp(&sub(5), common::now())
            },
        ),
        (
            "alg",
            GrantSpec {
                alg: "HS256",
                ..GrantSpec::mvp(&sub(5), common::now())
            },
        ),
    ] {
        let jws = mint(&spec);
        assert_eq!(
            handshake_line(relay, format!("ERT1 {jws} 1.2.3.4 25565\n").as_bytes()).await,
            "ERR auth",
            "{name}"
        );
    }
}

#[tokio::test]
async fn err_expired_on_expired_and_future_iat() {
    let relay = start_relay(test_config()).await;
    let now = common::now();

    let expired = GrantSpec {
        iat: now - 7200,
        exp: now - 3600,
        ..GrantSpec::mvp(&sub(6), now)
    };
    assert_eq!(
        handshake_line(
            relay,
            format!("ERT1 {} 1.2.3.4 25565\n", mint(&expired)).as_bytes()
        )
        .await,
        "ERR expired"
    );

    let future = GrantSpec {
        iat: now + 3600,
        exp: now + 7200,
        ..GrantSpec::mvp(&sub(6), now)
    };
    assert_eq!(
        handshake_line(
            relay,
            format!("ERT1 {} 1.2.3.4 25565\n", mint(&future)).as_bytes()
        )
        .await,
        "ERR expired"
    );
}

#[tokio::test]
async fn err_notmvp_without_mvp_flag() {
    let relay = start_relay(test_config()).await;
    // Destination is a private address: `notmvp` must win over `dest` (grant
    // is checked first), which also proves the ordering.
    let supporter = GrantSpec {
        flags: vec!["supporter".into()],
        ..GrantSpec::mvp(&sub(7), common::now())
    };
    assert_eq!(
        handshake_line(
            relay,
            format!("ERT1 {} 10.0.0.1 25565\n", mint(&supporter)).as_bytes()
        )
        .await,
        "ERR notmvp"
    );
    let empty = GrantSpec {
        flags: vec![],
        ..GrantSpec::mvp(&sub(8), common::now())
    };
    assert_eq!(
        handshake_line(
            relay,
            format!("ERT1 {} 1.2.3.4 25565\n", mint(&empty)).as_bytes()
        )
        .await,
        "ERR notmvp"
    );
}

#[tokio::test]
async fn err_dest_for_private_and_bad_ports() {
    let relay = start_relay(test_config()).await;
    let grant = mint(&GrantSpec::mvp(&sub(9), common::now()));
    for dest in [
        "10.0.0.1 25565",
        "192.168.1.1 25565",
        "169.254.169.254 25565",
        "100.64.0.1 25565",
        "::ffff:127.0.0.1 25565",
        "fc00::1 25565",
        "2001:db8::1 25565",
        "64:ff9b::a00:1 25565",
        "8.8.8.8 443",
        "8.8.8.8 3306",
        "1.1.1.1 27017",
    ] {
        assert_eq!(
            handshake_line(relay, format!("ERT1 {grant} {dest}\n").as_bytes()).await,
            "ERR dest",
            "{dest}"
        );
    }
}

#[tokio::test]
async fn err_connect_when_upstream_refuses() {
    let relay = start_relay(test_config()).await;
    let grant = mint(&GrantSpec::mvp(&sub(10), common::now()));
    // Bind then drop, so the port is (almost certainly) free.
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = l.local_addr().unwrap();
    drop(l);
    assert_eq!(
        handshake_line(
            relay,
            format!("ERT1 {grant} {} {}\n", closed.ip(), closed.port()).as_bytes()
        )
        .await,
        "ERR connect"
    );
}

#[tokio::test]
async fn per_sub_concurrency_limit() {
    let upstream = echo_upstream().await;
    let mut cfg = test_config();
    cfg.per_sub_concurrency = 2;
    let relay = start_relay(cfg).await;
    let grant = mint(&GrantSpec::mvp(&sub(11), common::now()));
    let line = format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port());

    // Two live sessions fit.
    let mut a = connect(relay).await;
    a.write_all(line.as_bytes()).await.unwrap();
    assert_eq!(read_line(&mut a).await, "OK");
    let mut b = connect(relay).await;
    b.write_all(line.as_bytes()).await.unwrap();
    assert_eq!(read_line(&mut b).await, "OK");

    // The third is refused while both are open.
    assert_eq!(handshake_line(relay, line.as_bytes()).await, "ERR limit");

    // A different sub is unaffected.
    let other = mint(&GrantSpec::mvp(&sub(12), common::now()));
    let mut c = connect(relay).await;
    c.write_all(format!("ERT1 {other} {} {}\n", upstream.ip(), upstream.port()).as_bytes())
        .await
        .unwrap();
    assert_eq!(read_line(&mut c).await, "OK");

    // Closing one frees its slot (the relay notices EOF asynchronously).
    drop(a);
    let mut last = String::new();
    for _ in 0..50 {
        let mut again = connect(relay).await;
        again.write_all(line.as_bytes()).await.unwrap();
        last = read_line(&mut again).await;
        if last == "OK" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    panic!("slot was never released, last reply {last:?}");
}

#[tokio::test]
async fn per_sub_rate_limit() {
    let upstream = echo_upstream().await;
    let mut cfg = test_config();
    cfg.per_sub_rate_per_min = 2;
    let relay = start_relay(cfg).await;
    let grant = mint(&GrantSpec::mvp(&sub(13), common::now()));
    let line = format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port());

    for _ in 0..2 {
        let mut s = connect(relay).await;
        s.write_all(line.as_bytes()).await.unwrap();
        assert_eq!(read_line(&mut s).await, "OK");
    }
    assert_eq!(handshake_line(relay, line.as_bytes()).await, "ERR limit");
}

#[tokio::test]
async fn per_ip_concurrency_limit() {
    let upstream = echo_upstream().await;
    let mut cfg = test_config();
    cfg.per_sub_concurrency = 10;
    cfg.per_ip_concurrency = 1;
    let relay = start_relay(cfg).await;

    // Holding a connection from the same IP (127.0.0.1) consumes the only slot.
    let mut a = connect(relay).await;
    let grant = mint(&GrantSpec::mvp(&sub(14), common::now()));
    a.write_all(format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port()).as_bytes())
        .await
        .unwrap();
    assert_eq!(read_line(&mut a).await, "OK");

    assert_eq!(
        handshake_line(relay, b"ERT1 a.b.c 1.2.3.4 25565\n").await,
        "ERR limit"
    );
}

#[tokio::test]
async fn global_cap_reports_busy() {
    let mut cfg = test_config();
    cfg.global_max = 1;
    cfg.per_ip_concurrency = 10;
    let relay = start_relay(cfg).await;

    // A connection that stalls before its header holds the single global slot.
    let _a = connect(relay).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        handshake_line(relay, b"ERT1 a.b.c 1.2.3.4 25565\n").await,
        "ERR busy"
    );
}

#[tokio::test]
async fn header_timeout_is_enforced() {
    let mut cfg = test_config();
    cfg.header_timeout = Duration::from_millis(200);
    let relay = start_relay(cfg).await;
    let mut s = connect(relay).await;
    // Send a partial line and stop.
    s.write_all(b"ERT1 abc").await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), read_line(&mut s))
        .await
        .expect("relay did not time out");
    assert_eq!(reply, "ERR proto");
}

#[tokio::test]
async fn oversize_header_is_rejected() {
    let relay = start_relay(test_config()).await;
    let mut s = connect(relay).await;
    let mut line = b"ERT1 ".to_vec();
    line.extend(std::iter::repeat(b'a').take(5000));
    line.extend_from_slice(b" 1.2.3.4 25565\n");
    s.write_all(&line).await.unwrap();
    assert_eq!(read_line(&mut s).await, "ERR proto");
}

#[tokio::test]
async fn idle_timeout_closes_a_quiet_session() {
    let upstream = echo_upstream().await;
    let mut cfg = test_config();
    cfg.idle_timeout = Duration::from_millis(300);
    let relay = start_relay(cfg).await;
    let grant = mint(&GrantSpec::mvp(&sub(15), common::now()));

    let mut s = connect(relay).await;
    s.write_all(format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port()).as_bytes())
        .await
        .unwrap();
    assert_eq!(read_line(&mut s).await, "OK");
    let mut b = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut b))
        .await
        .expect("relay did not close an idle session")
        .unwrap();
    assert_eq!(n, 0, "expected EOF, not data");
}

#[tokio::test]
async fn daily_quota_stops_the_session() {
    let upstream = echo_upstream().await;
    let mut cfg = test_config();
    cfg.per_sub_daily_bytes = 1000;
    let relay = start_relay(cfg).await;
    let grant = mint(&GrantSpec::mvp(&sub(16), common::now()));

    let mut s = connect(relay).await;
    s.write_all(format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port()).as_bytes())
        .await
        .unwrap();
    assert_eq!(read_line(&mut s).await, "OK");

    // 4 KiB through a 1 KiB budget: the relay must cut the session.
    let _ = s.write_all(&vec![7u8; 4096]).await;
    let mut sink = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut sink)).await;
    assert!(sink.len() < 4096, "quota did not cut the stream");
}

/// Hand-built PROXY v2 IPv4 header, written out byte by byte rather than via
/// `proxy_v2::encode`, so this end-to-end check does not share its encoder.
fn expected_v4_header(src: SocketAddr, dst: SocketAddr) -> Vec<u8> {
    let (SocketAddr::V4(s), SocketAddr::V4(d)) = (src, dst) else {
        panic!("test addresses are IPv4");
    };
    let mut h = vec![
        0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A, 0x21, 0x11,
        0x00, 0x0C,
    ];
    h.extend_from_slice(&s.ip().octets());
    h.extend_from_slice(&d.ip().octets());
    h.extend_from_slice(&s.port().to_be_bytes());
    h.extend_from_slice(&d.port().to_be_bytes());
    h
}

/// Starts an upstream that records the first `n` bytes it receives. Returns
/// its address and a handle that resolves to those bytes.
async fn capture_upstream(n: usize) -> (SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut got = vec![0u8; n];
        s.read_exact(&mut got).await.unwrap();
        got
    });
    (addr, task)
}

#[tokio::test]
async fn proxy_v2_header_precedes_payload_for_listed_upstream() {
    let payload: &[u8] = &[0x10, 0x00, 0x2f, 0xff, b'h', b'i', 0x00];
    // A 28-byte IPv4 PROXY v2 header (12 signature + 4 fixed + 12 addresses).
    let (upstream, captured) = capture_upstream(28 + payload.len()).await;
    let cfg = Config {
        proxy_v2: BTreeSet::from([upstream]),
        ..test_config()
    };
    let relay = start_relay(cfg).await;
    let grant = mint(&GrantSpec::mvp(&sub(1), common::now()));

    let mut s = connect(relay).await;
    let src = s.local_addr().unwrap();
    let line = format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port());
    s.write_all(line.as_bytes()).await.unwrap();
    assert_eq!(read_line(&mut s).await, "OK");
    s.write_all(payload).await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), captured)
        .await
        .expect("capture timed out")
        .unwrap();
    let header = expected_v4_header(src, upstream);
    assert_eq!(&got[..header.len()], &header[..]);
    assert_eq!(&got[header.len()..], payload);
    drop(s);
}

#[tokio::test]
async fn no_proxy_header_for_upstreams_not_listed() {
    let payload: &[u8] = b"not a PROXY front";
    let (upstream, captured) = capture_upstream(payload.len()).await;
    let cfg = Config {
        // Listed, but it is not the upstream above, which is a plain server.
        proxy_v2: BTreeSet::from(["127.0.0.2:25564".parse::<SocketAddr>().unwrap()]),
        ..test_config()
    };
    let relay = start_relay(cfg).await;
    let grant = mint(&GrantSpec::mvp(&sub(1), common::now()));

    let mut s = connect(relay).await;
    let line = format!("ERT1 {grant} {} {}\n", upstream.ip(), upstream.port());
    s.write_all(line.as_bytes()).await.unwrap();
    assert_eq!(read_line(&mut s).await, "OK");
    s.write_all(payload).await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), captured)
        .await
        .expect("capture timed out")
        .unwrap();
    assert_eq!(got, payload);
    drop(s);
}
