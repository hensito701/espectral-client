# Ruta Espectral — relay protocol v1 ("ERT1")

MVP-only, opt-in per server, Minecraft (TCP) traffic only. Relay host: `ruta.uhce.top`
(Cloudflare DNS-only A record → OVM2 `<ovm2-ip>`). The relay listens on **25565 and 443**
(both open in OVM2's OCI security list and ufw). Client: connect to `ruta.uhce.top:25565`; if the
TCP connect fails or times out (3 s), retry once on `ruta.uhce.top:443` (networks that block
Minecraft's port usually allow 443). Host and ports are constants in one place on the client; the
server takes repeatable `--listen` flags.

## Handshake
Immediately after the TCP connect, the client sends ONE ASCII line:

```
ERT1 <grant> <dest-ip> <dest-port>\n
```

- `<grant>`: the player's espectral-id entitlement grant, verbatim compact JWS (the same string the
  mod already reads from `config/espectral-cosmetics.json` key `grant` and verifies with
  `GrantVerifier`).
- `<dest-ip>`: the destination server's already-resolved IP literal (client resolves DNS + SRV
  itself; the relay NEVER does DNS). IPv4 dotted quad or IPv6 text form without brackets.
- `<dest-port>`: decimal 1–65535.
- Whole line ≤ 4096 bytes including `\n`; fields separated by single spaces. Anything else → `ERR proto`.
- The client must send the line within 10 s of connecting.

Relay answers with one line:
- `OK\n` — from the next byte on, the connection is a transparent bidirectional byte pipe to the
  destination (the client then speaks normal Minecraft protocol: handshake/status/login).
- `ERR <code>\n` then close. Codes: `proto` (malformed), `auth` (bad/unknown signature, wrong aud/iss/typ/kid/alg),
  `expired` (exp passed / iat in future), `notmvp` (flags lack `mvp`), `dest` (destination not allowed),
  `limit` (rate/concurrency/quota limit hit), `connect` (upstream unreachable / timeout), `busy` (global cap).

The client treats anything other than `OK\n` as a failed connection and shows a clear localized
message (es/en) naming Ruta Espectral and the code; it must NOT silently fall back to a direct
connection (the player chose the route, e.g. to hide their IP).

## Grant verification (relay) — identical rules to the mod's GrantVerifier
Compact JWS `b64url(header).b64url(payload).b64url(sig)`, Ed25519 over ASCII of the first two parts.
Header `{"alg":"EdDSA","kid":"eid1","typ":"ESG"}`. Payload
`{"v":1,"iss":"https://id.uhce.top"|"https://id.espectral.es","aud":"espectral-client","sub":"<32 lowercase hex>","flags":[...],"iat":s,"exp":s}`.
Pinned key kid `eid1`, SPKI DER base64 `MCowBQYDK2VwAyEA77kg5+/Nn6NvArCPuDvLYu3E4rvmYJHzaCGxwyvdlCI=`.
Reject on: exp < now, iat > now+300, sub not 32-hex, flags without `mvp`.

## Destination policy (relay)
Only public unicast IPs. Deny IPv4: 0/8, 10/8, 100.64/10, 127/8, 169.254/16, 172.16/12, 192.0.0/24,
192.0.2/24, 192.88.99/24, 192.168/16, 198.18/15, 198.51.100/24, 203.0.113/24, 224/4, 240/4.
Deny IPv6: ::/128, ::1, fc00::/7, fe80::/10, ff00::/8, 2001:db8::/32, 64:ff9b::/96, 100::/64, and
`::ffff:a.b.c.d` / NAT64 forms are checked against the IPv4 list. Ports: 1024–65535 only, minus
{3306, 3389, 4567, 5432, 5900, 6379, 6380, 9200, 11211, 27017}.

## Limits (defaults, all configurable)
per `sub`: ≤ 4 concurrent, ≤ 30 new connections / minute, ≤ 8 GiB per UTC day;
per source IP: ≤ 8 concurrent; global: ≤ 400 concurrent; header timeout 10 s; upstream connect
timeout 8 s; idle timeout 300 s (no bytes either direction).

## Logging
One line per connection end: time, `sub` first 8 hex, source IP, dest ip:port, bytes up/down,
duration, result code. Never payload bytes. No other persistence.

## Client behaviour (mod)
- Toggle per server in the multiplayer list, default OFF, visible/usable only when the player's own
  verified grant has `mvp`. Persisted per server address in the mod's own config file (not servers.dat).
- With the toggle ON, BOTH the server-list status ping and the actual join go through the relay
  (so the server never sees the player's IP, and the displayed ping is the real routed ping).
  No direct ping is made for routed servers.
- Espectral's own servers (`*.uhce.top`, `*.espectral.es`) are never routed (toggle hidden there):
  routing them would break AuthMe per-IP limits / AdvancedBan IP bans for every MVP.

---

## Relay implementation notes

*Appended by `services/ruta-relay`. Everything above is the shared spec,
unchanged; this block only records the four details the spec leaves open and
how the relay reads them. The client implementation is not affected by any of
it.*

1. **NAT64.** The IPv6 deny list contains `64:ff9b::/96`, and the same
   paragraph says `::ffff:a.b.c.d` / NAT64 forms are checked against the IPv4
   list. Both readings reject a NAT64 address that embeds a private IPv4; they
   differ only for one embedding a *public* IPv4. The relay denies the whole
   `64:ff9b::/96` prefix (the literal deny-list reading, stricter) and applies
   the IPv4 rules to `::ffff:0:0/96` (IPv4-mapped) and `2002::/16` (6to4).
2. **Unlisted claims.** For a grant that omits or malforms `exp`/`iat` the
   relay answers `ERR expired`; for `v != 1` it answers `ERR auth`. This
   matches the mod's `GrantVerifier` (missing `exp`/`iat` fail the window
   check, `v` must be 1).
3. **Handshake timeout.** A client that does not finish its line within 10 s
   gets `ERR proto` (the spec lists no dedicated code; `proto` is the closest
   of the eight and is what a truncated/garbled line produces anyway).
4. **Quota accounting.** The per-`sub` 8 GiB/day budget counts bytes in both
   directions combined. Each 16 KiB chunk is charged before it is forwarded and
   a chunk that does not fit is not forwarded at all, so a session stops exactly
   at the budget instead of overshooting it.
