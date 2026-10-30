//! Port of `src/engine/skins.mjs` — skin texture cache (`<data>/skins`), skin
//! gallery (`<data>/skin-library` + `skin-library.json`), vanilla launcher
//! skin import, skin-head avatar derivation (PNG decode/encode with flate2).
//!
//! **Owner: accounts-skins.** Status: PORTED.
//!
//! Library layout (global per installation, like the vanilla launcher):
//!   `<data>/skin-library.json`     — index `[{ id, name, variant, created_at, source }]`
//!   `<data>/skin-library/<id>.png` — art bytes
//! Mojang texture cache (per account, TTL 1h): `<data>/skins/<uuid>.png`.
//!
//! Tests: tests/skins.test.mjs, skin-avatar.test.mjs, skin.test.mjs.
//!
//! The 3D skin viewer stays in the UI (three.js in a small webview or a
//! native renderer — UI phase decision); the engine only serves PNG bytes.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::types::{Account, SkinLibraryEntry, SkinVariant, TokenKind};
use crate::util::PNG_MAGIC;

pub const SKIN_VARIANTS: [&str; 2] = ["classic", "slim"];
pub const SKIN_CACHE_TTL_MS: f64 = 60.0 * 60.0 * 1000.0;
pub const MAX_NAME_LEN: usize = 40;

pub fn skins_dir(engine: &Engine) -> PathBuf {
    engine.paths().skins_dir()
}

pub fn library_dir(engine: &Engine) -> PathBuf {
    engine.paths().skin_library_dir()
}

pub fn library_index_path(engine: &Engine) -> PathBuf {
    engine.paths().skin_library_index()
}

fn skin_cache_path(engine: &Engine, uuid: &str) -> PathBuf {
    skins_dir(engine).join(format!("{uuid}.png"))
}

fn library_png_path(engine: &Engine, id: &str) -> PathBuf {
    library_dir(engine).join(format!("{id}.png"))
}

/// PNG IHDR dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngDims {
    pub width: u32,
    pub height: u32,
}

