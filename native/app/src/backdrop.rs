//! The launcher's background (`Backdrop` global). FOUNDATION-OWNED.
//!
//! Two modes: "image" (default) shows one picture cover-fit with the in-game
//! menu's shading; "glass" shows the code-drawn glows (over Windows 11's
//! acrylic on a glass window). The five shipped pictures are embedded
//! (`builtin:<n>`); the player's own live in `<data dir>/launcher-backgrounds/`
//! (`custom:<file name>`).
//!
//! All decoding runs on `spawn_blocking`; the full picture is shrunk to
//! `util::fit(buf, 2560)` (never upscaled) and only the current one is held.
//! In glass mode `image` stays empty — the decode only happens when the mode
//! switches back to "image".

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::bridge::Ctx;
use crate::{dialogs, i18n, prefs, util, AppWindow, Backdrop, BackdropItem};

/// Shipped pictures: `launcher-bg-1.jpg` is the default (`builtin:1`).
const BUILTIN: [&[u8]; 5] = [
    include_bytes!("../ui/assets/backgrounds/launcher-bg-1.jpg"),
    include_bytes!("../ui/assets/backgrounds/launcher-bg-2.jpg"),
    include_bytes!("../ui/assets/backgrounds/launcher-bg-3.jpg"),
    include_bytes!("../ui/assets/backgrounds/launcher-bg-4.jpg"),
    include_bytes!("../ui/assets/backgrounds/launcher-bg-5.jpg"),
];

const CUSTOM_DIR: &str = "launcher-backgrounds";
const CUSTOM_PREFIX: &str = "custom:";
/// Longest side of the kept full picture (it covers the window, cover-fit).
const FULL_EDGE: u32 = 2560;
/// Thumb edge when the display scale is not known yet.
const THUMB_FALLBACK: u32 = 240;
/// Refused on import: bigger files are skipped (decode cap below still applies).
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
/// Refused at decode time (headers and full picture): keeps a hostile file
/// from allocating gigabytes.
const MAX_SIDE: u32 = 8192;
const MAX_PIXELS: u64 = 36_000_000;

/// ui-prefs.json keys: "image" (default) | "glass"; the selected item key.
const PREF_MODE: &str = "backdrop.mode";
const PREF_CURRENT: &str = "backdrop.current";

