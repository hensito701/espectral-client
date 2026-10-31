//! Shared app data (port of the cross-page parts of lib/stores.ts):
//! instances, accounts (+ active account), live launches. FOUNDATION-OWNED.
//!
//! Pages read `AppState.*` in Slint; Rust modules read the cached lists with
//! [`instances`] / [`launches`] / [`accounts`] and ask for a refresh with
//! [`refresh_instances`] etc. after a mutation (create / delete / rename …).
//! Engine events keep the lists fresh (launch-exit, train-done, import-done).

// Public API for page modules: not every helper is used by the shell itself.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use espectral_engine::api::game_launch::LaunchesReply;
use espectral_engine::events::EngineEvent;
use espectral_engine::types::{InstanceSummary, LaunchHandle, PublicAccount, ServerStatus, TokenKind};
use slint::{ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::bridge::{self, Ctx};
use crate::{i18n, router, toast, util};
use crate::{AccountRow, AppState, AppWindow, InstanceRow, LaunchRow, Page, ServerRow};

thread_local! {
    static INSTANCES: RefCell<Vec<InstanceSummary>> = const { RefCell::new(Vec::new()) };
    static ACCOUNTS: RefCell<Vec<PublicAccount>> = const { RefCell::new(Vec::new()) };
    static LAUNCHES: RefCell<Vec<LaunchHandle>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
    static POLL: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    static SERVERS: RefCell<Vec<ServerStatus>> = const { RefCell::new(Vec::new()) };
    static SERVER_POLL: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}

/// Last fetched server statuses (UI thread).
pub fn servers() -> Vec<ServerStatus> {
    SERVERS.with(|v| v.borrow().clone())
}

/// Last fetched instance list (UI thread).
pub fn instances() -> Vec<InstanceSummary> {
    INSTANCES.with(|v| v.borrow().clone())
}

/// Last fetched account list (UI thread).
pub fn accounts() -> Vec<PublicAccount> {
    ACCOUNTS.with(|v| v.borrow().clone())
}

/// Last fetched launches (UI thread).
pub fn launches() -> Vec<LaunchHandle> {
    LAUNCHES.with(|v| v.borrow().clone())
}

fn ctx() -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    let s = ui.global::<AppState>();
    s.set_version(SharedString::from(espectral_engine::VERSION));
    s.set_data_dir(SharedString::from(ctx.engine().paths().data_dir().to_string_lossy().as_ref()));
    s.on_refresh_instances(refresh_instances);
    s.on_refresh_accounts(refresh_accounts);
    s.on_refresh_launches(refresh_launches);
    s.on_refresh_servers(refresh_servers);
    // servers: fetched + polled every 60 s only while a page that shows them is open
    router::on_route(|_ui, route| {
        let wants = matches!(route.page, Page::Home | Page::Servers);
        SERVER_POLL.with(|p| {
            let mut p = p.borrow_mut();
            if wants && p.is_none() {
                refresh_servers();
                let t = slint::Timer::default();
                t.start(slint::TimerMode::Repeated, Duration::from_secs(60), || {
                    // pauseWhenRunning: quiet while a game boots / runs
                    if !LAUNCHES.with(|l| l.borrow().iter().any(|x| x.running)) {
                        refresh_servers();
                    }
                });
                *p = Some(t);
            } else if !wants {
                *p = None;
            }
        });
    });
    let c = ctx.clone();
    s.on_open_folder(move |path| {
        let path = path.to_string();
        c.fire(move |e| async move { e.open_folder(&path).await });
    });

    bridge::on_event(|ui, ev| match ev {
        EngineEvent::LaunchExit(_) => {
            refresh_launches();
            refresh_instances();
        }
        EngineEvent::LaunchReady(r) => {
            refresh_launches();
            maybe_close_launcher(ui, r.key.clone());
        }
        EngineEvent::TrainDone(_) => refresh_instances(),
        EngineEvent::ImportDone(d) => {
            refresh_instances();
            let name = d.instance.as_str();
            if d.ok && d.already_exists != Some(true) {
                toast::push(
                    crate::ToastKind::Ok,
                    i18n::tf("toast.mrpackInstalled", &[("name", name)]),
                    router::instance_route(name),
                );
            } else if d.ok {
                toast::info(i18n::tf("toast.mrpackAlready", &[("name", name)]));
            } else {
                toast::err(i18n::tf("toast.mrpackError", &[("error", d.error.as_deref().unwrap_or(""))]));
            }
        }
        _ => {}
    });

    refresh_instances();
    refresh_accounts();
    refresh_launches();
}

