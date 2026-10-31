//! Discord identity session + LoginGate logic (port of lib/discord.svelte.ts).
//! FOUNDATION-OWNED. State lives in the `Session` global; the cached session
//! (localStorage 'horizon:discord-session' in the Svelte UI) is the
//! "discord_session" pref: `{ status, user, anonChosen, sessionId }`.

// Public API for page modules: not every helper is used by the shell itself.
#![allow(dead_code)]

use std::cell::RefCell;
use std::time::Duration;

use espectral_engine::api::identity_discord::DiscordUser;
use espectral_engine::download::FetchOptions;
use serde_json::{json, Value};
use slint::{ComponentHandle, SharedString};

use crate::bridge::Ctx;
use crate::{prefs, util, AppWindow, Session};

#[derive(Default, Clone)]
struct State {
    status: String, // loading | anon | authed
    user: Option<DiscordUser>,
    anon_chosen: bool,
    session_id: Option<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State { status: "loading".into(), ..Default::default() });
}

fn load_cached() -> State {
    let Some(v) = prefs::get("discord_session") else {
        return State { status: "loading".into(), ..Default::default() };
    };
    let user: Option<DiscordUser> = v.get("user").cloned().and_then(|u| serde_json::from_value(u).ok());
    let status =
        if v.get("status").and_then(Value::as_str) == Some("authed") && user.is_some() { "authed" } else { "anon" };
    State {
        status: status.into(),
        user,
        anon_chosen: v.get("anonChosen").and_then(Value::as_bool).unwrap_or(false),
        session_id: v.get("sessionId").and_then(Value::as_str).map(str::to_string),
    }
}

fn save(s: &State) {
    prefs::set(
        "discord_session",
        json!({ "status": s.status, "user": s.user, "anonChosen": s.anon_chosen, "sessionId": s.session_id }),
    );
}

fn publish(ui: &AppWindow, s: &State) {
    let g = ui.global::<Session>();
    g.set_status(SharedString::from(s.status.as_str()));
    g.set_anon_chosen(s.anon_chosen);
    g.set_show_gate(s.status != "authed" && !s.anon_chosen);
    g.set_user_id(s.user.as_ref().map(|u| u.id.as_str()).unwrap_or("").into());
    g.set_user_name(s.user.as_ref().map(|u| u.name.as_str()).unwrap_or("").into());
}

fn update(ui: &AppWindow, f: impl FnOnce(&mut State)) {
    let s = STATE.with(|st| {
        let mut st = st.borrow_mut();
        f(&mut st);
        st.clone()
    });
    save(&s);
    publish(ui, &s);
}

/// Current Discord session id (for engine calls that need it, e.g. wardrobe).
pub fn session_id() -> Option<String> {
    STATE.with(|s| s.borrow().session_id.clone())
}

/// Logged-in Discord user, if any.
pub fn user() -> Option<DiscordUser> {
    STATE.with(|s| s.borrow().user.clone())
}

/// `skip_gate`: screenshots of pages behind the gate.
pub fn init(ctx: &Ctx, ui: &AppWindow, skip_gate: Option<bool>) {
    let mut cached = load_cached();
    if let Some(skip) = skip_gate {
        cached.anon_chosen = skip;
        if !skip {
            cached.status = "anon".into();
            cached.user = None;
        }
    }
    STATE.with(|s| *s.borrow_mut() = cached.clone());
    publish(ui, &cached);
    load_avatar(ctx, cached.user.as_ref());

    // Revalidate against the engine (checkEngineSession)
    if skip_gate.is_none() {
        check_engine_session(ctx);
    }

    let g = ui.global::<Session>();
    let c = ctx.clone();
    g.on_continue_anonymous(move || {
        if let Some(ui) = c.ui() {
            update(&ui, |s| {
                s.status = "anon".into();
                s.anon_chosen = true;
            });
            ui.global::<Session>().set_authenticating(false);
            ui.global::<Session>().set_auth_error("".into());
        }
    });
    let c = ctx.clone();
    g.on_logout(move || {
        let sid = session_id();
        c.fire(move |e| async move { e.discord_logout(sid.as_deref()).await });
        if let Some(ui) = c.ui() {
            update(&ui, |s| {
                s.status = "anon".into();
                s.user = None;
                s.anon_chosen = false;
                s.session_id = None;
            });
        }
    });
    let c = ctx.clone();
    g.on_login_discord(move || login(&c));
}

