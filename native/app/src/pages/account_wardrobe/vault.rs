//! Account vault (#/account, AccountVault.svelte) + the TopChrome account
//! popover (AccountPopover.svelte): both read `AccountBridge.accounts`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use espectral_engine::api::accounts_skins::{MsaPollOutcome, SaveSkinRequest, SkinPatch};
use espectral_engine::events::EngineEvent;
use espectral_engine::types::{MsDeviceFlow, PublicAccount, SkinLibraryEntry, SkinVariant, TokenKind};
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use super::image::{file_to_png_base64, stem40, ImageError};
use super::{err_msg, valid_offline_name};
use crate::bridge::{self, Ctx};
use crate::router::{self, Route};
use crate::{dialogs, i18n, skin, store, toast, util};
use crate::{AccountBridge, AccountRow, AppWindow, Page, VaultSkinEntry};

#[derive(Default)]
struct State {
    accounts: Vec<PublicAccount>,
    active: String,
    loaded: bool,
    ms_flow: Option<MsDeviceFlow>,
    ms_timer: Option<slint::Timer>,
    /// bumped on cancel / new flow: stale poll results are dropped
    ms_gen: u64,
    gallery: Vec<SkinLibraryEntry>,
    staged_path: Option<PathBuf>,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State::default());
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

fn ctx() -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

fn bridge(ui: &AppWindow) -> AccountBridge<'_> {
    ui.global::<AccountBridge>()
}

fn active_name() -> String {
    ST.with(|s| s.borrow().active.clone())
}

fn active_account() -> Option<PublicAccount> {
    ST.with(|s| {
        let s = s.borrow();
        s.accounts.iter().find(|a| a.username == s.active).cloned()
    })
}

fn variant_str(v: SkinVariant) -> &'static str {
    match v {
        SkinVariant::Classic => "classic",
        SkinVariant::Slim => "slim",
    }
}

fn parse_variant(s: &str) -> SkinVariant {
    if s == "slim" {
        SkinVariant::Slim
    } else {
        SkinVariant::Classic
    }
}

pub fn init(init_ctx: &Ctx, ui: &AppWindow) {
    CTX.with(|c| *c.borrow_mut() = Some(init_ctx.clone()));
    let b = bridge(ui);

    b.on_refresh(|| reload_accounts(true));
    b.on_set_active(|u| set_active(u.to_string()));
    b.on_create_offline(|u| create_offline(u.trim().to_string(), false));
    b.on_popover_create(|u| create_offline(u.trim().to_string(), true));
    b.on_popover_switch(|u| popover_switch(u.to_string()));
    b.on_popover_opened(|| reload_accounts(false));

    b.on_start_ms(start_ms);
    b.on_cancel_ms(|| {
        if let Some(ui) = ctx().and_then(|c| c.ui()) {
            cancel_ms(&ui);
        }
    });
    b.on_logout_msa(|u| logout_msa(u.to_string()));
    b.on_set_avatar_color(|u, hue| set_avatar_color(u.to_string(), hue));
    b.on_upload_avatar(|u| upload_avatar(u.to_string()));
    b.on_remove_avatar(|u| remove_avatar(u.to_string()));
    b.on_load_client_id(load_client_id);
    b.on_save_client_id(|v| save_client_id(v.trim().to_string()));

    b.on_pick_skin_file(pick_skin_file);
    b.on_clear_staged(|| {
        if let Some(ui) = ctx().and_then(|c| c.ui()) {
            clear_staged(&ui);
        }
    });
    b.on_save_to_gallery(|name| save_to_gallery(name.trim().to_string()));
    b.on_reset_skin(reset_skin);
    b.on_refresh_skin(|| {
        if let Some(ui) = ctx().and_then(|c| c.ui()) {
            let name = active_name();
            if name.is_empty() || bridge(&ui).get_skin_loading() {
                return;
            }
            skin::invalidate(&ui, &format!("account:{name}"));
            load_skin(name);
        }
    });
    b.on_reload_gallery(load_gallery);
    b.on_import_vanilla(import_vanilla);
    b.on_preview_entry(|id| preview_entry(id.to_string()));
    b.on_apply_entry(|id| apply_entry(id.to_string()));
    b.on_delete_entry(|id| delete_entry(id.to_string()));
    b.on_rename_entry(|id, name| rename_entry(id.to_string(), name.trim().to_string()));

    // Route enter = Svelte onMount (the page component is re-created): fresh
    // component state, reload accounts + gallery. Leaving = onDestroy (stop MSA polling).
    router::on_route(|ui, route: &Route| {
        if route.page == Page::Account {
            reset_page_state(ui);
            reload_accounts(false);
            load_gallery();
        } else {
            cancel_ms(ui);
        }
    });

    // Lunar / mrpack imports may add accounts.
    bridge::on_event(|_ui, ev| {
        if let EngineEvent::ImportDone(d) = ev {
            if d.ok {
                reload_accounts(true);
            }
        }
    });

    reload_accounts(false);
}

