//! Port of `src/engine/grant.mjs` — compact EdDSA (Ed25519) grant
//! verification against the pinned espectral-id key (`kid` eid1, SPKI DER).
//!
//! **Owner: identity-discord.** Status: PORTED.
//!
//! A grant is `b64url(header).b64url(payload).b64url(sig)` where the signature
//! covers the ASCII bytes of the first two parts. The production signing key
//! is pinned here (public half only). Tests register their own throwaway
//! keypair under another kid via [`register_grant_key`], so the production
//! pin is never overridden.
//!
//! Header:  `{"alg":"EdDSA","kid":"eid1","typ":"ESG"}`
//! Payload: `{"v":1,"iss":"https://id.uhce.top","aud":"espectral-client",
//!           "sub":"<32 lowercase hex uuid>","flags":[...],"iat":s,"exp":s}`
//!
//! BOTH issuers verify: the canonical `https://id.uhce.top` and the legacy
//! `https://id.espectral.es`. [`verify_grant`] fails with a [`GrantError`]
//! on ANY mismatch — a missing/invalid grant means "no ★ and no cosmetics",
//! never a soft pass.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine as _;
use ed25519_dalek::pkcs8::DecodePublicKey;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::{Map, Value};

use crate::error::EngineError;

pub const DEFAULT_KID: &str = "eid1";
/// Canonical issuer — what espectral-id mints after the GRANT_ISS flip.
pub const GRANT_ISS: &str = "https://id.uhce.top";
/// Issuer minted by espectral-id until the ops GRANT_ISS flip.
pub const LEGACY_GRANT_ISS: &str = "https://id.espectral.es";
/// Every `iss` value accepted; anything else is GRANT_BAD_ISS.
pub const GRANT_ISS_ACCEPTED: [&str; 2] = [GRANT_ISS, LEGACY_GRANT_ISS];
pub const GRANT_AUD: &str = "espectral-client";
/// Leeway on iat so a freshly minted grant on a fast server clock verifies.
pub const MAX_IAT_SKEW_S: i64 = 300;

/// Pinned production verifying key (SPKI DER, base64) — kid 'eid1'.
const PINNED_SPKI_B64: &str = "MCowBQYDK2VwAyEA77kg5+/Nn6NvArCPuDvLYu3E4rvmYJHzaCGxwyvdlCI=";

/// JS `GrantError` (status 400, specific `GRANT_*` code).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct GrantError {
    pub code: &'static str,
    pub message: String,
}

impl GrantError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        GrantError { code, message: message.into() }
    }
}

impl From<GrantError> for EngineError {
    fn from(e: GrantError) -> Self {
        EngineError::api(400, e.code, e.message)
    }
}

/// A verified grant payload.
#[derive(Debug, Clone, PartialEq)]
pub struct GrantPayload {
    /// 32 lowercase hex.
    pub sub: String,
    /// String entries of `flags` (empty when absent).
    pub flags: Vec<String>,
    /// Unix seconds.
    pub iat: f64,
    /// Unix seconds.
    pub exp: f64,
    pub iss: String,
    pub aud: String,
    /// The whole payload object.
    pub raw: Map<String, Value>,
}

fn keys() -> &'static RwLock<HashMap<String, VerifyingKey>> {
    static KEYS: OnceLock<RwLock<HashMap<String, VerifyingKey>>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let der = base64::engine::general_purpose::STANDARD.decode(PINNED_SPKI_B64).expect("pinned key base64");
        let key = VerifyingKey::from_public_key_der(&der).expect("pinned key SPKI");
        let mut m = HashMap::new();
        m.insert(DEFAULT_KID.to_string(), key);
        RwLock::new(m)
    })
}

/// Register a verifying key for a kid (tests use their own keypair + kid).
pub fn register_grant_key(kid: &str, key: VerifyingKey) -> Result<(), EngineError> {
    if kid.is_empty() {
        return Err(EngineError::other("kid must be a non-empty string"));
    }
    keys().write().unwrap_or_else(|p| p.into_inner()).insert(kid.to_string(), key);
    Ok(())
}

/// Register a key given as base64 SPKI DER (the JS string form).
pub fn register_grant_key_spki_b64(kid: &str, spki_b64: &str) -> Result<(), EngineError> {
    let der = base64::engine::general_purpose::STANDARD
        .decode(spki_b64.trim())
        .map_err(|e| EngineError::other(format!("bad key base64: {e}")))?;
    let key = VerifyingKey::from_public_key_der(&der).map_err(|e| EngineError::other(format!("bad SPKI key: {e}")))?;
    register_grant_key(kid, key)
}

