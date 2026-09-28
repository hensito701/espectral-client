"""Outbound HTTP wrappers (Mojang session server, Discord API).

Every function takes the shared httpx.AsyncClient so tests can substitute a
mocked transport. Nothing here logs tokens or request bodies.
"""
from __future__ import annotations

import httpx

from . import config
from .errors import ApiError

TIMEOUT = httpx.Timeout(10.0, connect=5.0)


def _json_dict(resp: httpx.Response, what: str) -> dict:
    """Parse a response body as a JSON object; ApiError(502) otherwise."""
    try:
        data = resp.json()
    except ValueError:
        raise ApiError(502, "upstream_error", f"Bad {what} response")
    if not isinstance(data, dict):
        raise ApiError(502, "upstream_error", f"Bad {what} response")
    return data

async def mojang_has_joined(client: httpx.AsyncClient, username: str,
                            server_id: str) -> dict | None:
    """Return the hasJoined profile dict {id, name, ...} or None if Mojang
    reports no such join (204). Raises ApiError on upstream trouble."""
    try:
        resp = await client.get(
            config.MOJANG_HASJOINED_URL,
            params={"username": username, "serverId": server_id},
            timeout=TIMEOUT,
        )
    except httpx.HTTPError:
        raise ApiError(502, "upstream_error", "Mojang session server unreachable")
    if resp.status_code == 204:
        return None
    if resp.status_code == 200:
        return _json_dict(resp, "Mojang")
    if resp.status_code == 429:
        raise ApiError(502, "upstream_error", "Mojang rate limited")
    raise ApiError(502, "upstream_error", "Mojang session server error")


async def discord_exchange_code(client: httpx.AsyncClient, code: str,
                                redirect_uri: str, client_id: str,
                                client_secret: str) -> dict:
    """Exchange an OAuth code; returns {discord:{id,username,avatar}}."""
    try:
        token_resp = await client.post(
            config.DISCORD_TOKEN_URL,
            data={
                "grant_type": "authorization_code",
                "code": code,
                "redirect_uri": redirect_uri,
                "client_id": client_id,
                "client_secret": client_secret,
            },
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            timeout=TIMEOUT,
        )
    except httpx.HTTPError:
        raise ApiError(502, "upstream_error", "Discord token endpoint unreachable")
    if token_resp.status_code != 200:
        raise ApiError(401, "discord_code_invalid",
                       "Discord rejected the authorization code")
    access_token = _json_dict(token_resp, "Discord").get("access_token")

    try:
        me = await client.get(
            f"{config.DISCORD_API}/users/@me",
            headers={"Authorization": f"Bearer {access_token}"},
            timeout=TIMEOUT,
        )
    except httpx.HTTPError:
        raise ApiError(502, "upstream_error", "Discord API unreachable")
    if me.status_code != 200:
        raise ApiError(502, "upstream_error", "Discord /users/@me failed")
    u = _json_dict(me, "Discord")
    return {"discord": {"id": str(u.get("id", "")),
                        "username": u.get("username", ""),
                        "avatar": u.get("avatar")}}


async def discord_member_roles(client: httpx.AsyncClient, bot_token: str,
                               discord_id: str) -> list[str] | None:
    """Role IDs the member holds in the guild; None if not a member.
    Raises ApiError on upstream trouble."""
    try:
        resp = await client.get(
            f"{config.DISCORD_API}/guilds/{config.GUILD_ID}/members/{discord_id}",
            headers={"Authorization": f"Bot {bot_token}"},
            timeout=TIMEOUT,
        )
    except httpx.HTTPError:
        raise ApiError(502, "upstream_error", "Discord API unreachable")
    if resp.status_code == 404:
        return None
    if resp.status_code != 200:
        raise ApiError(502, "upstream_error", "Discord member lookup failed")
    roles = _json_dict(resp, "Discord").get("roles", [])
    if not isinstance(roles, list):
        raise ApiError(502, "upstream_error", "Bad Discord response")
    return [str(r) for r in roles]


def flags_from_roles(roles: list[str] | None) -> list[str]:
    if not roles:
        return []
    rs = set(roles)
    flags = []
    if rs & config.DONOR_ROLE_IDS:
        flags.append("supporter")
    if rs & config.MVP_ROLE_IDS:
        flags.append("mvp")
    return flags