/// Component-local state of AccountVault.svelte, reset on every mount.
fn reset_page_state(ui: &AppWindow) {
    let b = bridge(ui);
    b.set_offline_error("".into());
    b.set_ms_error("".into());
    b.set_ms_pending(false);
    b.set_confirming_logout("".into());
    b.set_client_id_error("".into());
    b.set_preview_id("".into());
    b.set_renaming_id("".into());
    b.set_confirming_delete_id("".into());
    b.set_confirming_skin_reset(false);
    clear_staged(ui);
}

// ---------------------------------------------------------------------------
// accounts
// ---------------------------------------------------------------------------

fn account_row(a: &PublicAccount, avatar: Option<SharedPixelBuffer<Rgba8Pixel>>) -> AccountRow {
    let hue = a.avatar_color.map(|h| h as i32).unwrap_or(-1);
    AccountRow {
        username: a.username.as_str().into(),
        uuid: a.uuid.as_str().into(),
        token_kind: match a.token_kind {
            TokenKind::Msa => "msa".into(),
            TokenKind::Offline => "offline".into(),
        },
        avatar_color: hue,
        monogram_hue: util::name_hue(&a.username, hue),
        has_avatar: avatar.is_some(),
        avatar: avatar.map(util::to_image).unwrap_or_default(),
        created_at: a.created_at.as_deref().unwrap_or("").into(),
        last_used: a.last_used.as_deref().unwrap_or("").into(),
    }
}

fn last_used_ms(a: &PublicAccount) -> f64 {
    a.last_used.as_deref().and_then(util::parse_time_ms).unwrap_or(f64::NEG_INFINITY)
}

/// GET accounts (+ avatars). `silent`: refresh button spinner instead of the
/// first-load state.
pub fn reload_accounts(silent: bool) {
    let Some(c) = ctx() else { return };
    if let Some(ui) = c.ui() {
        let b = bridge(&ui);
        if silent {
            b.set_refreshing(true);
        } else if !ST.with(|s| s.borrow().loaded) {
            b.set_loading(true);
        }
    }
    c.call_with(
        |e| async move {
            let list = e.list_accounts().await?;
            let mut avatars = Vec::with_capacity(list.len());
            for a in &list {
                let img = if a.has_avatar {
                    e.account_avatar(&a.username).await.ok().and_then(|b| util::decode_png(&b))
                } else {
                    None
                };
                avatars.push(img);
            }
            Ok((list, avatars))
        },
        |ui, (list, avatars)| apply_accounts(ui, list, avatars),
        |ui, e| {
            let b = bridge(ui);
            b.set_loading(false);
            b.set_refreshing(false);
            toast::err(i18n::tf("vault.errorLoad", &[("error", &err_msg(&e))]));
        },
    );
}