/// Re-fetch `list_instances` (+ icons) into AppState.instances.
pub fn refresh_instances() {
    let Some(c) = ctx() else { return };
    c.call(
        |e| async move {
            let list = e.list_instances().await?;
            let mut icons = Vec::with_capacity(list.len());
            for i in &list {
                let icon = if i.has_icon {
                    e.instance_icon(&i.name).await.ok().and_then(|b| util::decode_png(&b))
                } else {
                    None
                };
                icons.push(icon);
            }
            Ok((list, icons))
        },
        |ui, (list, icons)| {
            let rows: Vec<InstanceRow> = list.iter().zip(icons).map(|(i, icon)| instance_row(i, icon)).collect();
            INSTANCES.with(|v| *v.borrow_mut() = list);
            let s = ui.global::<AppState>();
            s.set_instances(ModelRc::new(VecModel::from(rows)));
            s.set_instances_loaded(true);
            crate::palette::refresh(ui);
        },
    );
}

/// Re-fetch `list_accounts` (+ avatars) into AppState.accounts / active-account.
pub fn refresh_accounts() {
    let Some(c) = ctx() else { return };
    c.call(
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
        |ui, (list, avatars)| {
            let rows: Vec<AccountRow> = list.iter().zip(avatars).map(|(a, img)| account_row(a, img)).collect();
            // TopChrome: most recently used account is the active one
            let active = rows
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    let ta = util::parse_time_ms(&a.last_used).unwrap_or(f64::NEG_INFINITY);
                    let tb = util::parse_time_ms(&b.last_used).unwrap_or(f64::NEG_INFINITY);
                    ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i);
            let s = ui.global::<AppState>();
            match active {
                Some(i) => {
                    s.set_active_account(rows[i].clone());
                    s.set_has_active_account(true);
                }
                None => s.set_has_active_account(false),
            }
            ACCOUNTS.with(|v| *v.borrow_mut() = list);
            s.set_accounts(ModelRc::new(VecModel::from(rows)));
        },
    );
}

/// Re-fetch `list_launches` into AppState.launches / running-count.
/// Polls every 15 s while a game is running (the Svelte store polled always).
pub fn refresh_launches() {
    let Some(c) = ctx() else { return };
    c.call(
        |e| async move { e.list_launches().await },
        |ui, LaunchesReply { launches }| {
            let rows: Vec<LaunchRow> = launches.iter().map(launch_row).collect();
            let running = launches.iter().filter(|l| l.running).count() as i32;
            LAUNCHES.with(|v| *v.borrow_mut() = launches);
            let s = ui.global::<AppState>();
            s.set_launches(ModelRc::new(VecModel::from(rows)));
            s.set_running_count(running);
            POLL.with(|p| {
                let mut p = p.borrow_mut();
                if running > 0 && p.is_none() {
                    let t = slint::Timer::default();
                    t.start(slint::TimerMode::Repeated, Duration::from_secs(15), refresh_launches);
                    *p = Some(t);
                } else if running == 0 {
                    *p = None;
                }
            });
            crate::palette::refresh(ui);
        },
    );
}

/// Re-fetch `server_statuses` into AppState.servers.
pub fn refresh_servers() {
    let Some(c) = ctx() else { return };
    c.call(
        |e| async move {
            let reply = e.server_statuses().await?;
            let icons: Vec<Option<SharedPixelBuffer<Rgba8Pixel>>> =
                reply.servers.iter().map(|s| s.icon.as_deref().and_then(decode_data_url_png)).collect();
            Ok((reply.servers, icons))
        },
        |ui, (list, icons)| {
            let rows: Vec<ServerRow> = list.iter().zip(icons).map(|(s, icon)| server_row(s, icon)).collect();
            SERVERS.with(|v| *v.borrow_mut() = list);
            let s = ui.global::<AppState>();
            s.set_servers(ModelRc::new(VecModel::from(rows)));
            s.set_servers_loaded(true);
        },
    );
}

