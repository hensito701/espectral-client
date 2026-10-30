//! Port of `src/engine/bridge.mjs` — the in-game mod bridge: per-launch
//! token registry, visible-player cosmetics cache, batched presence queries.
//!
//! **Owner: identity-discord.** Status: STUB (mint_token implemented).
//!
//! JS exports to port: BRIDGE_HEADER CACHE_TTL_MS NEGATIVE_TTL_MS
//! QUERY_BATCH QUERY_INTERVAL_MS MAX_VISIBLE_INPUT attach detach launchesFor
//! allLaunches authenticate _seedCache _reset visible.
//! Tests: tests/bridge.test.mjs.
//! The mod calls `POST http://127.0.0.1:<port>/api/bridge/visible` with
//! headers `x-espectral-client: 1` + `x-espectral-bridge: <token>` — served
//! by crate::loopback, which calls `Engine::bridge_visible`.

use rand::RngCore;

pub const BRIDGE_HEADER: &str = "x-espectral-bridge";
pub const CACHE_TTL_MS: u64 = 60_000;
pub const NEGATIVE_TTL_MS: u64 = 60_000;
pub const QUERY_BATCH: usize = 100;
pub const QUERY_INTERVAL_MS: u64 = 3_000;
pub const MAX_VISIBLE_INPUT: usize = 1000;

/// Per-launch bridge token: 32 random bytes, hex.
pub fn mint_token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokens_are_64_hex_and_unique() {
        let a = super::mint_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, super::mint_token());
    }
}
