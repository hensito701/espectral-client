"""Bearer auth dependency and modest sliding-window rate limiting.

IP key: CF-Connecting-IP is trusted ONLY when the direct peer is loopback
(nginx) — never as an auth signal, only for rate-limit bucketing.
"""
from __future__ import annotations

import hashlib
import re
import time
from collections import defaultdict, deque

from fastapi import Depends, Request
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

from .errors import ApiError

_bearer = HTTPBearer(auto_error=False)
_TOKEN_RE = re.compile(r"^[A-Za-z0-9_\-]+$")


def hash_token(token: str) -> str:
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def ip_key(request: Request) -> str:
    """Client IP key: CF-Connecting-IP only when the peer is loopback."""
    peer = request.client.host if request.client else ""
    if peer == "127.0.0.1":
        cf_ip = request.headers.get("cf-connecting-ip")
        if cf_ip:
            return "ip:" + cf_ip
    return "ip:" + peer


def client_key(request: Request) -> str:
    """Stable rate-limit identity. Never logged raw beyond this use."""
    creds = getattr(request.state, "bearer_creds", None)
    if creds is not None:
        return "tok:" + hash_token(creds.credentials)[:16]
    return ip_key(request)


class RateLimiter:
    """Sliding-window limiter: max `limit` requests per `window` seconds.

    Entries are keyed by (bucket, key) and hold (window, deque). Empty
    deques are deleted on the spot; a periodic sweep removes drained keys
    and the whole map is hard-capped so memory stays bounded.
    """

    _MAX_KEYS = 100_000
    _SWEEP_EVERY = 512

    def __init__(self):
        self._hits: dict[tuple[str, str], tuple[int, deque]] = {}
        self._ops = 0

    def _sweep(self, now: float):
        for k, (w, dq) in list(self._hits.items()):
            if not dq or now - dq[-1] > w:
                del self._hits[k]

    def check(self, bucket: str, key: str, limit: int, window: int):
        now = time.monotonic()
        self._ops += 1
        if self._ops >= self._SWEEP_EVERY or len(self._hits) >= self._MAX_KEYS:
            self._ops = 0
            self._sweep(now)
        entry = self._hits.get((bucket, key))
        if entry is not None:
            w, dq = entry
            while dq and now - dq[0] > w:
                dq.popleft()
            if not dq:
                del self._hits[(bucket, key)]
            elif len(dq) >= limit:
                raise ApiError(429, "rate_limited", "Too many requests")
            else:
                dq.append(now)
                return
        self._hits[(bucket, key)] = (window, deque([now]))

# Per-IP cap on bearer-auth attempts (missing/invalid/malformed tokens)
# applied in get_session BEFORE the session-store lookup.
_BEARER_IP_LIMIT = 120

def get_session(request: Request,
                creds: HTTPAuthorizationCredentials | None = Depends(_bearer)
                ) -> dict:
    # Invalid/missing-bearer attempts are rate-limited per IP before the
    # DB lookup so token-guessing cannot hammer the store unbounded.
    limiter: RateLimiter | None = getattr(
        request.app.state, "limiter", None)
    if limiter is not None:
        limiter.check("auth.bearer", ip_key(request), _BEARER_IP_LIMIT, 60)
    if creds is None or creds.scheme.lower() != "bearer":
        raise ApiError(401, "unauthorized", "Missing bearer token")
    request.state.bearer_creds = creds
    token = creds.credentials
    if not _TOKEN_RE.match(token):
        raise ApiError(401, "unauthorized", "Malformed token")
    token_hash = hash_token(token)
    session = request.app.state.db.get_session(token_hash)
    if session is None:
        raise ApiError(401, "unauthorized", "Invalid or expired token")
    session["token_hash"] = token_hash
    return session
