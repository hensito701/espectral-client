"""SQLite persistence for challenges and sessions.

Sessions: opaque 32-byte bearer tokens, stored hashed (SHA-256).
Challenges: single-use, TTL 120 s (consumed atomically in /v1/auth/verify).
Discord bindings: keyed by uuid — they survive session re-verification.
Presence lives in memory only (see presence.py) — never persisted.
"""
from __future__ import annotations

import os
import sqlite3
import threading
import time

_SCHEMA = """
CREATE TABLE IF NOT EXISTS challenges (
    challenge_id TEXT PRIMARY KEY,
    server_id    TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    used         INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sessions (
    token_hash  TEXT PRIMARY KEY,
    uuid        TEXT NOT NULL,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_uuid ON sessions(uuid);
CREATE INDEX IF NOT EXISTS idx_sessions_exp  ON sessions(expires_at);
CREATE TABLE IF NOT EXISTS discord_links (
    uuid        TEXT PRIMARY KEY,
    discord_id  TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);
"""


class Db:
    def __init__(self, path: str):
        if path != ":memory:":
            os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
        self._conn = sqlite3.connect(path, check_same_thread=False)
        self._conn.row_factory = sqlite3.Row
        self._conn.execute("PRAGMA journal_mode=WAL")
        self._conn.execute("PRAGMA foreign_keys=ON")
        self._lock = threading.Lock()
        with self._lock:
            self._conn.executescript(_SCHEMA)
            self._migrate()

    def _migrate(self):
        """Carry per-session discord_id rows into discord_links, then drop
        the legacy column. Idempotent: no-ops once the column is gone.
        CALLER HOLDS self._lock."""
        cols = {r["name"] for r in self._conn.execute(
            "PRAGMA table_info(sessions)")}
        if "discord_id" not in cols:
            return
        self._conn.execute(
            "INSERT OR IGNORE INTO discord_links(uuid, discord_id, updated_at)"
            " SELECT uuid, discord_id, created_at FROM sessions"
            " WHERE discord_id IS NOT NULL")
        self._conn.execute("ALTER TABLE sessions DROP COLUMN discord_id")
        self._conn.commit()

    def close(self):
        self._conn.close()

    # -- challenges ---------------------------------------------------------

    def create_challenge(self, challenge_id: str, server_id: str,
                         ttl_seconds: int) -> int:
        now = int(time.time())
        expires_at = now + ttl_seconds
        with self._lock:
            self._conn.execute(
                "INSERT INTO challenges(challenge_id, server_id, created_at,"
                " expires_at, used) VALUES (?,?,?,?,0)",
                (challenge_id, server_id, now, expires_at),
            )
            self._conn.commit()
        return expires_at

    def consume_challenge(self, challenge_id: str) -> str | None:
        """Atomically mark a challenge used; return its server_id if it was
        live and unspent, else None."""
        now = int(time.time())
        with self._lock:
            cur = self._conn.execute(
                "UPDATE challenges SET used=1 WHERE challenge_id=? AND used=0"
                " AND expires_at>?",
                (challenge_id, now),
            )
            self._conn.commit()
            if cur.rowcount == 0:
                return None
            row = self._conn.execute(
                "SELECT server_id FROM challenges WHERE challenge_id=?",
                (challenge_id,),
            ).fetchone()
        return row["server_id"] if row else None

    # -- sessions -----------------------------------------------------------

    def create_session(self, token_hash: str, uuid: str, name: str,
                       ttl_seconds: int) -> int:
        now = int(time.time())
        expires_at = now + ttl_seconds
        with self._lock:
            # One live session per account: drop stale ones for this uuid.
            self._conn.execute(
                "DELETE FROM sessions WHERE uuid=? OR expires_at<=?",
                (uuid, now),
            )
            self._conn.execute(
                "INSERT INTO sessions(token_hash, uuid, name,"
                " created_at, expires_at) VALUES (?,?,?,?,?)",
                (token_hash, uuid, name, now, expires_at),
            )
            self._conn.commit()
        return expires_at

    def get_session(self, token_hash: str) -> dict | None:
        now = int(time.time())
        with self._lock:
            row = self._conn.execute(
                "SELECT s.uuid, s.name, dl.discord_id, s.expires_at"
                " FROM sessions s"
                " LEFT JOIN discord_links dl ON dl.uuid = s.uuid"
                " WHERE s.token_hash=? AND s.expires_at>?",
                (token_hash, now),
            ).fetchone()
        return dict(row) if row else None

    def set_discord_link(self, uuid: str, discord_id: str | None):
        """Bind (or clear) a Discord id for a Minecraft uuid. Persists
        across session re-verification."""
        with self._lock:
            if discord_id is None:
                self._conn.execute(
                    "DELETE FROM discord_links WHERE uuid=?", (uuid,))
            else:
                self._conn.execute(
                    "INSERT INTO discord_links(uuid, discord_id, updated_at)"
                    " VALUES (?,?,?)"
                    " ON CONFLICT(uuid) DO UPDATE SET"
                    " discord_id=excluded.discord_id,"
                    " updated_at=excluded.updated_at",
                    (uuid, discord_id, int(time.time())),
                )
            self._conn.commit()

    def delete_session(self, token_hash: str):
        with self._lock:
            self._conn.execute(
                "DELETE FROM sessions WHERE token_hash=?", (token_hash,)
            )
            self._conn.commit()

    def purge_expired(self):
        now = int(time.time())
        with self._lock:
            self._conn.execute("DELETE FROM challenges WHERE expires_at<=?", (now,))
            self._conn.execute("DELETE FROM sessions WHERE expires_at<=?", (now,))
            self._conn.commit()
