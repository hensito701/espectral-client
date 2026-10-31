//! SettingsDashboard (#/settings): engine config form, theme / accent /
//! language (via the Shell callbacks), JDK probe, data folders, diagnostics.

use std::time::Duration;

use espectral_engine::api::core::ConfigPatch;
use espectral_engine::types::{AppConfig, ClientPatch};
use slint::{ComponentHandle, SharedString};

use super::err_text;
use crate::bridge::Ctx;
use crate::router::{self, Route};
use crate::{i18n, toast, util, AppWindow, Page, SettingsBridge};

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let b = ui.global::<SettingsBridge>();

    let c = ctx.clone();
    b.on_settings_reload(move || load_config(&c));

    let c = ctx.clone();
    b.on_save_memory(move |mb| {
        if !(512..=65536).contains(&mb) {
            return;
        }
        save_field(
            &c,
            "memory",
            ConfigPatch { default_memory_mb: Some(Some(mb as i64)), ..Default::default() },
            i18n::t("settings.flash.memory"),
            false,
        );
    });

    let c = ctx.clone();
    b.on_save_memory_auto(move || {
        let Some(ui) = c.ui() else { return };
        ui.global::<SettingsBridge>().set_saving_field("memory".into());
        c.call_with(
            |e| async move { e.patch_config(ConfigPatch { default_memory_mb: Some(None), ..Default::default() }).await },
            |ui, cfg| {
                let b = ui.global::<SettingsBridge>();
                b.set_saving_field("".into());
                b.set_memory(cfg.default_memory_mb as f32);
                apply_config(ui, &cfg, false);
                toast::ok(i18n::t("settings.flash.memory"));
            },
            |ui, e| {
                ui.global::<SettingsBridge>().set_saving_field("".into());
                toast::err(err_text(&e));
            },
        );
    });

    let c = ctx.clone();
    b.on_save_concurrency(move |n| {
        let n = n.clamp(1, 16);
        if let Some(ui) = c.ui() {
            ui.global::<SettingsBridge>().set_concurrency(n as f32);
        }
        save_field(
            &c,
            "concurrency",
            ConfigPatch { download_concurrency: Some(n as i64), ..Default::default() },
            i18n::t("settings.flash.concurrency"),
            false,
        );
    });

    let c = ctx.clone();
    b.on_save_jdk(move |path| {
        save_field(
            &c,
            "jdk",
            ConfigPatch { jdk_path_override: Some(Some(path.trim().to_string())), ..Default::default() },
            i18n::t("settings.flash.jdk"),
            false,
        );
    });

    let c = ctx.clone();
    b.on_save_flag(move |field, on| {
        let field = field.to_string();
        let mut patch = ConfigPatch::default();
        let (on_key, off_key) = match field.as_str() {
            "aot" => {
                patch.aot_auto_train = Some(on);
                ("settings.flash.aotOn", "settings.flash.aotOff")
            }
            "fastboot" => {
                patch.fast_boot = Some(on);
                ("settings.flash.fastBootOn", "settings.flash.fastBootOff")
            }
            "discord" => {
                patch.discord_enabled = Some(on);
                ("settings.flash.discordOn", "settings.flash.discordOff")
            }
            "closeOnStart" => {
                patch.close_on_game_start = Some(on);
                ("settings.flash.closeOnStartOn", "settings.flash.closeOnStartOff")
            }
            // The engine whitelist (JS sanitizeConfigPatch too) has no
            // fullbright_on_launch: the PATCH is a no-op, exactly like the Svelte UI.
            "fullbright" => ("settings.flash.fullbrightOn", "settings.flash.fullbrightOff"),
            _ => return,
        };
        let text = i18n::t(if on { on_key } else { off_key });
        // close-on-start reverts the switch on failure (it is not part of the batch save)
        save_field(&c, &field, patch, text, field == "closeOnStart");
    });

    let c = ctx.clone();
    b.on_save_all(move || save_all(&c));

    let c = ctx.clone();
    b.on_reset_all(move || {
        let Some(ui) = c.ui() else { return };
        reset_drafts_from_cfg(&ui);
        toast::info(i18n::t("dash.reloading"));
    });

    let c = ctx.clone();
    b.on_probe_jdk(move || probe(&c));

    let c = ctx.clone();
    b.on_toggle_reduced_motion(move || toggle_reduced_motion(&c));

    let c = ctx.clone();
    b.on_open_data_folder(move |sub| {
        let Some(ui) = c.ui() else { return };
        let root = ui.global::<SettingsBridge>().get_cfg_data_dir().to_string();
        if root.is_empty() {
            return;
        }
        let clean = root.trim_end_matches(['/', '\\']).to_string();
        let path = if sub.is_empty() {
            clean
        } else {
            let sep = if clean.contains('\\') { '\\' } else { '/' };
            format!("{clean}{sep}{sub}")
        };
        c.call_with(
            move |e| async move { e.open_folder(&path).await },
            |_ui, _| {},
            |_ui, _e| toast::err(i18n::t("settings.openFolderError")),
        );
    });

    let c = ctx.clone();
    b.on_copy_data_dir(move || {
        let Some(ui) = c.ui() else { return };
        let b = ui.global::<SettingsBridge>();
        util::copy_text(&b.get_cfg_data_dir());
        b.set_data_dir_copied(true);
        let weak = c.weak();
        slint::Timer::single_shot(Duration::from_millis(2000), move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<SettingsBridge>().set_data_dir_copied(false);
            }
        });
    });

    let c = ctx.clone();
    b.on_recheck_health(move || {
        let Some(ui) = c.ui() else { return };
        ui.global::<SettingsBridge>().set_checking_health(true);
        c.call_with(
            |e| async move { e.health().await },
            |ui, _h| {
                ui.global::<SettingsBridge>().set_checking_health(false);
                toast::ok(i18n::t("dash.health.online"));
            },
            |ui, e| {
                ui.global::<SettingsBridge>().set_checking_health(false);
                toast::err(format!("{}: {}", i18n::t("dash.health.offline"), err_text(&e)));
            },
        );
    });

    // Svelte remounts the page on every visit: reload the config, clear the probe.
    let c = ctx.clone();
    router::on_route(move |ui, route: &Route| {
        if route.page == Page::Settings {
            let b = ui.global::<SettingsBridge>();
            b.set_probe_msg("".into());
            b.set_probe_ok(false);
            b.set_saving_field("".into());
            load_config(&c);
        }
    });
}

