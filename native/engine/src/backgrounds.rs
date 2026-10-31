//! Port of `src/engine/backgrounds.mjs` — menu background packs
//! (`<data>/backgrounds/<id>/pack.json` + frames), personalization write, and
//! staging the selected pack into `<gameDir>/config/espectral-menu-bg/<id>/`.
//!
//! **Owner: accounts-skins.** Status: PORTED.
//!
//! Store layout under `<data>/backgrounds/<packId>/`:
//!   `pack.json` `{ id, name, type: "image"|"frames", fps, frame_count, width, height }`,
//!   `image.png` (type image) or `frames/NNNN.png` (type frames, 1-based, 4 digits).
//!
//! Import limits (contract): image packs PNG magic + IHDR, <= 8192 px per
//! side, <= 64 MB file; frame packs <= 300 frames, <= 1280x720 sharing one
//! resolution, <= 80 MB total, fps clamped to [1, 60].
//!
//! `writePersonalization` goes through identity-discord's
//! [`crate::client::patch_client_config`] (owner of the personalization
//! whitelist); this module adds the pack-existence check.
//!
//! Tests: tests/backgrounds.test.mjs, personalization-routes.test.mjs.
//!
//! CONTRACT (called by game-launch resolveLaunch): stage_background_pack,
//! migrate_legacy_background.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use rand::RngCore;
use serde_json::Value;

use crate::api::accounts_skins::{ImportFramesPackRequest, ImportImagePackRequest};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::skins::{skin_png_dimensions, PngDims};
use crate::types::{BackgroundPack, ClientInfo, ClientPatch, PersonalizationPatch};
use crate::util::PNG_MAGIC;

pub const MAX_FRAMES: usize = 300;
pub const MAX_FRAME_W: u32 = 1280;
pub const MAX_FRAME_H: u32 = 720;
pub const MAX_FRAME_PACK_BYTES: usize = 80 * 1024 * 1024;
pub const MAX_IMAGE_DIM: u32 = 8192;
pub const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_NAME: usize = 60;
const DEFAULT_FPS: f64 = 24.0;
const MIN_FPS: f64 = 1.0;
const MAX_FPS: f64 = 60.0;
const LEGACY_BACKGROUND: &str = "espectral-menu-background.png";

pub fn backgrounds_root(engine: &Engine) -> PathBuf {
    engine.paths().backgrounds_dir()
}

pub fn pack_dir(engine: &Engine, id: &str) -> PathBuf {
    backgrounds_root(engine).join(id)
}

/// `^[a-z0-9][a-z0-9_-]{0,39}$`
pub fn is_valid_pack_id(id: &str) -> bool {
    let b = id.as_bytes();
    (1..=40).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

fn assert_pack_id(id: &str) -> EngineResult<()> {
    if is_valid_pack_id(id) {
        Ok(())
    } else {
        Err(EngineError::bad_request("BAD_PACK_ID", "pack id must match ^[a-z0-9][a-z0-9_-]{0,39}$"))
    }
}

fn assert_name(name: Option<&str>) -> EngineResult<String> {
    let t = name.map(str::trim).unwrap_or("");
    if t.is_empty() || t.encode_utf16().count() > MAX_NAME {
        return Err(EngineError::bad_request("BAD_NAME", format!("name must be 1-{MAX_NAME} characters")));
    }
    Ok(t.to_string())
}

/// NFD + strip combining marks for the common Latin letters (the JS used
/// `normalize('NFD').replace(/[̀-ͯ]/g, '')`).
fn fold_diacritic(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

/// Slug of a pack name: lowercase, accents folded, `[^a-z0-9]+` -> '-', trimmed, <= 32 chars.
pub fn slug_for(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.to_lowercase().chars().map(fold_diacritic) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let trimmed: String = out.trim_matches('-').chars().take(32).collect();
    if is_valid_pack_id(&trimmed) {
        trimmed
    } else {
        "pack".into()
    }
}

/// id derived from the pack name, uniquified against the store.
fn unique_pack_id(engine: &Engine, name: &str) -> String {
    let base = slug_for(name);
    if !pack_dir(engine, &base).exists() {
        return base;
    }
    for i in 2..1000 {
        let id = format!("{base}-{i}");
        if !pack_dir(engine, &id).exists() {
            return id;
        }
    }
    let mut b = [0u8; 3];
    rand::thread_rng().fill_bytes(&mut b);
    format!("{base}-{}", hex::encode(b))
}

/// JS `Number(v)` for pack.json fields (numbers and numeric strings).
fn js_number(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) if s.trim().is_empty() => 0.0,
        Some(Value::String(s)) => s.trim().parse().unwrap_or(f64::NAN),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Null) => 0.0,
        _ => f64::NAN,
    }
}

fn nonneg_floor(x: f64) -> u32 {
    if x.is_finite() && x > 0.0 {
        x.floor().min(u32::MAX as f64) as u32
    } else {
        0
    }
}