/// Parse PNG IHDR dimensions (bytes 16-19 / 20-23, big-endian). None when
/// too short, not a PNG, the first chunk is not IHDR, or a side is 0.
pub fn skin_png_dimensions(buf: &[u8]) -> Option<PngDims> {
    if buf.len() < 24 || buf[..8] != PNG_MAGIC || &buf[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(buf[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(buf[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some(PngDims { width, height })
}

/// True for buffers Mojang accepts as Java skins: PNG 64x64 (64x32 legacy).
pub fn is_valid_skin_png(buf: &[u8]) -> bool {
    matches!(skin_png_dimensions(buf), Some(PngDims { width: 64, height: 64 | 32 }))
}

/// `classic` | `slim` -> variant; anything else 400 BAD_SKIN_VARIANT.
pub fn assert_skin_variant(variant: Option<&str>) -> EngineResult<SkinVariant> {
    match variant {
        Some("classic") => Ok(SkinVariant::Classic),
        Some("slim") => Ok(SkinVariant::Slim),
        _ => Err(EngineError::bad_request("BAD_SKIN_VARIANT", "variant must be classic|slim")),
    }
}

pub fn variant_str(v: SkinVariant) -> &'static str {
    match v {
        SkinVariant::Classic => "classic",
        SkinVariant::Slim => "slim",
    }
}

/// Decode a `data:image/png;base64,…` URL (or raw base64) into PNG bytes.
/// Anything else is a 400 BAD_IMAGE.
pub fn decode_skin_png(image_base64: Option<&str>) -> EngineResult<Vec<u8>> {
    let bad = || EngineError::bad_request("BAD_IMAGE", "image must be a PNG");
    let Some(s) = image_base64.filter(|s| !s.is_empty()) else {
        return Err(EngineError::bad_request("BAD_IMAGE", "image_base64 is required"));
    };
    let b64 = match s.find(',') {
        Some(comma) => {
            if !s[..comma].eq_ignore_ascii_case("data:image/png;base64") {
                return Err(bad());
            }
            &s[comma + 1..]
        }
        None => s,
    };
    let buf = crate::util::lenient_base64_decode(b64);
    if buf.len() < 8 || buf[..8] != PNG_MAGIC {
        return Err(bad());
    }
    Ok(buf)
}

fn assert_skin_name(name: Option<&str>) -> EngineResult<String> {
    let clean = name.map(str::trim).unwrap_or("");
    if clean.is_empty() || clean.encode_utf16().count() > MAX_NAME_LEN {
        return Err(EngineError::bad_request("BAD_SKIN_NAME", format!("name must be 1-{MAX_NAME_LEN} characters")));
    }
    Ok(clean.to_string())
}

// ---------------------------------------------------------------------------
// Mojang texture cache (<dataDir>/skins/<uuid>.png, TTL 1h)
// ---------------------------------------------------------------------------

/// Fresh (<1h by mtime) cached skin bytes, or None. Never fails.
pub fn read_skin_cache(engine: &Engine, uuid: &str) -> Option<Vec<u8>> {
    let p = skin_cache_path(engine, uuid);
    let meta = std::fs::metadata(&p).ok()?;
    if crate::platform::now_ms() as f64 - crate::platform::mtime_ms(&meta) > SKIN_CACHE_TTL_MS {
        return None;
    }
    std::fs::read(p).ok()
}

/// Store skin bytes in the cache. Best-effort.
pub fn write_skin_cache(engine: &Engine, uuid: &str, png: &[u8]) {
    let _ = std::fs::create_dir_all(skins_dir(engine));
    let _ = std::fs::write(skin_cache_path(engine, uuid), png);
}

/// Drop a cached skin (missing file is a no-op).
pub fn clear_skin_cache(engine: &Engine, uuid: &str) {
    let _ = std::fs::remove_file(skin_cache_path(engine, uuid));
}

/// Resolve the MSA account + live Minecraft token for a skin endpoint.
/// 404 UNKNOWN_ACCOUNT / 404 OFFLINE_ACCOUNT / 401 MSA_REFRESH_FAILED.
pub async fn resolve_skin_account(engine: &Engine, username: &str) -> EngineResult<(Account, String)> {
    let account = crate::accounts::get_account(engine, username)
        .ok_or_else(|| EngineError::not_found("UNKNOWN_ACCOUNT", format!("account '{username}' does not exist")))?;
    if account.token_kind != TokenKind::Msa {
        return Err(EngineError::not_found(
            "OFFLINE_ACCOUNT",
            format!("account '{username}' is offline — skins need a Microsoft sign-in"),
        ));
    }
    match crate::msauth::ensure_minecraft_token(engine, &account).await? {
        Some(t) if !t.access_token.is_empty() => Ok((account, t.access_token)),
        _ => Err(EngineError::api(401, "MSA_REFRESH_FAILED", "session expired — sign in with Microsoft again")),
    }
}

// ---------------------------------------------------------------------------
// Library index (<dataDir>/skin-library.json)
// ---------------------------------------------------------------------------

/// One stored index entry. Unknown keys survive rewrites.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredSkin {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub variant: Value,
    #[serde(default)]
    pub created_at: Value,
    #[serde(default)]
    pub source: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl StoredSkin {
    pub fn variant(&self) -> SkinVariant {
        if self.variant.as_str() == Some("slim") {
            SkinVariant::Slim
        } else {
            SkinVariant::Classic
        }
    }
}

/// Public shape `{ id, name, variant, created_at, source }`.
pub fn public_skin_entry(e: &StoredSkin) -> SkinLibraryEntry {
    let s = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    SkinLibraryEntry {
        id: e.id.clone(),
        name: e.name.clone(),
        variant: e.variant(),
        created_at: s(&e.created_at),
        source: s(&e.source),
    }
}

/// Read the index; missing/corrupt file -> []. Entries without string id+name are dropped.
pub fn load_library_index(engine: &Engine) -> Vec<StoredSkin> {
    let Ok(raw) = std::fs::read_to_string(library_index_path(engine)) else { return vec![] };
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Array(a)) => a
            .into_iter()
            .filter(|e| {
                e.get("id").map(Value::is_string).unwrap_or(false)
                    && e.get("name").map(Value::is_string).unwrap_or(false)
            })
            .filter_map(|e| serde_json::from_value(e).ok())
            .collect(),
        _ => vec![],
    }
}

fn save_library_index(engine: &Engine, entries: &[StoredSkin]) -> EngineResult<()> {
    crate::paths::write_json_atomic(&library_index_path(engine), &entries)?;
    Ok(())
}

fn base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = vec![];
    while n > 0 {
        out.push(D[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// `skin_<ms base36>_<6 hex>`
fn new_skin_id() -> String {
    let mut b = [0u8; 3];
    rand::thread_rng().fill_bytes(&mut b);
    format!("skin_{}_{}", base36(crate::platform::now_ms().max(0) as u64), hex::encode(b))
}

/// Save PNG bytes as a named library entry. Validates name, variant, dims.
pub fn save_library_skin(
    engine: &Engine,
    name: Option<&str>,
    variant: Option<SkinVariant>,
    png: &[u8],
    source: &str,
    created_at: Option<String>,
) -> EngineResult<SkinLibraryEntry> {
    let clean = assert_skin_name(name)?;
    let variant =
        variant.ok_or_else(|| EngineError::bad_request("BAD_SKIN_VARIANT", "variant must be classic|slim"))?;
    if !is_valid_skin_png(png) {
        return Err(EngineError::bad_request("BAD_SKIN_DIMS", "skin must be a 64x64 PNG (64x32 legacy accepted)"));
    }
    let entry = StoredSkin {
        id: new_skin_id(),
        name: clean,
        variant: Value::from(variant_str(variant)),
        created_at: Value::from(created_at.unwrap_or_else(crate::platform::now_iso)),
        source: Value::from(source),
        extra: Map::new(),
    };
    std::fs::create_dir_all(library_dir(engine))?;
    std::fs::write(library_png_path(engine, &entry.id), png)?;
    let mut index = load_library_index(engine);
    index.push(entry.clone());
    save_library_index(engine, &index)?;
    Ok(public_skin_entry(&entry))
}

fn unknown_skin(id: &str) -> EngineError {
    EngineError::not_found("UNKNOWN_SKIN", format!("skin '{id}' does not exist"))
}

/// 404 UNKNOWN_SKIN when the id is not in the index.
pub fn get_library_entry(engine: &Engine, id: &str) -> EngineResult<StoredSkin> {
    load_library_index(engine).into_iter().find(|e| e.id == id).ok_or_else(|| unknown_skin(id))
}

/// Rename / re-variant an entry (each part optional). Returns the public entry.
pub fn update_library_skin(
    engine: &Engine,
    id: &str,
    name: Option<&str>,
    variant: Option<SkinVariant>,
) -> EngineResult<SkinLibraryEntry> {
    let mut index = load_library_index(engine);
    let found = index.iter_mut().find(|e| e.id == id).ok_or_else(|| unknown_skin(id))?;
    if let Some(n) = name {
        found.name = assert_skin_name(Some(n))?;
    }
    if let Some(v) = variant {
        found.variant = Value::from(variant_str(v));
    }
    let out = public_skin_entry(found);
    save_library_index(engine, &index)?;
    Ok(out)
}

/// Remove an entry + its PNG (missing file is a no-op).
pub fn delete_library_skin(engine: &Engine, id: &str) -> EngineResult<()> {
    let mut index = load_library_index(engine);
    let at = index.iter().position(|e| e.id == id).ok_or_else(|| unknown_skin(id))?;
    index.remove(at);
    save_library_index(engine, &index)?;
    let _ = std::fs::remove_file(library_png_path(engine, id));
    Ok(())
}

/// Raw PNG bytes for a library entry. 404 UNKNOWN_SKIN when absent.
pub fn read_library_png(engine: &Engine, id: &str) -> EngineResult<Vec<u8>> {
    get_library_entry(engine, id)?;
    std::fs::read(library_png_path(engine, id))
        .map_err(|_| EngineError::not_found("UNKNOWN_SKIN", format!("skin '{id}' has no image")))
}

// ---------------------------------------------------------------------------
// Vanilla launcher import (%APPDATA%/.minecraft/launcher_custom_skins.json)
// ---------------------------------------------------------------------------

/// `%APPDATA%` (Windows: `%USERPROFILE%\AppData\Roaming` fallback), else `$HOME`,
/// + `.minecraft/launcher_custom_skins.json`.
pub fn default_vanilla_skins_path() -> PathBuf {
    let roaming = std::env::var_os("APPDATA").filter(|s| !s.is_empty()).map(PathBuf::from).or_else(|| {
        cfg!(windows)
            .then(|| PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("AppData").join("Roaming"))
    });
    let home = roaming
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .or_else(crate::paths::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".minecraft").join("launcher_custom_skins.json")
}

/// `{ imported, skipped, total }`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VanillaImport {
    pub imported: u32,
    pub skipped: u32,
    pub total: u32,
}

/// Import named skins from the vanilla launcher's launcher_custom_skins.json.
/// Idempotent: names that already exist (case-insensitive) are skipped;
/// imports land as 'classic' (the vanilla file carries no model).
pub fn import_vanilla_skins(engine: &Engine, file: &Path) -> EngineResult<VanillaImport> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(EngineError::not_found("NO_VANILLA_SKINS", "vanilla launcher has no saved skins on this PC"))
        }
        Err(_) => {
            return Err(EngineError::bad_request("BAD_VANILLA_SKINS", "could not read the vanilla launcher skins file"))
        }
    };
    let doc: Value = serde_json::from_str(&text)
        .map_err(|_| EngineError::bad_request("BAD_VANILLA_SKINS", "could not read the vanilla launcher skins file"))?;
    let raw: Vec<Value> = match doc.get("customSkins") {
        Some(Value::Object(m)) => m.values().cloned().collect(),
        Some(Value::Array(a)) => a.clone(),
        _ => vec![],
    };
    let mut index = load_library_index(engine);
    let mut known: std::collections::HashSet<String> = index.iter().map(|e| e.name.to_lowercase()).collect();
    let (mut imported, mut skipped) = (0u32, 0u32);
    for s in &raw {
        let attempt = (|| -> EngineResult<bool> {
            let name = assert_skin_name(s.get("name").and_then(Value::as_str))?;
            if known.contains(&name.to_lowercase()) {
                return Ok(false);
            }
            let png = decode_skin_png(s.get("skinImage").and_then(Value::as_str))?;
            if !is_valid_skin_png(&png) {
                return Ok(false);
            }
            let created =
                s.get("created").and_then(Value::as_str).map(str::to_string).unwrap_or_else(crate::platform::now_iso);
            let entry = StoredSkin {
                id: new_skin_id(),
                name: name.clone(),
                variant: Value::from("classic"),
                created_at: Value::from(created),
                source: Value::from("vanilla"),
                extra: Map::new(),
            };
            std::fs::create_dir_all(library_dir(engine))?;
            std::fs::write(library_png_path(engine, &entry.id), &png)?;
            index.push(entry);
            known.insert(name.to_lowercase());
            Ok(true)
        })();
        match attempt {
            Ok(true) => imported += 1,
            _ => skipped += 1,
        }
    }
    if imported > 0 {
        save_library_index(engine, &index)?;
    }
    Ok(VanillaImport { imported, skipped, total: raw.len() as u32 })
}

// ---------------------------------------------------------------------------
// MSA avatar auto = skin head (64x64 PNG derived from the Mojang skin)
// ---------------------------------------------------------------------------

fn crc_table() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (n, slot) in t.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    })
}