/// Node `Buffer.from(s, 'base64url')`: padding optional, trailing bits ignored,
/// a dangling final sextet dropped.
const B64URL_LENIENT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

fn b64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn decode_part(part: &str, what: &str) -> Result<Vec<u8>, GrantError> {
    let ok_chars = part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if part.is_empty() || part.len() > 8192 || !ok_chars {
        return Err(GrantError::new("GRANT_MALFORMED", format!("grant {what} part is not base64url")));
    }
    let usable = if part.len() % 4 == 1 { &part[..part.len() - 1] } else { part };
    B64URL_LENIENT
        .decode(usable)
        .map_err(|_| GrantError::new("GRANT_MALFORMED", format!("grant {what} part is not decodable")))
}

fn parse_json_part(part: &str, what: &str) -> Result<Map<String, Value>, GrantError> {
    let bytes = decode_part(part, what)?;
    let text = String::from_utf8_lossy(&bytes);
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(GrantError::new("GRANT_MALFORMED", format!("grant {what} is not a JSON object"))),
        Err(_) => Err(GrantError::new("GRANT_MALFORMED", format!("grant {what} is not valid JSON"))),
    }
}

fn js(v: Option<&Value>) -> String {
    match v {
        None => "undefined".to_string(),
        Some(v) => v.to_string(),
    }
}

fn is_uuid32(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Current unix seconds.
pub fn now_s() -> i64 {
    crate::platform::now_ms().div_euclid(1000)
}

/// Verify a compact EdDSA grant. `sub`: when given, the grant's `sub` must
/// equal it (a grant for account A must never unlock account B). `now`:
/// unix seconds override for tests (None = now).
pub fn verify_grant(token: &str, sub: Option<&str>, now: Option<i64>) -> Result<GrantPayload, GrantError> {
    let now = now.unwrap_or_else(now_s) as f64;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(GrantError::new("GRANT_MALFORMED", "grant must have exactly 3 dot-separated parts"));
    }
    let (h64, p64, s64) = (parts[0], parts[1], parts[2]);

    let header = parse_json_part(h64, "header")?;
    if header.get("alg").and_then(Value::as_str) != Some("EdDSA") {
        return Err(GrantError::new(
            "GRANT_BAD_ALG",
            format!("grant alg must be EdDSA (got {})", js(header.get("alg"))),
        ));
    }
    let key = match header.get("kid").and_then(Value::as_str) {
        Some(kid) => keys().read().unwrap_or_else(|p| p.into_inner()).get(kid).copied(),
        None => None,
    };
    let Some(key) = key else {
        return Err(GrantError::new(
            "GRANT_UNKNOWN_KID",
            format!("grant signed with unknown kid {}", js(header.get("kid"))),
        ));
    };

    let sig = decode_part(s64, "signature")?;
    let signing_input = format!("{h64}.{p64}");
    let ok = Signature::from_slice(&sig).map(|s| key.verify(signing_input.as_bytes(), &s).is_ok()).unwrap_or(false);
    if !ok {
        return Err(GrantError::new("GRANT_BAD_SIGNATURE", "grant signature does not verify"));
    }

    let payload = parse_json_part(p64, "payload")?;
    if payload.get("v").and_then(Value::as_f64) != Some(1.0) {
        return Err(GrantError::new("GRANT_BAD_VERSION", format!("grant v must be 1 (got {})", js(payload.get("v")))));
    }
    let iss = payload.get("iss").and_then(Value::as_str).unwrap_or_default();
    if !payload.get("iss").is_some_and(Value::is_string) || !GRANT_ISS_ACCEPTED.contains(&iss) {
        return Err(GrantError::new("GRANT_BAD_ISS", format!("grant iss mismatch (got {})", js(payload.get("iss")))));
    }
    if payload.get("aud").and_then(Value::as_str) != Some(GRANT_AUD) {
        return Err(GrantError::new("GRANT_BAD_AUD", format!("grant aud mismatch (got {})", js(payload.get("aud")))));
    }
    let psub = match payload.get("sub").and_then(Value::as_str) {
        Some(s) if is_uuid32(s) => s.to_string(),
        _ => return Err(GrantError::new("GRANT_BAD_SUB", "grant sub must be a 32-char lowercase hex uuid")),
    };
    if let Some(want) = sub {
        if psub != want {
            return Err(GrantError::new("GRANT_WRONG_SUB", "grant is bound to a different uuid"));
        }
    }
    let exp = payload.get("exp").and_then(Value::as_f64).filter(|f| f.is_finite());
    let Some(exp) = exp.filter(|e| *e > now) else {
        return Err(GrantError::new("GRANT_EXPIRED", "grant is expired"));
    };
    let iat = payload.get("iat").and_then(Value::as_f64).filter(|f| f.is_finite());
    let Some(iat) = iat.filter(|i| *i <= now + MAX_IAT_SKEW_S as f64) else {
        return Err(GrantError::new("GRANT_BAD_IAT", "grant iat is in the future"));
    };
    let flags = match payload.get("flags") {
        None => vec![],
        Some(Value::Array(a)) => a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect(),
        Some(_) => return Err(GrantError::new("GRANT_MALFORMED", "grant flags must be an array")),
    };
    Ok(GrantPayload { sub: psub, flags, iat, exp, iss: iss.to_string(), aud: GRANT_AUD.to_string(), raw: payload })
}

