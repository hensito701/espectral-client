# ruta-relay

The **Ruta Espectral** relay: a tiny, stateless TCP relay that lets an Espectral
Client player route Minecraft traffic through `ruta.uhce.top` so the game server
never sees the player's IP. MVP-only, opt-in per server, TCP only.

- Wire contract and every rule: [`../PROTOCOL.md`](../PROTOCOL.md) — canonical
  copy of the shared spec; the spec text is verbatim, with a short
  "Relay implementation notes" block appended at the end listing the three
  ambiguities found and how the relay resolves them (none change the wire).
  If this README ever disagrees with `PROTOCOL.md`, the protocol wins.
- Code: `src/` — `proto` (header line), `grant` (Ed25519 compact JWS),
  `dest` (destination policy), `limits` (concurrency/rate/quota), `server`
  (accept loop, state machine, byte pump), `config` (flags/env).
- No files, no cache, no DNS, no TLS. One log line per connection to stdout
  (journald).

## Requirements

Rust ≥ 1.75 in WSL on this workstation (`~/.cargo/bin/cargo`, musl target
installed). Runtime target: Ubuntu 22.04 x86_64 (glibc 2.35), but the shipped
binary is **static** so glibc does not matter.

## Build

```bash
# from services/ruta-relay, inside WSL
~/.cargo/bin/cargo test --offline
~/.cargo/bin/cargo clippy --offline --all-targets -- -D warnings
~/.cargo/bin/cargo build --release --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/ruta-relay   # static-pie linked
```

If the target is missing: `rustup target add x86_64-unknown-linux-musl`.
No C toolchain is needed — every dependency (`ed25519-dalek`, `tokio`,
`base64`, `serde_json`) is pure Rust.

Release profile: `opt-level=z`, `lto`, `codegen-units=1`, `panic=abort`,
`strip` → ~758 KB (740 KiB), `static-pie linked`, `ldd` prints
"statically linked".

## Test + smoke

```bash
~/.cargo/bin/cargo test --offline           # 51 unit + 16 end-to-end
```

The end-to-end suite boots a real relay on 127.0.0.1 with the test-key seam
(`Server::new_with_verifier`) and a real echo upstream; it covers `OK` plus
byte round-trip (including a 100 KB burst and pipelined handshake bytes),
`proto/auth/expired/notmvp/dest/connect/limit/busy`, the 10 s header deadline,
the 300 s idle deadline, the daily quota and all three concurrency caps.

For a manual smoke run, build with `--features test-key` (never deployed) and
point the relay at a loopback echo server:

```bash
~/.cargo/bin/cargo build --release --features test-key
RUTA_TEST_KEY_SEED=000102...1f \
  target/release/ruta-relay --listen 127.0.0.1:25599 --allow-loopback-destinations
# then a tiny client that mints a grant with that seed and sends
#   ERT1 <grant> 127.0.0.1 <echo-port>\n
# gets `OK`, and sees its bytes echoed back. A grant without `mvp` gets
# `ERR notmvp`; `10.0.0.1` gets `ERR dest`.
```

## Flags

Every flag has an `RUTA_*` environment fallback; command line wins. Nothing is
read from a file.

| Flag | Env | Default |
|---|---|---|
| `--listen <IP:PORT>` (repeatable, or comma-separated) | `RUTA_LISTEN` | `0.0.0.0:25565` |
| `--per-sub-concurrency <N>` | `RUTA_PER_SUB_CONCURRENCY` | 4 |
| `--per-sub-rate <N>` (new connections/minute) | `RUTA_PER_SUB_RATE_PER_MIN` | 30 |
| `--per-sub-daily-bytes <SIZE>` (`8GiB`, `8g`, plain bytes) | `RUTA_PER_SUB_DAILY_BYTES` | 8 GiB, UTC day |
| `--per-ip-concurrency <N>` | `RUTA_PER_IP_CONCURRENCY` | 8 |
| `--global-max <N>` | `RUTA_GLOBAL_MAX` | 400 |
| `--header-timeout <DUR>` (`10s`, `500ms`, plain seconds) | `RUTA_HEADER_TIMEOUT` | 10 s |
| `--connect-timeout <DUR>` | `RUTA_CONNECT_TIMEOUT` | 8 s |
| `--idle-timeout <DUR>` (no bytes either way) | `RUTA_IDLE_TIMEOUT` | 300 s |
| `--workers <N>` (tokio threads) | `RUTA_WORKERS` | 1 |
| `--allow-loopback-destinations` | `RUTA_ALLOW_LOOPBACK_DESTINATIONS` | off |

`--allow-loopback-destinations` exists only so the loopback end-to-end tests and
a manual smoke run can reach a local echo server: it relaxes 127/8 and ::1 and
nothing else (private ranges, the port rules and every other check still apply).
**Never set it on the relay host.**

A `--features test-key` build (used only for local smoke runs, never deployed)
additionally honours `RUTA_TEST_KEY_SEED` (64 hex) to verify grants against a
throwaway Ed25519 key; without that env var it still uses the pinned key.