/// CRC-32 (PNG / zlib polynomial).
pub fn crc32(buf: &[u8]) -> u32 {
    let t = crc_table();
    let mut c = 0xffff_ffffu32;
    for b in buf {
        c = t[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xffff_ffff
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[4..]);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a16, b16, c16) = (a as i16, b as i16, c as i16);
    let p = a16 + b16 - c16;
    let (pa, pb, pc) = ((p - a16).abs(), (p - b16).abs(), (p - c16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn bad_image(msg: &str) -> EngineError {
    EngineError::bad_request("BAD_IMAGE", msg)
}

/// Decode skin bytes to raw RGBA pixels: 64x64 / 64x32, 8-bit RGBA, non-interlaced.
fn decode_skin_rgba(png: &[u8]) -> EngineResult<(usize, usize, Vec<u8>)> {
    if !is_valid_skin_png(png) {
        return Err(bad_image("skin must be a 64x64 PNG (64x32 legacy accepted)"));
    }
    let rd = |p: usize| u32::from_be_bytes([png[p], png[p + 1], png[p + 2], png[p + 3]]) as usize;
    let mut pos = 8;
    let (mut seen_ihdr, mut width, mut height) = (false, 0usize, 0usize);
    let mut idat = Vec::new();
    while pos + 8 <= png.len() {
        let len = rd(pos);
        let kind = &png[pos + 4..pos + 8];
        let start = pos + 8;
        let end = start.checked_add(len).ok_or_else(|| bad_image("truncated skin PNG"))?;
        if end + 4 > png.len() {
            return Err(bad_image("truncated skin PNG"));
        }
        if !seen_ihdr {
            if kind != b"IHDR" || len != 13 {
                return Err(bad_image("skin PNG is missing its IHDR"));
            }
            width = rd(start);
            height = rd(start + 4);
            let (depth, color, comp, filter, interlace) =
                (png[start + 8], png[start + 9], png[start + 10], png[start + 11], png[start + 12]);
            if depth != 8 || color != 6 {
                return Err(bad_image("skin PNG must be 8-bit RGBA"));
            }
            if comp != 0 || filter != 0 {
                return Err(bad_image("unsupported skin PNG encoding"));
            }
            if interlace != 0 {
                return Err(bad_image("interlaced skin PNGs are not supported"));
            }
            seen_ihdr = true;
        } else if kind == b"IDAT" {
            idat.extend_from_slice(&png[start..end]);
        } else if kind == b"IEND" {
            break;
        }
        pos = end + 4;
    }
    if !seen_ihdr || idat.is_empty() {
        return Err(bad_image("skin PNG has no image data"));
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(idat.as_slice())
        .read_to_end(&mut raw)
        .map_err(|_| bad_image("could not decode skin PNG image data"))?;
    let stride = width * 4;
    if raw.len() != height * (stride + 1) {
        return Err(bad_image("corrupt skin PNG image data"));
    }
    let mut px = vec![0u8; width * height * 4];
    let mut prev = vec![0u8; stride];
    for y in 0..height {
        let f = raw[y * (stride + 1)];
        let row = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        let out = &mut px[y * stride..(y + 1) * stride];
        match f {
            0 => out.copy_from_slice(row),
            1..=4 => {
                for i in 0..stride {
                    let a = if i >= 4 { out[i - 4] } else { 0 };
                    let b = prev[i];
                    let c = if i >= 4 { prev[i - 4] } else { 0 };
                    let pred = match f {
                        1 => a,
                        2 => b,
                        3 => ((a as u16 + b as u16) >> 1) as u8,
                        _ => paeth(a, b, c),
                    };
                    out[i] = row[i].wrapping_add(pred);
                }
            }
            _ => return Err(bad_image("corrupt skin PNG filter")),
        }
        prev.copy_from_slice(out);
    }
    Ok((width, height, px))
}

/// JS `Math.round` for non-negative values.
fn js_round(x: f64) -> u8 {
    (x + 0.5).floor().clamp(0.0, 255.0) as u8
}

/// Extract the 64x64 avatar head PNG from skin bytes: face (8,8,8,8)
/// source-over the hat overlay (40,8,8,8), nearest-neighbor scaled 8x.
pub fn extract_head_png(skin: &[u8]) -> EngineResult<Vec<u8>> {
    let (width, _h, px) = decode_skin_rgba(skin)?;
    let at = |x: usize, y: usize| (y * width + x) * 4;
    let mut head = [0u8; 8 * 8 * 4];
    for hy in 0..8 {
        for hx in 0..8 {
            let f = at(8 + hx, 8 + hy);
            let h = at(40 + hx, 8 + hy);
            let fa = px[f + 3] as f64;
            let ha = px[h + 3] as f64;
            let base = fa * (255.0 - ha) / 255.0;
            let out_a = (ha + base + 0.5).floor();
            let o = (hy * 8 + hx) * 4;
            if out_a == 0.0 {
                head[o..o + 4].fill(0);
            } else {
                for k in 0..3 {
                    head[o + k] = js_round((px[h + k] as f64 * ha + px[f + k] as f64 * base) / out_a);
                }
                head[o + 3] = out_a as u8;
            }
        }
    }
    const S: usize = 64;
    let rs = S * 4;
    let mut raw = vec![0u8; S * (rs + 1)];
    for y in 0..S {
        raw[y * (rs + 1)] = 0; // filter None
        let hy = y >> 3;
        for x in 0..S {
            let src = (hy * 8 + (x >> 3)) * 4;
            let dst = y * (rs + 1) + 1 + x * 4;
            raw[dst..dst + 4].copy_from_slice(&head[src..src + 4]);
        }
    }
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&(S as u32).to_be_bytes());
    ihdr[4..8].copy_from_slice(&(S as u32).to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 6; // RGBA
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&raw)?;
    let deflated = enc.finish()?;
    let mut png = PNG_MAGIC.to_vec();
    png.extend(png_chunk(b"IHDR", &ihdr));
    png.extend(png_chunk(b"IDAT", &deflated));
    png.extend(png_chunk(b"IEND", &[]));
    Ok(png)
}

/// True when the avatar may be (re)derived from the skin: no avatar file yet,
/// or the current one is itself skin-derived (avatar_auto marker).
pub fn should_sync_avatar_from_skin(engine: &Engine, username: &str) -> bool {
    match crate::accounts::get_account(engine, username) {
        Some(a) => !crate::accounts::has_avatar(engine, &a) || a.avatar_auto == Some(true),
        None => false,
    }
}

/// Outcome of [`sync_account_avatar_from_skin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarSync {
    /// `{ ok: true, auto: true }`
    Synced,
    /// `{ skipped: true, reason: 'custom' }` — user-uploaded avatar kept.
    SkippedCustom,
}

/// Derive the avatar head from skin bytes and store it, marking `avatar_auto`.
/// 404 UNKNOWN_ACCOUNT when missing; 400 BAD_IMAGE on junk art.
pub fn sync_account_avatar_from_skin(engine: &Engine, username: &str, skin: &[u8]) -> EngineResult<AvatarSync> {
    let account = crate::accounts::get_account(engine, username)
        .ok_or_else(|| EngineError::not_found("UNKNOWN_ACCOUNT", format!("account '{username}' does not exist")))?;
    if crate::accounts::has_avatar(engine, &account) && account.avatar_auto != Some(true) {
        return Ok(AvatarSync::SkippedCustom);
    }
    let head = extract_head_png(skin)?;
    crate::accounts::write_account_avatar(engine, username, &head)?;
    crate::accounts::set_avatar_auto(engine, username, true)?;
    Ok(AvatarSync::Synced)
}

/// Best-effort "refresh the derived avatar unless custom" (route hooks).
pub fn sync_avatar_best_effort(engine: &Engine, username: &str, skin: &[u8]) {
    if should_sync_avatar_from_skin(engine, username) {
        if let Err(e) = sync_account_avatar_from_skin(engine, username, skin) {
            tracing::debug!("[skins] avatar sync skipped for '{username}': {e}");
        }
    }
}

/// Drop a skin-derived avatar (after a Mojang skin reset). Returns true when
/// one was removed (`{ ok: true }`), false when skipped. Never fails for a
/// missing account.
pub fn clear_account_auto_avatar(engine: &Engine, username: &str) -> EngineResult<bool> {
    match crate::accounts::get_account(engine, username) {
        Some(a) if a.avatar_auto == Some(true) => {
            crate::accounts::remove_account_avatar(engine, username)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::temp_engine;
    use serde_json::json;

    /// Minimal PNG: magic + IHDR with the given dims (no IDAT).
    pub(crate) fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0u8; 30];
        b[..8].copy_from_slice(&PNG_MAGIC);
        b[8..12].copy_from_slice(&13u32.to_be_bytes());
        b[12..16].copy_from_slice(b"IHDR");
        b[16..20].copy_from_slice(&w.to_be_bytes());
        b[20..24].copy_from_slice(&h.to_be_bytes());
        b[24] = 8;
        b[25] = 6;
        b
    }

    fn data_url(b: &[u8]) -> String {
        format!("data:image/png;base64,{}", crate::util::base64_encode(b))
    }

    // ---- independent PNG fixture builder (tests/skin-avatar.test.mjs) ----

    fn chunk(kind: &str, data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind.as_bytes());
        out.extend_from_slice(data);
        let mut h = flate2::Crc::new();
        h.update(&out[4..]);
        out.extend_from_slice(&h.sum().to_be_bytes());
        out
    }

    fn encode_png(w: u32, h: u32, rows: &[(u8, Vec<u8>)], color: u8, interlace: u8) -> Vec<u8> {
        let mut raw = vec![];
        for (f, d) in rows {
            raw.push(*f);
            raw.extend_from_slice(d);
        }
        let mut ihdr = vec![];
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, color, 0, 0, interlace]);
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&raw).unwrap();
        let mut png = PNG_MAGIC.to_vec();
        png.extend(chunk("IHDR", &ihdr));
        png.extend(chunk("IDAT", &enc.finish().unwrap()));
        png.extend(chunk("IEND", &[]));
        png
    }

    type Rgba = [u8; 4];

    fn skin_pixels(face: Rgba, hat: Rgba, height: usize) -> Vec<u8> {
        let base: Rgba = [0, 0, 255, 255];
        let mut px = vec![0u8; 64 * height * 4];
        let mut fill = |x0: usize, y0: usize, w: usize, h: usize, c: Rgba| {
            for y in y0..(y0 + h).min(height) {
                for x in x0..x0 + w {
                    let o = (y * 64 + x) * 4;
                    px[o..o + 4].copy_from_slice(&c);
                }
            }
        };
        fill(0, 0, 64, height, base);
        fill(8, 8, 8, 8, face);
        fill(40, 8, 8, 8, hat);
        px
    }

    fn filter_row(row: &[u8], prev: &[u8], f: u8) -> Vec<u8> {
        (0..row.len())
            .map(|i| {
                let a = if i >= 4 { row[i - 4] } else { 0 };
                let b = prev[i];
                let c = if i >= 4 { prev[i - 4] } else { 0 };
                let pred = match f {
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) >> 1) as u8,
                    _ => paeth(a, b, c),
                };
                row[i].wrapping_sub(pred)
            })
            .collect()
    }

    pub(crate) fn skin_png_filtered(face: Rgba, hat: Rgba, height: usize, filter_for: impl Fn(usize) -> u8) -> Vec<u8> {
        let px = skin_pixels(face, hat, height);
        let stride = 256;
        let mut prev = vec![0u8; stride];
        let mut rows = vec![];
        for y in 0..height {
            let row = &px[y * stride..(y + 1) * stride];
            let f = filter_for(y);
            rows.push((f, if f == 0 { row.to_vec() } else { filter_row(row, &prev, f) }));
            prev.copy_from_slice(row);
        }
        encode_png(64, height as u32, &rows, 6, 0)
    }

    pub(crate) fn skin_png(face: Rgba, hat: Rgba) -> Vec<u8> {
        skin_png_filtered(face, hat, 64, |_| 0)
    }

    /// Minimal decoder for assertions (filter-0 RGBA rows).
    fn decode_rgba(png: &[u8]) -> (usize, usize, Vec<u8>) {
        let mut pos = 8;
        let (mut w, mut h, mut parts) = (0, 0, vec![]);
        while pos + 8 <= png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = &png[pos + 4..pos + 8];
            let ds = pos + 8;
            if kind == b"IHDR" {
                w = u32::from_be_bytes(png[ds..ds + 4].try_into().unwrap()) as usize;
                h = u32::from_be_bytes(png[ds + 4..ds + 8].try_into().unwrap()) as usize;
            } else if kind == b"IDAT" {
                parts.extend_from_slice(&png[ds..ds + len]);
            } else if kind == b"IEND" {
                break;
            }
            pos = ds + len + 4;
        }
        let mut raw = vec![];
        flate2::read::ZlibDecoder::new(parts.as_slice()).read_to_end(&mut raw).unwrap();
        let stride = w * 4;
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            assert_eq!(raw[y * (stride + 1)], 0, "filter-0 avatar rows");
            px[y * stride..(y + 1) * stride].copy_from_slice(&raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)]);
        }
        (w, h, px)
    }

    fn pixel(d: &(usize, usize, Vec<u8>), x: usize, y: usize) -> Rgba {
        let o = (y * d.0 + x) * 4;
        [d.2[o], d.2[o + 1], d.2[o + 2], d.2[o + 3]]
    }

    const TINY_PNG_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    #[test]
    fn crc32_matches_zlib() {
        let mut h = flate2::Crc::new();
        h.update(b"IHDR hello");
        assert_eq!(crc32(b"IHDR hello"), h.sum());
    }

    #[test]
    fn png_dimension_vectors() {
        assert_eq!(skin_png_dimensions(&png_header(64, 64)), Some(PngDims { width: 64, height: 64 }));
        assert_eq!(skin_png_dimensions(&png_header(64, 32)), Some(PngDims { width: 64, height: 32 }));
        assert_eq!(skin_png_dimensions(&png_header(16, 16)), Some(PngDims { width: 16, height: 16 }));
        assert_eq!(skin_png_dimensions(b"hello"), None);
        assert_eq!(skin_png_dimensions(&[0u8; 10]), None);
        assert_eq!(skin_png_dimensions(&[]), None);
        assert_eq!(skin_png_dimensions(&png_header(64, 64)[..10]), None);
        let mut not_ihdr = png_header(64, 64);
        not_ihdr[12..16].copy_from_slice(b"IDAT");
        assert_eq!(skin_png_dimensions(&not_ihdr), None);
        assert_eq!(skin_png_dimensions(&png_header(0, 0)), None);
    }

    #[test]
    fn valid_skin_dims() {
        assert!(is_valid_skin_png(&png_header(64, 64)));
        assert!(is_valid_skin_png(&png_header(64, 32)));
        assert!(!is_valid_skin_png(&png_header(16, 16)));
        assert!(!is_valid_skin_png(&png_header(128, 128)));
    }

    #[test]
    fn decode_skin_png_shapes() {
        assert_eq!(decode_skin_png(Some(&data_url(&png_header(64, 64)))).unwrap(), png_header(64, 64));
        assert_eq!(decode_skin_png(Some("DATA:IMAGE/PNG;BASE64,iVBORw0KGgo=")).unwrap().len(), 8);
        for bad in [Some("aGVsbG8="), Some("data:image/jpeg;base64,aGVsbG8="), Some(""), None] {
            assert_eq!(decode_skin_png(bad).unwrap_err().envelope().code, "BAD_IMAGE", "{bad:?}");
        }
    }

    #[test]
    fn library_crud_roundtrip() {
        let (_d, e) = temp_engine();
        let saved =
            save_library_skin(&e, Some("hen chad"), Some(SkinVariant::Slim), &png_header(64, 64), "upload", None)
                .unwrap();
        assert_eq!(saved.name, "hen chad");
        assert_eq!(saved.variant, SkinVariant::Slim);
        assert_eq!(saved.source, "upload");
        assert!(saved.id.starts_with("skin_"));
        assert_eq!(load_library_index(&e).len(), 1);
        assert_eq!(read_library_png(&e, &saved.id).unwrap()[..8], PNG_MAGIC);
        let renamed = update_library_skin(&e, &saved.id, Some("hen chad v2"), Some(SkinVariant::Classic)).unwrap();
        assert_eq!((renamed.name.as_str(), renamed.variant), ("hen chad v2", SkinVariant::Classic));
        // on-disk index shape is the JS one
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(library_index_path(&e)).unwrap()).unwrap();
        assert_eq!(raw[0]["variant"], "classic");
        assert_eq!(
            raw[0].as_object().unwrap().keys().collect::<Vec<_>>(),
            ["id", "name", "variant", "created_at", "source"]
        );
        delete_library_skin(&e, &saved.id).unwrap();
        assert!(load_library_index(&e).is_empty());
        assert!(!library_png_path(&e, &saved.id).exists());
    }

    #[test]
    fn library_rejects_bad_input() {
        let (_d, e) = temp_engine();
        let code = |r: EngineResult<SkinLibraryEntry>| r.unwrap_err().envelope().code;
        let p = png_header(64, 64);
        assert_eq!(
            code(save_library_skin(&e, Some("  "), Some(SkinVariant::Classic), &p, "upload", None)),
            "BAD_SKIN_NAME"
        );
        assert_eq!(
            code(save_library_skin(&e, Some(&"x".repeat(41)), Some(SkinVariant::Classic), &p, "upload", None)),
            "BAD_SKIN_NAME"
        );
        assert_eq!(code(save_library_skin(&e, Some("ok"), None, &p, "upload", None)), "BAD_SKIN_VARIANT");
        assert_eq!(
            code(save_library_skin(&e, Some("ok"), Some(SkinVariant::Classic), &png_header(16, 16), "upload", None)),
            "BAD_SKIN_DIMS"
        );
        assert_eq!(get_library_entry(&e, "nope").unwrap_err().envelope().code, "UNKNOWN_SKIN");
        assert_eq!(delete_library_skin(&e, "nope").unwrap_err().envelope().code, "UNKNOWN_SKIN");
        assert_eq!(code(update_library_skin(&e, "nope", Some("x"), None)), "UNKNOWN_SKIN");
        assert_eq!(assert_skin_variant(Some("wide")).unwrap_err().envelope().code, "BAD_SKIN_VARIANT");
    }

    #[test]
    fn library_index_is_lenient() {
        let (_d, e) = temp_engine();
        std::fs::write(
            library_index_path(&e),
            r#"[{"id":"a","name":"A","variant":"slim","created_at":"c","source":"upload","x":1},{"id":3},null,{"name":"n"}]"#,
        )
        .unwrap();
        let idx = load_library_index(&e);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].extra["x"], 1);
        std::fs::write(library_index_path(&e), "{corrupt").unwrap();
        assert!(load_library_index(&e).is_empty());
    }

    #[test]
    fn vanilla_import_is_idempotent() {
        let (d, e) = temp_engine();
        let fixture = json!({
            "version": 1,
            "customSkins": {
                "a": { "id": "a", "name": "hen mujer", "created": "2023-12-24T04:14:18.586Z", "skinImage": data_url(&png_header(64, 64)) },
                "b": { "id": "b", "name": "HEN MUJER", "created": "2024-01-01T00:00:00.000Z", "skinImage": data_url(&png_header(64, 64)) },
                "c": { "id": "c", "name": "broken", "created": "2024-01-01T00:00:00.000Z", "skinImage": data_url(&png_header(16, 16)) },
                "d": { "id": "d", "name": "garbage", "created": "2024-01-01T00:00:00.000Z", "skinImage": "not-an-image" }
            }
        });
        let fp = d.path().join("launcher_custom_skins.json");
        std::fs::write(&fp, fixture.to_string()).unwrap();
        assert_eq!(import_vanilla_skins(&e, &fp).unwrap(), VanillaImport { imported: 1, skipped: 3, total: 4 });
        let idx = load_library_index(&e);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].name, "hen mujer");
        assert_eq!(idx[0].variant(), SkinVariant::Classic);
        assert_eq!(idx[0].source, "vanilla");
        assert_eq!(idx[0].created_at, "2023-12-24T04:14:18.586Z");
        assert_eq!(import_vanilla_skins(&e, &fp).unwrap(), VanillaImport { imported: 0, skipped: 4, total: 4 });
        let missing = import_vanilla_skins(&e, &d.path().join("does-not-exist.json")).unwrap_err().envelope();
        assert_eq!((missing.status, missing.code.as_str()), (404, "NO_VANILLA_SKINS"));
        std::fs::write(&fp, "{bad").unwrap();
        assert_eq!(import_vanilla_skins(&e, &fp).unwrap_err().envelope().code, "BAD_VANILLA_SKINS");
    }

    #[test]
    fn skin_cache_ttl() {
        let (_d, e) = temp_engine();
        assert!(read_skin_cache(&e, "u").is_none());
        write_skin_cache(&e, "u", b"png");
        assert_eq!(read_skin_cache(&e, "u").as_deref(), Some(&b"png"[..]));
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 3600);
        let f = std::fs::File::options().write(true).open(skin_cache_path(&e, "u")).unwrap();
        f.set_modified(old).unwrap();
        assert!(read_skin_cache(&e, "u").is_none());
        clear_skin_cache(&e, "u");
        assert!(!skin_cache_path(&e, "u").exists());
    }

    // ---- extractHeadPng ----

    #[test]
    fn head_transparent_hat_shows_the_face() {
        let head = extract_head_png(&skin_png([255, 0, 0, 255], [0, 0, 0, 0])).unwrap();
        assert_eq!(skin_png_dimensions(&head), Some(PngDims { width: 64, height: 64 }));
        let d = decode_rgba(&head);
        for (x, y) in [(32, 32), (0, 0), (63, 63)] {
            assert_eq!(pixel(&d, x, y), [255, 0, 0, 255]);
        }
    }

    #[test]
    fn head_hat_composites() {
        let d = decode_rgba(&extract_head_png(&skin_png([255, 0, 0, 255], [0, 255, 0, 255])).unwrap());
        assert_eq!(pixel(&d, 32, 32), [0, 255, 0, 255]);
        let d = decode_rgba(&extract_head_png(&skin_png([255, 0, 0, 255], [0, 0, 255, 128])).unwrap());
        assert_eq!(pixel(&d, 32, 32), [127, 0, 128, 255]);
    }

    #[test]
    fn head_decodes_all_filter_types_and_legacy_skins() {
        let plain = extract_head_png(&skin_png([255, 0, 0, 255], [0, 255, 0, 255])).unwrap();
        let filtered =
            extract_head_png(&skin_png_filtered([255, 0, 0, 255], [0, 255, 0, 255], 64, |y| (y % 4) as u8 + 1))
                .unwrap();
        assert_eq!(filtered, plain);
        let legacy =
            decode_rgba(&extract_head_png(&skin_png_filtered([255, 0, 0, 255], [0, 0, 0, 0], 32, |_| 0)).unwrap());
        assert_eq!((legacy.0, legacy.1), (64, 64));
        assert_eq!(pixel(&legacy, 32, 32), [255, 0, 0, 255]);
    }

    #[test]
    fn head_rejects_bad_art() {
        let px = skin_pixels([255, 0, 0, 255], [0, 0, 0, 0], 64);
        let rows64: Vec<(u8, Vec<u8>)> = (0..64).map(|y| (0, px[y * 256..(y + 1) * 256].to_vec())).collect();
        let rgb_rows: Vec<(u8, Vec<u8>)> = (0..64)
            .map(|y| (0, (0..64).flat_map(|x| px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 3].to_vec()).collect()))
            .collect();
        let tiny_rows: Vec<(u8, Vec<u8>)> = (0..16).map(|_| (0, vec![0x80; 64])).collect();
        for (label, buf) in [
            ("rgb-not-rgba", encode_png(64, 64, &rgb_rows, 2, 0)),
            ("interlaced", encode_png(64, 64, &rows64, 6, 1)),
            ("wrong-size", encode_png(16, 16, &tiny_rows, 6, 0)),
            ("junk", b"not a png".to_vec()),
        ] {
            let err = extract_head_png(&buf).unwrap_err().envelope();
            assert_eq!((err.status, err.code.as_str()), (400, "BAD_IMAGE"), "{label}");
        }
    }

    // ---- sync / no-clobber / marker lifecycle ----

    #[test]
    fn sync_stores_head_and_resyncs_auto_avatars() {
        let (_d, e) = temp_engine();
        crate::accounts::create_account(&e, "HeadUser").unwrap();
        assert_eq!(
            sync_account_avatar_from_skin(&e, "HeadUser", &skin_png([255, 0, 0, 255], [0; 4])).unwrap(),
            AvatarSync::Synced
        );
        let a = crate::accounts::get_account(&e, "HeadUser").unwrap();
        assert_eq!(a.avatar_auto, Some(true));
        assert!(crate::accounts::public_account(&e, &a).has_avatar);
        let av = crate::accounts::read_account_avatar(&e, "HeadUser").unwrap();
        assert_eq!(pixel(&decode_rgba(&av), 32, 32), [255, 0, 0, 255]);
        assert_eq!(
            sync_account_avatar_from_skin(&e, "HeadUser", &skin_png([0, 255, 0, 255], [0; 4])).unwrap(),
            AvatarSync::Synced
        );
        let av = crate::accounts::read_account_avatar(&e, "HeadUser").unwrap();
        assert_eq!(pixel(&decode_rgba(&av), 32, 32), [0, 255, 0, 255]);
    }

    #[test]
    fn sync_never_clobbers_custom_avatars() {
        let (_d, e) = temp_engine();
        let tiny = crate::util::lenient_base64_decode(TINY_PNG_B64);
        crate::accounts::create_account(&e, "CustomUser").unwrap();
        crate::accounts::write_account_avatar(&e, "CustomUser", &tiny).unwrap();
        assert_eq!(crate::accounts::get_account(&e, "CustomUser").unwrap().avatar_auto, None);
        assert!(!should_sync_avatar_from_skin(&e, "CustomUser"));
        assert_eq!(
            sync_account_avatar_from_skin(&e, "CustomUser", &skin_png([255, 0, 0, 255], [0; 4])).unwrap(),
            AvatarSync::SkippedCustom
        );
        assert_eq!(crate::accounts::read_account_avatar(&e, "CustomUser").unwrap(), tiny);
        assert_eq!(crate::accounts::get_account(&e, "CustomUser").unwrap().avatar_auto, None);
        assert_eq!(
            sync_account_avatar_from_skin(&e, "NobodyHere", &skin_png([0; 4], [0; 4])).unwrap_err().envelope().status,
            404
        );
        assert!(!should_sync_avatar_from_skin(&e, "NobodyHere"));
    }

    #[test]
    fn custom_upload_clears_marker_and_auto_avatar_reset() {
        let (_d, e) = temp_engine();
        let tiny = crate::util::lenient_base64_decode(TINY_PNG_B64);
        crate::accounts::create_account(&e, "MarkerUser").unwrap();
        sync_account_avatar_from_skin(&e, "MarkerUser", &skin_png([255, 0, 0, 255], [0; 4])).unwrap();
        crate::accounts::write_account_avatar(&e, "MarkerUser", &tiny).unwrap();
        let a = crate::accounts::get_account(&e, "MarkerUser").unwrap();
        assert_eq!(a.avatar_auto, None);
        let v = serde_json::to_value(crate::accounts::public_account(&e, &a)).unwrap();
        assert_eq!(v["has_avatar"], true);
        assert!(v.get("avatar_auto").is_none());

        crate::accounts::create_account(&e, "AutoDrop").unwrap();
        sync_account_avatar_from_skin(&e, "AutoDrop", &skin_png([255, 0, 0, 255], [0; 4])).unwrap();
        assert!(clear_account_auto_avatar(&e, "AutoDrop").unwrap());
        let a = crate::accounts::get_account(&e, "AutoDrop").unwrap();
        assert!(!crate::accounts::public_account(&e, &a).has_avatar);
        assert_eq!(a.avatar_auto, None);
        crate::accounts::create_account(&e, "CustomKeep").unwrap();
        crate::accounts::write_account_avatar(&e, "CustomKeep", &tiny).unwrap();
        assert!(!clear_account_auto_avatar(&e, "CustomKeep").unwrap());
        assert_eq!(crate::accounts::read_account_avatar(&e, "CustomKeep").unwrap(), tiny);
        assert!(!clear_account_auto_avatar(&e, "Ghost").unwrap());
    }

    #[test]
    fn base36_ids() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
        assert_eq!(base36(1_730_000_000_000), "m2r1bz7k");
    }
}