/// Sign a grant — TEST/tooling helper only (production grants come from
/// espectral-id, which owns the private key).
pub fn sign_grant(payload: &Value, kid: &str, key: &SigningKey) -> String {
    let header = serde_json::json!({ "alg": "EdDSA", "kid": kid, "typ": "ESG" });
    let h64 = b64url_encode(header.to_string().as_bytes());
    let p64 = b64url_encode(payload.to_string().as_bytes());
    let sig = key.sign(format!("{h64}.{p64}").as_bytes());
    format!("{h64}.{p64}.{}", b64url_encode(&sig.to_bytes()))
}

/// Fresh random signing key (tests).
pub fn generate_signing_key() -> SigningKey {
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    SigningKey::from_bytes(&seed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::OnceLock;

    pub(crate) const TEST_KID: &str = "test1";

    /// The suite-wide test key registered under `test1`.
    pub(crate) fn test_key() -> &'static SigningKey {
        static K: OnceLock<SigningKey> = OnceLock::new();
        K.get_or_init(|| {
            let k = generate_signing_key();
            register_grant_key(TEST_KID, k.verifying_key()).unwrap();
            k
        })
    }

    pub(crate) fn sign_test_grant(sub: &str, flags: &[&str], exp_in_s: i64) -> String {
        let now = now_s();
        sign_grant(
            &json!({ "v": 1, "iss": GRANT_ISS, "aud": GRANT_AUD, "sub": sub, "flags": flags, "iat": now, "exp": now + exp_in_s }),
            TEST_KID,
            test_key(),
        )
    }

    const SUB: &str = "aabbccddeeff00112233445566778899";

    fn payload(over: Value) -> Value {
        let now = now_s();
        let mut p = json!({
            "v": 1, "iss": GRANT_ISS, "aud": GRANT_AUD, "sub": SUB,
            "flags": ["supporter", "mvp"], "iat": now, "exp": now + 3600,
        });
        for (k, v) in over.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    }

    fn grant(over: Value) -> String {
        sign_grant(&payload(over), TEST_KID, test_key())
    }

    fn code(r: Result<GrantPayload, GrantError>) -> &'static str {
        r.expect_err("must fail").code
    }

    #[test]
    fn valid_grant_verifies() {
        let p = verify_grant(&grant(json!({})), Some(SUB), None).unwrap();
        assert_eq!(p.sub, SUB);
        assert_eq!(p.flags, vec!["supporter", "mvp"]);
        assert_eq!(p.iss, GRANT_ISS);
        assert_eq!(p.aud, GRANT_AUD);
    }

    #[test]
    fn tampered_payload_and_signature_fail() {
        let t = grant(json!({}));
        let parts: Vec<&str> = t.split('.').collect();
        let forged = b64url_encode(payload(json!({ "flags": ["mvp", "admin"] })).to_string().as_bytes());
        assert_eq!(
            code(verify_grant(&format!("{}.{}.{}", parts[0], forged, parts[2]), Some(SUB), None)),
            "GRANT_BAD_SIGNATURE"
        );
        let s = parts[2];
        let idx = s.len() / 2;
        let repl = if &s[idx..idx + 1] == "A" { "B" } else { "A" };
        let bad = format!("{}{}{}", &s[..idx], repl, &s[idx + 1..]);
        assert_eq!(
            code(verify_grant(&format!("{}.{}.{}", parts[0], parts[1], bad), Some(SUB), None)),
            "GRANT_BAD_SIGNATURE"
        );
    }

    #[test]
    fn expired_rejected() {
        let now = now_s();
        assert_eq!(
            code(verify_grant(&grant(json!({ "iat": now - 90000, "exp": now - 10 })), Some(SUB), None)),
            "GRANT_EXPIRED"
        );
    }

    #[test]
    fn wrong_aud_iss_version() {
        assert_eq!(code(verify_grant(&grant(json!({ "aud": "minecraft" })), Some(SUB), None)), "GRANT_BAD_AUD");
        assert_eq!(
            code(verify_grant(&grant(json!({ "iss": "https://evil.example" })), Some(SUB), None)),
            "GRANT_BAD_ISS"
        );
        assert_eq!(code(verify_grant(&grant(json!({ "v": 2 })), Some(SUB), None)), "GRANT_BAD_VERSION");
    }

    #[test]
    fn both_issuers_verify() {
        assert_eq!(GRANT_ISS, "https://id.uhce.top");
        assert_eq!(LEGACY_GRANT_ISS, "https://id.espectral.es");
        assert_eq!(GRANT_ISS_ACCEPTED, [GRANT_ISS, LEGACY_GRANT_ISS]);
        assert_eq!(verify_grant(&grant(json!({ "iss": GRANT_ISS })), Some(SUB), None).unwrap().iss, GRANT_ISS);
        assert_eq!(
            verify_grant(&grant(json!({ "iss": LEGACY_GRANT_ISS })), Some(SUB), None).unwrap().iss,
            LEGACY_GRANT_ISS
        );
    }

    #[test]
    fn issuer_matching_is_exact() {
        for iss in [
            "https://id.uhce.top.evil.example",
            "https://id.uhce.top/",
            "http://id.uhce.top",
            "https://ID.UHCE.TOP",
            "https://id.espectral.es/",
            "id.uhce.top",
        ] {
            assert_eq!(code(verify_grant(&grant(json!({ "iss": iss })), Some(SUB), None)), "GRANT_BAD_ISS", "{iss}");
        }
    }

    #[test]
    fn wrong_sub() {
        let t = grant(json!({ "sub": "00000000000000000000000000000001" }));
        assert_eq!(code(verify_grant(&t, Some(SUB), None)), "GRANT_WRONG_SUB");
        assert_eq!(verify_grant(&t, None, None).unwrap().sub, "00000000000000000000000000000001");
    }

    #[test]
    fn unknown_kid() {
        let t = sign_grant(&payload(json!({})), "unknown-kid", test_key());
        assert_eq!(code(verify_grant(&t, Some(SUB), None)), "GRANT_UNKNOWN_KID");
    }

    #[test]
    fn wrong_key_under_known_kid() {
        let _ = test_key();
        let other = generate_signing_key();
        let t = sign_grant(&payload(json!({})), TEST_KID, &other);
        assert_eq!(code(verify_grant(&t, Some(SUB), None)), "GRANT_BAD_SIGNATURE");
    }

    #[test]
    fn pinned_production_key_rejects_foreign_signatures() {
        let t = sign_grant(&payload(json!({})), DEFAULT_KID, test_key());
        assert_eq!(code(verify_grant(&t, Some(SUB), None)), "GRANT_BAD_SIGNATURE");
    }

    #[test]
    fn non_eddsa_alg() {
        let _ = test_key();
        let h = b64url_encode(json!({ "alg": "HS256", "kid": TEST_KID, "typ": "ESG" }).to_string().as_bytes());
        let p = b64url_encode(payload(json!({})).to_string().as_bytes());
        assert_eq!(code(verify_grant(&format!("{h}.{p}.AAAA"), None, None)), "GRANT_BAD_ALG");
    }

    #[test]
    fn future_iat_beyond_skew() {
        let now = now_s();
        assert_eq!(
            code(verify_grant(&grant(json!({ "iat": now + 3600, "exp": now + 7200 })), Some(SUB), None)),
            "GRANT_BAD_IAT"
        );
        assert_eq!(verify_grant(&grant(json!({ "iat": now + 120 })), Some(SUB), None).unwrap().sub, SUB);
    }

    #[test]
    fn malformed_inputs() {
        for bad in ["", "abc", "a.b", "a.b.c.d", "a.b.!"] {
            let e = verify_grant(bad, None, None).expect_err(bad);
            assert!(e.code.starts_with("GRANT_"), "{bad}: {}", e.code);
        }
        // Non-string grants (JS null/42) cannot be expressed in Rust; the
        // nearest equivalent is an empty or garbage string, covered above.
    }

    #[test]
    fn grant_error_maps_to_400_envelope() {
        let e: EngineError = GrantError::new("GRANT_EXPIRED", "grant is expired").into();
        let env = e.envelope();
        assert_eq!((env.status, env.code.as_str()), (400, "GRANT_EXPIRED"));
    }
}