fn check_engine_session(ctx: &Ctx) {
    let sid = session_id();
    let c = ctx.clone();
    ctx.call_with(
        move |e| async move { e.discord_me(sid.as_deref(), false).await },
        move |ui, reply| {
            let had_user = reply.user.is_some();
            let user = reply.user.clone();
            update(ui, |s| {
                if let Some(u) = user {
                    s.status = "authed".into();
                    s.user = Some(u);
                    s.anon_chosen = false;
                } else if s.status == "authed" {
                    s.status = "anon".into();
                    s.user = None;
                } else if s.status == "loading" {
                    s.status = "anon".into();
                }
            });
            if had_user {
                load_avatar(&c, reply.user.as_ref());
            }
        },
        |ui, _e| {
            update(ui, |s| {
                if s.status == "loading" {
                    s.status = "anon".into();
                }
            });
        },
    );
}

/// loginWithDiscord: open the authorize URL in the system browser, then poll
/// the one-time claim for up to 3 minutes.
fn login(ctx: &Ctx) {
    if let Some(ui) = ctx.ui() {
        let g = ui.global::<Session>();
        g.set_authenticating(true);
        g.set_auth_error("".into());
    }
    let c = ctx.clone();
    ctx.call_with(
        |e| async move { e.discord_auth_start().await },
        move |ui, start| {
            if let Err(e) = open::that_detached(&start.url) {
                ui.global::<Session>().set_authenticating(false);
                ui.global::<Session>().set_auth_error(
                    format!("No se pudo abrir el navegador. Abre este enlace manualmente: {} ({e})", start.url).into(),
                );
                return;
            }
            poll_claim(&c, start.state);
        },
        |ui, e| {
            let g = ui.global::<Session>();
            g.set_authenticating(false);
            g.set_auth_error(SharedString::from(e.envelope().message));
        },
    );
}

fn poll_claim(ctx: &Ctx, state: String) {
    let engine = ctx.engine().clone();
    let c = ctx.clone();
    ctx.spawn(async move {
        for _ in 0..120 {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let Ok(claim) = engine.discord_claim(&state).await else { continue };
            if claim.status == "ok" {
                if let Some(user) = claim.user {
                    let sid = claim.session_id.clone();
                    let c2 = c.clone();
                    c.on_ui(move |ui| {
                        let u2 = user.clone();
                        update(ui, |s| {
                            s.status = "authed".into();
                            s.user = Some(user);
                            s.anon_chosen = false;
                            if sid.is_some() {
                                s.session_id = sid;
                            }
                        });
                        let g = ui.global::<Session>();
                        g.set_authenticating(false);
                        g.set_auth_error("".into());
                        load_avatar(&c2, Some(&u2));
                        // brief success state, then the gate closes (show-gate already false)
                        g.set_just_authed(true);
                        let w = ui.as_weak();
                        slint::Timer::single_shot(Duration::from_millis(1200), move || {
                            if let Some(ui) = w.upgrade() {
                                ui.global::<Session>().set_just_authed(false);
                            }
                        });
                    });
                    return;
                }
            } else if claim.status == "invalid" {
                break;
            }
        }
        c.on_ui(|ui| ui.global::<Session>().set_authenticating(false));
    });
}

fn load_avatar(ctx: &Ctx, user: Option<&DiscordUser>) {
    let Some(url) = user.and_then(|u| u.avatar.clone()).filter(|u| u.starts_with("https://")) else { return };
    let engine = ctx.engine().clone();
    let c = ctx.clone();
    ctx.spawn(async move {
        let Ok(bytes) = engine.http().fetch_bytes(&url, &FetchOptions::default()).await else { return };
        let Some(buf) = util::decode_png(&bytes) else { return };
        c.on_ui(move |ui| {
            let g = ui.global::<Session>();
            g.set_user_avatar(util::to_image(buf));
            g.set_has_user_avatar(true);
        });
    });
}
