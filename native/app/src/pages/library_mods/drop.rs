//! The window's one winit event callback (Slint allows a single one).
//!
//! OS drag & drop of `.mrpack` files onto the window (the Svelte ImportCenter
//! dropzone, made window-wide): winit `HoveredFile` lights the dropzone,
//! `DroppedFile` selects the pack — in the open MrpackModal if there is one,
//! otherwise in ImportCenter's lane A (navigating there first).
//!
//! It also forwards scale-factor changes to the theme, which keeps the pixel
//! font on whole pixels.
//!
//! Needs Slint's `unstable-winit-030` feature (WinitWindowAccessor); a no-op
//! on non-winit platforms (the headless screenshot renderer).

use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
use slint::ComponentHandle;

use crate::bridge::Ctx;
use crate::single_instance::is_mrpack_arg;
use crate::{router, toast, AppWindow, LibraryBridge, Page};

pub fn init(_ctx: &Ctx, ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |w, ev| {
        let Some(ui) = weak.upgrade() else { return EventResult::Propagate };
        match ev {
            winit::event::WindowEvent::HoveredFile(path) => {
                let is_pack = is_mrpack_arg(&path.to_string_lossy());
                ui.global::<LibraryBridge>().set_dragging_file(is_pack);
            }
            winit::event::WindowEvent::HoveredFileCancelled => {
                ui.global::<LibraryBridge>().set_dragging_file(false);
            }
            winit::event::WindowEvent::DroppedFile(path) => {
                ui.global::<LibraryBridge>().set_dragging_file(false);
                dropped(&ui, &path.to_string_lossy());
            }
            winit::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                crate::theme::set_ui_scale(&ui, *scale_factor as f32);
            }
            // A new window reports scale 1 until it exists; its first size
            // event is the earliest point where the real factor is known.
            winit::event::WindowEvent::Resized(_) => {
                crate::theme::set_ui_scale(&ui, w.scale_factor());
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

fn dropped(ui: &AppWindow, path: &str) {
    if !is_mrpack_arg(path) {
        toast::err("Solo se admiten archivos con extensión .mrpack");
        return;
    }
    let b = ui.global::<LibraryBridge>();
    if b.get_mrpack_open() {
        if b.get_mp_phase() == 0 || b.get_mp_phase() >= 4 {
            b.set_mp_path(path.into());
        }
        return;
    }
    if router::current().page != Page::Import {
        router::navigate(ui, "#/library/import");
    }
    super::import::set_file(path);
}
