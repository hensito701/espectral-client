//! OWNER: library-mods agent. Rust side of VersionArmory / ImportCenter /
//! ModCatalog / MrpackModal (ui/pages/library-mods).
//!
//! Contract with the shell: keep `pub fn init(ctx, ui)`. Every page keeps its
//! cross-visit state in a `thread_local!` of its child module (UI thread only)
//! and mirrors it onto `LibraryBridge`.
//!
//! | module      | page / piece |
//! |-------------|--------------|
//! | `versions`  | VersionArmory (`#/library/versions`) |
//! | `mods`      | ModCatalog (`#/mods`) + `icons` (Modrinth icon LRU) |
//! | `import`    | ImportCenter (`#/library/import`) |
//! | `mrpack`    | MrpackModal (any page hosting `MrpackModal {}`) |
//! | `drop`      | OS drag & drop of `.mrpack` files onto the window |
//! | `donations` | DonationsPavilion (`#/donaciones`) |
//! | `debug`     | screenshot-only states (`ESPECTRAL_SHOT_LIB`) |

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::bridge::Ctx;
use crate::router::{self, Route};
use crate::{AppWindow, LibraryBridge, Page};

mod debug;
mod donations;
mod drop;
mod icons;
mod import;
mod mods;
mod mrpack;
mod versions;

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    versions::init(ctx, ui);
    mods::init(ctx, ui);
    import::init(ctx, ui);
    mrpack::init(ctx, ui);
    drop::init(ctx, ui);
    donations::init(ctx, ui);

    router::on_route(move |ui, route: &Route| match route.page {
        Page::Versions => versions::on_enter(ui),
        Page::Mods => mods::on_enter(ui),
        Page::Import => import::on_enter(ui),
        _ => {}
    });

    // Instances list changed while one of our pages is shown (pages call it
    // from a `changed AppState.instances.length` handler).
    ui.global::<LibraryBridge>().on_instances_changed(|| {
        let Some(ui) = ui_handle() else { return };
        match router::current().page {
            Page::Versions => versions::recompute(&ui),
            Page::Mods => mods::on_instances_changed(&ui),
            Page::Import => import::on_instances_changed(&ui),
            _ => {}
        }
    });
}

thread_local! {
    static CTX: std::cell::RefCell<Option<Ctx>> = const { std::cell::RefCell::new(None) };
}

/// Remember the context (UI thread) so callbacks without captures can reach the window.
fn set_ctx(ctx: &Ctx) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
}

fn ui_handle() -> Option<AppWindow> {
    CTX.with(|c| c.borrow().as_ref().and_then(|c| c.ui()))
}

fn ctx() -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

/// `Vec<T>` -> Slint model.
fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

fn ss(s: impl AsRef<str>) -> SharedString {
    SharedString::from(s.as_ref())
}

/// Last path component (`\` or `/` separators), like ImportCenter extractFilename.
fn file_name_of(path: &str) -> String {
    let clean = path.replace('\\', "/");
    clean.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(file_name_of("C:\\packs\\Fabulously.mrpack"), "Fabulously.mrpack");
        assert_eq!(file_name_of("/home/a/b.mrpack"), "b.mrpack");
        assert_eq!(file_name_of("x.mrpack"), "x.mrpack");
    }
}
