"""Discord member-role cache (<= 60 s) feeding flag computation."""
from __future__ import annotations

import time

from . import config, external


class MemberRoleCache:
    def __init__(self, ttl_seconds: int = config.MEMBER_ROLES_CACHE_SECONDS):
        self._ttl = ttl_seconds
        self._cache: dict[str, tuple[list[str] | None, float]] = {}

    async def roles(self, client, bot_token: str,
                    discord_id: str) -> list[str] | None:
        now = time.monotonic()
        hit = self._cache.get(discord_id)
        if hit and now - hit[1] < self._ttl:
            return hit[0]
        try:
            roles = await external.discord_member_roles(client, bot_token,
                                                        discord_id)
        except Exception:
            if hit:
                return hit[0]  # serve stale rather than fail
            raise
        self._cache[discord_id] = (roles, now)
        return roles
