//! Port of `src/engine/ziputil.mjs` — minimal ZIP reader for NeoForge
//! installer jars and mrpack archives.
//!
//! Pure central-directory parsing (EOCD -> central directory -> local header
//! -> payload). Method 8 (deflate) is inflated with flate2, method 0 (stored)
//! is copied verbatim. Sizes come from the central directory, so data
//! descriptors never matter. Error messages match the JS module
//! (`zip corrupt: …`, `zip unsupported: …`).
//!
//! For bulk extraction (JDK archives, natives) the pinned `zip` crate is also
//! available; this module exists for byte-for-byte behavioral parity where
//! the JS used it.

use std::io::Read;
use std::path::Path;

const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_CENTRAL: u32 = 0x0201_4b50;
const SIG_LOCAL: u32 = 0x0403_4b50;

#[derive(Debug, thiserror::Error)]
pub enum ZipError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("zip corrupt: {0}")]
    Corrupt(String),
    #[error("zip unsupported: {0}")]
    Unsupported(String),
}

impl From<ZipError> for crate::EngineError {
    fn from(e: ZipError) -> Self {
        match e {
            ZipError::Io(io) => crate::EngineError::Io(io),
            other => crate::EngineError::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    method: u16,
    comp_size: u32,
    local_offset: u32,
}

fn u16le(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32le(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn find_eocd(buf: &[u8]) -> Result<(usize, usize, usize), ZipError> {
    // The EOCD lives in the last 65557 bytes (22-byte record + 64KB comment).
    let not_found = || ZipError::Corrupt("end-of-central-directory record not found".into());
    if buf.len() < 22 {
        return Err(not_found());
    }
    let window_start = buf.len().saturating_sub(65557);
    let mut i = buf.len() - 22;
    loop {
        if u32le(buf, i) == Some(SIG_EOCD) {
            let total = u16le(buf, i + 10).ok_or_else(not_found)? as usize;
            let cd_size = u32le(buf, i + 12).ok_or_else(not_found)? as usize;
            let cd_offset = u32le(buf, i + 16).ok_or_else(not_found)? as usize;
            return Ok((total, cd_offset, cd_size));
        }
        if i == window_start || i == 0 {
            return Err(not_found());
        }
        i -= 1;
    }
}

fn scan_central_directory(buf: &[u8]) -> Result<Vec<Entry>, ZipError> {
    let (total, cd_offset, cd_size) = find_eocd(buf)?;
    let mut off = cd_offset;
    let end = cd_offset + cd_size;
    let mut entries = Vec::with_capacity(total);
    for _ in 0..total {
        if off + 46 > end || u32le(buf, off) != Some(SIG_CENTRAL) {
            return Err(ZipError::Corrupt(format!("bad central directory header at offset {off}")));
        }
        let bad = || ZipError::Corrupt(format!("bad central directory header at offset {off}"));
        let method = u16le(buf, off + 10).ok_or_else(bad)?;
        let comp_size = u32le(buf, off + 20).ok_or_else(bad)?;
        let name_len = u16le(buf, off + 28).ok_or_else(bad)? as usize;
        let extra_len = u16le(buf, off + 30).ok_or_else(bad)? as usize;
        let comment_len = u16le(buf, off + 32).ok_or_else(bad)? as usize;
        let local_offset = u32le(buf, off + 42).ok_or_else(bad)?;
        let name_bytes = buf.get(off + 46..off + 46 + name_len).ok_or_else(bad)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        entries.push(Entry { name, method, comp_size, local_offset });
        off += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

fn entry_payload<'a>(buf: &'a [u8], e: &Entry) -> Result<&'a [u8], ZipError> {
    let off = e.local_offset as usize;
    if off + 30 > buf.len() || u32le(buf, off) != Some(SIG_LOCAL) {
        return Err(ZipError::Corrupt(format!("bad local header for '{}'", e.name)));
    }
    let name_len = u16le(buf, off + 26).unwrap_or(0) as usize;
    let extra_len = u16le(buf, off + 28).unwrap_or(0) as usize;
    let start = off + 30 + name_len + extra_len;
    let end = start + e.comp_size as usize;
    if end > buf.len() {
        return Err(ZipError::Corrupt(format!("truncated data for '{}'", e.name)));
    }
    Ok(&buf[start..end])
}

fn decompress(e: &Entry, data: &[u8]) -> Result<Vec<u8>, ZipError> {
    match e.method {
        0 => Ok(data.to_vec()),
        8 => {
            let mut out = Vec::new();
            flate2::read::DeflateDecoder::new(data)
                .read_to_end(&mut out)
                .map_err(|err| ZipError::Corrupt(format!("bad deflate stream for '{}': {err}", e.name)))?;
            Ok(out)
        }
        m => Err(ZipError::Unsupported(format!("compression method {m} for '{}'", e.name))),
    }
}

/// Read one entry as bytes; `Ok(None)` when the entry does not exist.
pub fn read_zip_entry(zip_path: &Path, entry_name: &str) -> Result<Option<Vec<u8>>, ZipError> {
    let buf = std::fs::read(zip_path)?;
    read_zip_entry_from(&buf, entry_name)
}

/// [`read_zip_entry`] over an in-memory archive.
pub fn read_zip_entry_from(buf: &[u8], entry_name: &str) -> Result<Option<Vec<u8>>, ZipError> {
    let entries = scan_central_directory(buf)?;
    match entries.iter().find(|e| e.name == entry_name) {
        None => Ok(None),
        Some(e) => decompress(e, entry_payload(buf, e)?).map(Some),
    }
}

/// Every entry name (files and directories), in central-directory order.
pub fn list_zip_entries(zip_path: &Path) -> Result<Vec<String>, ZipError> {
    let buf = std::fs::read(zip_path)?;
    Ok(scan_central_directory(&buf)?.into_iter().map(|e| e.name).collect())
}

/// Whether the archive contains `entry_name`.
pub fn zip_has_entry(zip_path: &Path, entry_name: &str) -> Result<bool, ZipError> {
    let buf = std::fs::read(zip_path)?;
    Ok(scan_central_directory(&buf)?.iter().any(|e| e.name == entry_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;

    #[test]
    fn extracts_version_json_from_the_fake_installer() {
        let buf = read_zip_entry(&fixture("fake-neoforge-installer.jar"), "version.json").unwrap().unwrap();
        assert!(buf.len() > 1000);
        assert_eq!(buf[0], b'{');
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["id"], "neoforge-21.1.242");
        assert_eq!(v["inheritsFrom"], "1.21.1");
        assert!(v["libraries"].is_array());
    }

    #[test]
    fn missing_entry_is_none() {
        let f = fixture("fake-neoforge-installer.jar");
        assert!(read_zip_entry(&f, "nope.json").unwrap().is_none());
        assert!(read_zip_entry(&f, "").unwrap().is_none());
    }

    #[test]
    fn reads_mrpack_index_and_overrides() {
        let f = fixture("mrpack-mini.mrpack");
        let idx = read_zip_entry(&f, "modrinth.index.json").unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&idx).unwrap();
        assert_eq!(v["name"], "Pack de Prueba");
        let toml = read_zip_entry(&f, "overrides/config/prueba.toml").unwrap().unwrap();
        assert!(String::from_utf8_lossy(&toml).contains("ok = true"));
    }

    #[test]
    fn has_entry_and_list() {
        let f = fixture("mrpack-mini.mrpack");
        assert!(zip_has_entry(&f, "modrinth.index.json").unwrap());
        assert!(!zip_has_entry(&f, "missing.bin").unwrap());
        assert!(zip_has_entry(&f, "overrides/config/prueba.toml").unwrap());
        let entries = list_zip_entries(&f).unwrap();
        assert!(entries.iter().any(|e| e == "modrinth.index.json"));
        assert!(entries.len() >= 4, "{}", entries.len());
    }

    #[test]
    fn corrupt_and_missing_files_error() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("not-a-zip.bin");
        std::fs::write(&fake, [0u8, 1, 2, 3, 4]).unwrap();
        let err = read_zip_entry(&fake, "x").unwrap_err();
        assert!(err.to_string().contains("zip corrupt"), "{err}");
        let err = list_zip_entries(&dir.path().join("no-such.zip")).unwrap_err();
        assert!(matches!(err, ZipError::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound));
    }
}