/// Parse + normalize a pack.json inside a pack dir; None when unreadable/invalid.
pub fn read_pack_meta(dir: &Path) -> Option<BackgroundPack> {
    let j: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("pack.json")).ok()?).ok()?;
    let kind = j.get("type").and_then(Value::as_str)?;
    if kind != "image" && kind != "frames" {
        return None;
    }
    let base = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let frames = kind == "frames";
    let fps = js_number(j.get("fps"));
    Some(BackgroundPack {
        id: j.get("id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| base.clone()),
        name: j.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or(base),
        kind: kind.to_string(),
        // u32 in the shared type: a fractional fps from a hand-made pack.json rounds.
        fps: if frames {
            if fps.is_finite() && fps != 0.0 {
                fps.round().max(0.0) as u32
            } else {
                DEFAULT_FPS as u32
            }
        } else {
            0
        },
        frame_count: if frames { nonneg_floor(js_number(j.get("frame_count"))) } else { 1 },
        width: nonneg_floor(js_number(j.get("width"))),
        height: nonneg_floor(js_number(j.get("height"))),
    })
}

/// Preview PNG path for a pack meta (first frame for animated packs).
fn preview_file(engine: &Engine, meta: &BackgroundPack) -> Option<PathBuf> {
    let dir = pack_dir(engine, &meta.id);
    let p = if meta.kind == "frames" { dir.join("frames").join("0001.png") } else { dir.join("image.png") };
    p.exists().then_some(p)
}

/// All importable packs (store scan, name-sorted).
pub fn list_packs(engine: &Engine) -> Vec<BackgroundPack> {
    let Ok(rd) = std::fs::read_dir(backgrounds_root(engine)) else { return vec![] };
    let mut packs: Vec<BackgroundPack> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| read_pack_meta(&e.path()))
        .filter(|m| preview_file(engine, m).is_some()) // half-written or broken pack
        .collect();
    packs.sort_by(|a, b| locale_cmp(&a.name, &b.name).then_with(|| locale_cmp(&a.id, &b.id)));
    packs
}

/// Approximation of `String.prototype.localeCompare` (case-insensitive first).
fn locale_cmp(a: &str, b: &str) -> Ordering {
    let fold = |s: &str| s.to_lowercase().chars().map(fold_diacritic).collect::<String>();
    fold(a).cmp(&fold(b)).then_with(|| a.cmp(b))
}

/// Natural sort for frame filenames (frame2 < frame10), case-insensitive.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ia, mut ib) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut ia), take(&mut ib));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ia.next();
                ib.next();
            }
        }
    }
}

/// Absolute path of the pack preview PNG; 400 BAD_PACK_ID / 404 PACK_NOT_FOUND.
pub fn pack_preview_path(engine: &Engine, id: &str) -> EngineResult<PathBuf> {
    assert_pack_id(id)?;
    read_pack_meta(&pack_dir(engine, id))
        .and_then(|m| preview_file(engine, &m))
        .ok_or_else(|| EngineError::not_found("PACK_NOT_FOUND", format!("no such background pack '{id}'")))
}

fn assert_png(buf: &[u8], what: &str) -> EngineResult<PngDims> {
    if buf.len() < 8 || buf[..8] != PNG_MAGIC {
        return Err(EngineError::bad_request("BAD_IMAGE", format!("{what} must be a PNG")));
    }
    skin_png_dimensions(buf)
        .ok_or_else(|| EngineError::bad_request("BAD_IMAGE", format!("{what} is not a readable PNG (bad IHDR)")))
}

/// Read a PNG file from a local path (size-capped). Callers validate the bytes.
fn read_png_file(p: &str) -> EngineResult<Vec<u8>> {
    if p.trim().is_empty() {
        return Err(EngineError::bad_request("BAD_PATH", "source path must be a non-empty string"));
    }
    let resolved = std::path::absolute(p).unwrap_or_else(|_| PathBuf::from(p));
    let meta =
        std::fs::metadata(&resolved).map_err(|_| EngineError::bad_request("BAD_PATH", format!("cannot read {p}")))?;
    if !meta.is_file() {
        return Err(EngineError::bad_request("BAD_PATH", format!("{p} is not a file")));
    }
    if meta.len() > MAX_IMAGE_FILE_BYTES {
        return Err(EngineError::api(413, "IMAGE_TOO_LARGE", format!("file exceeds {MAX_IMAGE_FILE_BYTES} bytes")));
    }
    std::fs::read(&resolved).map_err(|_| EngineError::bad_request("BAD_PATH", format!("cannot read {p}")))
}