fn apply_accounts(ui: &AppWindow, list: Vec<PublicAccount>, avatars: Vec<Option<SharedPixelBuffer<Rgba8Pixel>>>) {
    // active = most recently used (set_active_account stamps last_used);
    // sorted active first, then last_used desc (sortedAccounts)
    let mut pairs: Vec<(PublicAccount, Option<SharedPixelBuffer<Rgba8Pixel>>)> =
        list.into_iter().zip(avatars).collect();
    pairs.sort_by(|(a, _), (b, _)| last_used_ms(b).partial_cmp(&last_used_ms(a)).unwrap_or(std::cmp::Ordering::Equal));
    let active = pairs.first().map(|(a, _)| a.username.clone()).unwrap_or_default();
    let rows: Vec<AccountRow> = pairs.iter().map(|(a, img)| account_row(a, img.clone())).collect();
    let prev_active = ST.with(|s| {
        let mut s = s.borrow_mut();
        let prev = std::mem::replace(&mut s.active, active.clone());
        s.accounts = pairs.into_iter().map(|(a, _)| a).collect();
        s.loaded = true;
        prev
    });
    let b = bridge(ui);
    match rows.first() {
        Some(r) => {
            b.set_active(r.clone());
            b.set_has_active(true);
        }
        None => b.set_has_active(false),
    }
    b.set_popover_others(ModelRc::new(VecModel::from(rows.iter().skip(1).cloned().collect::<Vec<_>>())));
    b.set_accounts(ModelRc::new(VecModel::from(rows)));
    b.set_loading(false);
    b.set_refreshing(false);
    if prev_active != active {
        on_active_changed(ui);
    }
}

/// $effect on activeAccount: reset the atelier and reload the skin (MSA only).
fn on_active_changed(ui: &AppWindow) {
    let b = bridge(ui);
    clear_staged(ui);
    b.set_preview_id("".into());
    b.set_confirming_skin_reset(false);
    b.set_confirming_delete_id("".into());
    b.set_renaming_id("".into());
    match active_account() {
        Some(a) if a.token_kind == TokenKind::Msa => load_skin(a.username),
        _ => {
            b.set_skin_info(false);
            b.set_skin_has_skin(false);
            b.set_skin_cape(false);
            b.set_skin_loading(false);
        }
    }
}

/// After any account mutation: vault list + the shared store (TopChrome, Home).
fn accounts_changed() {
    reload_accounts(true);
    store::refresh_accounts();
}

fn set_active(username: String) {
    if username.is_empty() || active_name() == username {
        return;
    }
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move { e.set_active_account(&username).await },
        |_ui, r| {
            accounts_changed();
            toast::ok(i18n::tf("vault.switchSuccess", &[("name", &r.active)]));
        },
        |_ui, e| toast::err(i18n::tf("vault.errorAction", &[("error", &err_msg(&e))])),
    );
}

fn popover_switch(username: String) {
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move { e.set_active_account(&username).await },
        |ui, _r| {
            bridge(ui).set_popover_error("".into());
            accounts_changed();
        },
        |ui, e| bridge(ui).set_popover_error(err_msg(&e).into()),
    );
}

/// Offline forge (vault) / quick add (popover): create, activate, reload.
fn create_offline(name: String, popover: bool) {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    let b = bridge(&ui);
    if !valid_offline_name(&name) {
        if popover {
            b.set_popover_error(i18n::t("account.errorInvalidName").into());
        }
        return;
    }
    if (popover && b.get_popover_creating()) || (!popover && b.get_offline_creating()) {
        return;
    }
    if popover {
        b.set_popover_creating(true);
        b.set_popover_error("".into());
    } else {
        b.set_offline_creating(true);
        b.set_offline_error("".into());
    }
    c.call_with(
        move |e| async move {
            let acc = e.create_account(&name).await?;
            e.set_active_account(&acc.username).await?;
            Ok(acc.username)
        },
        move |ui, username| {
            let b = bridge(ui);
            if popover {
                b.set_popover_creating(false);
                b.set_popover_created_serial(b.get_popover_created_serial() + 1);
            } else {
                b.set_offline_creating(false);
                b.set_offline_created_serial(b.get_offline_created_serial() + 1);
                toast::ok(i18n::tf("vault.offlineSuccess", &[("name", &username)]));
            }
            accounts_changed();
        },
        move |ui, e| {
            let b = bridge(ui);
            let msg = err_msg(&e);
            if popover {
                b.set_popover_creating(false);
                b.set_popover_error(msg.into());
            } else {
                b.set_offline_creating(false);
                b.set_offline_error(msg.as_str().into());
                toast::err(i18n::tf("vault.errorAction", &[("error", &msg)]));
            }
        },
    );
}

