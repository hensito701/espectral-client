"""All /v1 endpoints for espectral-id (see cosmetics contract)."""
from __future__ import annotations

import re
import secrets
import time

from fastapi import APIRouter, Depends, Request
from pydantic import BaseModel, Field

from . import catalog, config, external, signing
from .auth import RateLimiter, client_key, get_session, hash_token
from .errors import ApiError

router = APIRouter(prefix="/v1")

_USERNAME_RE = re.compile(r"^[A-Za-z0-9_]{1,16}$")
_UUID32_RE = re.compile(r"^[0-9a-f]{32}$")


class VerifyBody(BaseModel):
    challenge_id: str
    username: str


class DiscordCodeBody(BaseModel):
    code: str
    redirect_uri: str


class PresenceBody(BaseModel):
    loadout: dict = Field(default_factory=dict)


class QueryBody(BaseModel):
    uuids: list[str] = Field(default_factory=list)


def _limit(request: Request, bucket: str, limit: int, window: int = 60):
    limiter: RateLimiter = request.app.state.limiter
    limiter.check(bucket, client_key(request), limit, window)


async def _async_flags_for_session(request: Request, session: dict) -> list[str]:
    discord_id = session.get("discord_id")
    if not discord_id:
        return []
    settings = request.app.state.settings
    roles = await request.app.state.member_cache.roles(
        request.app.state.http, settings.discord_bot_token, discord_id)
    return external.flags_from_roles(roles)


@router.get("/healthz")
async def healthz():
    return {"ok": True, "kid": config.GRANT_KID}


@router.post("/auth/challenge", status_code=201)
async def auth_challenge(request: Request):
    _limit(request, "auth.challenge", 20)
    challenge_id = secrets.token_urlsafe(24)
    server_id = secrets.token_hex(20)  # 40 lowercase hex chars
    request.app.state.db.create_challenge(
        challenge_id, server_id, config.CHALLENGE_TTL_SECONDS)
    return {
        "challenge_id": challenge_id,
        "server_id": server_id,
        "expires_in": config.CHALLENGE_TTL_SECONDS,
    }


@router.post("/auth/verify")
async def auth_verify(request: Request, body: VerifyBody):
    _limit(request, "auth.verify", 10)
    if not body.challenge_id or not body.username:
        raise ApiError(400, "bad_request", "challenge_id and username required")
    if not _USERNAME_RE.match(body.username):
        raise ApiError(400, "bad_username", "Invalid Minecraft username")

    server_id = request.app.state.db.consume_challenge(body.challenge_id)
    if server_id is None:
        raise ApiError(401, "challenge_invalid",
                       "Unknown, used, or expired challenge")

    profile = await external.mojang_has_joined(
        request.app.state.http, body.username, server_id)
    if profile is None:
        raise ApiError(401, "not_joined",
                       "Mojang reports no joined session for this username")

    uuid32 = str(profile.get("id", "")).lower()
    name = profile.get("name", body.username)
    if not _UUID32_RE.match(uuid32):
        raise ApiError(502, "upstream_error", "Malformed Mojang profile id")

    token = secrets.token_urlsafe(32)
    expires_at = request.app.state.db.create_session(
        hash_token(token), uuid32, name, config.SESSION_TTL_SECONDS)
    return {"token": token, "uuid": uuid32, "name": name,
            "expires_at": expires_at}


@router.post("/discord/identify")
async def discord_identify(request: Request, body: DiscordCodeBody):
    _limit(request, "discord", 10)
    settings = request.app.state.settings
    if not body.code:
        raise ApiError(400, "bad_request", "code required")
    return await external.discord_exchange_code(
        request.app.state.http, body.code, body.redirect_uri,
        settings.discord_client_id, settings.discord_client_secret)


@router.post("/discord/link")
async def discord_link(request: Request, body: DiscordCodeBody,
                       session: dict = Depends(get_session)):
    _limit(request, "discord", 10)
    settings = request.app.state.settings
    if not body.code:
        raise ApiError(400, "bad_request", "code required")
    result = await external.discord_exchange_code(
        request.app.state.http, body.code, body.redirect_uri,
        settings.discord_client_id, settings.discord_client_secret)
    discord_id = result["discord"]["id"]
    if not discord_id:
        raise ApiError(502, "upstream_error", "Discord returned no user id")
    request.app.state.db.set_discord_link(session["uuid"], discord_id)
    session["discord_id"] = discord_id
    flags = await _async_flags_for_session(request, session)
    return {**result, "flags": flags}


@router.delete("/discord/link")
async def discord_unlink(request: Request, session: dict = Depends(get_session)):
    _limit(request, "default", 60)
    request.app.state.db.set_discord_link(session["uuid"], None)
    return {"ok": True}


@router.get("/grant")
async def grant(request: Request, session: dict = Depends(get_session)):
    _limit(request, "default", 60)
    try:
        flags = await _async_flags_for_session(request, session)
    except ApiError:
        flags = []  # grant is always issued, even on upstream failure
    grant, exp = _issue_grant(request, session["uuid"], flags)
    return {"grant": grant, "flags": flags, "exp": exp}

def _issue_grant(request: Request, uuid32: str, flags: list[str]):
    key = request.app.state.signing_key
    if key is None:
        raise ApiError(503, "signing_unavailable", "Signing key not configured")
    return signing.issue_grant(key, uuid32, flags)



@router.put("/presence")
async def presence_put(request: Request, body: PresenceBody,
                       session: dict = Depends(get_session)):
    _limit(request, "default", 60)
    flags = await _safe_flags(request, session)
    loadout = catalog.validate_loadout(body.loadout, flags)
    grant, _exp = _issue_grant(request, session["uuid"], flags)
    expires_at = request.app.state.presence.put(session["uuid"], loadout, grant)
    return {"ok": True, "expires_at": expires_at, "loadout": loadout}


@router.delete("/presence")
async def presence_delete(request: Request,
                          session: dict = Depends(get_session)):
    _limit(request, "default", 60)
    request.app.state.presence.delete(session["uuid"])
    return {"ok": True}


@router.post("/presence/query")
async def presence_query(request: Request, body: QueryBody,
                         session: dict = Depends(get_session)):
    _limit(request, "presence.query", 30)
    if len(body.uuids) > 100:
        raise ApiError(400, "too_many_uuids", "At most 100 uuids per query")
    uuids = [u.lower() for u in body.uuids
             if isinstance(u, str) and _UUID32_RE.match(u.lower())]
    players = request.app.state.presence.query(uuids)
    return {"players": players}


async def _safe_flags(request: Request, session: dict) -> list[str]:
    try:
        return await _async_flags_for_session(request, session)
    except ApiError:
        return []
