//! Shared helpers for the integration tests: a throwaway keypair and a
//! grant minter that produces the exact wire format of `espectral-id`.

#![allow(dead_code)]

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};

/// Fixed test seed (never a production key).
pub const SEED: [u8; 32] = [42u8; 32];

pub fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&SEED)
}

pub fn b64url(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

pub struct GrantSpec {
    pub alg: &'static str,
    pub kid: &'static str,
    pub typ: &'static str,
    pub v: i64,
    pub iss: &'static str,
    pub aud: &'static str,
    pub sub: String,
    pub flags: Vec<String>,
    pub iat: i64,
    pub exp: i64,
}

impl GrantSpec {
    /// A valid MVP grant for `sub` from the canonical issuer.
    pub fn mvp(sub: &str, now: i64) -> Self {
        GrantSpec {
            alg: "EdDSA",
            kid: "eid1",
            typ: "ESG",
            v: 1,
            iss: "https://id.uhce.top",
            aud: "espectral-client",
            sub: sub.to_string(),
            flags: vec!["supporter".into(), "mvp".into()],
            iat: now,
            exp: now + 86_400,
        }
    }

    fn header(&self) -> String {
        format!(
            r#"{{"alg":"{}","kid":"{}","typ":"{}"}}"#,
            self.alg, self.kid, self.typ
        )
    }

    fn payload(&self) -> String {
        let flags = self
            .flags
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"v":{},"iss":"{}","aud":"{}","sub":"{}","flags":[{flags}],"iat":{},"exp":{}}}"#,
            self.v, self.iss, self.aud, self.sub, self.iat, self.exp
        )
    }

    /// Signs with `key` (the test key by default via [`mint`]).
    pub fn sign_with(&self, key: &SigningKey) -> String {
        let h = b64url(self.header().as_bytes());
        let p = b64url(self.payload().as_bytes());
        let msg = format!("{h}.{p}");
        let sig = key.sign(msg.as_bytes());
        format!("{msg}.{}", b64url(&sig.to_bytes()))
    }
}

pub fn mint(spec: &GrantSpec) -> String {
    spec.sign_with(&signing_key())
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// A 32-hex subject that is unlikely to collide between tests.
pub fn sub(seed: u8) -> String {
    format!("{:02x}{}", seed, "ab".repeat(15))
}