fn logout_msa(username: String) {
    let Some(c) = ctx() else { return };
    if let Some(ui) = c.ui() {
        bridge(&ui).set_logging_out(true);
    }
    let was_active = active_name() == username;
    c.call_with(
        move |e| async move {
            e.msa_logout(&username).await?;
            // the signed-out account was active: activate the next one
            if was_active {
                let rest = e.list_accounts().await?;
                if let Some(next) = rest
                    .iter()
                    .max_by(|a, b| last_used_ms(a).partial_cmp(&last_used_ms(b)).unwrap_or(std::cmp::Ordering::Equal))
                {
                    e.set_active_account(&next.username).await?;
                }
            }
            Ok(username)
        },
        |ui, username| {
            let b = bridge(ui);
            b.set_logging_out(false);
            b.set_confirming_logout("".into());
            accounts_changed();
            // hard-coded English in the Svelte UI too
            toast::ok(format!("Microsoft account {username} signed out."));
        },
        |ui, e| {
            bridge(ui).set_logging_out(false);
            toast::err(i18n::tf("vault.errorAction", &[("error", &err_msg(&e))]));
        },
    );
}

// ---------------------------------------------------------------------------
// avatars
// ---------------------------------------------------------------------------

fn avatar_busy(ui: &AppWindow) -> bool {
    !bridge(ui).get_avatar_busy().is_empty()
}

fn set_avatar_color(username: String, hue: i32) {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    if avatar_busy(&ui) {
        return;
    }
    bridge(&ui).set_avatar_busy(username.as_str().into());
    let color = (hue >= 0).then_some(hue as i64);
    c.call_with(
        move |e| async move { e.set_account_avatar_color(&username, color).await },
        |ui, _acc| {
            bridge(ui).set_avatar_busy("".into());
            accounts_changed();
            toast::ok(i18n::t("vault.colorSaved"));
        },
        |ui, e| {
            bridge(ui).set_avatar_busy("".into());
            toast::err(i18n::tf("vault.errorAction", &[("error", &err_msg(&e))]));
        },
    );
}

enum AvatarOutcome {
    Cancelled,
    Saved,
    Unsupported(String, String),
}

fn upload_avatar(username: String) {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    if avatar_busy(&ui) {
        return;
    }
    let c2 = c.clone();
    let u2 = username.clone();
    c.call_with(
        move |e| async move {
            let title = i18n::t("vault.avatarUpload");
            let Some(path) = dialogs::pick_file(&title, &[("PNG", &["png"])]).await else {
                return Ok(AvatarOutcome::Cancelled);
            };
            c2.on_ui(move |ui| bridge(ui).set_avatar_busy(u2.as_str().into()));
            let file_name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let b64 = match tokio::task::spawn_blocking(move || file_to_png_base64(&path, 256)).await {
                Ok(Ok(b)) => b,
                Ok(Err(ImageError::Unsupported(label))) => return Ok(AvatarOutcome::Unsupported(label, file_name)),
                Ok(Err(ImageError::Io(m))) => return Err(espectral_engine::EngineError::other(m)),
                Err(j) => return Err(espectral_engine::EngineError::other(j.to_string())),
            };
            e.upload_account_avatar(&username, &b64).await?;
            Ok(AvatarOutcome::Saved)
        },
        |ui, outcome| {
            bridge(ui).set_avatar_busy("".into());
            match outcome {
                AvatarOutcome::Cancelled => {}
                AvatarOutcome::Saved => {
                    accounts_changed();
                    toast::ok(i18n::t("vault.avatarSaved"));
                }
                AvatarOutcome::Unsupported(label, name) => {
                    toast::err(i18n::tf("vault.avatarUnsupported", &[("label", &label), ("name", &name)]))
                }
            }
        },
        |ui, e| {
            bridge(ui).set_avatar_busy("".into());
            toast::err(i18n::tf("vault.errorAction", &[("error", &err_msg(&e))]));
        },
    );
}

