//! Page groups. FOUNDATION-OWNED list: each group owns exactly one module
//! file here (plus any child modules it creates under `pages/<group>/`).
//!
//! | module                    | owner agent             | Slint files                         |
//! |---------------------------|-------------------------|-------------------------------------|
//! | `home_instances`          | home-instances          | ui/pages/home-instances/*           |
//! | `library_mods`            | library-mods            | ui/pages/library-mods/*             |
//! | `account_wardrobe`        | account-wardrobe        | ui/pages/account-wardrobe/*         |
//! | `settings_suite_servers`  | settings-suite-servers  | ui/pages/settings-suite-servers/*   |
//! | `crate::skin`             | skin-renderer           | ui/skin3d/*, native/skin3d/         |

use crate::bridge::Ctx;
use crate::AppWindow;

pub mod account_wardrobe;
pub mod home_instances;
pub mod library_mods;
pub mod settings_suite_servers;

/// Called once on the UI thread after the engine is ready, before the window shows.
pub fn init_all(ctx: &Ctx, ui: &AppWindow) {
    home_instances::init(ctx, ui);
    library_mods::init(ctx, ui);
    account_wardrobe::init(ctx, ui);
    settings_suite_servers::init(ctx, ui);
}