/// Decode one base64 PNG upload with a caller-chosen byte cap (frame packs).
fn decode_png_bytes(image_base64: &str, max_bytes: usize, what: &str) -> EngineResult<Vec<u8>> {
    let raw = image_base64.trim();
    if raw.is_empty() {
        return Err(EngineError::bad_request("BAD_IMAGE", format!("{what} is required")));
    }
    let b64 = if raw.starts_with("data:") {
        raw.strip_prefix("data:image/png;base64,").ok_or_else(|| {
            EngineError::bad_request("BAD_IMAGE", format!("{what} must be a PNG data URL or raw base64 PNG"))
        })?
    } else {
        raw
    };
    let buf = crate::util::lenient_base64_decode(b64);
    if buf.len() > max_bytes {
        return Err(EngineError::api(413, "IMAGE_TOO_LARGE", format!("{what} exceeds {max_bytes} bytes decoded")));
    }
    assert_png(&buf, what)?;
    Ok(buf)
}

fn write_pack_json(dir: &Path, meta: &BackgroundPack) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(meta).map_err(std::io::Error::other)? + "\n";
    std::fs::write(dir.join("pack.json"), text)
}

/// Write a pack dir via a temp dir + rename so a failed import leaves nothing behind.
fn commit_pack(engine: &Engine, id: &str, write: impl FnOnce(&Path) -> std::io::Result<()>) -> EngineResult<()> {
    let root = backgrounds_root(engine);
    std::fs::create_dir_all(&root)?;
    let mut b = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut b);
    let tmp = root.join(format!(".tmp-{}", hex::encode(b)));
    std::fs::create_dir_all(&tmp)?;
    let r = write(&tmp).and_then(|_| std::fs::rename(&tmp, pack_dir(engine, id)));
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// Import a still-image pack: `image_base64` (500 KB icon cap) or `source_path`
/// (PNG file, <= 64 MB / 8192 px per side).
pub fn import_image_pack(engine: &Engine, req: &ImportImagePackRequest) -> EngineResult<BackgroundPack> {
    let name = assert_name(Some(&req.name))?;
    let buf = match &req.source_path {
        Some(p) => {
            let b = read_png_file(p)?;
            assert_png(&b, "image")?;
            b
        }
        None => crate::util::decode_png_image(req.image_base64.as_deref())?,
    };
    let dims = skin_png_dimensions(&buf).filter(|d| d.width <= MAX_IMAGE_DIM && d.height <= MAX_IMAGE_DIM).ok_or_else(
        || {
            EngineError::bad_request(
                "BAD_IMAGE",
                format!("image must be a PNG no larger than {MAX_IMAGE_DIM}px per side"),
            )
        },
    )?;
    let id = unique_pack_id(engine, &name);
    let meta = BackgroundPack {
        id: id.clone(),
        name,
        kind: "image".into(),
        fps: 0,
        frame_count: 1,
        width: dims.width,
        height: dims.height,
    };
    commit_pack(engine, &id, |tmp| {
        std::fs::write(tmp.join("image.png"), &buf)?;
        write_pack_json(tmp, &meta)
    })?;
    Ok(meta)
}

enum FrameSource {
    Path(String),
    Base64(String),
}

fn frame_sources(req: &ImportFramesPackRequest) -> EngineResult<Vec<(String, FrameSource)>> {
    let too_many = || EngineError::bad_request("TOO_MANY_FRAMES", format!("at most {MAX_FRAMES} frames per pack"));
    if let Some(paths) = &req.frame_paths {
        if paths.is_empty() {
            return Err(EngineError::bad_request("NO_FRAMES", "frame_paths is empty"));
        }
        if paths.len() > MAX_FRAMES {
            return Err(too_many());
        }
        return Ok(paths
            .iter()
            .map(|p| {
                let label =
                    Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
                (label, FrameSource::Path(p.clone()))
            })
            .collect());
    }
    if let Some(src) = &req.source_path {
        if src.trim().is_empty() {
            return Err(EngineError::bad_request("BAD_PATH", "source_path must be a non-empty string"));
        }
        let dir = std::path::absolute(src).unwrap_or_else(|_| PathBuf::from(src));
        let rd =
            std::fs::read_dir(&dir).map_err(|_| EngineError::bad_request("BAD_PATH", format!("cannot read {src}")))?;
        let mut pngs: Vec<String> = rd
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_lowercase().ends_with(".png"))
            .collect();
        pngs.sort_by(|a, b| natural_cmp(a, b));
        if pngs.is_empty() {
            return Err(EngineError::bad_request("NO_FRAMES", format!("no PNG files in {src}")));
        }
        if pngs.len() > MAX_FRAMES {
            return Err(too_many());
        }
        return Ok(pngs
            .into_iter()
            .map(|n| {
                let p = dir.join(&n).to_string_lossy().into_owned();
                (n, FrameSource::Path(p))
            })
            .collect());
    }
    if let Some(frames) = &req.frames {
        if frames.is_empty() {
            return Err(EngineError::bad_request("NO_FRAMES", "frames is empty"));
        }
        if frames.len() > MAX_FRAMES {
            return Err(too_many());
        }
        return Ok(frames
            .iter()
            .enumerate()
            .map(|(i, f)| (format!("frames[{i}]"), FrameSource::Base64(f.clone())))
            .collect());
    }
    Err(EngineError::bad_request("NO_FRAMES", "provide frames[] (base64), frame_paths[] or source_path"))
}

