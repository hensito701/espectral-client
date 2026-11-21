//! Theme + accent (port of lib/theme.svelte.ts and the accent store).
//! FOUNDATION-OWNED. Source of truth: the engine config (`theme`, `accent`),
//! exactly like the Svelte UI which mirrored them to the engine.

use std::sync::atomic::{AtomicU32, Ordering};

use slint::{ComponentHandle, SharedString};

use crate::bridge::Ctx;
use crate::util;
use crate::{AppState, AppWindow, Shell, Theme};

/// The window's scale factor (f32 bits), readable off the UI thread; 0 until
/// the window reports it.
static UI_SCALE: AtomicU32 = AtomicU32::new(0);

/// `logical` px in physical pixels on this display, rounded up: the size to
/// shrink an image to so it is drawn 1:1 (see [`util::fit`]). 0 (= don't
/// shrink) until the scale is known.
pub fn phys(logical: f32) -> u32 {
    (logical * f32::from_bits(UI_SCALE.load(Ordering::Relaxed))).ceil() as u32
}

/// Apply the persisted theme/accent and wire Shell.set-theme-mode / toggle-theme / set-accent.
pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let cfg = ctx.engine().config();
    let mode = cfg.theme();
    let accent = cfg.accent();
    apply(ui, &mode, &accent);

    let c = ctx.clone();
    ui.global::<Shell>().on_set_theme_mode(move |mode| set_mode(&c, mode.to_string()));
    let c = ctx.clone();
    ui.global::<Shell>().on_toggle_theme(move || {
        let Some(ui) = c.ui() else { return };
        let dark = ui.global::<Theme>().get_dark();
        set_mode(&c, if dark { "light".into() } else { "dark".into() });
    });
    let c = ctx.clone();
    ui.global::<Shell>().on_set_accent(move |id| {
        let id = id.to_string();
        if let Some(ui) = c.ui() {
            ui.global::<Theme>().set_accent_id(SharedString::from(id.as_str()));
        }
        c.fire(move |e| async move { e.set_accent(&id).await });
    });
}

/// Override from the command line (screenshots): "dark" | "light" | "system".
pub fn apply(ui: &AppWindow, mode: &str, accent: &str) {
    let t = ui.global::<Theme>();
    t.set_mode(SharedString::from(mode));
    t.set_accent_id(SharedString::from(accent));
}

/// Tell the theme the window's scale factor (`Theme.pix` snaps the pixel font
/// to it), then redo the images sized for it: the logos and instance icons.
pub fn set_ui_scale(ui: &AppWindow, scale: f32) {
    if UI_SCALE.swap(scale.to_bits(), Ordering::Relaxed) == scale.to_bits() {
        return;
    }
    tracing::info!("[ui] display scale factor {scale}");
    ui.global::<Theme>().set_ui_scale(scale);
    set_logos(ui);
    crate::store::refresh_instances();
}

/// Drawn sizes (logical px) of the round logo: header, login screen, Home.
const LOGO_SIZES: [f32; 3] = [34.0, 64.0, 120.0];

fn set_logos(ui: &AppWindow) {
    let Some(logo) = util::decode_png(include_bytes!("../ui/assets/logo_circle.png")) else { return };
    let [sm, md, lg] = LOGO_SIZES.map(|px| util::to_image(util::fit(logo.clone(), phys(px))));
    let s = ui.global::<AppState>();
    s.set_logo_sm(sm);
    s.set_logo_md(md);
    s.set_logo_lg(lg);
}

fn set_mode(ctx: &Ctx, mode: String) {
    if let Some(ui) = ctx.ui() {
        ui.global::<Theme>().set_mode(SharedString::from(mode.as_str()));
    }
    ctx.fire(move |e| async move { e.set_theme(&mode).await });
}
