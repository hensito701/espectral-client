//! Port of `src/engine/error.mjs` — the engine error type.
//!
//! The JS engine threw `ApiError(status, code, message)` and the HTTP layer
//! normalized every failure into the envelope
//! `{ error: { code: 'UPPER_SNAKE', message } }` with the status on the
//! response line. The native engine has no HTTP layer, but the UI still needs
//! the same `(status, code, message)` triple (i18n keys on `code`, retry logic
//! on `status`), so every [`EngineError`] can be normalized with
//! [`EngineError::envelope`] using exactly the JS `normalizeError` rules:
//!
//! - `code` that is UPPER_SNAKE (`^[A-Z][A-Z0-9_]*$`) passes through;
//! - otherwise status >= 500 collapses to `INTERNAL_ERROR`;
//! - otherwise the raw code, or `ERROR` when there is none.
//!
//! A few JS routes (routes/mods.mjs `sendError`) bypassed `normalizeError`
//! and sent their lowercase codes verbatim (`mods_list_failed`,
//! `mod_conflicts`, …). Use [`EngineError::api_verbatim`] for those so the
//! envelope keeps the exact code.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Result alias used by every engine function.
pub type EngineResult<T> = Result<T, EngineError>;

/// The canonical error envelope the UI consumes (`{ status, code, message }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub status: u16,
    pub code: String,
    pub message: String,
}

