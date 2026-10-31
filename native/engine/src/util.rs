//! Small helpers shared across clusters (ported from routes/misc.mjs and
//! assorted JS one-liners).

use std::path::{Component, Path, PathBuf};

use base64::Engine as _;

use crate::error::{EngineError, EngineResult};

/// PNG signature.
pub const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/// Decoded cap for icon / avatar / skin uploads (routes/misc.mjs MAX_IMAGE_BYTES).
pub const MAX_IMAGE_BYTES: usize = 500 * 1024;

/// Node `Buffer.from(s, 'base64')`: lenient — accepts base64url, ignores
/// characters outside the alphabet, padding optional, trailing partial
/// sextets dropped. Never fails.
pub fn lenient_base64_decode(input: &str) -> Vec<u8> {
    let mut clean: String = input
        .chars()
        .filter_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => Some(c),
            '-' => Some('+'),
            '_' => Some('/'),
            _ => None,
        })
        .collect();
    // Node stops at the first '=': everything after padding is ignored.
    if let Some(i) = input.find('=') {
        let before: usize =
            input[..i].chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '-' | '_')).count();
        clean.truncate(before);
    }
    if clean.len() % 4 == 1 {
        clean.pop();
    }
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    engine.decode(clean.as_bytes()).unwrap_or_default()
}

/// Standard base64 (with padding) of bytes.
pub fn base64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// routes/misc.mjs `decodePngImage`: a `data:image/png;base64,…` URL or raw
/// base64 PNG, <= 500 KB decoded, PNG magic required. 400 BAD_IMAGE otherwise.
pub fn decode_png_image(image_base64: Option<&str>) -> EngineResult<Vec<u8>> {
    let raw = match image_base64 {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return Err(EngineError::bad_request("BAD_IMAGE", "image_base64 is required")),
    };
    let b64 = if raw.starts_with("data:") {
        match raw.strip_prefix("data:image/png;base64,") {
            Some(rest) => rest,
            None => {
                return Err(EngineError::bad_request("BAD_IMAGE", "image must be a PNG data URL or raw base64 PNG"))
            }
        }
    } else {
        raw
    };
    let buf = lenient_base64_decode(b64);
    if buf.len() > MAX_IMAGE_BYTES {
        return Err(EngineError::bad_request("BAD_IMAGE", format!("image exceeds {MAX_IMAGE_BYTES} bytes decoded")));
    }
    if buf.len() < 8 || buf[..8] != PNG_MAGIC {
        return Err(EngineError::bad_request("BAD_IMAGE", "image must be a PNG"));
    }
    Ok(buf)
}

/// Lexically normalize a path (resolve `.` / `..` without touching the fs),
/// like Node `path.resolve` on an absolute path.
pub fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `target === root` or nested under it (case-insensitive on Windows).
/// Both paths should already be normalized absolute paths.
pub fn is_within_dir(target: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let t = target.to_string_lossy().to_lowercase().replace('/', "\\");
        let r = root.to_string_lossy().to_lowercase().replace('/', "\\");
        let r = r.trim_end_matches('\\');
        t == r || t.starts_with(&format!("{r}\\"))
    } else {
        target == root || target.starts_with(root)
    }
}

/// Instance names (instances.mjs NAME_RE `^[A-Za-z0-9 ._-]{1,40}$`).
pub fn is_valid_instance_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=40).contains(&n) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
}

/// Trash dirs left by deleteInstance: `<name>.deleted.<ms>[-<n>]`.
pub fn is_trash_dir_name(name: &str) -> bool {
    let Some(idx) = name.rfind(".deleted.") else { return false };
    let tail = &name[idx + ".deleted.".len()..];
    let (ms, n) = match tail.split_once('-') {
        Some((a, b)) => (a, Some(b)),
        None => (tail, None),
    };
    !ms.is_empty()
        && ms.chars().all(|c| c.is_ascii_digit())
        && n.map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY_PNG_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    #[test]
    fn decode_png_image_parity() {
        let tiny = lenient_base64_decode(TINY_PNG_B64);
        assert_eq!(tiny.len(), 70); // node: Buffer.from(TINY_PNG_B64, 'base64').length
        assert_eq!(decode_png_image(Some(&format!("data:image/png;base64,{TINY_PNG_B64}"))).unwrap(), tiny);
        assert_eq!(decode_png_image(Some(TINY_PNG_B64)).unwrap(), tiny);
        let code = |r: EngineResult<Vec<u8>>| {
            let e = r.unwrap_err().envelope();
            (e.status, e.code)
        };
        assert_eq!(code(decode_png_image(Some("not-base64!!!"))), (400, "BAD_IMAGE".into()));
        assert_eq!(
            code(decode_png_image(Some(&format!("data:image/jpeg;base64,{TINY_PNG_B64}")))),
            (400, "BAD_IMAGE".into())
        );
        assert_eq!(code(decode_png_image(Some(""))), (400, "BAD_IMAGE".into()));
        assert_eq!(code(decode_png_image(None)), (400, "BAD_IMAGE".into()));
        let mut big = PNG_MAGIC.to_vec();
        big.resize(500 * 1024 + 1, 0);
        assert_eq!(code(decode_png_image(Some(&base64_encode(&big)))), (400, "BAD_IMAGE".into()));
        let mut edge = PNG_MAGIC.to_vec();
        edge.resize(500 * 1024, 0);
        assert_eq!(decode_png_image(Some(&base64_encode(&edge))).unwrap().len(), 500 * 1024);
    }

    #[test]
    fn lenient_base64_matches_node() {
        assert_eq!(lenient_base64_decode("aGVsbG8"), b"hello");
        assert_eq!(lenient_base64_decode("aGVs bG8="), b"hello");
        assert_eq!(lenient_base64_decode("aGVsbG8=garbage"), b"hello");
        assert_eq!(lenient_base64_decode("_-8"), vec![0xff, 0xef]);
        // values produced by node's Buffer.from(…, 'base64')
        assert_eq!(hex::encode(lenient_base64_decode("not-base64!!!")), "9e8b7e6dab1eeb");
        assert_eq!(lenient_base64_decode("a").len(), 0);
        assert_eq!(hex::encode(lenient_base64_decode("ab=c")), "69");
    }

    #[test]
    fn names_and_trash() {
        assert!(is_valid_instance_name("Espectral default"));
        assert!(is_valid_instance_name("a.b_c-1"));
        assert!(!is_valid_instance_name(""));
        assert!(!is_valid_instance_name("a/b"));
        assert!(!is_valid_instance_name(&"x".repeat(41)));
        assert!(is_trash_dir_name("x.deleted.1730000000000"));
        assert!(is_trash_dir_name("x.deleted.1730000000000-2"));
        assert!(!is_trash_dir_name("x.deleted."));
        assert!(!is_trash_dir_name("x.deleted.12a"));
        assert!(!is_trash_dir_name("x"));
    }

    #[test]
    fn within_dir() {
        let root = Path::new("/data");
        assert!(is_within_dir(Path::new("/data"), root));
        assert!(is_within_dir(Path::new("/data/instances/x"), root));
        assert!(!is_within_dir(Path::new("/data-other"), root));
        assert_eq!(normalize_path(Path::new("/data/instances/../../etc")), PathBuf::from("/etc"));
    }
}