/// `data:image/png;base64,...` -> pixels.
pub fn decode_data_url_png(url: &str) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    use base64::Engine as _;
    let b64 = url.split_once("base64,").map(|(_, b)| b)?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    util::decode_png(&bytes)
}

fn server_row(s: &ServerStatus, icon: Option<SharedPixelBuffer<Rgba8Pixel>>) -> ServerRow {
    let lines = |v: &Vec<String>| {
        ModelRc::new(VecModel::from(v.iter().map(|l| SharedString::from(l.as_str())).collect::<Vec<_>>()))
    };
    ServerRow {
        host: s.host.as_str().into(),
        online: s.online,
        hostname: s.hostname.as_str().into(),
        ip: s.ip.as_deref().unwrap_or("").into(),
        port: s.port as i32,
        version: s.version.as_deref().unwrap_or("").into(),
        software: s.software.as_deref().unwrap_or("").into(),
        motd_raw: lines(&s.motd_raw),
        motd_clean: lines(&s.motd_clean),
        players_online: s.players_online as i32,
        players_max: s.players_max as i32,
        has_icon: icon.is_some(),
        icon: icon.map(util::to_image).unwrap_or_default(),
        fetched_at: s.fetched_at.to_string().into(),
        error: s.error.as_deref().unwrap_or("").into(),
    }
}

fn instance_row(i: &InstanceSummary, icon: Option<SharedPixelBuffer<Rgba8Pixel>>) -> InstanceRow {
    InstanceRow {
        name: i.name.as_str().into(),
        version: i.version.as_str().into(),
        loader: i.loader.as_str().into(),
        loader_version: i.loader_version.as_deref().unwrap_or("").into(),
        modpack: i.modpack.as_deref().unwrap_or("").into(),
        modpack_version: i.modpack_version.as_deref().unwrap_or("").into(),
        memory_mb: i.memory_mb as i32,
        memory_auto: i.memory_auto,
        hue: i.hue.map(|h| h as i32).unwrap_or(-1),
        game_dir: i.game_dir.as_deref().unwrap_or("").into(),
        has_icon: icon.is_some(),
        icon: icon.map(util::to_image).unwrap_or_default(),
        mod_count: i.mod_count as i32,
        enabled_mod_count: i.enabled_mod_count as i32,
        aot_key: i.aot_key.as_deref().unwrap_or("").into(),
        aot_cache_exists: i.aot_cache_exists,
        imported_from: i.imported_from.as_deref().unwrap_or("").into(),
        capture_tools: i.capture_tools,
        created_at: i.created_at.as_deref().unwrap_or("").into(),
        running: i.running.unwrap_or(false),
    }
}

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

fn launch_row(l: &LaunchHandle) -> LaunchRow {
    LaunchRow {
        key: l.key.as_str().into(),
        instance: l.instance.as_str().into(),
        version: l.version.as_str().into(),
        account: l.account.as_deref().unwrap_or("").into(),
        running: l.running,
        started_at: l.started_at.to_string().into(),
        menu_at: l.menu_at.map(|v| v.to_string()).unwrap_or_default().into(),
        ended_at: l.ended_at.map(|v| v.to_string()).unwrap_or_default().into(),
        agent: false,
    }
}

/// "Close the launcher when the game starts" (stores.ts maybeCloseLauncher):
/// once a game reaches its menu, close the window unless another launch is
/// still preparing. The game is detached and keeps running.
fn maybe_close_launcher(_ui: &AppWindow, ready_key: String) {
    let Some(c) = ctx() else { return };
    c.call_with(
        move |e| async move {
            let cfg = e.get_config().await?;
            if !cfg.close_on_game_start {
                return Ok(false);
            }
            let l = e.list_launches().await?;
            Ok(!l.launches.iter().any(|x| x.running && x.menu_at.is_none() && x.key != ready_key))
        },
        |ui, close| {
            if close {
                let _ = ui.hide();
                let _ = slint::quit_event_loop();
            }
        },
        |_ui, _e| {},
    );
}

/// Find an instance row model index by name (helper for pages).
pub fn instance_index(ui: &AppWindow, name: &str) -> Option<usize> {
    let m = ui.global::<AppState>().get_instances();
    (0..m.row_count()).find(|&i| m.row_data(i).is_some_and(|r| r.name == name))
}

#[allow(dead_code)]
fn _unused(_: Rc<()>) {}
