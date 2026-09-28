"""Endpoint tests with mocked Mojang + Discord HTTP and a throwaway key."""
from __future__ import annotations

import base64
import json
import time
import uuid as uuidlib

import pytest
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from app import config, external
from app.errors import ApiError


def _mojang_profile(name="Steve", uuid=None):
    return {"id": uuid or uuidlib.uuid4().hex, "name": name,
            "properties": []}


def verify_grant(token: str, pubkey: Ed25519PrivateKey, uuid32: str):
    """Independent verifier implementing the contract checks."""
    h, p, s = token.split(".")
    pad = lambda x: x + "=" * (-len(x) % 4)
    header = json.loads(base64.urlsafe_b64decode(pad(h)))
    payload = json.loads(base64.urlsafe_b64decode(pad(p)))
    sig = base64.urlsafe_b64decode(pad(s))
    assert header == {"alg": "EdDSA", "kid": "eid1", "typ": "ESG"}
    pubkey.verify(sig, f"{h}.{p}".encode())
    assert payload["iss"] == "https://id.espectral.es"
    assert payload["aud"] == "espectral-client"
    assert payload["sub"] == uuid32
    now = int(time.time())
    assert payload["exp"] > now
    assert payload["iat"] <= now + 300
    assert payload["exp"] == payload["iat"] + 86400
    return payload


def _verified_token(client, monkeypatch, username="Steve", uuid=None,
                    roles=None):
    uuid = uuid or uuidlib.uuid4().hex

    async def fake_has_joined(http, username_, server_id):
        assert len(server_id) == 40
        return _mojang_profile(username_, uuid)

    async def fake_roles(http_, token_, discord_id_):
        return roles

    monkeypatch.setattr(external, "mojang_has_joined", fake_has_joined)
    monkeypatch.setattr(external, "discord_member_roles", fake_roles)

    ch = client.post("/v1/auth/challenge")
    assert ch.status_code == 201
    body = ch.json()
    assert len(body["server_id"]) == 40
    assert body["expires_in"] == 120

    r = client.post("/v1/auth/verify", json={
        "challenge_id": body["challenge_id"], "username": username})
    assert r.status_code == 200, r.text
    data = r.json()
    assert data["uuid"] == uuid
    assert data["expires_at"] > int(time.time())
    return data["token"], uuid


def test_healthz(client):
    r = client.get("/v1/healthz")
    assert r.json() == {"ok": True, "kid": "eid1"}


def test_verify_bogus_username_401(client, monkeypatch):
    async def fake_has_joined(http, username_, server_id):
        return None  # Mojang 204: no join recorded
    monkeypatch.setattr(external, "mojang_has_joined", fake_has_joined)
    ch = client.post("/v1/auth/challenge").json()
    r = client.post("/v1/auth/verify", json={
        "challenge_id": ch["challenge_id"], "username": "NotARealAcct"})
    assert r.status_code == 401
    body = r.json()
    assert set(body) == {"error"}
    assert body["error"]["code"] == "not_joined"


def test_challenge_single_use(client, monkeypatch):
    async def fake_has_joined(http, u, s):
        return _mojang_profile(u)
    monkeypatch.setattr(external, "mojang_has_joined", fake_has_joined)
    ch = client.post("/v1/auth/challenge").json()
    ok = client.post("/v1/auth/verify", json={
        "challenge_id": ch["challenge_id"], "username": "Steve"})
    assert ok.status_code == 200
    again = client.post("/v1/auth/verify", json={
        "challenge_id": ch["challenge_id"], "username": "Steve"})
    assert again.status_code == 401
    assert again.json()["error"]["code"] == "challenge_invalid"


def test_verify_requires_auth_for_grant(client, monkeypatch):
    r = client.get("/v1/grant")
    assert r.status_code == 401


def test_grant_verifies(client, monkeypatch, settings):
    token, uuid = _verified_token(client, monkeypatch, roles=["1190433806499446895"])
    # link discord so flags resolve
    async def fake_exchange(http, code, redirect_uri, cid, secret):
        return {"discord": {"id": "111", "username": "u", "avatar": None}}
    monkeypatch.setattr(external, "discord_exchange_code", fake_exchange)
    r = client.post("/v1/discord/link",
                    headers={"Authorization": f"Bearer {token}"},
                    json={"code": "x", "redirect_uri": "http://127.0.0.1/cb"})
    assert r.status_code == 200, r.text
    assert r.json()["flags"] == ["supporter", "mvp"]

    g = client.get("/v1/grant", headers={"Authorization": f"Bearer {token}"})
    assert g.status_code == 200
    payload = verify_grant(g.json()["grant"], _pubkey(settings), uuid)
    assert payload["flags"] == ["supporter", "mvp"]


def _pubkey(settings):
    key = __import__("cryptography.hazmat.primitives.serialization",
                     fromlist=["x"])
    from cryptography.hazmat.primitives.serialization import load_der_private_key
    priv = load_der_private_key(base64.b64decode(settings.signing_key_b64),
                                password=None)
    return priv.public_key()


