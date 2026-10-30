//! Port of `src/engine/servers.mjs` — Espectral server status (MAIN + 24/7
//! Meetups) via the mcsrvstat-style status API, 150 s cache.
//!
//! **Owner: content.** Status: STUB.
//!
//! JS exports to port: HOSTS getServerStatus getAllServerStatuses.
//! Cache lives in ContentState (api/content.rs).

/// Hosts shown on the home screen.
pub const HOSTS: [&str; 2] = ["main.uhce.top", "24h.uhce.top"];