fn remove_avatar(username: String) {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    if avatar_busy(&ui) {
        return;
    }
    bridge(&ui).set_avatar_busy(username.as_str().into());
    c.call_with(
        move |e| async move { e.remove_account_avatar(&username).await },
        |ui, _| {
            bridge(ui).set_avatar_busy("".into());
            accounts_changed();
            toast::ok(i18n::t("vault.avatarRemoved"));
        },
        |ui, e| {
            bridge(ui).set_avatar_busy("".into());
            toast::err(i18n::tf("vault.errorAction", &[("error", &err_msg(&e))]));
        },
    );
}

// ---------------------------------------------------------------------------
// Microsoft device flow
// ---------------------------------------------------------------------------

fn is_msa_pending(e: &espectral_engine::EngineError) -> bool {
    e.has_code("MSA_DEVICE_FAILED")
}

fn set_flow(ui: &AppWindow, flow: Option<&MsDeviceFlow>) {
    let b = bridge(ui);
    b.set_ms_flow(flow.is_some());
    b.set_ms_user_code(flow.map(|f| f.user_code.as_str()).unwrap_or("").into());
    b.set_ms_verification_uri(flow.map(|f| f.verification_uri.as_str()).unwrap_or("").into());
    b.set_ms_verification_uri_complete(flow.and_then(|f| f.verification_uri_complete.as_deref()).unwrap_or("").into());
}

fn start_ms() {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    let b = bridge(&ui);
    if b.get_ms_polling() {
        return;
    }
    b.set_ms_error("".into());
    b.set_ms_pending(false);
    b.set_ms_polling(true);
    let gen = ST.with(|s| {
        let mut s = s.borrow_mut();
        s.ms_gen += 1;
        s.ms_gen
    });
    c.call_with(
        |e| async move { e.msa_device_code().await },
        move |ui, flow| {
            if ST.with(|s| s.borrow().ms_gen) != gen {
                return;
            }
            set_flow(ui, Some(&flow));
            let interval = flow.interval;
            ST.with(|s| s.borrow_mut().ms_flow = Some(flow));
            schedule_poll(interval, gen);
        },
        |ui, e| {
            let b = bridge(ui);
            b.set_ms_polling(false);
            if is_msa_pending(&e) {
                b.set_ms_pending(true);
            } else {
                let m = err_msg(&e);
                b.set_ms_error(m.as_str().into());
                toast::err(m);
            }
        },
    );
}

fn schedule_poll(delay_s: u64, gen: u64) {
    let t = slint::Timer::default();
    t.start(slint::TimerMode::SingleShot, Duration::from_secs(delay_s.max(3)), move || poll_ms(gen));
    ST.with(|s| s.borrow_mut().ms_timer = Some(t));
}

fn poll_ms(gen: u64) {
    let Some(c) = ctx() else { return };
    let Some(flow) = ST.with(|s| {
        let s = s.borrow();
        (s.ms_gen == gen).then(|| s.ms_flow.clone()).flatten()
    }) else {
        return;
    };
    let interval = flow.interval;
    let flow_id = flow.flow_id.clone();
    c.call_with(
        move |e| async move {
            match e.msa_poll(&flow_id).await? {
                MsaPollOutcome::Pending { retry_after, .. } => Ok(Err(retry_after)),
                MsaPollOutcome::Account(acc) => {
                    e.set_active_account(&acc.username).await?;
                    Ok(Ok(acc.username))
                }
            }
        },
        move |ui, outcome| {
            if ST.with(|s| s.borrow().ms_gen) != gen {
                return;
            }
            match outcome {
                Err(retry_after) => schedule_poll(retry_after.unwrap_or(if interval > 0 { interval } else { 5 }), gen),
                Ok(username) => {
                    stop_flow(ui);
                    accounts_changed();
                    toast::ok(i18n::tf("vault.msSuccess", &[("name", &username)]));
                }
            }
        },
        move |ui, e| {
            if ST.with(|s| s.borrow().ms_gen) != gen {
                return;
            }
            stop_flow(ui);
            let b = bridge(ui);
            if is_msa_pending(&e) {
                b.set_ms_pending(true);
            } else {
                let m = err_msg(&e);
                b.set_ms_error(m.as_str().into());
                toast::err(m);
            }
        },
    );
}

