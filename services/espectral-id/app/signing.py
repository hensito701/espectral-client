"""Compact JWS (EdDSA) grant signing for espectral-id.

Grant format (contract):
    b64url(header) + "." + b64url(payload) + "." + b64url(sig)
header  {"alg":"EdDSA","kid":"eid1","typ":"ESG"}
payload {"v":1,"iss":..., "aud":"espectral-client","sub":"<uuid32>",
         "flags":[...], "iat":<unix s>, "exp":<iat+86400>}
No padding anywhere; sig over the ASCII of the first two parts.
"""
from __future__ import annotations

import base64
import json
import time

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from . import config


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def load_signing_key(b64: str) -> Ed25519PrivateKey:
    raw = base64.b64decode(b64)
    if len(raw) == 32:
        # raw seed form
        return Ed25519PrivateKey.from_private_bytes(raw)
    return serialization.load_der_private_key(raw, password=None)  # PKCS8 DER


def public_spki_b64(key: Ed25519PrivateKey) -> str:
    der = key.public_key().public_bytes(
        serialization.Encoding.DER,
        serialization.PublicFormat.SubjectPublicKeyInfo,
    )
    return base64.b64encode(der).decode("ascii")


def issue_grant(
    key: Ed25519PrivateKey,
    sub_uuid32: str,
    flags: list[str],
    now: int | None = None,
) -> tuple[str, int]:
    """Return (compact JWS, exp unix)."""
    now = int(time.time()) if now is None else int(now)
    header = {"alg": "EdDSA", "kid": config.GRANT_KID, "typ": "ESG"}
    payload = {
        "v": 1,
        "iss": config.GRANT_ISS,
        "aud": config.GRANT_AUD,
        "sub": sub_uuid32,
        "flags": list(flags),
        "iat": now,
        "exp": now + config.GRANT_TTL_SECONDS,
    }
    h = b64url(json.dumps(header, separators=(",", ":")).encode())
    p = b64url(json.dumps(payload, separators=(",", ":")).encode())
    sig = b64url(key.sign(f"{h}.{p}".encode("ascii")))
    return f"{h}.{p}.{sig}", payload["exp"]
