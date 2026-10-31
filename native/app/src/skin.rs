//! Skin viewer glue: `SkinBridge.render` -> espectral-skin3d. OWNER: skin-renderer agent.
//!
//! `source` strings: "account:<username>" (engine.account_skin_png),
//! "library:<id>" (engine.library_skin_png), "file:<path>" (local PNG).
//! Textures load asynchronously and are cached per source; when one arrives
//! SkinBridge.revision is bumped so every SkinView re-renders.

// Public API for page modules: not every helper is used by the shell itself.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use espectral_skin3d::{default_renderer, ArmModel, RenderParams, SkinRenderer, SkinTexture};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::bridge::Ctx;
use crate::{util, AppWindow, SkinBridge, SkinFrameRequest};

thread_local! {
    static TEXTURES: RefCell<HashMap<String, SkinTexture>> = RefCell::new(HashMap::new());
    static LOADING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static RENDERER: Box<dyn SkinRenderer> = default_renderer();
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let c = ctx.clone();
    ui.global::<SkinBridge>().on_render(move |req| render(&c, &req));
    ui.global::<SkinBridge>().on_ready(|source, _rev| TEXTURES.with(|t| t.borrow().contains_key(source.as_str())));
}

/// Forget a cached texture (e.g. after the account skin changed) and reload it.
pub fn invalidate(ui: &AppWindow, source: &str) {
    TEXTURES.with(|t| t.borrow_mut().remove(source));
    let b = ui.global::<SkinBridge>();
    b.set_revision(b.get_revision() + 1);
}

fn render(ctx: &Ctx, req: &SkinFrameRequest) -> Image {
    let source = req.source.to_string();
    if source.is_empty() || req.width <= 0 || req.height <= 0 {
        return Image::default();
    }
    let frame = TEXTURES.with(|t| {
        t.borrow().get(&source).map(|tex| {
            let params = RenderParams {
                width: req.width as u32,
                height: req.height as u32,
                yaw_deg: req.yaw,
                pitch_deg: req.pitch,
                zoom: req.zoom,
                model: ArmModel::from_variant(&req.variant),
                time_s: req.time,
                overlay: true,
            };
            RENDERER.with(|r| r.render(tex, &params))
        })
    });
    match frame {
        Some(f) => Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&f.pixels, f.width, f.height)),
        None => {
            load(ctx, source);
            Image::default()
        }
    }
}

fn load(ctx: &Ctx, source: String) {
    let fresh = LOADING.with(|l| l.borrow_mut().insert(source.clone()));
    if !fresh {
        return;
    }
    let src = source.clone();
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
            Ok(util::decode_png(&bytes))
        },
        move |ui, buf| {
            LOADING.with(|l| l.borrow_mut().remove(&source));
            if let Some(tex) = buf.and_then(|b| SkinTexture::new(b.width(), b.height(), b.as_bytes().to_vec())) {
                TEXTURES.with(|t| t.borrow_mut().insert(source, tex));
                let b = ui.global::<SkinBridge>();
                b.set_revision(b.get_revision() + 1);
            }
        },
        |_ui, e| tracing::debug!("skin texture load failed: {e}"),
    );
}