fn stop_flow(ui: &AppWindow) {
    ST.with(|s| {
        let mut s = s.borrow_mut();
        s.ms_timer = None;
        s.ms_flow = None;
        s.ms_gen += 1;
    });
    set_flow(ui, None);
    bridge(ui).set_ms_polling(false);
}

fn cancel_ms(ui: &AppWindow) {
    stop_flow(ui);
}

// ---------------------------------------------------------------------------
// advanced client id
// ---------------------------------------------------------------------------

fn load_client_id() {
    let Some(c) = ctx() else { return };
    if let Some(ui) = c.ui() {
        let b = bridge(&ui);
        b.set_client_id_loading(true);
        b.set_client_id_error("".into());
    }
    c.call_with(
        |e| async move { e.msa_client_id().await },
        |ui, r| {
            let b = bridge(ui);
            b.set_client_id_loading(false);
            b.set_client_id(r.client_id.into());
            b.set_client_id_serial(b.get_client_id_serial() + 1);
        },
        |ui, e| {
            let b = bridge(ui);
            b.set_client_id_loading(false);
            b.set_client_id_error(err_msg(&e).into());
        },
    );
}

fn save_client_id(value: String) {
    let Some(c) = ctx() else { return };
    if let Some(ui) = c.ui() {
        let b = bridge(&ui);
        b.set_client_id_saving(true);
        b.set_client_id_error("".into());
        b.set_client_id(value.as_str().into());
    }
    c.call_with(
        move |e| async move { e.set_msa_client_id(&value).await },
        |ui, _r| {
            bridge(ui).set_client_id_saving(false);
            toast::ok(i18n::t("vault.clientIdSuccess"));
        },
        |ui, e| {
            let b = bridge(ui);
            b.set_client_id_saving(false);
            let m = err_msg(&e);
            b.set_client_id_error(m.as_str().into());
            toast::err(m);
        },
    );
}

// ---------------------------------------------------------------------------
// Skin Atelier
// ---------------------------------------------------------------------------

fn skin_error(e: &espectral_engine::EngineError) {
    toast::err(i18n::tf("vault.skin.loadError", &[("error", &err_msg(e))]));
}

fn load_skin(username: String) {
    let Some(c) = ctx() else { return };
    if let Some(ui) = c.ui() {
        bridge(&ui).set_skin_loading(true);
    }
    let u2 = username.clone();
    c.call_with(
        move |e| async move { e.account_skin(&username).await },
        move |ui, info| {
            let b = bridge(ui);
            b.set_skin_loading(false);
            // race guard: still looking at the same account
            if active_name() == u2 {
                b.set_skin_info(true);
                b.set_skin_has_skin(info.has_skin);
                b.set_skin_cape(info.cape);
                b.set_skin_variant(variant_str(info.variant).into());
            }
        },
        |ui, e| {
            let b = bridge(ui);
            b.set_skin_loading(false);
            b.set_skin_info(false);
            if !e.has_code("OFFLINE_ACCOUNT") {
                skin_error(&e);
            }
        },
    );
}

fn clear_staged(ui: &AppWindow) {
    ST.with(|s| s.borrow_mut().staged_path = None);
    let b = bridge(ui);
    b.set_staged(false);
    b.set_staged_path("".into());
    b.set_staged_name("".into());
    b.set_staged_serial(b.get_staged_serial() + 1);
}

fn pick_skin_file() {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    if bridge(&ui).get_skin_busy() {
        return;
    }
    c.call_with(
        |_e| async move {
            let title = i18n::t("vault.skin.uploadLabel");
            let Some(path) = dialogs::pick_file(&title, &[("PNG", &["png"])]).await else { return Ok(None) };
            let p2 = path.clone();
            let ok = tokio::task::spawn_blocking(move || {
                std::fs::read(&p2).ok().and_then(|b| util::decode_png(&b)).map(|b| (b.width(), b.height()))
            })
            .await
            .ok()
            .flatten();
            match ok {
                Some(_) => Ok(Some(path)),
                None => Err(espectral_engine::EngineError::bad_request("BAD_IMAGE", "image must be a PNG")),
            }
        },
        |ui, path| {
            let Some(path) = path else { return };
            let b = bridge(ui);
            let p = path.to_string_lossy().to_string();
            skin::invalidate(ui, &format!("file:{p}"));
            if b.get_staged_name().trim().is_empty() {
                b.set_staged_name(stem40(&path).into());
            }
            ST.with(|s| s.borrow_mut().staged_path = Some(path));
            b.set_staged(true);
            b.set_staged_path(p.into());
            b.set_preview_id("".into());
            b.set_staged_serial(b.get_staged_serial() + 1);
        },
        |_ui, e| skin_error(&e),
    );
}

