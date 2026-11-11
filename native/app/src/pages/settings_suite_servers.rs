//! Rust side of SettingsDashboard (#/settings) and the ServerRadarCard helpers
//! used by the Home server strip (ui/pages/settings-suite-servers).
//!
//! Contract with the shell: keep `pub fn init(ctx, ui)`. Everything lives on
//! the `SettingsBridge` global; each page has its own child module:
//!
//! | module       | page                       | Svelte source                       |
//! |--------------|----------------------------|-------------------------------------|
//! | `settings`   | `#/settings`               | pages/SettingsDashboard.svelte      |
//! | `agent`      | `#/settings` agent section | pages/SettingsDashboard.svelte (agent) |
//! | `radar_card` | Home server strip helpers  | components/ServerRadarCard + lib/motd |

use crate::bridge::Ctx;
use crate::AppWindow;

mod agent;
mod radar_card;
mod settings;

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    settings::init(ctx, ui);
    agent::init(ctx, ui);
    radar_card::init(ctx, ui);
}

/// `e instanceof Error ? e.message : String(e)` for engine errors.
pub(crate) fn err_text(e: &espectral_engine::error::EngineError) -> String {
    e.to_string()
}
