//! Entitlement grant verification — the same rules as the mod's
//! `GrantVerifier` (`branding-mod/.../cosmetics/GrantVerifier.java`).
//!
//! Token: `b64url(header).b64url(payload).b64url(sig)`, Ed25519 over the ASCII
//! of the first two parts, no padding. Header
//! `{"alg":"EdDSA","kid":"eid1","typ":"ESG"}`; payload carries `v`, `iss`,
//! `aud`, `sub`, `flags`, `iat`, `exp`.
//!
//! The pinned key (`kid eid1`) is the default. [`Verifier::with_key`] is the
//! seam used by the tests (and by the end-to-end loopback test) to inject a
//! throwaway keypair; production code always uses [`Verifier::pinned`].

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};

use crate::util::now_unix;
use crate::Code;

/// Pinned key id from the protocol.
pub const KID: &str = "eid1";
/// Pinned Ed25519 public key, SPKI DER, base64 (as published in `PROTOCOL.md`).
pub const PINNED_SPKI_B64: &str = "MCowBQYDK2VwAyEA77kg5+/Nn6NvArCPuDvLYu3E4rvmYJHzaCGxwyvdlCI=";

const ASN1_SPKI_ED25519_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// Accepted issuers: the canonical one and the legacy one still minted by the
/// service until the ops `GRANT_ISS` flip.
pub const ISSUERS: [&str; 2] = ["https://id.uhce.top", "https://id.espectral.es"];
pub const AUDIENCE: &str = "espectral-client";
/// Maximum `iat` skew into the future, seconds.
pub const MAX_IAT_SKEW_S: i64 = 300;

/// Verified claim set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    pub sub: String,
    pub exp: i64,
    pub flags: Vec<String>,
}

impl Claims {
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    /// The relay is MVP-only: no `mvp` flag, no route.
    pub fn is_mvp(&self) -> bool {
        self.has_flag("mvp")
    }

    /// First 8 hex of `sub`, for logging.
    pub fn sub_short(&self) -> &str {
        &self.sub[..8.min(self.sub.len())]
    }
}

/// Grant verifier holding one Ed25519 key.
#[derive(Clone)]
pub struct Verifier {
    key: VerifyingKey,
}

impl Verifier {
    pub fn with_key(key: VerifyingKey) -> Self {
        Verifier { key }
    }

    /// The production pinned key. Panics only if the compiled-in constant is
    /// corrupt, which the unit tests pin down.
    pub fn pinned() -> Self {
        Verifier {
            key: pinned_verifying_key(),
        }
    }

    pub fn verify(&self, jws: &str) -> Result<Claims, Code> {
        self.verify_at(jws, now_unix())
    }

    /// Verification with an explicit clock (tests).
    pub fn verify_at(&self, jws: &str, now: i64) -> Result<Claims, Code> {
        let mut parts = jws.split('.');
        let (h64, p64, s64) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(h), Some(p), Some(s), None)
                if !h.is_empty() && !p.is_empty() && !s.is_empty() =>
            {
                (h, p, s)
            }
            _ => return Err(Code::Auth),
        };

        let header = decode_json(h64).ok_or(Code::Auth)?;
        if header.get("alg").and_then(|v| v.as_str()) != Some("EdDSA") {
            return Err(Code::Auth);
        }
        if header.get("kid").and_then(|v| v.as_str()) != Some(KID) {
            return Err(Code::Auth);
        }
        if header.get("typ").and_then(|v| v.as_str()) != Some("ESG") {
            return Err(Code::Auth);
        }

        let sig_bytes = URL_SAFE_NO_PAD.decode(s64).map_err(|_| Code::Auth)?;
        let sig = Signature::from_slice(&sig_bytes).map_err(|_| Code::Auth)?;
        let signed = &jws.as_bytes()[..h64.len() + 1 + p64.len()];
        self.key
            .verify_strict(signed, &sig)
            .map_err(|_| Code::Auth)?;

        let payload = decode_json(p64).ok_or(Code::Auth)?;
        if payload.get("v").and_then(|v| v.as_i64()) != Some(1) {
            return Err(Code::Auth);
        }
        match payload.get("iss").and_then(|v| v.as_str()) {
            Some(iss) if ISSUERS.contains(&iss) => {}
            _ => return Err(Code::Auth),
        }
        if payload.get("aud").and_then(|v| v.as_str()) != Some(AUDIENCE) {
            return Err(Code::Auth);
        }

        // `exp` and `iat` must be integers; a missing or non-numeric claim is
        // treated as expired rather than authenticated-but-odd.
        let exp = payload
            .get("exp")
            .and_then(|v| v.as_i64())
            .ok_or(Code::Expired)?;
        if exp < now {
            return Err(Code::Expired);
        }
        let iat = payload
            .get("iat")
            .and_then(|v| v.as_i64())
            .ok_or(Code::Expired)?;
        if iat > now.saturating_add(MAX_IAT_SKEW_S) {
            return Err(Code::Expired);
        }

        let sub = payload
            .get("sub")
            .and_then(|v| v.as_str())
            .ok_or(Code::Auth)?;
        if !is_hex32(sub) {
            return Err(Code::Auth);
        }

        let mut flags = Vec::new();
        if let Some(arr) = payload.get("flags").and_then(|v| v.as_array()) {
            for f in arr {
                if let Some(s) = f.as_str() {
                    let lower = s.to_ascii_lowercase();
                    if (lower == "supporter" || lower == "mvp") && !flags.contains(&lower) {
                        flags.push(lower);
                    }
                }
            }
        }

        Ok(Claims {
            sub: sub.to_owned(),
            exp,
            flags,
        })
    }
}