/// Failure of an HTTP request made by the engine (download.rs, fetch_text).
/// Mirrors the JS `Error` objects decorated with `status` / `code` / `url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpErrorKind {
    /// Connect / DNS / TLS / reset — no HTTP status (JS: `status` undefined).
    Network,
    /// Request (headers) or body stall timeout.
    Timeout,
    /// Non-2xx answer.
    Status(u16),
    /// Downloaded bytes did not match the expected SHA-1 (JS code `sha1_mismatch`, status 502).
    Sha1Mismatch,
    /// Downloaded bytes did not match the expected size.
    SizeMismatch,
    /// The body stream broke mid-transfer and retries were exhausted.
    Interrupted,
    /// Response body was not valid JSON (fetch_json).
    InvalidJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError {
    pub kind: HttpErrorKind,
    pub url: String,
    pub message: String,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl HttpError {
    /// HTTP status the JS error carried (`err.status`), if any.
    pub fn status(&self) -> Option<u16> {
        match self.kind {
            HttpErrorKind::Status(s) => Some(s),
            HttpErrorKind::Sha1Mismatch => Some(502),
            _ => None,
        }
    }

    /// JS `isOutage`: network failure or 5xx — never a 4xx (a real answer)
    /// nor a sha1 mismatch. Outages fall through to a host mirror.
    pub fn is_outage(&self) -> bool {
        match self.kind {
            HttpErrorKind::Sha1Mismatch => false,
            HttpErrorKind::Status(s) => s >= 500,
            HttpErrorKind::Network | HttpErrorKind::Timeout | HttpErrorKind::Interrupted => true,
            HttpErrorKind::SizeMismatch | HttpErrorKind::InvalidJson => true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// `httpError(status, code, message)` from the JS engine.
    #[error("{message}")]
    Api {
        status: u16,
        code: String,
        message: String,
        /// true = envelope keeps `code` verbatim (routes/mods.mjs sendError).
        verbatim: bool,
    },

    /// A facade method or contract function that has not been ported yet.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error(transparent)]
    Http(#[from] HttpError),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Json(#[from] serde_json::Error),

    /// Plain `throw new Error(message)` — no status, no code (=> 500 INTERNAL_ERROR).
    #[error("{0}")]
    Other(String),
}

impl std::error::Error for HttpError {}

fn is_upper_snake(code: &str) -> bool {
    let mut chars = code.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

impl EngineError {
    /// `httpError(status, code, message)`.
    pub fn api(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        EngineError::Api { status, code: code.into(), message: message.into(), verbatim: false }
    }

    /// Like [`EngineError::api`] but the envelope keeps a non-UPPER_SNAKE code
    /// verbatim (routes/mods.mjs `sendError` semantics).
    pub fn api_verbatim(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        EngineError::Api { status, code: code.into(), message: message.into(), verbatim: true }
    }

    pub fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::api(400, code, message)
    }

    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::api(404, code, message)
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::api(409, code, message)
    }

    pub fn other(message: impl Into<String>) -> Self {
        EngineError::Other(message.into())
    }

    /// HTTP status the JS engine would have answered with.
    pub fn status(&self) -> u16 {
        match self {
            EngineError::Api { status, .. } => *status,
            EngineError::NotImplemented(_) => 501,
            EngineError::Http(h) => h.status().unwrap_or(500),
            _ => 500,
        }
    }

    /// Raw code carried by the error (JS `err.code`), before normalization.
    pub fn raw_code(&self) -> Option<&str> {
        match self {
            EngineError::Api { code, .. } => Some(code.as_str()),
            EngineError::NotImplemented(_) => Some("NOT_IMPLEMENTED"),
            EngineError::Http(h) if h.kind == HttpErrorKind::Sha1Mismatch => Some("sha1_mismatch"),
            _ => None,
        }
    }

    /// True when `raw_code()` equals `code` (JS `err?.code === '...'` checks).
    pub fn has_code(&self, code: &str) -> bool {
        self.raw_code() == Some(code)
    }

    /// JS `normalizeError(err)` -> `{ status, code, message }`.
    pub fn envelope(&self) -> ErrorEnvelope {
        let status = self.status();
        let raw = self.raw_code();
        let verbatim = matches!(self, EngineError::Api { verbatim: true, .. });
        let code = match raw {
            Some(c) if verbatim => c.to_string(),
            Some(c) if is_upper_snake(c) => c.to_string(),
            _ if status >= 500 => "INTERNAL_ERROR".to_string(),
            Some(c) => c.to_string(),
            None => "ERROR".to_string(),
        };
        let mut message = self.to_string();
        if message.is_empty() {
            message = "Internal server error".to_string();
        }
        ErrorEnvelope { status, code, message }
    }
}

impl From<ErrorEnvelope> for EngineError {
    fn from(e: ErrorEnvelope) -> Self {
        EngineError::api(e.status, e.code, e.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upper_snake_codes_pass_through() {
        let e = EngineError::api(404, "NOT_FOUND", "nope");
        assert_eq!(e.envelope(), ErrorEnvelope { status: 404, code: "NOT_FOUND".into(), message: "nope".into() });
    }

    #[test]
    fn lowercase_5xx_codes_collapse_to_internal_error() {
        let e = EngineError::api(502, "sha1_mismatch", "bad");
        assert_eq!(e.envelope().code, "INTERNAL_ERROR");
        assert_eq!(e.envelope().status, 502);
    }

    #[test]
    fn lowercase_4xx_codes_are_kept() {
        let e = EngineError::api(404, "version_not_found", "x");
        assert_eq!(e.envelope().code, "version_not_found");
    }

    #[test]
    fn plain_errors_are_500_internal() {
        let e = EngineError::other("boom");
        let env = e.envelope();
        assert_eq!((env.status, env.code.as_str(), env.message.as_str()), (500, "INTERNAL_ERROR", "boom"));
        let e = EngineError::other("");
        assert_eq!(e.envelope().message, "Internal server error");
    }

    #[test]
    fn verbatim_codes_survive() {
        let e = EngineError::api_verbatim(500, "mods_list_failed", "x");
        assert_eq!(e.envelope().code, "mods_list_failed");
    }

    #[test]
    fn not_implemented_is_501() {
        let env = EngineError::NotImplemented("x").envelope();
        assert_eq!((env.status, env.code.as_str()), (501, "NOT_IMPLEMENTED"));
    }

    #[test]
    fn http_errors_map_status_and_outage() {
        let h = HttpError { kind: HttpErrorKind::Status(404), url: "u".into(), message: "m".into() };
        assert!(!h.is_outage());
        assert_eq!(EngineError::from(h).envelope().code, "ERROR");
        let h = HttpError { kind: HttpErrorKind::Sha1Mismatch, url: "u".into(), message: "m".into() };
        assert!(!h.is_outage());
        let e = EngineError::from(h);
        assert!(e.has_code("sha1_mismatch"));
        assert_eq!(e.status(), 502);
        let h = HttpError { kind: HttpErrorKind::Network, url: "u".into(), message: "m".into() };
        assert!(h.is_outage());
        assert_eq!(EngineError::from(h).status(), 500);
    }

    #[test]
    fn upper_snake_regex_parity() {
        assert!(is_upper_snake("A"));
        assert!(is_upper_snake("BAD_JSON2"));
        assert!(!is_upper_snake("_A"));
        assert!(!is_upper_snake("1A"));
        assert!(!is_upper_snake("Abc"));
        assert!(!is_upper_snake(""));
    }
}
