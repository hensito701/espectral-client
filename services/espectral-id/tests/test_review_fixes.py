"""Regression tests for review fixes: per-uuid discord binding migration,
rate-limiter eviction/bounds, non-string loadout values, defensive
upstream JSON parsing, and grant issuance on Discord failure."""
from __future__ import annotations

import asyncio
import sqlite3
import time

import httpx
import pytest

from app import auth, catalog, external
from app.auth import RateLimiter
from app.db import Db
from app.errors import ApiError

from test_service import _pubkey, _verified_token, verify_grant


def _link(client, token, monkeypatch, discord_id="555"):
    async def fake_exchange(http, code, ruri, cid, secret):
        return {"discord": {"id": discord_id, "username": "u",
                            "avatar": None}}
    monkeypatch.setattr(external, "discord_exchange_code", fake_exchange)
    return client.post("/v1/discord/link",
                       headers={"Authorization": f"Bearer {token}"},
                       json={"code": "x", "redirect_uri": "cb"})


# -- finding 1: discord binding persists per uuid -------------------------


def test_discord_binding_survives_reverify(client, monkeypatch, settings):
    roles = ["1190433806499446895"]  # MVP
    token, uuid = _verified_token(client, monkeypatch, roles=roles)
    r = _link(client, token, monkeypatch)
    assert r.status_code == 200, r.text
    assert r.json()["flags"] == ["supporter", "mvp"]

    # Re-verify the same account: the old session row is wiped entirely.
    token2, uuid2 = _verified_token(client, monkeypatch, uuid=uuid,
                                    roles=roles)
    assert uuid2 == uuid and token2 != token
    assert client.get("/v1/grant",
                      headers={"Authorization": f"Bearer {token}"}
                      ).status_code == 401

    g = client.get("/v1/grant",
                   headers={"Authorization": f"Bearer {token2}"})
    assert g.status_code == 200
    assert verify_grant(g.json()["grant"], _pubkey(settings),
                        uuid)["flags"] == ["supporter", "mvp"]

    # Unlink removes the uuid-keyed binding.
    d = client.delete("/v1/discord/link",
                      headers={"Authorization": f"Bearer {token2}"})
    assert d.json() == {"ok": True}
    g2 = client.get("/v1/grant",
                    headers={"Authorization": f"Bearer {token2}"})
    assert g2.json()["flags"] == []


def test_migration_carries_existing_discord_ids(tmp_path):
    """An old-schema DB (discord_id on sessions) migrates without loss."""
    path = str(tmp_path / "old.db")
    conn = sqlite3.connect(path)
    conn.executescript(
        "CREATE TABLE challenges (challenge_id TEXT PRIMARY KEY,"
        " server_id TEXT NOT NULL, created_at INTEGER NOT NULL,"
        " expires_at INTEGER NOT NULL, used INTEGER NOT NULL DEFAULT 0);"
        "CREATE TABLE sessions (token_hash TEXT PRIMARY KEY,"
        " uuid TEXT NOT NULL, name TEXT NOT NULL, discord_id TEXT,"
        " created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);"
    )
    far_future = int(time.time()) + 86400
    conn.execute("INSERT INTO sessions VALUES (?,?,?,?,?,?)",
                 ("h1", "a" * 32, "Steve", "12345", 1, far_future))
    conn.execute("INSERT INTO sessions VALUES (?,?,?,?,?,?)",
                 ("h2", "b" * 32, "Alex", None, 1, far_future))
    conn.commit()
    conn.close()

    db = Db(path)
    try:
        row = db._conn.execute(
            "SELECT discord_id FROM discord_links WHERE uuid=?",
            ("a" * 32,)).fetchone()
        assert row["discord_id"] == "12345"
        assert db._conn.execute(
            "SELECT COUNT(*) c FROM discord_links").fetchone()["c"] == 1
        cols = {r["name"] for r in db._conn.execute(
            "PRAGMA table_info(sessions)")}
        assert "discord_id" not in cols
        # Sessions still resolvable, binding visible through get_session.
        sess = db.get_session("h1")
        assert sess["uuid"] == "a" * 32
        assert sess["discord_id"] == "12345"
    finally:
        db.close()


# -- finding 3: rate limiter eviction / bounds ----------------------------


def test_rate_limiter_evicts_drained_keys():
    rl = RateLimiter()
    rl.check("b", "k1", 5, window=0)   # every hit is instantly stale
    rl.check("b", "k1", 5, window=0)   # drained deque deleted, re-created
    assert list(rl._hits) == [("b", "k1")]

    rl.check("b", "k2", 5, window=0)
    rl._sweep(time.monotonic() + 1)
    assert rl._hits == {}


def test_rate_limiter_deque_bounded_by_limit():
    rl = RateLimiter()
    for _ in range(3):
        rl.check("b", "k", 3, window=60)
    with pytest.raises(ApiError) as ei:
        rl.check("b", "k", 3, window=60)
    assert ei.value.status == 429
    # The deque never grew past the limit.
    assert len(rl._hits[("b", "k")][1]) == 3


