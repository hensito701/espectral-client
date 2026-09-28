"""Environment-driven configuration for espectral-id."""
from __future__ import annotations

import os
from dataclasses import dataclass


# Discord guild and role facts (queried live 2026-09-28, see cosmetics contract).
GUILD_ID = "648307511648321536"
DONOR_ROLE_IDS = frozenset({
    "648324691659849754",   # VIP  [Vip-Donador]
    "1190433903698268211",  # VIP+
    "1190433806499446895",  # MVP
    "1189380795744387182",  # MVP+
})
MVP_ROLE_IDS = frozenset({
    "1190433806499446895",  # MVP
    "1189380795744387182",  # MVP+
})

GRANT_TTL_SECONDS = 86400          # grant exp = iat + 24 h
GRANT_AUD = "espectral-client"
GRANT_ISS = "https://id.espectral.es"
GRANT_KID = "eid1"

CHALLENGE_TTL_SECONDS = 120        # contract: challenge expires_in 120
SESSION_TTL_SECONDS = 30 * 86400   # session token TTL 30 d
PRESENCE_TTL_SECONDS = 600         # in-memory presence TTL
MEMBER_ROLES_CACHE_SECONDS = 60    # Discord member-role cache <= 60 s

MOJANG_HASJOINED_URL = (
    "https://sessionserver.mojang.com/session/minecraft/hasJoined"
)
DISCORD_API = "https://discord.com/api/v10"
DISCORD_TOKEN_URL = "https://discord.com/api/oauth2/token"


@dataclass(frozen=True)
class Settings:
    discord_bot_token: str
    discord_client_id: str
    discord_client_secret: str
    signing_key_b64: str          # PKCS8 DER base64, server-only
    db_path: str
    bind_host: str = "127.0.0.1"
    bind_port: int = 8091
    log_level: str = "info"


def load_settings() -> Settings:
    return Settings(
        discord_bot_token=os.environ.get("DISCORD_TOKEN", ""),
        discord_client_id=os.environ.get("DISCORD_CLIENT_ID", ""),
        discord_client_secret=os.environ.get("DISCORD_CLIENT_SECRET", ""),
        signing_key_b64=os.environ.get("ESPECTRAL_ID_SIGNING_KEY", ""),
        db_path=os.environ.get("ESPECTRAL_ID_DB", "data/espectral-id.db"),
        bind_host=os.environ.get("ESPECTRAL_ID_HOST", "127.0.0.1"),
        bind_port=int(os.environ.get("ESPECTRAL_ID_PORT", "8091")),
        log_level=os.environ.get("ESPECTRAL_ID_LOG", "info"),
    )
