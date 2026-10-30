//! Port of `src/engine/discordAuth.mjs` — Discord OAuth2 (launcher login and
//! identity link), state registry, in-memory sessions, `<data>/discord-users.json`.
//!
//! **Owner: identity-discord.** Status: STUB.
//!
//! JS exports to port: REDIRECT_URI DEFAULT_DISCORD_CLIENT_ID
//! getDiscordCredentials (ESPECTRAL_DISCORD_CLIENT_ID / DISCORD_CLIENT_ID)
//! getAuthStartUrl claimSessionUser handleCallback upsertDiscordUser
//! getSessionUser (ESPECTRAL_DISCORD_MOCK) destroySession extractSessionId
//! createSessionCookie clearSessionCookie. Tests: tests/discord-auth.test.mjs.
//!
//! IMPORTANT: the redirect URI registered with Discord is
//! `http://127.0.0.1:4199/api/discord/callback`, so the native app still
//! needs a loopback listener for that one path (crate::loopback). Cookies
//! are not needed natively: the UI holds the session id from `discord_claim`.

pub const REDIRECT_URI: &str = "http://127.0.0.1:4199/api/discord/callback";
pub const DEFAULT_DISCORD_CLIENT_ID: &str = "1454618588659843072";
