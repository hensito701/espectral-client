//! Toast store (port of lib/toast.svelte.ts). FOUNDATION-OWNED.
//!
//! `toast::ok("…")`, `toast::err("…")`, `toast::info("…")`,
//! `toast::push(kind, text, href)` — callable from ANY thread. Auto-dismiss
//! after 6 s; clicking a toast with an href navigates (`#/…`) or opens the URL.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use espectral_engine::EngineError;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{AppWindow, Shell, ToastItem, ToastKind};

const TOAST_MS: u64 = 6000;

thread_local! {
    static MODEL: RefCell<Option<Rc<VecModel<ToastItem>>>> = const { RefCell::new(None) };
    static NEXT_ID: Cell<i32> = const { Cell::new(1) };
    static UI: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
}

/// Wire Shell.toast / dismiss-toast / toast-clicked (UI thread, once).
pub fn init(ui: &AppWindow) {
    let model = Rc::new(VecModel::<ToastItem>::default());
    ui.global::<Shell>().set_toasts(ModelRc::from(model.clone()));
    MODEL.with(|m| *m.borrow_mut() = Some(model));
    UI.with(|u| *u.borrow_mut() = Some(ui.as_weak()));

    let shell = ui.global::<Shell>();
    shell.on_toast(|kind, text, href| push_local(kind, text.to_string(), href.to_string()));
    shell.on_dismiss_toast(dismiss);
    let weak = ui.as_weak();
    shell.on_toast_clicked(move |id| {
        let href =
            MODEL.with(|m| m.borrow().as_ref().and_then(|m| m.iter().find(|t| t.id == id).map(|t| t.href.to_string())));
        if let Some(href) = href.filter(|h| !h.is_empty()) {
            if href.starts_with('#') {
                if let Some(ui) = weak.upgrade() {
                    crate::router::navigate(&ui, &href);
                }
            } else {
                crate::util::open_url(&href);
            }
        }
        dismiss(id);
    });
}

fn push_local(kind: ToastKind, text: String, href: String) {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    MODEL.with(|m| {
        if let Some(m) = m.borrow().as_ref() {
            m.push(ToastItem { id, kind, text: SharedString::from(text), href: SharedString::from(href) });
        }
    });
    slint::Timer::single_shot(Duration::from_millis(TOAST_MS), move || dismiss(id));
}

/// Remove a toast now (no-op if already gone).
pub fn dismiss(id: i32) {
    MODEL.with(|m| {
        if let Some(m) = m.borrow().as_ref() {
            if let Some(i) = m.iter().position(|t| t.id == id) {
                m.remove(i);
            }
        }
    });
}

/// Push a toast from any thread.
pub fn push(kind: ToastKind, text: impl Into<String>, href: impl Into<String>) {
    let (text, href) = (text.into(), href.into());
    let on_ui_thread = MODEL.with(|m| m.borrow().is_some());
    if on_ui_thread {
        push_local(kind, text, href);
    } else {
        let _ = slint::invoke_from_event_loop(move || push_local(kind, text, href));
    }
}

pub fn ok(text: impl Into<String>) {
    push(ToastKind::Ok, text, "");
}

pub fn err(text: impl Into<String>) {
    push(ToastKind::Err, text, "");
}

pub fn info(text: impl Into<String>) {
    push(ToastKind::Info, text, "");
}

/// Standard error toast for a failed engine call (the JS UI showed `String(err)`,
/// i.e. the ApiError message).
pub fn engine_error(e: &EngineError) {
    err(e.envelope().message);
}
