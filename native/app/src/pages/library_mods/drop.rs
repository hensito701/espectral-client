//! The window's one winit event callback (Slint allows a single one).
//!
//! OS drag & drop onto the window:
//! * `.mrpack` -> the ImportCenter flow (the open MrpackModal, else ImportCenter's lane A);
//! * `.jar` -> copied into the instance the Mods page shows (else the Home capsule's);
//! * `.zip` -> a resource pack, shader pack or modpack, decided by its contents.
//!
//! Winit reports one `DroppedFile` per file. Jars and zips are queued and installed
//! as one batch once the drops pause: one refresh and one summary toast. The
//! `dragging-file` highlight stays mrpack-only, since it lights ImportCenter's dropzone.
//!
//! It also forwards scale-factor changes to the theme, which keeps the pixel
//! font on whole pixels.
//!
//! Needs Slint's `unstable-winit-030` feature (WinitWindowAccessor); a no-op
//! on non-winit platforms (the headless screenshot renderer).

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use espectral_engine::fabric_compat::{CompatSeverity, ModCompatIssue};
use espectral_engine::packs::ZipPackKind;
use espectral_engine::Engine;
use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Timer, TimerMode};

use super::mods;
use crate::bridge::Ctx;
use crate::single_instance::is_mrpack_arg;
use crate::{i18n, router, toast, AppWindow, HomeBridge, LibraryBridge, Page};

/// Quiet period after the last dropped jar or zip before the batch installs.
const DEBOUNCE: Duration = Duration::from_millis(150);