def test_grant_empty_when_not_linked(client, monkeypatch, settings):
    token, uuid = _verified_token(client, monkeypatch)
    g = client.get("/v1/grant", headers={"Authorization": f"Bearer {token}"})
    assert g.status_code == 200
    payload = verify_grant(g.json()["grant"], _pubkey(settings), uuid)
    assert payload["flags"] == []


@pytest.mark.parametrize("roles,expected", [
    ([], []),
    (["655199440352837646"], []),                    # booster ≠ donor
    (["648324691659849754"], ["supporter"]),          # VIP
    (["1190433903698268211"], ["supporter"]),         # VIP+
    (["1190433806499446895"], ["supporter", "mvp"]),  # MVP
    (["1189380795744387182"], ["supporter", "mvp"]),  # MVP+
    (["648324691659849754", "1190433806499446895"],
     ["supporter", "mvp"]),
])
def test_flag_matrix(roles, expected):
    assert external.flags_from_roles(roles) == expected


def test_presence_roundtrip_and_drop(client, monkeypatch):
    token, uuid = _verified_token(client, monkeypatch, roles=["648324691659849754"])
    async def fake_exchange(http, code, ruri, cid, secret):
        return {"discord": {"id": "222", "username": "u", "avatar": None}}
    monkeypatch.setattr(external, "discord_exchange_code", fake_exchange)
    client.post("/v1/discord/link",
                headers={"Authorization": f"Bearer {token}"},
                json={"code": "x", "redirect_uri": "cb"})
    hdr = {"Authorization": f"Bearer {token}"}

    # supporter but NOT mvp: mvp items dropped, unknown items dropped
    r = client.put("/v1/presence", headers=hdr, json={"loadout": {
        "espalda": "manto_espectral",
        "cuello": "bufanda_llama_alma",
        "cabeza": "not_a_item",
    }})
    assert r.status_code == 200
    assert r.json()["loadout"] == {
        "espalda": None, "cuello": None, "cabeza": None,
        "brazos": None, "elitros": None}

    # nothing in query since loadout empty? presence still published.
    q = client.post("/v1/presence/query", headers=hdr,
                    json={"uuids": [uuid, uuidlib.uuid4().hex]})
    assert q.status_code == 200
    players = q.json()["players"]
    assert len(players) == 1 and players[0]["uuid"] == uuid

    d = client.delete("/v1/presence", headers=hdr)
    assert d.json() == {"ok": True}
    q2 = client.post("/v1/presence/query", headers=hdr, json={"uuids": [uuid]})
    assert q2.json()["players"] == []


def test_presence_exclusivity(client, monkeypatch):
    token, uuid = _verified_token(client, monkeypatch, roles=["1190433806499446895"])
    async def fake_exchange(http, code, ruri, cid, secret):
        return {"discord": {"id": "333", "username": "u", "avatar": None}}
    monkeypatch.setattr(external, "discord_exchange_code", fake_exchange)
    client.post("/v1/discord/link",
                headers={"Authorization": f"Bearer {token}"},
                json={"code": "x", "redirect_uri": "cb"})
    hdr = {"Authorization": f"Bearer {token}"}
    r = client.put("/v1/presence", headers=hdr, json={"loadout": {
        "espalda": "manto_espectral",      # cloak
        "cuello": "bufanda_llama_alma",    # scarf — exclusive with cloak
    }})
    assert r.json()["loadout"]["espalda"] == "manto_espectral"
    assert r.json()["loadout"]["cuello"] is None


def test_query_limit_and_bad_auth(client, monkeypatch):
    token, _ = _verified_token(client, monkeypatch)
    hdr = {"Authorization": f"Bearer {token}"}
    r = client.post("/v1/presence/query", headers=hdr,
                    json={"uuids": ["0" * 32] * 101})
    assert r.status_code == 400
    r2 = client.post("/v1/presence/query",
                     headers={"Authorization": "Bearer deadbeefdeadbeef"},
                     json={"uuids": []})
    assert r2.status_code == 401


def test_discord_identify(client, monkeypatch):
    async def fake_exchange(http, code, ruri, cid, secret):
        assert cid == "1454618588659843072"
        return {"discord": {"id": "42", "username": "p", "avatar": "a"}}
    monkeypatch.setattr(external, "discord_exchange_code", fake_exchange)
    r = client.post("/v1/discord/identify",
                    json={"code": "c", "redirect_uri": "http://127.0.0.1:4199/api/discord/callback"})
    assert r.json() == {"discord": {"id": "42", "username": "p", "avatar": "a"}}


def test_session_token_is_hashed(client, monkeypatch):
    token, uuid = _verified_token(client, monkeypatch)
    # the raw token must not appear in the DB; only its sha256
    import hashlib, sqlite3
    db = client.app.state.db
    row = db._conn.execute(
        "SELECT token_hash FROM sessions WHERE uuid=?", (uuid,)).fetchone()
    assert row["token_hash"] == hashlib.sha256(token.encode()).hexdigest()
    assert row["token_hash"] != token