# -- finding 2: bearer auth hardening -------------------------------------


def test_bearer_attempts_rate_limited_per_ip(client, monkeypatch):
    monkeypatch.setattr(auth, "_BEARER_IP_LIMIT", 3)
    for _ in range(3):
        r = client.get("/v1/grant",
                       headers={"Authorization": "Bearer deadbeef"})
        assert r.status_code == 401
    r = client.get("/v1/grant",
                   headers={"Authorization": "Bearer deadbeef"})
    assert r.status_code == 429
    assert r.json()["error"]["code"] == "rate_limited"


def test_rate_limit_keyed_per_token(client, monkeypatch):
    token1, _ = _verified_token(client, monkeypatch, username="Steve")
    token2, _ = _verified_token(client, monkeypatch, username="Alex")
    h1 = {"Authorization": f"Bearer {token1}"}
    h2 = {"Authorization": f"Bearer {token2}"}
    for _ in range(60):
        assert client.delete("/v1/presence", headers=h1).status_code == 200
    assert client.delete("/v1/presence", headers=h1).status_code == 429
    # token2 on the same IP is NOT limited: bucket is per token.
    assert client.delete("/v1/presence", headers=h2).status_code == 200


# -- finding 4: non-string loadout values ---------------------------------


def test_validate_loadout_non_string_values():
    result = catalog.validate_loadout(
        {"espalda": ["manto_espectral"],   # unhashable → would TypeError
         "cuello": {"x": 1},
         "cabeza": 42,
         "brazos": True,
         "elitros": ""},
        ["supporter", "mvp"])
    assert all(v is None for v in result.values())


def test_presence_put_non_string_loadout(client, monkeypatch):
    token, _ = _verified_token(client, monkeypatch)
    r = client.put("/v1/presence",
                   headers={"Authorization": f"Bearer {token}"},
                   json={"loadout": {"espalda": ["manto_espectral"]}})
    assert r.status_code == 200
    assert r.json()["loadout"]["espalda"] is None


# -- finding 5: defensive upstream JSON -----------------------------------


def test_mojang_has_joined_malformed_json():
    async def run():
        client = httpx.AsyncClient(transport=httpx.MockTransport(
            lambda req: httpx.Response(200, content=b"<html>oops</html>")))
        with pytest.raises(ApiError) as ei:
            await external.mojang_has_joined(client, "Steve", "x" * 40)
        assert ei.value.status == 502
        assert ei.value.code == "upstream_error"

        client2 = httpx.AsyncClient(transport=httpx.MockTransport(
            lambda req: httpx.Response(200, json=[1, 2, 3])))
        with pytest.raises(ApiError) as ei2:
            await external.mojang_has_joined(client2, "Steve", "x" * 40)
        assert ei2.value.status == 502
        await client.aclose()
        await client2.aclose()
    asyncio.run(run())


def test_discord_endpoints_malformed_json():
    async def run():
        # Token endpoint returns non-JSON → 502, not a bare ValueError/500.
        bad = httpx.AsyncClient(transport=httpx.MockTransport(
            lambda req: httpx.Response(200, content=b"not json")))
        with pytest.raises(ApiError) as ei:
            await external.discord_exchange_code(
                bad, "code", "cb", "cid", "secret")
        assert ei.value.status == 502

        # Member endpoint returns JSON object without a roles list → 502.
        bad2 = httpx.AsyncClient(transport=httpx.MockTransport(
            lambda req: httpx.Response(200, json={"roles": "nope"})))
        with pytest.raises(ApiError) as ei2:
            await external.discord_member_roles(bad2, "bot", "42")
        assert ei2.value.status == 502
        await bad.aclose()
        await bad2.aclose()
    asyncio.run(run())


def test_grant_issued_when_discord_roles_fail(client, monkeypatch, settings):
    """Link stores the binding, then the member-role lookup blows up:
    the link call 502s but /v1/grant must still issue with flags []."""
    token, uuid = _verified_token(client, monkeypatch)

    async def fake_exchange(http, code, ruri, cid, secret):
        return {"discord": {"id": "999", "username": "u", "avatar": None}}

    async def boom(http, token_, did):
        raise ApiError(502, "upstream_error", "Discord API unreachable")

    monkeypatch.setattr(external, "discord_exchange_code", fake_exchange)
    monkeypatch.setattr(external, "discord_member_roles", boom)

    r = client.post("/v1/discord/link",
                    headers={"Authorization": f"Bearer {token}"},
                    json={"code": "x", "redirect_uri": "cb"})
    assert r.status_code == 502
    assert r.json()["error"]["code"] == "upstream_error"

    g = client.get("/v1/grant",
                   headers={"Authorization": f"Bearer {token}"})
    assert g.status_code == 200
    assert g.json()["flags"] == []
    assert verify_grant(g.json()["grant"], _pubkey(settings),
                        uuid)["flags"] == []