`-h/--help`, `-V/--version`. Bad input exits 2 with a message on stderr.

## Behaviour in one paragraph

A client connects and writes one line
`ERT1 <grant> <dest-ip> <dest-port>\n` (≤ 4096 bytes, 10 s). The relay answers
`OK\n` or `ERR <code>\n` and closes. After `OK` the socket is a transparent
bidirectional byte pipe with `TCP_NODELAY` on both sides, 16 KiB copy buffers
per direction, idle tracking (300 s) and byte counting. Order of checks
(what `ERR` you get when several apply): `proto` → `auth` → `expired` →
`notmvp` → `dest` → `limit` → `connect`; `busy` (global cap) and `limit`
(per-source-IP cap) are decided at accept, before the header is read.

## Log format

```
time=2026-11-07T12:26:38Z sub=a1b2c3d4 src=203.0.113.7 dest=1.2.3.4:25566 up=15 down=15 dur=0.01s result=closed
```

`sub=` is the first 8 hex of the verified subject (`-` if the failure came
before the grant verified), `up`/`down` are bytes client→dest and dest→client,
`result` is one of `closed` (both directions ended normally), `idle`, `limit`,
`error`, or one of the `ERR` codes above when the failure happened before `OK`.
Never payload bytes, never the grant.

## Deploy (OVM2)

1. Copy the static binary:
   ```bash
   scp target/x86_64-unknown-linux-musl/release/ruta-relay \
     ubuntu@<ovm2-ip>:/tmp/ruta-relay
   sudo install -m 0755 -o root -g root /tmp/ruta-relay /usr/local/bin/ruta-relay
   ```
2. Install the unit:
   ```bash
   sudo install -m 0644 deploy/ruta-relay.service /etc/systemd/system/ruta-relay.service
   sudo systemctl daemon-reload
   sudo systemctl enable --now ruta-relay
   ```
   The unit listens on **25565 and 443** and runs as a `DynamicUser` with only
   `CAP_NET_BIND_SERVICE` (for 443) — everything else is confined.
3. Open the ports (both are already open in the OCI security list per the
   protocol doc; ufw also needs them):
   ```bash
   sudo ufw allow 25565/tcp
   sudo ufw allow 443/tcp
   sudo ufw status numbered
   ```
4. Health check — the relay has no HTTP endpoint, but it answers any TCP
   connection with `ERR proto` when it is alive:
   ```bash
   systemctl is-active ruta-relay
   ss -ltnp | grep -E ':(25565|443)\b'
   printf 'PING\n' | timeout 3 nc 127.0.0.1 25565     # -> ERR proto
   journalctl -u ruta-relay -n 20 --no-pager
   ```
   From outside: `nc -vz ruta.uhce.top 25565` / `443`, and a real routed join
   from the client.

### Do not co-locate with anything that needs privileges

OVM2 also runs `garbd` and `tailscale`. The relay binds only 25565/443, has
`MemoryMax=128M`, `TasksMax=256`, `OOMScoreAdjust=200`, and at the 400-session
cap uses about 400 × (2 × 16 KiB + task overhead) ≈ 20 MB, so it cannot starve
garbd. It never resolves DNS and refuses every private, loopback, link-local,
CGNAT, multicast, documentation, NAT64 and 6to4-with-private-IPv4 destination,
so it cannot be used as a pivot into the tailnet.

## Protocol-doc notes

`PROTOCOL.md` holds the shared spec verbatim, with a short "Relay
implementation notes" block appended at the end. Four points where the
document could be read two ways, and what this implementation does:

1. **NAT64.** The deny list contains `64:ff9b::/96` *and* says "`::ffff:a.b.c.d`
   / NAT64 forms are checked against the IPv4 list". Both readings reject a
   NAT64 address embedding a private IPv4; they differ only for one embedding a
   *public* IPv4. This build denies the whole `64:ff9b::/96` prefix (literal
   reading, stricter) and additionally applies the IPv4 rules to
   `::ffff:0:0/96` and `2002::/16` (6to4). Ambiguity resolved conservatively;
   no protocol change.
2. **`exp`/`iat` and `v`.** The document lists `exp < now`, `iat > now+300` and
   a 32-hex `sub`, but does not say what to do with a grant that omits `exp`,
   `iat` or `v=1`. This build treats missing/non-numeric `exp`/`iat` as
   `ERR expired` and `v != 1` as `ERR auth`, matching the mod's
   `GrantVerifier`. No wire change.
3. **Handshake timeout code.** The eight codes have no dedicated "slow client"
   entry, so an unfinished line at 10 s produces `ERR proto` — the same answer
   a truncated line gets. No wire change.
4. **Quota direction.** The 8 GiB/day figure is a per-`sub` budget for the
   session's traffic; this build counts both directions combined (up + down)
   and charges before forwarding.

Everything else — codes, order of checks, limits, log fields — follows the
document as written.