/// Import an animated frame pack (<= 300 PNG frames, <= 1280x720, one
/// resolution, <= 80 MB). fps defaults to 24 (clamped 1..60); a pack.json
/// next to a directory import supplies name/fps defaults.
pub fn import_frames_pack(engine: &Engine, req: &ImportFramesPackRequest) -> EngineResult<BackgroundPack> {
    let embedded: Option<Value> = req
        .source_path
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| std::fs::read_to_string(Path::new(s).join("pack.json")).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object);
    let name = if !req.name.is_empty() {
        assert_name(Some(&req.name))?
    } else {
        assert_name(embedded.as_ref().and_then(|j| j.get("name")).and_then(Value::as_str))?
    };
    let mut fps = match (req.fps, embedded.as_ref().and_then(|j| j.get("fps"))) {
        (Some(f), _) => f as f64,
        (None, Some(v)) => js_number(Some(v)),
        (None, None) => DEFAULT_FPS,
    };
    if !fps.is_finite() {
        fps = DEFAULT_FPS;
    }
    let fps = fps.clamp(MIN_FPS, MAX_FPS);

    let sources = frame_sources(req)?;
    let mut bufs = Vec::with_capacity(sources.len());
    let (mut total, mut width, mut height) = (0usize, 0u32, 0u32);
    for (i, (label, src)) in sources.iter().enumerate() {
        let buf = match src {
            FrameSource::Path(p) => read_png_file(p)?,
            FrameSource::Base64(b) => decode_png_bytes(b, MAX_FRAME_PACK_BYTES, label)?,
        };
        total += buf.len();
        if total > MAX_FRAME_PACK_BYTES {
            return Err(EngineError::api(
                413,
                "PACK_TOO_LARGE",
                format!("frame pack exceeds {MAX_FRAME_PACK_BYTES} bytes decoded"),
            ));
        }
        let dims = assert_png(&buf, label)?;
        if dims.width > MAX_FRAME_W || dims.height > MAX_FRAME_H {
            return Err(EngineError::bad_request(
                "FRAME_TOO_LARGE",
                format!("{label}: frames must be <= {MAX_FRAME_W}x{MAX_FRAME_H}"),
            ));
        }
        if i == 0 {
            (width, height) = (dims.width, dims.height);
        } else if dims.width != width || dims.height != height {
            return Err(EngineError::bad_request(
                "FRAME_DIMS",
                format!("{label}: all frames must share one resolution ({width}x{height})"),
            ));
        }
        bufs.push(buf);
    }
    let id = unique_pack_id(engine, &name);
    let meta = BackgroundPack {
        id: id.clone(),
        name,
        kind: "frames".into(),
        fps: fps.round() as u32,
        frame_count: bufs.len() as u32,
        width,
        height,
    };
    commit_pack(engine, &id, |tmp| {
        let fdir = tmp.join("frames");
        std::fs::create_dir_all(&fdir)?;
        for (i, b) in bufs.iter().enumerate() {
            std::fs::write(fdir.join(format!("{:04}.png", i + 1)), b)?;
        }
        write_pack_json(tmp, &meta)
    })?;
    Ok(meta)
}