fn load_config(ctx: &Ctx) {
    if let Some(ui) = ctx.ui() {
        ui.global::<SettingsBridge>().set_cfg_error("".into());
    }
    ctx.call_with(
        |e| async move { e.get_config().await },
        |ui, cfg| {
            apply_config(ui, &cfg, true);
        },
        |ui, e| {
            let msg = err_text(&e);
            ui.global::<SettingsBridge>().set_cfg_error(msg.clone().into());
            toast::err(format!("{}: {msg}", i18n::t("dash.saveError")));
        },
    );
}

/// Mirror the engine config into the `cfg-*` properties (and the drafts when `reset`).
fn apply_config(ui: &AppWindow, cfg: &AppConfig, reset: bool) {
    let b = ui.global::<SettingsBridge>();
    b.set_cfg_memory(cfg.default_memory_mb as i32);
    b.set_cfg_memory_auto(cfg.default_memory_auto);
    b.set_cfg_concurrency(cfg.download_concurrency as i32);
    b.set_cfg_jdk(SharedString::from(cfg.jdk_path_override.as_deref().unwrap_or("")));
    b.set_cfg_aot(cfg.aot_auto_train);
    b.set_cfg_fastboot(cfg.fast_boot);
    b.set_cfg_fullbright(cfg.fullbright_on_launch);
    b.set_cfg_discord(cfg.discord_enabled);
    b.set_cfg_data_dir(SharedString::from(cfg.data_dir.as_str()));
    b.set_cfg_loaded(true);
    if reset {
        reset_drafts_from_cfg(ui);
        b.set_close_on_start(cfg.close_on_game_start);
        b.set_reduced_motion(cfg.reduced_motion);
    }
}

fn reset_drafts_from_cfg(ui: &AppWindow) {
    let b = ui.global::<SettingsBridge>();
    b.set_memory(b.get_cfg_memory() as f32);
    b.set_concurrency(b.get_cfg_concurrency() as f32);
    b.set_jdk(b.get_cfg_jdk());
    b.set_aot(b.get_cfg_aot());
    b.set_fastboot(b.get_cfg_fastboot());
    b.set_fullbright(b.get_cfg_fullbright());
    b.set_discord(b.get_cfg_discord());
}

/// One-field PATCH (savingField = field) with an ok toast; `revert` flips the
/// `close-on-start` draft back on failure.
fn save_field(ctx: &Ctx, field: &str, patch: ConfigPatch, ok_text: String, revert: bool) {
    let Some(ui) = ctx.ui() else { return };
    ui.global::<SettingsBridge>().set_saving_field(field.into());
    ctx.call_with(
        move |e| async move { e.patch_config(patch).await },
        move |ui, cfg| {
            ui.global::<SettingsBridge>().set_saving_field("".into());
            apply_config(ui, &cfg, false);
            toast::ok(ok_text);
        },
        move |ui, e| {
            let b = ui.global::<SettingsBridge>();
            b.set_saving_field("".into());
            if revert {
                b.set_close_on_start(!b.get_close_on_start());
            }
            toast::err(err_text(&e));
        },
    );
}