fn decode_json(b64: &str) -> Option<serde_json::Value> {
    let bytes = URL_SAFE_NO_PAD.decode(b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if v.is_object() {
        Some(v)
    } else {
        None
    }
}

/// 32 lowercase hex characters.
pub fn is_hex32(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Decodes the pinned SPKI DER blob into a verifying key.
pub fn pinned_verifying_key() -> VerifyingKey {
    let der = URL_SAFE_NO_PAD
        .decode(PINNED_SPKI_B64)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(PINNED_SPKI_B64))
        .expect("pinned SPKI constant is valid base64");
    let key: [u8; 32] = der
        .strip_prefix(&ASN1_SPKI_ED25519_PREFIX)
        .and_then(|raw| raw.try_into().ok())
        .expect("pinned SPKI constant is a 44-byte Ed25519 SubjectPublicKeyInfo");
    VerifyingKey::from_bytes(&key).expect("pinned key is a valid Ed25519 point")
}

/// Encodes bytes as unpadded base64url (helper shared with the tests and the
/// small `grant` example).
pub fn b64url(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const NOW: i64 = 1_790_000_000;
    const SUB: &str = "0123456789abcdef0123456789abcdef";

    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn header_json(alg: &str, kid: &str, typ: &str) -> String {
        format!(r#"{{"alg":"{alg}","kid":"{kid}","typ":"{typ}"}}"#)
    }

    fn payload_json(
        v: i64,
        iss: &str,
        aud: &str,
        sub: &str,
        flags: &[&str],
        iat: i64,
        exp: i64,
    ) -> String {
        let flags_json = flags
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"v":{v},"iss":"{iss}","aud":"{aud}","sub":"{sub}","flags":[{flags_json}],"iat":{iat},"exp":{exp}}}"#
        )
    }

    fn sign(key: &SigningKey, header: &str, payload: &str) -> String {
        let (h, p) = (b64url(header.as_bytes()), b64url(payload.as_bytes()));
        let msg = format!("{h}.{p}");
        let sig = key.sign(msg.as_bytes());
        format!("{h}.{p}.{}", b64url(&sig.to_bytes()))
    }

    /// A grant with the canonical claim set and `flags`; other fields can be
    /// overridden by the caller for negative tests.
    fn good(key: &SigningKey) -> String {
        sign(
            key,
            &header_json("EdDSA", "eid1", "ESG"),
            &payload_json(
                1,
                "https://id.uhce.top",
                "espectral-client",
                SUB,
                &["supporter", "mvp"],
                NOW,
                NOW + 86_400,
            ),
        )
    }

    fn verifier(key: &SigningKey) -> Verifier {
        Verifier::with_key(key.verifying_key())
    }

    #[test]
    fn pinned_key_parses() {
        let der = base64::engine::general_purpose::STANDARD
            .decode(PINNED_SPKI_B64)
            .unwrap();
        assert_eq!(der.len(), 44);
        assert_eq!(&der[..12], &ASN1_SPKI_ED25519_PREFIX);
        // Must decode without panicking.
        let _ = pinned_verifying_key();
    }

    #[test]
    fn accepts_valid_grant_from_both_issuers() {
        let key = test_key();
        let v = verifier(&key);
        let claims = v.verify_at(&good(&key), NOW).unwrap();
        assert_eq!(claims.sub, SUB);
        assert!(claims.is_mvp());
        assert!(claims.has_flag("supporter"));
        assert_eq!(claims.exp, NOW + 86_400);
        assert_eq!(claims.sub_short(), "01234567");

        for iss in ISSUERS {
            let jws = sign(
                &key,
                &header_json("EdDSA", "eid1", "ESG"),
                &payload_json(1, iss, "espectral-client", SUB, &["mvp"], NOW, NOW + 60),
            );
            assert!(v.verify_at(&jws, NOW).is_ok(), "{iss}");
        }
    }

    #[test]
    fn rejects_bad_signature() {
        let key = test_key();
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let v = verifier(&key);
        assert_eq!(v.verify_at(&good(&other), NOW), Err(Code::Auth));
    }

    #[test]
    fn rejects_tampered_payload() {
        let key = test_key();
        let v = verifier(&key);
        let jws = good(&key);
        let mut parts: Vec<String> = jws.split('.').map(|s| s.to_owned()).collect();
        let tampered_payload = payload_json(
            1,
            "https://id.uhce.top",
            "espectral-client",
            SUB,
            &["mvp"],
            NOW,
            NOW + 999_999,
        );
        parts[1] = b64url(tampered_payload.as_bytes());
        assert_eq!(v.verify_at(&parts.join("."), NOW), Err(Code::Auth));
    }

    #[test]
    fn rejects_wrong_aud_iss_typ_kid_alg() {
        let key = test_key();
        let v = verifier(&key);
        let hdr = header_json("EdDSA", "eid1", "ESG");
        let cases: Vec<(String, String)> = vec![
            (
                hdr.clone(),
                payload_json(
                    1,
                    "https://evil.example",
                    "espectral-client",
                    SUB,
                    &["mvp"],
                    NOW,
                    NOW + 60,
                ),
            ),
            (
                hdr.clone(),
                payload_json(
                    1,
                    "https://id.uhce.top",
                    "someone-else",
                    SUB,
                    &["mvp"],
                    NOW,
                    NOW + 60,
                ),
            ),
            (
                header_json("EdDSA", "eid1", "JWT"),
                payload_json(
                    1,
                    "https://id.uhce.top",
                    "espectral-client",
                    SUB,
                    &["mvp"],
                    NOW,
                    NOW + 60,
                ),
            ),
            (
                header_json("EdDSA", "eid2", "ESG"),
                payload_json(
                    1,
                    "https://id.uhce.top",
                    "espectral-client",
                    SUB,
                    &["mvp"],
                    NOW,
                    NOW + 60,
                ),
            ),
            (
                header_json("HS256", "eid1", "ESG"),
                payload_json(
                    1,
                    "https://id.uhce.top",
                    "espectral-client",
                    SUB,
                    &["mvp"],
                    NOW,
                    NOW + 60,
                ),
            ),
            (
                header_json("EdDSA", "eid1", "ESG"),
                payload_json(
                    2,
                    "https://id.uhce.top",
                    "espectral-client",
                    SUB,
                    &["mvp"],
                    NOW,
                    NOW + 60,
                ),
            ),
        ];
        for (h, p) in cases {
            assert_eq!(
                v.verify_at(&sign(&key, &h, &p), NOW),
                Err(Code::Auth),
                "{h} / {p}"
            );
        }
    }

    #[test]
    fn rejects_expired_and_future_iat() {
        let key = test_key();
        let v = verifier(&key);
        let h = header_json("EdDSA", "eid1", "ESG");

        let expired = sign(
            &key,
            &h,
            &payload_json(
                1,
                "https://id.uhce.top",
                "espectral-client",
                SUB,
                &["mvp"],
                NOW - 100,
                NOW - 1,
            ),
        );
        assert_eq!(v.verify_at(&expired, NOW), Err(Code::Expired));

        // exp == now is still valid (doc rejects only exp < now).
        let boundary = sign(
            &key,
            &h,
            &payload_json(
                1,
                "https://id.uhce.top",
                "espectral-client",
                SUB,
                &["mvp"],
                NOW - 10,
                NOW,
            ),
        );
        assert!(v.verify_at(&boundary, NOW).is_ok());

        let future = sign(
            &key,
            &h,
            &payload_json(
                1,
                "https://id.uhce.top",
                "espectral-client",
                SUB,
                &["mvp"],
                NOW + MAX_IAT_SKEW_S + 1,
                NOW + 86_400,
            ),
        );
        assert_eq!(v.verify_at(&future, NOW), Err(Code::Expired));

        let skew_ok = sign(
            &key,
            &h,
            &payload_json(
                1,
                "https://id.uhce.top",
                "espectral-client",
                SUB,
                &["mvp"],
                NOW + MAX_IAT_SKEW_S,
                NOW + 86_400,
            ),
        );
        assert!(v.verify_at(&skew_ok, NOW).is_ok());
    }

    #[test]
    fn rejects_without_mvp_flag() {
        let key = test_key();
        let v = verifier(&key);
        let h = header_json("EdDSA", "eid1", "ESG");
        for flags in [vec![], vec!["supporter"], vec!["MVPX"], vec!["verified"]] {
            let jws = sign(
                &key,
                &h,
                &payload_json(
                    1,
                    "https://id.uhce.top",
                    "espectral-client",
                    SUB,
                    &flags,
                    NOW,
                    NOW + 60,
                ),
            );
            let claims = v.verify_at(&jws, NOW).unwrap();
            assert!(!claims.is_mvp(), "{flags:?}");
        }
        // Sanity: uppercase `MVP` in the token is normalised to lowercase, as
        // the mod does.
        let jws = sign(
            &key,
            &h,
            &payload_json(
                1,
                "https://id.uhce.top",
                "espectral-client",
                SUB,
                &["MVP"],
                NOW,
                NOW + 60,
            ),
        );
        assert!(v.verify_at(&jws, NOW).unwrap().is_mvp());
    }

    #[test]
    fn rejects_bad_sub() {
        let key = test_key();
        let v = verifier(&key);
        let h = header_json("EdDSA", "eid1", "ESG");
        for sub in [
            "",
            "0123456789abcdef0123456789abcde",   // 31
            "0123456789abcdef0123456789abcdef0", // 33
            "0123456789ABCDEF0123456789ABCDEF",  // uppercase
            "0123456789abcdef0123456789abcdeg",  // non-hex
            "0123-56789abcdef0123456789abcdef",  // dash
            "0123456789abcdef0123456789abcdeﬀ",  // non-ascii
        ] {
            let jws = sign(
                &key,
                &h,
                &payload_json(
                    1,
                    "https://id.uhce.top",
                    "espectral-client",
                    sub,
                    &["mvp"],
                    NOW,
                    NOW + 60,
                ),
            );
            assert_eq!(v.verify_at(&jws, NOW), Err(Code::Auth), "sub {sub:?}");
        }
    }

    #[test]
    fn rejects_malformed_envelopes() {
        let key = test_key();
        let v = verifier(&key);
        let jws = good(&key);
        let parts: Vec<&str> = jws.split('.').collect();
        let cases = [
            "".to_string(),
            "abc".to_string(),
            format!("{}.{}", parts[0], parts[1]),
            format!("{jws}."),
            format!(".{}.{}", parts[1], parts[2]),
            format!("{}.{}.", parts[0], parts[1]),
            format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[2]),
            format!("{}==.{}.{}", parts[0], parts[1], parts[2]),
            format!("{}.{}!.{}", parts[0], parts[1], parts[2]),
        ];
        for c in cases {
            assert_eq!(v.verify_at(&c, NOW), Err(Code::Auth), "{c:?}");
        }
    }

    #[test]
    fn rejects_non_object_json() {
        let key = test_key();
        let v = verifier(&key);
        let h = b64url(b"[]");
        let p = b64url(b"\"x\"");
        let msg = format!("{h}.{p}");
        let sig = key.sign(msg.as_bytes());
        let jws = format!("{msg}.{}", b64url(&sig.to_bytes()));
        assert_eq!(v.verify_at(&jws, NOW), Err(Code::Auth));
    }

    #[test]
    fn pinned_key_rejects_test_key_grants() {
        let key = test_key();
        assert_eq!(
            Verifier::pinned().verify_at(&good(&key), NOW),
            Err(Code::Auth)
        );
    }

    #[test]
    fn hex32_helper() {
        assert!(is_hex32(SUB));
        assert!(!is_hex32("0123456789ABCDEF0123456789ABCDEF"));
        assert!(!is_hex32(""));
        assert!(!is_hex32("0123456789abcdef0123456789abcde"));
    }
}
