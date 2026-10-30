//! Port of `src/engine/grant.mjs` — compact EdDSA (Ed25519) grant
//! verification against the pinned espectral-id key (`kid` eid1, SPKI DER).
//!
//! **Owner: identity-discord.** Status: STUB.
//!
//! JS exports to port: DEFAULT_KID GRANT_ISS LEGACY_GRANT_ISS
//! GRANT_ISS_ACCEPTED GRANT_AUD MAX_IAT_SKEW_S GrantError registerGrantKey
//! verifyGrant signGrant (test-only signer). Tests: tests/grant.test.mjs.
//! Crypto: ed25519-dalek (`VerifyingKey::from_public_key_der`, features
//! pkcs8 + alloc pinned in the workspace), base64url via `base64`.

pub const DEFAULT_KID: &str = "eid1";
pub const GRANT_ISS: &str = "https://id.uhce.top";
pub const LEGACY_GRANT_ISS: &str = "https://id.espectral.es";
pub const GRANT_ISS_ACCEPTED: [&str; 2] = [GRANT_ISS, LEGACY_GRANT_ISS];
pub const GRANT_AUD: &str = "espectral-client";
pub const MAX_IAT_SKEW_S: i64 = 300;