fn save_all(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    if !b.get_cfg_loaded() {
        return;
    }
    let mem = b.get_memory().round() as i64;
    if !(512..=65536).contains(&mem) {
        return;
    }
    let conc = (b.get_concurrency().round() as i64).clamp(1, 16);
    let mut patch = ConfigPatch::default();
    if mem != b.get_cfg_memory() as i64 {
        patch.default_memory_mb = Some(Some(mem));
    }
    if conc != b.get_cfg_concurrency() as i64 {
        patch.download_concurrency = Some(conc);
    }
    let jdk = b.get_jdk().trim().to_string();
    if jdk != b.get_cfg_jdk().trim() {
        patch.jdk_path_override = Some(Some(jdk));
    }
    if b.get_aot() != b.get_cfg_aot() {
        patch.aot_auto_train = Some(b.get_aot());
    }
    if b.get_fastboot() != b.get_cfg_fastboot() {
        patch.fast_boot = Some(b.get_fastboot());
    }
    if b.get_discord() != b.get_cfg_discord() {
        patch.discord_enabled = Some(b.get_discord());
    }
    b.set_saving_all(true);
    ctx.call_with(
        move |e| async move { e.patch_config(patch).await },
        |ui, cfg| {
            ui.global::<SettingsBridge>().set_saving_all(false);
            apply_config(ui, &cfg, false);
            toast::ok(i18n::t("dash.allSaved"));
        },
        |ui, e| {
            ui.global::<SettingsBridge>().set_saving_all(false);
            toast::err(format!("{}: {}", i18n::t("dash.saveError"), err_text(&e)));
        },
    );
}

fn probe(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    b.set_probing(true);
    b.set_probe_msg("".into());
    b.set_probe_ok(false);
    ctx.call_with(
        |e| async move { e.jvm_info().await },
        |ui, jvm| {
            let b = ui.global::<SettingsBridge>();
            let r = &jvm.runtime;
            let source =
                serde_json::to_value(r.source).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            b.set_probing(false);
            b.set_probe_ok(true);
            b.set_probe_msg(format!("OK — {} · {} ({})", r.version, r.vendor, source).into());
            b.set_jvm_version(r.version.as_str().into());
            b.set_jvm_vendor(r.vendor.as_str().into());
            b.set_jvm_source(source.as_str().into());
            b.set_jvm_path(r.path.as_str().into());
            toast::ok(format!("{} ({})", i18n::t("dash.jdk.statusOk"), r.version));
        },
        |ui, e| {
            let b = ui.global::<SettingsBridge>();
            let msg = err_text(&e);
            b.set_probing(false);
            b.set_probe_ok(false);
            b.set_probe_msg(msg.as_str().into());
            toast::err(format!("{}: {msg}", i18n::t("dash.jdk.statusErr")));
        },
    );
}

/// personalization.reduced_motion: engine default + every instance's client config.
fn toggle_reduced_motion(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    if b.get_saving_motion() {
        return;
    }
    let next = !b.get_reduced_motion();
    b.set_saving_motion(true);
    ctx.call_with(
        move |e| async move {
            e.patch_config(ConfigPatch { reduced_motion: Some(next), ..Default::default() }).await?;
            let instances = e.list_instances().await?;
            // Promise.allSettled: in parallel, per-instance failures are ignored
            let tasks: Vec<_> = instances
                .into_iter()
                .map(|inst| {
                    let e = e.clone();
                    tokio::spawn(async move {
                        let patch = ClientPatch {
                            personalization: Some(serde_json::json!({ "reduced_motion": next })),
                            ..Default::default()
                        };
                        let _ = e.patch_client(&inst.name, patch).await;
                    })
                })
                .collect();
            for t in tasks {
                let _ = t.await;
            }
            Ok(next)
        },
        |ui, next| {
            let b = ui.global::<SettingsBridge>();
            b.set_saving_motion(false);
            b.set_reduced_motion(next);
            toast::ok(i18n::t(if next { "dash.motion.flashOn" } else { "dash.motion.flashOff" }));
        },
        |ui, e| {
            ui.global::<SettingsBridge>().set_saving_motion(false);
            toast::err(format!("{}: {}", i18n::t("dash.saveError"), err_text(&e)));
        },
    );
}
