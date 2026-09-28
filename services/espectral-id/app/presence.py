"""In-memory presence store. TTL 600 s, never persisted, no history.

An entry exists only for players actively sharing (PUT /v1/presence) and
expires quickly; DELETE removes it. query() returns only live entries —
uuid, loadout and the fresh signed grant. No coordinates, servers, or
timestamps beyond expiry are stored or returned.
"""
from __future__ import annotations

import time


class PresenceStore:
    def __init__(self, ttl_seconds: int):
        self._ttl = ttl_seconds
        self._entries: dict[str, dict] = {}

    def put(self, uuid: str, loadout: dict, grant: str) -> int:
        expires_at = int(time.time()) + self._ttl
        self._entries[uuid] = {
            "uuid": uuid,
            "loadout": loadout,
            "grant": grant,
            "expires_at": expires_at,
        }
        return expires_at

    def delete(self, uuid: str) -> bool:
        return self._entries.pop(uuid, None) is not None

    def get(self, uuid: str) -> dict | None:
        now = int(time.time())
        entry = self._entries.get(uuid)
        if not entry:
            return None
        if entry["expires_at"] <= now:
            self._entries.pop(uuid, None)
            return None
        return entry

    def query(self, uuids: list[str]) -> list[dict]:
        now = int(time.time())
        players = []
        for u in uuids:
            entry = self._entries.get(u)
            if not entry:
                continue
            if entry["expires_at"] <= now:
                self._entries.pop(u, None)
                continue
            players.append(
                {"uuid": u, "loadout": entry["loadout"], "grant": entry["grant"]}
            )
        return players

    def purge(self):
        now = int(time.time())
        for u in [u for u, e in self._entries.items() if e["expires_at"] <= now]:
            self._entries.pop(u, None)