thread_local! {
    /// Key of the picture currently in `Backdrop.image` — keeps `ensure_picture`
    /// from decoding again on every mode flip.
    static LOADED: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn loaded() -> Option<String> {
    LOADED.with(|l| l.borrow().clone())
}

fn mark_loaded(key: Option<&str>) {
    LOADED.with(|l| *l.borrow_mut() = key.map(str::to_string));
}

fn custom_dir(ctx: &Ctx) -> PathBuf {
    ctx.engine().paths().data_dir().join(CUSTOM_DIR)
}

/// Bytes to decode: embedded for `builtin:<n>`, a file for `custom:<name>`.
enum Source {
    Bytes(&'static [u8]),
    File(PathBuf),
}

fn source(ctx: &Ctx, key: &str) -> Option<Source> {
    if let Some(n) = key.strip_prefix("builtin:") {
        let i = n.parse::<usize>().ok()?.checked_sub(1)?;
        return Some(Source::Bytes(BUILTIN.get(i)?));
    }
    let name = key.strip_prefix(CUSTOM_PREFIX)?;
    valid_name(name).then(|| Source::File(custom_dir(ctx).join(name)))
}

/// A file name that stays inside `launcher-backgrounds/` (rejects path parts).
fn valid_name(name: &str) -> bool {
    !(name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == "..")
}

/// Decode any supported format, shrunk to fit `edge`×`edge` (never upscaled).
fn decode(source: Source, edge: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    match source {
        Source::Bytes(b) => decode_bytes(b, edge),
        Source::File(p) => decode_bytes(&std::fs::read(p).ok()?, edge),
    }
}

/// Decode any supported format, shrunk to fit `edge`×`edge` (never upscaled).
fn decode_bytes(bytes: &[u8], edge: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let (w, h) =
        image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()?;
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(util::fit(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()), edge))
}

/// Item keys (no thumbs): the shipped pictures first, then the player's own
/// files sorted by name. Runs off the UI thread.
fn item_keys(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    (1..=BUILTIN.len())
        .map(|i| format!("builtin:{i}"))
        .chain(names.into_iter().map(|n| format!("{CUSTOM_PREFIX}{n}")))
        .collect()
}

/// Rebuild `Backdrop.items` with decoded thumbs (off the UI thread), then run
/// `then` on the UI thread.
fn refresh_items(ctx: &Ctx, then: impl FnOnce(&AppWindow) + Send + 'static) {
    let c = ctx.clone();
    let dir = custom_dir(ctx);
    ctx.spawn(async move {
        let edge = match crate::theme::phys(200.0) {
            0 => THUMB_FALLBACK,
            e => e,
        };
        let rows: Vec<(String, Option<SharedPixelBuffer<Rgba8Pixel>>, bool)> = tokio::task::spawn_blocking(move || {
            item_keys(&dir)
                .into_iter()
                .map(|key| {
                    let custom = key.starts_with(CUSTOM_PREFIX);
                    // Thumbs decode by key; built-ins are embedded, custom
                    // files resolve in `dir`. `source` needs the ctx for
                    // the data dir — recompute it here instead.
                    let src = if custom {
                        Some(Source::File(dir.join(&key[CUSTOM_PREFIX.len()..])))
                    } else {
                        key.strip_prefix("builtin:")
                            .and_then(|n| n.parse::<usize>().ok())
                            .and_then(|i| BUILTIN.get(i - 1))
                            .map(|b| Source::Bytes(b))
                    };
                    let thumb = src.and_then(|s| decode(s, edge));
                    (key, thumb, custom)
                })
                .collect()
        })
        .await
        .unwrap_or_default();
        c.on_ui(move |ui| {
            let items = rows
                .into_iter()
                .map(|(key, thumb, custom)| BackdropItem {
                    key: SharedString::from(key),
                    thumb: thumb.map(util::to_image).unwrap_or_default(),
                    custom,
                })
                .collect::<Vec<_>>();
            ui.global::<Backdrop>().set_items(ModelRc::new(VecModel::from(items)));
            then(ui);
        });
    });
}

/// Index of `current` in `items`, and the row count.
fn current_index(b: Backdrop) -> (usize, usize) {
    let items = b.get_items();
    let n = items.row_count();
    let cur = b.get_current();
    let idx = (0..n).position(|i| items.row_data(i).is_some_and(|it| it.key == cur)).unwrap_or(0);
    (idx, n)
}

/// Wrap-around index `idx + step` in a list of `n` (`step` is ±1). Pure helper.
fn step_idx(idx: usize, n: usize, step: isize) -> usize {
    if n == 0 {
        0
    } else {
        ((idx as isize + step).rem_euclid(n as isize)) as usize
    }
}

/// Select the item at `idx` (already clamped by the caller): persist it and
/// decode it. The old picture stays until the new one lands — no black flash.
fn select_index(ctx: &Ctx, ui: &AppWindow, idx: usize) {
    let b = ui.global::<Backdrop>();
    let Some(it) = b.get_items().row_data(idx) else { return };
    select_key(ctx, ui, it.key.as_ref());
}

fn select_key(ctx: &Ctx, ui: &AppWindow, key: &str) {
    let b = ui.global::<Backdrop>();
    if b.get_current() == key {
        ensure_picture(ctx, ui);
        return;
    }
    b.set_current(SharedString::from(key));
    prefs::set(PREF_CURRENT, serde_json::Value::String(key.to_string()));
    ensure_picture(ctx, ui);
}

/// Decode the current picture (unless already decoded / in glass mode) and set
/// `image` + `has-image`. Stale results (the key or mode changed meanwhile)
/// are dropped.
fn ensure_picture(ctx: &Ctx, ui: &AppWindow) {
    let b = ui.global::<Backdrop>();
    if b.get_mode() != "image" {
        return;
    }
    let key = b.get_current().to_string();
    if loaded().as_deref() == Some(key.as_str()) {
        return;
    }
    let Some(src) = source(ctx, &key) else {
        mark_loaded(None);
        b.set_image(Image::default());
        b.set_has_image(false);
        return;
    };
    let c = ctx.clone();
    ctx.spawn(async move {
        let buf = tokio::task::spawn_blocking(move || decode(src, FULL_EDGE)).await.ok().flatten();
        c.on_ui(move |ui| {
            let b = ui.global::<Backdrop>();
            if b.get_mode() != "image" || b.get_current() != key {
                return;
            }
            match buf {
                Some(buf) => {
                    b.set_image(util::to_image(buf));
                    b.set_has_image(true);
                    mark_loaded(Some(&key));
                }
                None => {
                    tracing::warn!("[backdrop] could not decode {key}");
                    b.set_image(Image::default());
                    b.set_has_image(false);
                    mark_loaded(None);
                }
            }
        });
    });
}

/// A unique destination inside `dir`: `name`, or `stem-2.ext`, `stem-3.ext`, …
fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let cand = dir.join(name);
    if !cand.exists() {
        return cand;
    }
    let (stem, ext) = name.rsplit_once('.').map(|(s, e)| (s, format!(".{e}"))).unwrap_or((name, String::new()));
    for i in 2u32.. {
        let cand = dir.join(format!("{stem}-{i}{ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    unreachable!()
}

/// The src and dst paths name the same file (a picture re-picked from inside
/// `launcher-backgrounds/`). `dst` must exist.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Copy the picked files into the player's folder. Returns the keys of the
/// pictures that were actually added; files over [`MAX_FILE_BYTES`], files
/// that fail to decode and unreadable names are skipped (warned, not toasted —
/// one bad pick should not hide the others).
fn import_files(dir: &Path, files: Vec<PathBuf>) -> Vec<String> {
    if files.is_empty() {
        return Vec::new();
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("[backdrop] could not create {}: {e}", dir.display());
        return Vec::new();
    }
    let mut added = Vec::new();
    for src in files {
        let Some(name) = src.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            tracing::warn!("[backdrop] skipping {}: no usable file name", src.display());
            continue;
        };
        if !valid_name(&name) {
            continue;
        }
        // Re-picking a file that already lives in the folder selects it
        // instead of duplicating it.
        let (name, dest) = if same_file(&src, &dir.join(&name)) {
            (name.clone(), dir.join(&name))
        } else {
            let dest = unique_dest(dir, &name);
            (dest.file_name().and_then(|n| n.to_str()).unwrap_or(&name).to_string(), dest)
        };
        let meta = match std::fs::metadata(&src) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("[backdrop] skipping {}: {e}", src.display());
                continue;
            }
        };
        if meta.len() > MAX_FILE_BYTES {
            tracing::warn!("[backdrop] skipping {}: over {MAX_FILE_BYTES} bytes", src.display());
            continue;
        }
        let bytes = match std::fs::read(&src) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("[backdrop] skipping {}: {e}", src.display());
                continue;
            }
        };
        if decode_bytes(&bytes, FULL_EDGE).is_none() {
            tracing::warn!("[backdrop] skipping {}: not a readable image", src.display());
            continue;
        }
        // No copy needed when the file already is the destination.
        if !same_file(&src, &dest) {
            if let Err(e) = std::fs::write(&dest, &bytes) {
                tracing::warn!("[backdrop] could not copy to {}: {e}", dest.display());
                continue;
            }
        }
        added.push(format!("{CUSTOM_PREFIX}{name}"));
    }
    added
}