fn save_to_gallery(name: String) {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    let b = bridge(&ui);
    let Some(path) = ST.with(|s| s.borrow().staged_path.clone()) else { return };
    if b.get_skin_busy() {
        return;
    }
    b.set_skin_busy(true);
    let name = if name.is_empty() { i18n::t("vault.skin.stageNamePlaceholder") } else { name };
    let variant = parse_variant(&b.get_skin_variant());
    c.call_with(
        move |e| async move {
            let b64 = tokio::task::spawn_blocking(move || file_to_png_base64(&path, 256))
                .await
                .map_err(|j| espectral_engine::EngineError::other(j.to_string()))?
                .map_err(|err| match err {
                    ImageError::Unsupported(l) => espectral_engine::EngineError::bad_request("BAD_IMAGE", l),
                    ImageError::Io(m) => espectral_engine::EngineError::other(m),
                })?;
            e.save_library_skin(SaveSkinRequest { image_base64: b64, name: Some(name), variant: Some(variant) }).await
        },
        |ui, entry| {
            let b = bridge(ui);
            b.set_skin_busy(false);
            clear_staged(ui);
            set_preview(ui, &entry);
            toast::ok(i18n::tf("vault.skin.gallerySaved", &[("name", &entry.name)]));
            load_gallery();
        },
        |ui, e| {
            bridge(ui).set_skin_busy(false);
            skin_error(&e);
        },
    );
}

fn set_preview(ui: &AppWindow, entry: &SkinLibraryEntry) {
    let b = bridge(ui);
    b.set_preview_id(entry.id.as_str().into());
    b.set_preview_name(entry.name.as_str().into());
    b.set_preview_variant(variant_str(entry.variant).into());
}

fn load_gallery() {
    let Some(c) = ctx() else { return };
    if let Some(ui) = c.ui() {
        bridge(&ui).set_gallery_loading(true);
    }
    c.call_with(
        |e| async move { e.list_library_skins().await },
        |ui, r| {
            let b = bridge(ui);
            b.set_gallery_loading(false);
            let rows: Vec<VaultSkinEntry> = r
                .skins
                .iter()
                .map(|s| VaultSkinEntry {
                    id: s.id.as_str().into(),
                    name: s.name.as_str().into(),
                    variant: variant_str(s.variant).into(),
                })
                .collect();
            // drop a preview of an entry that no longer exists; refresh its name
            let pid = b.get_preview_id().to_string();
            if !pid.is_empty() {
                match r.skins.iter().find(|s| s.id == pid) {
                    Some(s) => set_preview(ui, s),
                    None => b.set_preview_id("".into()),
                }
            }
            ST.with(|s| s.borrow_mut().gallery = r.skins);
            b.set_gallery(ModelRc::new(VecModel::from(rows)));
        },
        |ui, e| {
            bridge(ui).set_gallery_loading(false);
            skin_error(&e);
        },
    );
}

fn gallery_entry(id: &str) -> Option<SkinLibraryEntry> {
    ST.with(|s| s.borrow().gallery.iter().find(|g| g.id == id).cloned())
}

fn preview_entry(id: String) {
    let Some(ui) = ctx().and_then(|c| c.ui()) else { return };
    if let Some(entry) = gallery_entry(&id) {
        set_preview(&ui, &entry);
        let b = bridge(&ui);
        b.set_preview_serial(b.get_preview_serial() + 1);
    }
}

