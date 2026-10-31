//! OWNER: home-instances agent. Rust side of Home / InstanceHub / PipLogDrawer (ui/pages/home-instances).
//!
//! * `launchlog` — per-launch ring buffers (lib/stores.ts launchLog) feeding
//!   the PiP drawer, the hub console and the capsule telemetry line.
//! * `home` — rail selection, server strip, capsule (launch / stop / account /
//!   AOT chip), dry-run, instance wizard, .mrpack import.
//! * `hub` — InstanceHub data + every action of its four tabs.
//!
//! Contract with the shell: `pub fn init(ctx, ui)`.

mod home;
mod hub;
mod launchlog;

use espectral_engine::events::EngineEvent;
use slint::ComponentHandle;

use crate::bridge::{self, Ctx};
use crate::{AppWindow, HomeBridge};

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    launchlog::init(ctx, ui);
    home::init(ctx, ui);
    hub::init(ctx, ui);

    // AppState lists changed (PipLogDrawer watches them: it is always mounted).
    let w = ui.as_weak();
    ui.global::<HomeBridge>().on_shared_changed(move || {
        let Some(ui) = w.upgrade() else { return };
        home::shared_changed(&ui);
        hub::shared_changed(&ui);
        launchlog::launches_changed(&ui);
    });

    // Engine events (UI thread, batched by the bridge pump).
    bridge::on_event(|ui, ev: &EngineEvent| {
        launchlog::on_event(ui, ev);
        home::on_event(ui, ev);
        hub::on_event(ui, ev);
    });
}
