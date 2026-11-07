//! Ruta Espectral relay ("ERT1").
//!
//! A small, stateless-in-memory TCP relay that lets an Espectral Client player
//! route Minecraft traffic through `ruta.uhce.top` so the destination server
//! never sees the player's IP. The wire contract, grant rules, destination
//! policy and limits are fixed by [`PROTOCOL.md`](../PROTOCOL.md).
//!
//! Nothing here is persistent: no files, no cache, no DNS.

#![forbid(unsafe_code)]

pub mod config;
pub mod dest;
pub mod grant;
pub mod limits;
pub mod proto;
pub mod proxy_v2;
pub mod server;
pub mod util;

pub use config::{Command, Config};
pub use grant::{Claims, Verifier};

/// Result of a connection, used both as the `ERR <code>` line and as the
/// `result=` field of the end-of-connection log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// Malformed handshake line (`proto`).
    Proto,
    /// Bad/unknown signature, wrong aud/iss/typ/kid/alg, malformed envelope.
    Auth,
    /// `exp` passed or `iat` too far in the future.
    Expired,
    /// Grant flags lack `mvp`.
    NotMvp,
    /// Destination not allowed by policy.
    Dest,
    /// Per-sub concurrency, per-sub rate, per-IP concurrency or daily quota.
    Limit,
    /// Upstream unreachable or connect timeout.
    Connect,
    /// Global connection cap reached.
    Busy,

    // Post-handshake outcomes: logged, never sent to the client.
    /// Idle timeout fired.
    Idle,
    /// Both directions reached EOF.
    Closed,
    /// Socket error / peer reset.
    Error,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Proto => "proto",
            Code::Auth => "auth",
            Code::Expired => "expired",
            Code::NotMvp => "notmvp",
            Code::Dest => "dest",
            Code::Limit => "limit",
            Code::Connect => "connect",
            Code::Busy => "busy",
            Code::Idle => "idle",
            Code::Closed => "closed",
            Code::Error => "error",
        }
    }

    /// True for codes the client is told about as `ERR <code>`; the rest can
    /// only happen after `OK` was already sent.
    pub fn is_client_error(self) -> bool {
        matches!(
            self,
            Code::Proto
                | Code::Auth
                | Code::Expired
                | Code::NotMvp
                | Code::Dest
                | Code::Limit
                | Code::Connect
                | Code::Busy
        )
    }
}