fn apply_entry(id: String) {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    let b = bridge(&ui);
    let username = active_name();
    let Some(entry) = gallery_entry(&id) else { return };
    if username.is_empty() || !b.get_applying_id().is_empty() {
        return;
    }
    b.set_applying_id(id.as_str().into());
    let u2 = username.clone();
    c.call_with(
        move |e| async move { e.apply_library_skin(&id, &username).await },
        move |ui, r| {
            let b = bridge(ui);
            b.set_applying_id("".into());
            b.set_skin_variant(variant_str(r.variant).into());
            skin::invalidate(ui, &format!("account:{u2}"));
            load_skin(u2.clone());
            accounts_changed(); // MSA avatar follows the skin head
            toast::ok(i18n::tf("vault.skin.galleryApplied", &[("name", &entry.name), ("username", &u2)]));
        },
        |ui, e| {
            bridge(ui).set_applying_id("".into());
            skin_error(&e);
        },
    );
}

fn delete_entry(id: String) {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    let b = bridge(&ui);
    let Some(entry) = gallery_entry(&id) else { return };
    if !b.get_deleting_id().is_empty() {
        return;
    }
    b.set_deleting_id(id.as_str().into());
    c.call_with(
        move |e| async move { e.delete_library_skin(&id).await.map(|_| id) },
        move |ui, id| {
            let b = bridge(ui);
            b.set_deleting_id("".into());
            b.set_confirming_delete_id("".into());
            if b.get_preview_id() == id.as_str() {
                b.set_preview_id("".into());
            }
            skin::invalidate(ui, &format!("library:{id}"));
            load_gallery();
            toast::ok(i18n::tf("vault.skin.galleryDeleted", &[("name", &entry.name)]));
        },
        |ui, e| {
            bridge(ui).set_deleting_id("".into());
            skin_error(&e);
        },
    );
}

fn rename_entry(id: String, name: String) {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    let b = bridge(&ui);
    if name.is_empty() || b.get_skin_busy() {
        return;
    }
    b.set_skin_busy(true);
    c.call_with(
        move |e| async move { e.patch_library_skin(&id, SkinPatch { name: Some(name), variant: None }).await },
        |ui, _entry| {
            let b = bridge(ui);
            b.set_skin_busy(false);
            b.set_renaming_id("".into());
            load_gallery();
            toast::ok(i18n::t("vault.skin.galleryRenamed"));
        },
        |ui, e| {
            bridge(ui).set_skin_busy(false);
            skin_error(&e);
        },
    );
}

fn import_vanilla() {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    let b = bridge(&ui);
    if b.get_importing_vanilla() {
        return;
    }
    b.set_importing_vanilla(true);
    c.call_with(
        |e| async move { e.import_vanilla_skins().await },
        |ui, r| {
            bridge(ui).set_importing_vanilla(false);
            load_gallery();
            if r.total == 0 {
                toast::ok(i18n::t("vault.skin.importVanillaNone"));
            } else {
                toast::ok(i18n::tf(
                    "vault.skin.importVanillaDone",
                    &[("imported", &r.imported.to_string()), ("skipped", &r.skipped.to_string())],
                ));
            }
        },
        |ui, e| {
            bridge(ui).set_importing_vanilla(false);
            if e.has_code("NO_VANILLA_SKINS") {
                toast::err(i18n::t("vault.skin.importVanillaNone"));
            } else {
                skin_error(&e);
            }
        },
    );
}

fn reset_skin() {
    let Some(c) = ctx() else { return };
    let Some(ui) = c.ui() else { return };
    let b = bridge(&ui);
    let username = active_name();
    if username.is_empty() || b.get_skin_busy() {
        return;
    }
    b.set_skin_busy(true);
    let u2 = username.clone();
    c.call_with(
        move |e| async move { e.reset_account_skin(&username).await },
        move |ui, _| {
            let b = bridge(ui);
            b.set_skin_busy(false);
            b.set_confirming_skin_reset(false);
            clear_staged(ui);
            skin::invalidate(ui, &format!("account:{u2}"));
            load_skin(u2.clone());
            accounts_changed();
            toast::ok(i18n::t("vault.skin.resetDone"));
        },
        |ui, e| {
            bridge(ui).set_skin_busy(false);
            skin_error(&e);
        },
    );
}