thread_local! {
    /// Jars and zips dropped since the last flush (UI thread only).
    static QUEUE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Restarted by every drop; fires `flush` once the drops stop.
    static DEBOUNCE_TIMER: Timer = Timer::default();
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let weak = ui.as_weak();
    let ctx = ctx.clone();
    ui.window().on_winit_window_event(move |w, ev| {
        let Some(ui) = weak.upgrade() else { return EventResult::Propagate };
        match ev {
            winit::event::WindowEvent::HoveredFile(path) => {
                ui.global::<LibraryBridge>().set_dragging_file(is_mrpack_arg(&path.to_string_lossy()));
            }
            winit::event::WindowEvent::HoveredFileCancelled => {
                ui.global::<LibraryBridge>().set_dragging_file(false);
            }
            winit::event::WindowEvent::DroppedFile(path) => {
                ui.global::<LibraryBridge>().set_dragging_file(false);
                dropped(&ui, &ctx, &path.to_string_lossy());
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

fn dropped(ui: &AppWindow, ctx: &Ctx, path: &str) {
    if is_mrpack_arg(path) {
        open_mrpack(ui, path);
        return;
    }
    if !has_ext(path, "jar") && !has_ext(path, "zip") {
        toast::err(i18n::t("mods.drop.unsupported"));
        return;
    }
    QUEUE.with(|q| q.borrow_mut().push(path.to_string()));
    let ctx = ctx.clone();
    DEBOUNCE_TIMER.with(|t| t.start(TimerMode::SingleShot, DEBOUNCE, move || flush(&ctx)));
}

/// `.mrpack`: the open MrpackModal takes it, else ImportCenter's lane A.
fn open_mrpack(ui: &AppWindow, path: &str) {
    let b = ui.global::<LibraryBridge>();
    if b.get_mrpack_open() {
        if b.get_mp_phase() == 0 || b.get_mp_phase() >= 4 {
            b.set_mp_path(path.into());
        }
        return;
    }
    navigate_import(ui);
    super::import::set_file(path);
}

fn navigate_import(ui: &AppWindow) {
    if router::current().page != Page::Import {
        router::navigate(ui, "#/library/import");
    }
}

/// Installs everything queued so far into one instance (the one shown at flush time).
fn flush(ctx: &Ctx) {
    let paths = QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if paths.is_empty() {
        return;
    }
    let Some(ui) = ctx.ui() else { return };
    let instance = drop_target(&ui);
    if instance.is_empty() {
        toast::err(i18n::t("mods.drop.noInstance"));
        return;
    }
    ctx.call_with(
        move |e| async move { Ok::<_, espectral_engine::EngineError>(install_batch(&e, &instance, &paths).await) },
        |ui, out| report(ui, out),
        |_ui, e| toast::engine_error(&e),
    );
}

/// The Mods page's instance while that page is open; otherwise the Home capsule's,
/// else the Mods page's last pick.
fn drop_target(ui: &AppWindow) -> String {
    let mods_target = mods::target_instance();
    if router::current().page == Page::Mods && !mods_target.is_empty() {
        return mods_target;
    }
    let capsule = ui.global::<HomeBridge>().get_capsule_instance().to_string();
    if capsule.is_empty() {
        mods_target
    } else {
        capsule
    }
}

/// What one batch did; gathered on the engine, reported on the UI thread.
#[derive(Default)]
struct DropOutcome {
    instance: String,
    /// Stored filenames of the jars copied into `mods/`.
    mods: Vec<String>,
    packs: usize,
    needs_iris: bool,
    /// Nested resource packs rewritten in place (the original is kept under `backups/`).
    unwrapped: usize,
    /// Zips that are modpacks: handed to the mrpack import.
    mrpacks: Vec<String>,
    unknown: Vec<String>,
    failed: Vec<String>,
    /// Compatibility issues of the jars just added.
    compat: Vec<ModCompatIssue>,
}

async fn install_batch(e: &Engine, instance: &str, paths: &[String]) -> DropOutcome {
    let mut out = DropOutcome { instance: instance.to_string(), ..Default::default() };
    for path in paths {
        let file = file_name(path);
        if has_ext(path, "jar") {
            match e.install_local_mod(instance, path) {
                Ok(stored) => out.mods.push(stored),
                Err(err) => out.failed.push(format!("{file}: {err}")),
            }
        } else {
            add_zip(e, instance, path, &file, &mut out).await;
        }
    }
    if !out.mods.is_empty() {
        if let Ok(report) = e.check_mods_compat(instance).await {
            out.compat = report.issues.into_iter().filter(|i| out.mods.contains(&i.filename)).collect();
        }
    }
    out
}

async fn add_zip(e: &Engine, instance: &str, path: &str, file: &str, out: &mut DropOutcome) {
    match e.classify_zip_pack(path) {
        Err(err) => out.failed.push(format!("{file}: {err}")),
        Ok(ZipPackKind::Mrpack) => out.mrpacks.push(path.to_string()),
        Ok(ZipPackKind::Unknown) => out.unknown.push(file.to_string()),
        Ok(ZipPackKind::ResourcePack { .. } | ZipPackKind::ShaderPack) => {
            match e.install_local_pack(instance, path).await {
                Err(err) => out.failed.push(format!("{file}: {err}")),
                Ok(res) => {
                    out.packs += 1;
                    out.needs_iris |= res.needs_iris;
                    if res.nested {
                        match e.unwrap_nested_resource_pack(instance, &res.filename).await {
                            Ok(_) => out.unwrapped += 1,
                            Err(err) => out.failed.push(format!("{file}: {err}")),
                        }
                    }
                }
            }
        }
    }
}

/// Batch result on the UI thread: navigate (which reloads the Mods page), then toast.
fn report(ui: &AppWindow, out: DropOutcome) {
    let instance = out.instance.as_str();
    if let Some(first) = out.mrpacks.first() {
        navigate_import(ui);
        super::import::set_file(first);
    } else if !out.mods.is_empty() {
        // Entering the route reloads the instance's mods and compatibility report.
        router::navigate(ui, &format!("#/mods?instance={}", router::encode_component(instance)));
    }

    let mut parts: Vec<String> = Vec::new();
    match out.mods.as_slice() {
        [] => {}
        [one] => parts.push(i18n::tf("mods.drop.modInstalled", &[("instance", instance), ("name", one)])),
        many => parts.push(i18n::tf(
            "mods.drop.modsInstalled",
            &[("instance", instance), ("count", &many.len().to_string())],
        )),
    }
    if out.packs > 0 {
        parts.push(i18n::tf(
            "mods.drop.packsInstalled",
            &[("instance", instance), ("count", &out.packs.to_string())],
        ));
    }
    if out.needs_iris {
        parts.push(i18n::tf("mods.drop.needsIris", &[("instance", instance)]));
    }
    if out.unwrapped > 0 {
        parts.push(i18n::t("mods.drop.unwrapped"));
    }
    if !out.unknown.is_empty() {
        parts.push(i18n::tf("mods.drop.unknown", &[("files", &out.unknown.join(", "))]));
    }
    if !out.failed.is_empty() {
        parts.push(i18n::tf("mods.drop.failed", &[("detail", &out.failed.join("; "))]));
    }
    if !parts.is_empty() {
        let text = parts.join(" · ");
        if out.failed.is_empty() && out.unknown.is_empty() {
            toast::ok(text);
        } else {
            toast::err(text);
        }
    }
    if !out.compat.is_empty() {
        let issues: Vec<String> =
            out.compat.iter().map(|i| format!("{}: {}", i.mod_name, mods::format_issue_text(i))).collect();
        let text = i18n::tf("mods.drop.compat", &[("issues", &issues.join("; "))]);
        if out.compat.iter().any(|i| i.severity == CompatSeverity::Error) {
            toast::err(text);
        } else {
            toast::info(text);
        }
    }
}

fn has_ext(path: &str, ext: &str) -> bool {
    Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}