/// Apply the persisted mode/current, publish items (decoded off the UI
/// thread) and wire the `Backdrop` callbacks (UI thread, once).
pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let b = ui.global::<Backdrop>();
    let mode = match prefs::get_str(PREF_MODE).as_deref() {
        Some("glass") => "glass",
        _ => "image",
    };
    b.set_mode(SharedString::from(mode));
    b.set_current(SharedString::from(prefs::get_str(PREF_CURRENT).unwrap_or_else(|| "builtin:1".into())));
    mark_loaded(None);

    // Items + a valid current key, then the picture (skipped in glass mode).
    let c = ctx.clone();
    refresh_items(ctx, move |ui| {
        let b = ui.global::<Backdrop>();
        let items = b.get_items();
        let cur = b.get_current();
        if (0..items.row_count()).any(|i| items.row_data(i).is_some_and(|it| it.key == cur)) {
            ensure_picture(&c, ui);
        } else {
            // `current` is a stale key (e.g. a deleted custom file): fall back
            // to the default picture.
            select_key(&c, ui, "builtin:1");
        }
    });

    let c = ctx.clone();
    b.on_set_mode(move |mode| {
        let Some(ui) = c.ui() else { return };
        let mode = if mode == "glass" { "glass" } else { "image" };
        let b = ui.global::<Backdrop>();
        if b.get_mode() == mode {
            return;
        }
        b.set_mode(SharedString::from(mode));
        prefs::set(PREF_MODE, serde_json::Value::String(mode.to_string()));
        if mode == "glass" {
            // Release the full picture; the decode is redone when the mode
            // goes back to "image".
            b.set_image(Image::default());
            b.set_has_image(false);
            mark_loaded(None);
        } else {
            ensure_picture(&c, &ui);
        }
    });

    let c = ctx.clone();
    b.on_prev(move || {
        let Some(ui) = c.ui() else { return };
        let (idx, n) = current_index(ui.global::<Backdrop>());
        select_index(&c, &ui, step_idx(idx, n, -1));
    });
    let c = ctx.clone();
    b.on_next(move || {
        let Some(ui) = c.ui() else { return };
        let (idx, n) = current_index(ui.global::<Backdrop>());
        select_index(&c, &ui, step_idx(idx, n, 1));
    });
    let c = ctx.clone();
    b.on_select(move |key| {
        let Some(ui) = c.ui() else { return };
        select_key(&c, &ui, key.as_ref());
    });

    let c = ctx.clone();
    b.on_add_custom(move || {
        let c2 = c.clone();
        let title = i18n::t("perso.bgImportImage");
        c.spawn(async move {
            let files = dialogs::pick_files(&title, &[("Imágenes / Images", &["png", "jpg", "jpeg", "webp"])]).await;
            if files.is_empty() {
                return;
            }
            let dir = custom_dir(&c2);
            let added = tokio::task::spawn_blocking(move || import_files(&dir, files)).await.unwrap_or_default();
            if added.is_empty() {
                return;
            }
            let c3 = c2.clone();
            refresh_items(&c2, move |ui| select_key(&c3, ui, &added[0]));
        });
    });

    let c = ctx.clone();
    b.on_remove(move |key| {
        let Some(ui) = c.ui() else { return };
        let key = key.to_string();
        let Some(name) = key.strip_prefix(CUSTOM_PREFIX).map(str::to_string).filter(|n| valid_name(n)) else { return };
        // Remember where the removed item sat: after the rebuild the previous
        // picture is the one now at its index.
        let (old_idx, _) = current_index(ui.global::<Backdrop>());
        let dir = custom_dir(&c);
        let c2 = c.clone();
        c.spawn(async move {
            let path = dir.join(name);
            let path2 = path.clone();
            match tokio::task::spawn_blocking(move || std::fs::remove_file(&path)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::warn!("[backdrop] could not delete {}: {e}", path2.display());
                    return;
                }
                Err(e) => {
                    tracing::warn!("[backdrop] remove task failed: {e}");
                    return;
                }
            }
            let c3 = c2.clone();
            refresh_items(&c2, move |ui| {
                let b = ui.global::<Backdrop>();
                if b.get_current() != key {
                    return; // the player moved on meanwhile
                }
                let n = b.get_items().row_count();
                if n == 0 {
                    select_key(&c3, ui, "builtin:1");
                } else {
                    select_index(&c3, ui, old_idx.saturating_sub(1).min(n - 1));
                }
            });
        });
    });
}

