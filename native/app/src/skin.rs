//! Skin viewer glue: `SkinBridge` <-> espectral-skin3d. OWNER: skin-renderer agent.
//!
//! `source` strings: "account:<username>" (engine.account_skin_png),
//! "library:<id>" (engine.library_skin_png), "file:<path>" (local PNG).
//! Textures load + decode + normalise (legacy 64x32 -> 64x64) on the tokio
//! runtime and are cached per source on the UI thread; when one arrives (or
//! fails) `SkinBridge.revision` is bumped so every `SkinView` re-evaluates.
//! Frames are rasterized on the UI thread inside the `render` pure callback
//! (~1 ms for 300x400 in release), only when an input of the frame changed.

// Public API for page modules: not every helper is used by the shell itself.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use espectral_skin3d::{ArmModel, Pose, Rasterizer, RgbaFrame, Scene, SkinTexture};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::bridge::Ctx;
use crate::{util, AppWindow, SkinBridge, SkinFrameRequest};

thread_local! {
    static TEXTURES: RefCell<HashMap<String, SkinTexture>> = RefCell::new(HashMap::new());
    /// Sources whose last load failed (cleared by `invalidate`).
    static FAILED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// In-flight loads: source -> generation (a newer `invalidate` discards older results).
    static LOADING: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
    static GENERATION: RefCell<u64> = const { RefCell::new(0) };
    static RASTER: RefCell<(Rasterizer, RgbaFrame)> = RefCell::new((Rasterizer::new(), RgbaFrame::transparent(0, 0)));
}

/// Largest frame edge we rasterize (physical px); bigger views are upscaled (pixelated).
const MAX_EDGE: i32 = 1600;

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let c = ctx.clone();
    let b = ui.global::<SkinBridge>();
    b.on_render(move |req| render(&c, &req));
    b.on_ready(|source, _rev| TEXTURES.with(|t| t.borrow().contains_key(source.as_str())));
    b.on_failed(|source, _rev| FAILED.with(|f| f.borrow().contains(source.as_str())));
    let c = ctx.clone();
    b.on_preload(move |source| {
        let source = source.to_string();
        let known = source.is_empty()
            || TEXTURES.with(|t| t.borrow().contains_key(&source))
            || FAILED.with(|f| f.borrow().contains(&source));
        if !known {
            load(&c, source);
        }
    });
    sync_window_height(ui);
}

/// Forget a cached texture (e.g. after the account skin changed) and reload it
/// the next time a `SkinView` shows it.
pub fn invalidate(ui: &AppWindow, source: &str) {
    invalidate_where(ui, |s| s == source);
}

/// [`invalidate`] every source starting with `prefix` (e.g. `"library:"` after
/// the gallery changed, `"account:"` after an account list refresh).
pub fn invalidate_prefix(ui: &AppWindow, prefix: &str) {
    invalidate_where(ui, |s| s.starts_with(prefix));
}

fn invalidate_where(ui: &AppWindow, pred: impl Fn(&str) -> bool) {
    TEXTURES.with(|t| t.borrow_mut().retain(|k, _| !pred(k)));
    FAILED.with(|f| f.borrow_mut().retain(|k| !pred(k)));
    LOADING.with(|l| l.borrow_mut().retain(|k, _| !pred(k)));
    bump(ui);
}

fn bump(ui: &AppWindow) {
    let b = ui.global::<SkinBridge>();
    b.set_revision(b.get_revision().wrapping_add(1));
}

/// Keeps `SkinBridge.window-height` (logical px) current so off-screen views
/// stop their animation timer. Called from `render`, which runs whenever a
/// view is laid out / resized / animated.
fn sync_window_height(ui: &AppWindow) {
    let w = ui.window();
    let h = w.size().height as f32 / w.scale_factor().max(0.1);
    let b = ui.global::<SkinBridge>();
    if (b.get_window_height() - h).abs() >= 1.0 {
        b.set_window_height(h);
    }
}

