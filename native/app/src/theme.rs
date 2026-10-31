//! Theme + accent (port of lib/theme.svelte.ts and the accent store).
//! FOUNDATION-OWNED. Source of truth: the engine config (`theme`, `accent`),
//! exactly like the Svelte UI which mirrored them to the engine.

use slint::{ComponentHandle, SharedString};

use crate::bridge::Ctx;
use crate::{AppWindow, Shell, Theme};

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

fn set_mode(ctx: &Ctx, mode: String) {
    if let Some(ui) = ctx.ui() {
        ui.global::<Theme>().set_mode(SharedString::from(mode.as_str()));
    }
    ctx.fire(move |e| async move { e.set_theme(&mode).await });
}