/// Screenshot mode (`--screenshot`, `run_screenshot`): deterministic state.
/// The picture is decoded synchronously so a tiny `--wait` still shows it;
/// `--backdrop glass` renders the glow mode instead. Nothing is persisted.
pub fn apply_for_shot(ctx: &Ctx, ui: &AppWindow, mode: Option<&str>) {
    let b = ui.global::<Backdrop>();
    b.set_current(SharedString::from("builtin:1"));
    if mode == Some("glass") {
        b.set_mode(SharedString::from("glass"));
        b.set_image(Image::default());
        b.set_has_image(false);
        mark_loaded(None);
        return;
    }
    b.set_mode(SharedString::from("image"));
    match source(ctx, "builtin:1").and_then(|s| decode(s, FULL_EDGE)) {
        Some(buf) => {
            b.set_image(util::to_image(buf));
            b.set_has_image(true);
            mark_loaded(Some("builtin:1"));
        }
        None => {
            b.set_image(Image::default());
            b.set_has_image(false);
            mark_loaded(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_around() {
        assert_eq!(step_idx(0, 5, -1), 4);
        assert_eq!(step_idx(4, 5, 1), 0);
        assert_eq!(step_idx(2, 5, 1), 3);
        assert_eq!(step_idx(2, 5, -1), 1);
        assert_eq!(step_idx(0, 0, 1), 0);
        assert_eq!(step_idx(0, 1, -1), 0);
    }

    #[test]
    fn unique_names() {
        let dir = tempfile::tempdir().unwrap();
        let first = unique_dest(dir.path(), "pic.png");
        assert_eq!(first.file_name().unwrap(), "pic.png");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_dest(dir.path(), "pic.png");
        assert_eq!(second.file_name().unwrap(), "pic-2.png");
        std::fs::write(&second, b"x").unwrap();
        assert_eq!(unique_dest(dir.path(), "pic.png").file_name().unwrap(), "pic-3.png");
        assert_eq!(unique_dest(dir.path(), "noext").file_name().unwrap(), "noext");
    }

    #[test]
    fn refuses_escaping_names() {
        for bad in ["", "..", ".", "a/b", "a\\b", "../x.png"] {
            assert!(!valid_name(bad));
        }
        for ok in ["pic.png", "mi foto.jpg", "a.b.webp"] {
            assert!(valid_name(ok));
        }
    }
}