fn render(ctx: &Ctx, req: &SkinFrameRequest) -> Image {
    let source = req.source.to_string();
    if source.is_empty() || req.width <= 0 || req.height <= 0 {
        return Image::default();
    }
    let Some(ui) = ctx.ui() else { return Image::default() };
    // Deferred: never write properties while a binding is being evaluated.
    let weak = ctx.weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        if let Some(ui) = weak.upgrade() {
            sync_window_height(&ui);
        }
    });
    // Keep the aspect ratio when clamping huge views.
    let k = (MAX_EDGE as f32 / req.width.max(req.height) as f32).min(1.0);
    let (w, h) = (((req.width as f32 * k) as u32).max(1), ((req.height as f32 * k) as u32).max(1));
    // Supersample still frames on standard-density screens (HiDPI is already sharp).
    let antialias = req.still && ui.window().scale_factor() < 1.5;
    let pixels = TEXTURES.with(|t| {
        let t = t.borrow();
        let tex = t.get(&source)?;
        let scene = Scene {
            zoom: if req.zoom > 0.0 { req.zoom } else { 1.0 },
            antialias,
            ..Scene::new(w, h, ArmModel::from_variant(&req.variant), Pose::idle(req.yaw, req.pitch, req.time))
        };
        Some(RASTER.with(|r| {
            let (raster, frame) = &mut *r.borrow_mut();
            raster.render_into(tex, &scene, frame);
            SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&frame.pixels, frame.width, frame.height)
        }))
    });
    match pixels {
        Some(buf) => Image::from_rgba8(buf),
        None => {
            if !FAILED.with(|f| f.borrow().contains(&source)) {
                load(ctx, source);
            }
            Image::default()
        }
    }
}

fn load(ctx: &Ctx, source: String) {
    if LOADING.with(|l| l.borrow().contains_key(&source)) {
        return;
    }
    let generation = GENERATION.with(|g| {
        let mut g = g.borrow_mut();
        *g += 1;
        *g
    });
    LOADING.with(|l| l.borrow_mut().insert(source.clone(), generation));
    let src = source.clone();
    let (ok_src, err_src) = (source.clone(), source);
    ctx.call_with(
        move |e| async move {
            let bytes = if let Some(u) = src.strip_prefix("account:") {
                e.account_skin_png(u).await?
            } else if let Some(id) = src.strip_prefix("library:") {
                e.library_skin_png(id).await?
            } else if let Some(p) = src.strip_prefix("file:") {
                tokio::fs::read(p).await?
            } else {
                return Err(espectral_engine::EngineError::other(format!("unknown skin source {src}")));
            };
            // Decode + normalise off the UI thread.
            Ok(util::decode_png(&bytes).and_then(|b| SkinTexture::new(b.width(), b.height(), b.as_bytes().to_vec())))
        },
        move |ui, tex| {
            if !finish(&ok_src, generation) {
                return;
            }
            match tex {
                Some(tex) => TEXTURES.with(|t| t.borrow_mut().insert(ok_src, tex)),
                None => {
                    tracing::debug!("skin texture {ok_src}: not a 64x64 / 64x32 PNG");
                    FAILED.with(|f| f.borrow_mut().insert(ok_src));
                    None
                }
            };
            bump(ui);
        },
        move |ui, e| {
            if !finish(&err_src, generation) {
                return;
            }
            tracing::debug!("skin texture {err_src} failed: {e}");
            FAILED.with(|f| f.borrow_mut().insert(err_src));
            bump(ui);
        },
    );
}

/// Ends the in-flight load of `source`; false when it was invalidated meanwhile.
fn finish(source: &str, generation: u64) -> bool {
    LOADING.with(|l| {
        let mut l = l.borrow_mut();
        if l.get(source) == Some(&generation) {
            l.remove(source);
            true
        } else {
            false
        }
    })
}
