# espectral-id

Identity + entitlement service for Espectral Client personalization/cosmetics.
Python 3.12 · FastAPI + uvicorn + httpx + cryptography · SQLite (sessions,
challenges) + in-memory presence. Production: `https://id.espectral.es` on
OVM3 (`127.0.0.1:8091` behind nginx).

## Endpoints (`/v1`, all JSON)

Errors always `{"error":{"code","message"}}`. Bearer = session token from
`/v1/auth/verify`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET  | `/healthz` | – | `{ok, kid:"eid1"}` |
| POST | `/auth/challenge` | – | `{challenge_id, server_id(40 hex), expires_in:120}` |
| POST | `/auth/verify` | – | `{challenge_id, username}` → Mojang hasJoined → `{token, uuid, name, expires_at}`; challenge is single-use, TTL 120 s |
| POST | `/discord/identify` | – | `{code, redirect_uri}` → `{discord:{id,username,avatar}}` (launcher login, no local secret) |
| POST | `/discord/link` | ✓ | exchange + bind discord id to session uuid → `{discord:{…}, flags}` |
| DELETE | `/discord/link` | ✓ | unlink |
| GET  | `/grant` | ✓ | `{grant, flags, exp}` — Ed25519 compact JWS (`kid eid1`, `iss https://id.espectral.es`, `aud espectral-client`, `sub` uuid32, `iat`, `exp=iat+86400`). Flags recomputed live each call; always issued, even `[]`. |
| PUT  | `/presence` | ✓ | `{loadout}` validated against catalog + flags (disallowed items dropped); stored in MEMORY, TTL 600 s → `{ok, expires_at, loadout}` |
| DELETE | `/presence` | ✓ | stop sharing |
| POST | `/presence/query` | ✓ | `{uuids:[≤100]}` → `{players:[{uuid, loadout, grant}]}` for live presence only |

UUIDs are 32-hex lowercase (no dashes), matching Mojang `hasJoined` `id`.

`flags`: `supporter` = any donor role (VIP `648324691659849754`, VIP+
`1190433903698268211`, MVP `1190433806499446895`, MVP+ `1189380795744387182`);
`mvp` = MVP or MVP+. Guild `648307511648321536`; member-role lookups via bot
token, cached ≤60 s.

Rate limiting: modest sliding window per token/IP
(`/auth/challenge` 20/min, `/auth/verify` + `/discord/*` 10/min, `/presence/query`
30/min, other Bearer routes 60/min). `CF-Connecting-IP` is trusted for bucketing
only when the direct peer is `127.0.0.1` (nginx); it is never an auth signal.
Logs never contain tokens or request bodies.

## Layout on OVM3

```
/home/ubuntu/espectral-id/{app,venv,data,.env}
systemd: espectral-id.service (EnvironmentFile=.env, uvicorn 127.0.0.1:8091)
nginx:   /etc/nginx/sites-available/id.espectral.es → sites-enabled/
```

## Deploy

```bash
ESPECTRAL_ID_ENV=E:/NPv2/espectral-backup-2026-08-05/local-workspace/connections/.env \
  bash services/espectral-id/deploy/deploy.sh
```

Idempotent; `.env` is written 0600 via ssh stdin and never printed
(`VALUES=refresh` to overwrite an existing one). The script re-syncs
`app/cosmetics-catalog.json` from `src/engine/cosmetics-catalog.json`
(byte-identical; `tests/test_catalog_sync.py` enforces it).

## DNS / TLS / firewall (one-time, already applied)

- Cloudflare zone is **Flexible** (edge HTTPS → origin HTTP). A proxied record
  would carry bearer tokens in plaintext CF→origin, and the zone API token has
  no ruleset/pagerule/tunnel scope to set per-hostname Full (strict) — so the
  `id.espectral.es` A record is **DNS-only → 150.136.181.209** with a Let's
  Encrypt cert on the origin (certbot `dns-cloudflare`, creds
  `/etc/letsencrypt/cf-dns.ini`; auto-renews).
- Host firewall `CF_WEB` chain drops non-Cloudflare sources on 80/443, so one
  targeted rule opens 443 for direct TLS:
  `iptables -A CF_WEB -p tcp --dport 443 -j ACCEPT` (persisted in
  `/etc/iptables/rules.v4` + `rules.v6` for `::/0`; port 80 stays CF-only and
  only serves the ACME fallback + redirect anyway).
- Re-proxying through CF later requires first setting `ssl=strict` for this
  hostname (config rules) or migrating the whole zone to Full.

## Tests

```bash
python -m venv .venv && .venv/Scripts/pip install -r requirements.txt pytest
.venv/Scripts/pytest -q    # Mojang/Discord HTTP mocked; throwaway Ed25519 key
```
