//! The Engine facade: one async method per JS HTTP route.
//!
//! Each file is an `impl Engine` block owned by ONE porting cluster (see
//! native/PORTING.md). Method signatures are the contract with the UI —
//! request/response structs serialize to the JS JSON shapes, so a future HTTP
//! shim (or a test) can compare byte-for-byte with the Node engine.
//!
//! | file                 | owner            | JS route files |
//! |----------------------|------------------|----------------|
//! | `core.rs`            | foundation (done) | server.mjs health/shutdown, events.mjs, misc.mjs theme/accent/config/open-folder |
//! | `game_resolve.rs`    | game-resolve     | routes/versions.mjs, misc.mjs GET /api/jvm |
//! | `game_launch.rs`     | game-launch      | routes/launch.mjs, instances.mjs version-change routes |
//! | `content.rs`         | content          | routes/instances.mjs, mods, mrpack, import, servers, client, misc.mjs instance icon |
//! | `accounts_skins.rs`  | accounts-skins   | routes/skins, cosmetics, backgrounds, misc.mjs accounts/avatars/MSA/skin |
//! | `identity_discord.rs`| identity-discord | routes/identity.mjs, routes/discord.mjs |
//! | `agent.rs`           | agent            | routes/agent.mjs (UI routes; the agent API + mod relay are HTTP, `loopback.rs`) |
//!
//! Each cluster file also owns its in-memory state struct (`*State`, held by
//! `Engine`) and its `on_engine_start` / `on_engine_shutdown` hooks.

pub mod accounts_skins;
pub mod agent;
pub mod content;
pub mod core;
pub mod game_launch;
pub mod game_resolve;
pub mod identity_discord;

use serde::{Deserialize, Serialize};

/// `{ deleted: <name> }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeletedReply {
    pub deleted: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trash: Option<String>,
}

/// `{ removed: true }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemovedReply {
    pub removed: bool,
}

/// `{ queued: true }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedReply {
    pub queued: bool,
}

/// `{ path }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathReply {
    pub path: String,
}

/// `{ url }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UrlReply {
    pub url: String,
}
