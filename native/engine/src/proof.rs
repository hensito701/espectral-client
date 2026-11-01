//! Loopback peer proofs (no JS counterpart).
//!
//! Any local user can listen on 127.0.0.1:4199 while the launcher is closed.
//! Secrets therefore never go to "whoever answers the port":
//!
//! - **Agent API / MCP shim.** Before sending its bearer token, the shim asks
//!   `GET /api/agent/proof?nonce=<32 hex>`; the engine answers
//!   `hmac(agent_token, "agent-proof\n" + nonce)`. Only a process that can read
//!   this user's `agent-token` can produce it ([`agent_proof`]).
//! - **Mod bridge.** The per-launch bridge token reaches the game through the
//!   `ESPECTRAL_BRIDGE_TOKEN` environment variable (not the JVM argv, which
//!   other users can read) and never travels on the wire. Each request carries
//!   [`BRIDGE_ID_HEADER`] (`sha256(token)[..16]`), a fresh [`NONCE_HEADER`] and
//!   [`MAC_HEADER`] = [`request_mac`]; every authenticated answer carries
//!   [`PROOF_HEADER`] = [`response_proof`] over its exact body, which the mod
//!   checks before acting on it.
//!
//! HMAC-SHA256 (RFC 2104); keys are the tokens' ASCII bytes; hex is lowercase.

use rand::RngCore;
use sha2::{Digest, Sha256};

pub const BRIDGE_ID_HEADER: &str = "x-espectral-bridge-id";
pub const NONCE_HEADER: &str = "x-espectral-nonce";
pub const MAC_HEADER: &str = "x-espectral-bridge-mac";
pub const PROOF_HEADER: &str = "x-espectral-proof";
/// Environment variable carrying the bridge token to the game.
pub const BRIDGE_TOKEN_ENV: &str = "ESPECTRAL_BRIDGE_TOKEN";

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// HMAC-SHA256 (RFC 2104, block size 64) as lowercase hex.
pub fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(msg).finalize();
    hex::encode(Sha256::new().chain_update(opad).chain_update(inner).finalize())
}

/// Constant-time byte comparison.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 16 random bytes as 32 lowercase hex.
pub fn mint_nonce() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Exactly 32 lowercase hex characters.
pub fn is_nonce(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Public identifier of a bridge token: the first 16 hex of its SHA-256.
pub fn bridge_id(token: &str) -> String {
    sha256_hex(token.as_bytes())[..16].to_string()
}

/// MAC the mod sends: over method, request target (path + query), nonce and body hash.
pub fn request_mac(token: &str, method: &str, target: &str, nonce: &str, body: &[u8]) -> String {
    let msg = format!("req\n{}\n{target}\n{nonce}\n{}", method.to_ascii_uppercase(), sha256_hex(body));
    hmac_sha256_hex(token.as_bytes(), msg.as_bytes())
}

/// Proof the engine attaches to an authenticated bridge answer.
pub fn response_proof(token: &str, nonce: &str, body: &[u8]) -> String {
    hmac_sha256_hex(token.as_bytes(), format!("res\n{nonce}\n{}", sha256_hex(body)).as_bytes())
}

/// Proof the engine gives the MCP shim before it sends the bearer token.
pub fn agent_proof(token: &str, nonce: &str) -> String {
    hmac_sha256_hex(token.as_bytes(), format!("agent-proof\n{nonce}").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4231 test cases 1 and 2 (the Java mod uses javax.crypto HmacSHA256).
    #[test]
    fn hmac_matches_rfc_4231() {
        assert_eq!(
            hmac_sha256_hex(&[0x0b; 20], b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Keys longer than the block are hashed first (RFC 4231 case 6).
        assert_eq!(
            hmac_sha256_hex(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First"),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn macs_bind_every_field() {
        let t = "a".repeat(64);
        let n = "0123456789abcdef0123456789abcdef";
        let base = request_mac(&t, "post", "/api/bridge/agent/poll", n, b"{}");
        assert_eq!(base, request_mac(&t, "POST", "/api/bridge/agent/poll", n, b"{}"));
        assert_ne!(base, request_mac(&t, "POST", "/api/bridge/agent/state", n, b"{}"));
        assert_ne!(base, request_mac(&t, "POST", "/api/bridge/agent/poll", &n.replace('0', "1"), b"{}"));
        assert_ne!(base, request_mac(&t, "POST", "/api/bridge/agent/poll", n, b"{ }"));
        assert_ne!(base, request_mac(&"b".repeat(64), "POST", "/api/bridge/agent/poll", n, b"{}"));
        assert_ne!(response_proof(&t, n, b"x"), response_proof(&t, n, b"y"));
        assert_eq!(bridge_id(&t).len(), 16);
        assert!(is_nonce(&mint_nonce()) && !is_nonce("XYZ") && !is_nonce(&"A".repeat(32)));
    }

    /// The same vectors are pinned in the Java mod's BridgeAuthTest: both
    /// sides must compute identical ids, MACs and proofs.
    #[test]
    fn wire_vectors_shared_with_the_mod() {
        let t = "a".repeat(64);
        let n = "0123456789abcdef0123456789abcdef";
        let body = br#"{"uuids":[]}"#;
        assert_eq!(bridge_id(&t), "ffe054fe7ae0cb6d");
        assert_eq!(
            request_mac(&t, "POST", "/api/bridge/visible", n, body),
            "d72611961d5c0ee6dbab6f30bdc227b0db1ac81228d96a141d16f455a19ea18f"
        );
        assert_eq!(response_proof(&t, n, body), "ec7f663724dd302b4bc9713436253e7dcb7158ae36d1ad8657f9b131cb3d5cdf");
        assert_eq!(agent_proof(&t, n), "e7a767ad260c9889fbcbaefe35a884bb082ddc152dbf2aaa71179ddaf3fa185a");
    }
}
