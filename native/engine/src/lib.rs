//! # espectral-engine
//!
//! Native (Rust) port of the Espectral Client launcher engine
//! (`src/engine/*.mjs`). The UI holds an [`Engine`] and calls its async
//! methods directly — one per former HTTP route (see `api/`). Events that
//! were Server-Sent Events are a broadcast channel ([`Engine::subscribe`]).
//!
//! On-disk data layout, JSON formats, network endpoints and verification are
//! identical to the JS engine so an existing install keeps working.
//! See `native/PORTING.md` for the module map, ownership and conventions.

#![forbid(unsafe_op_in_unsafe_fn)]

/// `return Err(EngineError::NotImplemented(..))` shorthand for unported stubs.
#[macro_export]
#[doc(hidden)]
macro_rules! not_ported {
    ($what:expr) => {
        Err($crate::EngineError::NotImplemented($what))
    };
}

// ---- foundation (ported) -------------------------------------------------
pub mod api;
pub mod config;
pub mod download;
pub mod engine;
pub mod error;
pub mod events;
pub mod fingerprint;
pub mod nbt;
pub mod onboarding;
pub mod paths;
pub mod platform;
pub mod rules;
pub mod stats;
#[doc(hidden)]
pub mod test_support;
pub mod types;
pub mod util;
pub mod ziputil;

// ---- game-resolve ----------------------------------------------------------
pub mod jvm;
pub mod neoforge;
pub mod resolver;
pub mod runtimes;

// ---- game-launch -----------------------------------------------------------
pub mod aot;
pub mod game_sessions;
pub mod launch;
pub mod version_change;

// ---- content ---------------------------------------------------------------
pub mod import;
pub mod instances;
pub mod mods;
pub mod mrpack;
pub mod servers;

// ---- accounts-skins --------------------------------------------------------
pub mod accounts;
pub mod backgrounds;
pub mod cosmetics;
pub mod msauth;
pub mod skins;

// ---- identity-discord ------------------------------------------------------
pub mod bridge;
pub mod client;
pub mod discord;
pub mod discord_auth;
pub mod grant;
pub mod identity;
pub mod loopback;
pub mod presence;

pub use engine::{ActiveInstances, Engine, EngineOptions, VERSION};
pub use error::{EngineError, EngineResult, ErrorEnvelope};
pub use events::{EngineEvent, EventBus};
