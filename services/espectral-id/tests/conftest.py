"""pytest fixtures: app with throwaway key, sqlite :memory:, no real HTTP."""
from __future__ import annotations

import base64
import os
import sys
import time
import uuid as uuidlib

import pytest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from app import config
from app.main import create_app


def make_key_b64() -> tuple[str, Ed25519PrivateKey]:
    key = Ed25519PrivateKey.generate()
    der = key.private_bytes(
        serialization.Encoding.DER,
        serialization.PrivateFormat.PKCS8,
        serialization.NoEncryption(),
    )
    return base64.b64encode(der).decode(), key


@pytest.fixture()
def settings(tmp_path):
    b64, _key = make_key_b64()
    return config.Settings(
        discord_bot_token="bot-token-test",
        discord_client_id="1454618588659843072",
        discord_client_secret="discord-secret-test",
        signing_key_b64=b64,
        db_path=str(tmp_path / "test.db"),
    )


@pytest.fixture()
def client(settings):
    app = create_app(settings)
    with TestClient(app) as c:
        yield c


@pytest.fixture()
def verified(client):
    """Return a helper that runs challenge+verify with a stubbed Mojang."""

    def _verify(username="Steve", uuid=None, mojang_status=200,
                mojang_json=None):
        uuid = uuid or uuidlib.uuid4().hex
        ch = client.post("/v1/auth/challenge").json()
        return ch, client.post("/v1/auth/verify", json={
            "challenge_id": ch["challenge_id"], "username": username})

    return _verify
