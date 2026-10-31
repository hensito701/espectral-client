//! OWNER: settings-suite-servers agent. Rust side of SettingsDashboard /
//! ClientSuite / ServerRadar (ui/pages/settings-suite-servers).
//!
//! Contract with the shell: keep `pub fn init(ctx, ui)`. Everything lives on
//! the `SettingsBridge` global; each page has its own child module:
//!
//! | module      | page                      | Svelte source                      |
//! |-------------|---------------------------|------------------------------------|
//! | `settings`  | `#/settings`              | pages/SettingsDashboard.svelte     |
//! | `agent`     | `#/settings` agent section| pages/SettingsDashboard.svelte (agent) |
//! | `client`    | `#/client`                | pages/ClientSuite.svelte           |
//! | `servers`   | `#/servers` + card helpers| pages/ServerRadar.svelte, components/ServerRadarCard + PingWaveform |

use crate::bridge::Ctx;
use crate::AppWindow;

mod agent;
mod client;
mod servers;
mod settings;

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    settings::init(ctx, ui);
    agent::init(ctx, ui);
    client::init(ctx, ui);
    servers::init(ctx, ui);
}

/// `e instanceof Error ? e.message : String(e)` for engine errors.
pub(crate) fn err_text(e: &espectral_engine::error::EngineError) -> String {
    e.to_string()
}
