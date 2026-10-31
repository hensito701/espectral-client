//! OWNER: library-mods agent. Rust side of VersionArmory / ImportCenter / ModCatalog / DonationsPavilion (ui/pages/library-mods).
//!
//! Contract with the shell: keep `pub fn init(ctx, ui)`; everything else in
//! this file (and any `pages/library_mods/*.rs` child modules you add with
//! `mod x;` here) is yours. Read native/app/UI-PORTING.md first.

#![allow(unused_imports)]

use espectral_engine::events::EngineEvent;
use slint::ComponentHandle;

use crate::bridge::{self, Ctx};
use crate::router::{self, Route};
use crate::{toast, AppWindow, LibraryBridge, Page};

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let _bridge = ui.global::<LibraryBridge>();

    // Load data whenever one of this group's pages is entered.
    let c = ctx.clone();
    router::on_route(move |_ui, route: &Route| {
        if matches!(route.page, Page::Versions | Page::Import | Page::Mods | Page::Donations) {
            let _ = &c; // e.g. c.call(|e| async move { e.list_versions().await }, |ui, v| { ... });
        }
    });

    // React to engine events (UI thread).
    bridge::on_event(|_ui, _ev: &EngineEvent| {});
}