/// Delete a pack from the store. 400 BAD_PACK_ID / 404 PACK_NOT_FOUND.
pub fn delete_pack(engine: &Engine, id: &str) -> EngineResult<()> {
    assert_pack_id(id)?;
    let dir = pack_dir(engine, id);
    if !dir.exists() {
        return Err(EngineError::not_found("PACK_NOT_FOUND", format!("no such background pack '{id}'")));
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

// --- personalization write --------------------------------------------------

/// Merge personalization fields into the instance's espectral-client.json via
/// `client::patch_client_config` and return its ClientInfo. A non-null
/// `background` must name an existing pack (400 BAD_PACK_ID / 404 PACK_NOT_FOUND).
pub async fn write_personalization(
    engine: &Engine,
    instance_name: &str,
    patch: &PersonalizationPatch,
) -> EngineResult<ClientInfo> {
    if let Some(Some(bg)) = &patch.background {
        if !is_valid_pack_id(bg) {
            return Err(EngineError::bad_request("BAD_PACK_ID", "background must be a pack id or null"));
        }
        if read_pack_meta(&pack_dir(engine, bg)).is_none() {
            return Err(EngineError::not_found("PACK_NOT_FOUND", format!("no such background pack '{bg}'")));
        }
    }
    let personalization = serde_json::to_value(patch)?;
    crate::client::patch_client_config(
        engine,
        instance_name,
        ClientPatch { personalization: Some(personalization), ..Default::default() },
    )
    .await
}

/// Selected background id from the canonical client config, or None.
pub fn selected_background(engine: &Engine, instance_name: &str) -> Option<String> {
    let cfg = crate::client::load_client_config(engine, instance_name).ok()?;
    let bg = cfg.personalization?.get("background")?.as_str()?.to_string();
    is_valid_pack_id(&bg).then_some(bg)
}

/// One-time cutover from a hand-placed `config/espectral-menu-background.png`
/// (instance dir or a gameDir mirror): imported as a regular image pack,
/// selected when nothing is selected yet; the legacy files are removed once
/// their bytes are in the store. Returns the new pack; None when there was
/// nothing to migrate or the migration failed (never errors).
pub async fn migrate_legacy_background_pack(
    engine: &Engine,
    instance_name: &str,
    dirs: &[PathBuf],
) -> Option<BackgroundPack> {
    let mut legacy: Vec<PathBuf> = vec![];
    for d in dirs {
        let p = d.join("config").join(LEGACY_BACKGROUND);
        if !legacy.contains(&p) && p.exists() {
            legacy.push(p);
        }
    }
    let first = legacy.first()?;
    let req = ImportImagePackRequest {
        name: "Mi fondo".into(),
        image_base64: None,
        source_path: Some(first.to_string_lossy().into_owned()),
    };
    let result: EngineResult<BackgroundPack> = async {
        let meta = import_image_pack(engine, &req)?;
        if selected_background(engine, instance_name).is_none() {
            let patch = PersonalizationPatch { background: Some(Some(meta.id.clone())), ..Default::default() };
            write_personalization(engine, instance_name, &patch).await?;
        }
        for p in &legacy {
            let _ = std::fs::remove_file(p);
        }
        Ok(meta)
    }
    .await;
    match result {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::debug!("[backgrounds] legacy background migration skipped: {e}");
            None
        }
    }
}

/// Move a pre-store legacy background into the pack store (idempotent, never fails).
pub async fn migrate_legacy_background(engine: &Engine, instance_name: &str, dirs: &[PathBuf]) -> EngineResult<()> {
    migrate_legacy_background_pack(engine, instance_name, dirs).await;
    Ok(())
}

// --- launch staging ---------------------------------------------------------

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dest.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir_recursive(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

fn rm_rf(p: &Path) {
    if p.is_dir() {
        let _ = std::fs::remove_dir_all(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Stage `pack_id` (or clear the staging dir for None) under
/// `<gameDir>/config/espectral-menu-bg/`. Stale packs are pruned; an
/// identical staged pack.json means the pack is already staged (packs are
/// immutable). Never fails — a launch must not fail over the background.
pub fn stage_background_pack(engine: &Engine, pack_id: Option<&str>, game_dir: &Path) -> EngineResult<()> {
    let r: std::io::Result<()> = (|| {
        let dest_root = crate::paths::menu_bg_staging_in(game_dir);
        let Some(id) = pack_id else {
            rm_rf(&dest_root);
            return Ok(());
        };
        if !is_valid_pack_id(id) {
            return Ok(());
        }
        let src = pack_dir(engine, id);
        if read_pack_meta(&src).is_none() {
            // Selected pack vanished from the store — stage nothing, prune stale copies.
            rm_rf(&dest_root);
            return Ok(());
        }
        std::fs::create_dir_all(&dest_root)?;
        for e in std::fs::read_dir(&dest_root)?.flatten() {
            if e.file_name().to_string_lossy() != id {
                rm_rf(&e.path());
            }
        }
        let dest = dest_root.join(id);
        if let (Ok(a), Ok(b)) =
            (std::fs::read_to_string(dest.join("pack.json")), std::fs::read_to_string(src.join("pack.json")))
        {
            if a == b {
                return Ok(());
            }
        }
        rm_rf(&dest);
        copy_dir_recursive(&src, &dest)
    })();
    if let Err(e) = r {
        tracing::debug!("[backgrounds] staging failed (ignored): {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_engine;

    fn fake_png(w: u32, h: u32) -> Vec<u8> {
        let mut b = PNG_MAGIC.to_vec();
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0]);
        b
    }

    fn b64(buf: &[u8]) -> String {
        format!("data:image/png;base64,{}", crate::util::base64_encode(buf))
    }

    fn code<T: std::fmt::Debug>(r: EngineResult<T>) -> String {
        r.unwrap_err().envelope().code
    }

    fn img(name: &str, b: &[u8]) -> ImportImagePackRequest {
        ImportImagePackRequest { name: name.into(), image_base64: Some(b64(b)), source_path: None }
    }

    #[test]
    fn ids_and_slugs() {
        assert!(is_valid_pack_id("a"));
        assert!(is_valid_pack_id("fondo-noche_2"));
        assert!(!is_valid_pack_id("-x"));
        assert!(!is_valid_pack_id("../escape"));
        assert!(!is_valid_pack_id(&"a".repeat(41)));
        assert_eq!(slug_for("Fondo Noche"), "fondo-noche");
        assert_eq!(slug_for("Mañana Épica!!"), "manana-epica");
        assert_eq!(slug_for("   "), "pack");
        assert_eq!(slug_for("日本"), "pack");
        assert_eq!(slug_for(&"x".repeat(50)).len(), 32);
    }

    #[test]
    fn natural_sort() {
        let mut v = vec!["f10.png", "f2.png", "F1.png", "a.png"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a.png", "F1.png", "f2.png", "f10.png"]);
    }

    #[test]
    fn image_pack_happy_path() {
        let (_d, e) = temp_engine();
        let pack = import_image_pack(&e, &img("Fondo Noche", &fake_png(64, 32))).unwrap();
        assert_eq!((pack.kind.as_str(), pack.width, pack.height, pack.fps, pack.frame_count), ("image", 64, 32, 0, 1));
        assert!(is_valid_pack_id(&pack.id));
        assert_eq!(pack.id, "fondo-noche");
        let dir = pack_dir(&e, &pack.id);
        assert!(dir.join("image.png").exists());
        let text = std::fs::read_to_string(dir.join("pack.json")).unwrap();
        assert!(text.ends_with("}\n"));
        let meta: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(meta["type"], "image");
        assert_eq!(meta["name"], "Fondo Noche");
        assert_eq!(
            meta.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["id", "name", "type", "fps", "frame_count", "width", "height"]
        );
        assert!(list_packs(&e).iter().any(|p| p.id == pack.id));
        assert!(pack_preview_path(&e, &pack.id).unwrap().ends_with("image.png"));
        // same name -> uniquified id
        assert_eq!(import_image_pack(&e, &img("Fondo Noche", &fake_png(8, 8))).unwrap().id, "fondo-noche-2");
        // no temp dirs left behind
        assert!(std::fs::read_dir(backgrounds_root(&e))
            .unwrap()
            .flatten()
            .all(|x| !x.file_name().to_string_lossy().starts_with(".tmp")));
    }

    #[test]
    fn image_pack_rejections() {
        let (_d, e) = temp_engine();
        assert_eq!(code(import_image_pack(&e, &img("x", b"not a png"))), "BAD_IMAGE");
        assert_eq!(code(import_image_pack(&e, &img("x", &fake_png(0, 0)))), "BAD_IMAGE");
        assert_eq!(code(import_image_pack(&e, &img("x", &fake_png(9000, 10)))), "BAD_IMAGE");
        assert_eq!(code(import_image_pack(&e, &img("", &fake_png(8, 8)))), "BAD_NAME");
        assert_eq!(code(import_image_pack(&e, &img(&"n".repeat(61), &fake_png(8, 8)))), "BAD_NAME");
    }

    #[test]
    fn image_pack_from_disk() {
        let (d, e) = temp_engine();
        let src = d.path().join("wall.png");
        std::fs::write(&src, fake_png(1920, 1080)).unwrap();
        let req = ImportImagePackRequest {
            name: "wall".into(),
            image_base64: None,
            source_path: Some(src.to_string_lossy().into()),
        };
        let pack = import_image_pack(&e, &req).unwrap();
        assert_eq!((pack.width, pack.height), (1920, 1080));
        let missing = ImportImagePackRequest {
            name: "nope".into(),
            image_base64: None,
            source_path: Some(d.path().join("missing.png").to_string_lossy().into()),
        };
        assert_eq!(code(import_image_pack(&e, &missing)), "BAD_PATH");
    }

    #[test]
    fn frames_from_folder_use_natural_order() {
        let (d, e) = temp_engine();
        let dir = d.path().join("frames-src");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("f10.png"), fake_png(320, 180)).unwrap();
        let mut f2 = fake_png(320, 180);
        f2.push(2);
        std::fs::write(dir.join("f2.png"), &f2).unwrap();
        std::fs::write(dir.join("f1.png"), fake_png(320, 180)).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let req = ImportFramesPackRequest {
            name: "loop".into(),
            fps: Some(30),
            source_path: Some(dir.to_string_lossy().into()),
            ..Default::default()
        };
        let pack = import_frames_pack(&e, &req).unwrap();
        assert_eq!(
            (pack.kind.as_str(), pack.frame_count, pack.fps, pack.width, pack.height),
            ("frames", 3, 30, 320, 180)
        );
        let fdir = pack_dir(&e, &pack.id).join("frames");
        let mut names: Vec<String> =
            std::fs::read_dir(&fdir).unwrap().flatten().map(|x| x.file_name().to_string_lossy().into()).collect();
        names.sort();
        assert_eq!(names, ["0001.png", "0002.png", "0003.png"]);
        assert_eq!(std::fs::read(fdir.join("0002.png")).unwrap(), f2, "f2 is the second frame");
        assert!(pack_preview_path(&e, &pack.id).unwrap().ends_with("frames/0001.png"));
    }

    #[test]
    fn frames_from_base64_clamp_fps_and_embedded_defaults() {
        let (d, e) = temp_engine();
        let req = ImportFramesPackRequest {
            name: "b64loop".into(),
            fps: Some(500),
            frames: Some(vec![b64(&fake_png(8, 8)), b64(&fake_png(8, 8))]),
            ..Default::default()
        };
        let pack = import_frames_pack(&e, &req).unwrap();
        assert_eq!((pack.frame_count, pack.fps), (2, 60));
        // pack.json beside a folder supplies name + fps
        let dir = d.path().join("emb");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("a.png"), fake_png(4, 4)).unwrap();
        std::fs::write(dir.join("pack.json"), r#"{"name":"Embedded","fps":12}"#).unwrap();
        let req = ImportFramesPackRequest { source_path: Some(dir.to_string_lossy().into()), ..Default::default() };
        let pack = import_frames_pack(&e, &req).unwrap();
        assert_eq!((pack.name.as_str(), pack.fps), ("Embedded", 12));
    }

    #[test]
    fn frame_limits() {
        let (d, e) = temp_engine();
        let frames = |v: Vec<Vec<u8>>| ImportFramesPackRequest {
            name: "x".into(),
            frames: Some(v.iter().map(|b| b64(b)).collect()),
            ..Default::default()
        };
        assert_eq!(code(import_frames_pack(&e, &frames(vec![fake_png(100, 100), fake_png(200, 100)]))), "FRAME_DIMS");
        assert_eq!(code(import_frames_pack(&e, &frames(vec![fake_png(1920, 1080)]))), "FRAME_TOO_LARGE");
        assert_eq!(code(import_frames_pack(&e, &frames(vec![]))), "NO_FRAMES");
        assert_eq!(
            code(import_frames_pack(&e, &ImportFramesPackRequest { name: "x".into(), ..Default::default() })),
            "NO_FRAMES"
        );
        let many: Vec<String> =
            (0..301).map(|i| d.path().join(format!("nope-{i}.png")).to_string_lossy().into()).collect();
        let req = ImportFramesPackRequest { name: "many".into(), frame_paths: Some(many), ..Default::default() };
        assert_eq!(code(import_frames_pack(&e, &req)), "TOO_MANY_FRAMES");
        // 80 MB total cap (each file under the 64 MB per-file cap)
        let (b1, b2) = (d.path().join("huge-1.png"), d.path().join("huge-2.png"));
        for p in [&b1, &b2] {
            std::fs::write(p, fake_png(16, 16)).unwrap();
            std::fs::File::options().write(true).open(p).unwrap().set_len(45 * 1024 * 1024).unwrap();
        }
        let req = ImportFramesPackRequest {
            name: "huge".into(),
            frame_paths: Some(vec![b1.to_string_lossy().into(), b2.to_string_lossy().into()]),
            ..Default::default()
        };
        let err = import_frames_pack(&e, &req).unwrap_err().envelope();
        assert_eq!((err.status, err.code.as_str()), (413, "PACK_TOO_LARGE"));
    }

    #[test]
    fn delete_and_listing() {
        let (_d, e) = temp_engine();
        let pack = import_image_pack(&e, &img("bye", &fake_png(8, 8))).unwrap();
        assert!(pack_dir(&e, &pack.id).exists());
        delete_pack(&e, &pack.id).unwrap();
        assert!(!pack_dir(&e, &pack.id).exists());
        assert_eq!(code(delete_pack(&e, &pack.id)), "PACK_NOT_FOUND");
        assert_eq!(code(delete_pack(&e, "../escape")), "BAD_PACK_ID");
        assert!(!list_packs(&e).iter().any(|p| p.id == pack.id));
        assert_eq!(code(pack_preview_path(&e, &pack.id)), "PACK_NOT_FOUND");
        // sorted by name; broken packs hidden
        import_image_pack(&e, &img("zeta", &fake_png(8, 8))).unwrap();
        import_image_pack(&e, &img("Alpha", &fake_png(8, 8))).unwrap();
        std::fs::create_dir_all(pack_dir(&e, "broken")).unwrap();
        std::fs::write(pack_dir(&e, "broken").join("pack.json"), r#"{"type":"image"}"#).unwrap();
        assert_eq!(list_packs(&e).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Alpha", "zeta"]);
    }

    #[test]
    fn pack_meta_is_lenient() {
        let (d, _e) = temp_engine();
        let dir = d.path().join("hand-made");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pack.json"), r#"{"type":"frames","fps":"x","frame_count":"7.9","width":-3}"#).unwrap();
        let m = read_pack_meta(&dir).unwrap();
        assert_eq!(
            (m.id.as_str(), m.name.as_str(), m.fps, m.frame_count, m.width, m.height),
            ("hand-made", "hand-made", 24, 7, 0, 0)
        );
        std::fs::write(dir.join("pack.json"), r#"{"type":"video"}"#).unwrap();
        assert!(read_pack_meta(&dir).is_none());
    }

    #[tokio::test]
    async fn personalization_checks_the_pack_before_delegating() {
        let (_d, e) = temp_engine();
        let p = |bg: &str| PersonalizationPatch { background: Some(Some(bg.into())), ..Default::default() };
        assert_eq!(code(write_personalization(&e, "inst", &p("nope")).await), "PACK_NOT_FOUND");
        assert_eq!(code(write_personalization(&e, "inst", &p("../x")).await), "BAD_PACK_ID");
        // the serialized patch keeps explicit nulls and drops absent keys
        let v = serde_json::to_value(PersonalizationPatch { background: Some(None), ..Default::default() }).unwrap();
        assert_eq!(v, serde_json::json!({ "background": null }));
    }

    #[test]
    fn staging_mirrors_prunes_and_clears() {
        let (d, e) = temp_engine();
        let req = ImportFramesPackRequest {
            name: "staged".into(),
            frames: Some(vec![b64(&fake_png(8, 8)), b64(&fake_png(8, 8))]),
            ..Default::default()
        };
        let pack = import_frames_pack(&e, &req).unwrap();
        let profile = d.path().join("instances/bg-stage/profiles/acct1");
        std::fs::create_dir_all(&profile).unwrap();
        stage_background_pack(&e, Some(&pack.id), &profile).unwrap();
        let staged = profile.join("config/espectral-menu-bg").join(&pack.id);
        for f in ["pack.json", "frames/0001.png", "frames/0002.png"] {
            assert!(staged.join(f).exists(), "{f}");
        }
        // restage is a no-op while pack.json is identical
        std::fs::write(staged.join("frames/0001.png"), b"mutated").unwrap();
        stage_background_pack(&e, Some(&pack.id), &profile).unwrap();
        assert_eq!(std::fs::read(staged.join("frames/0001.png")).unwrap(), b"mutated");
        // stale packs pruned; a changed pack.json restages
        let stale = profile.join("config/espectral-menu-bg/stale-pack");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(staged.join("pack.json"), "{}").unwrap();
        stage_background_pack(&e, Some(&pack.id), &profile).unwrap();
        assert!(!stale.exists());
        assert_eq!(std::fs::read(staged.join("frames/0001.png")).unwrap(), fake_png(8, 8));
        // deselect -> staging dir removed
        stage_background_pack(&e, None, &profile).unwrap();
        assert!(!profile.join("config/espectral-menu-bg").exists());
        // vanished store pack -> staging dir wiped
        let phantom = d.path().join("profiles-phantom");
        std::fs::create_dir_all(phantom.join("config/espectral-menu-bg/gone")).unwrap();
        stage_background_pack(&e, Some("gone"), &phantom).unwrap();
        assert!(!phantom.join("config/espectral-menu-bg").exists());
    }

    #[tokio::test]
    async fn legacy_migration_without_files_is_none() {
        let (d, e) = temp_engine();
        assert!(migrate_legacy_background_pack(&e, "x", &[d.path().join("nothing")]).await.is_none());
        migrate_legacy_background(&e, "x", &[]).await.unwrap();
    }

    /// Cross-cluster: client.rs (load/patch_client_config) + instances::create_instance.
    #[tokio::test]
    async fn legacy_migration_selects_the_imported_pack() {
        let (d, e) = temp_engine();
        e.update_content_settings(|s| s.background_on_create = false);
        crate::instances::create_instance(
            &e,
            serde_json::from_value(serde_json::json!({ "name": "bg-legacy", "version": "1.21.11" })).unwrap(),
        )
        .await
        .unwrap();
        let inst = d.path().join("legacy-inst");
        let profile = d.path().join("legacy-profile");
        for dir in [&inst, &profile] {
            std::fs::create_dir_all(dir.join("config")).unwrap();
            std::fs::write(dir.join("config").join(LEGACY_BACKGROUND), fake_png(40, 20)).unwrap();
        }
        let meta = migrate_legacy_background_pack(&e, "bg-legacy", &[inst.clone(), profile.clone()]).await.unwrap();
        assert_eq!(meta.kind, "image");
        assert_eq!(selected_background(&e, "bg-legacy").as_deref(), Some(meta.id.as_str()));
        for dir in [&inst, &profile] {
            assert!(!dir.join("config").join(LEGACY_BACKGROUND).exists());
        }
        assert!(migrate_legacy_background_pack(&e, "bg-legacy", &[inst.clone(), profile]).await.is_none());
        let other = import_image_pack(&e, &img("keep", &fake_png(8, 8))).unwrap();
        let patch = PersonalizationPatch { background: Some(Some(other.id.clone())), ..Default::default() };
        write_personalization(&e, "bg-legacy", &patch).await.unwrap();
        std::fs::write(inst.join("config").join(LEGACY_BACKGROUND), fake_png(40, 20)).unwrap();
        migrate_legacy_background_pack(&e, "bg-legacy", &[inst]).await;
        assert_eq!(selected_background(&e, "bg-legacy").as_deref(), Some(other.id.as_str()));
    }
}
