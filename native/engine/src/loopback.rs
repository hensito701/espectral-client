//! Loopback HTTP listener (NEW — no JS module): the only HTTP the native
//! engine still serves, because external processes call it:
//!
//! - `POST /api/bridge/visible` — the in-game Espectral mod (headers
//!   `x-espectral-client: 1` and `x-espectral-bridge: <per-launch token>`,
//!   body `{ uuids: [...] }`) -> `Engine::bridge_visible`;
//! - `GET /api/discord/callback` — Discord's OAuth redirect (registered as
//!   `http://127.0.0.1:4199/api/discord/callback`) -> `Engine::discord_callback`,
//!   answering the completion / error HTML from routes/discord.mjs.
//!
//! **Owner: identity-discord.** Status: STUB.
//!
//! Bind 127.0.0.1 only, port `ESPECTRAL_PORT` or 4199 (keep the JS Host
//! header check: 127.0.0.1|localhost[:port], and the x-espectral-client
//! guard on POST). Pinned server crate: axum 0.7 (http1, json, query).
//! Start it from `api::identity_discord::on_engine_start` and stop it on
//! `Engine::cancel_token()`; expose the bound port to game-launch for
//! `-Despectral.bridge.url`.

/// JS DEFAULT_PORT.
pub const DEFAULT_PORT: u16 = 4199;

/// `ESPECTRAL_PORT` env or 4199.
pub fn configured_port() -> u16 {
    std::env::var("ESPECTRAL_PORT").ok().and_then(|p| p.parse().ok()).filter(|p| *p > 0).unwrap_or(DEFAULT_PORT)
}
