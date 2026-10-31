//! Launch-log ring buffers (port of lib/stores.ts `launchLog`) feeding the
//! PiP drawer, the InstanceHub console and the capsule's telemetry line.
//!
//! One buffer per launch key (max 8, 2000 lines each), fed by
//! `EngineEvent::LaunchLog` / `LaunchExit`. The two visible views (PiP = the
//! active buffer, hub console = the latest buffer of the hub's instance) are
//! mirrored into `Rc<VecModel>`s and updated incrementally, so a chatty game
//! log costs one row insert per line instead of a model rebuild.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use espectral_engine::events::EngineEvent;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::bridge::Ctx;
use crate::{i18n, prefs, store, toast, AppWindow, HiLogLine, HiLogTab, HomeBridge, Shell};

const MAX_LINES: usize = 2000;
const MAX_BUFFERS: usize = 8;
const PIN_PREF: &str = "pip.pinned";

struct Buf {
    key: String,
    instance: String,
    account: String,
    /// (line number, cleaned text)
    lines: VecDeque<(i32, String)>,
    next_num: i32,
    /// Last engine `seq` applied (dedupes events vs. the launch_log() backfill).
    last_seq: u64,
    running: bool,
}

impl Buf {
    fn new(key: &str) -> Self {
        Buf {
            key: key.into(),
            instance: String::new(),
            account: String::new(),
            lines: VecDeque::new(),
            next_num: 1,
            last_seq: 0,
            running: true,
        }
    }
}

struct State {
    bufs: Vec<Buf>,
    active: Option<String>,
    pip: Rc<VecModel<HiLogLine>>,
    pip_key: Option<String>,
    hub: Rc<VecModel<HiLogLine>>,
    hub_key: Option<String>,
    hub_instance: String,
    capsule_instance: String,
    dismissed: HashSet<String>,
    seen: HashSet<String>,
    /// instance -> (spawn_ms, boot_ms) of its last launch-exit
    timing: HashMap<String, (Option<i64>, Option<i64>)>,
    ctx: Option<Ctx>,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State {
        bufs: Vec::new(),
        active: None,
        pip: Rc::new(VecModel::default()),
        pip_key: None,
        hub: Rc::new(VecModel::default()),
        hub_key: None,
        hub_instance: String::new(),
        capsule_instance: String::new(),
        dismissed: HashSet::new(),
        seen: HashSet::new(),
        timing: HashMap::new(),
        ctx: None,
    });
}

// ---------------------------------------------------------------------------
// line helpers
// ---------------------------------------------------------------------------

/// Strip ANSI escape sequences (`\x1b[..m`, `\x1b(B`).
pub fn clean_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            match it.peek() {
                Some('[') => {
                    it.next();
                    while let Some(&n) = it.peek() {
                        it.next();
                        if n.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some('(') => {
                    it.next();
                    it.next();
                }
                _ => {}
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Word at a "start" boundary (`(^|\s)WORD`), ASCII case-insensitive.
fn has_word_start(hay: &str, words: &[&str]) -> bool {
    let lower = hay.to_ascii_lowercase();
    words.iter().any(|w| {
        let w = w.to_ascii_lowercase();
        lower.match_indices(&w).any(|(i, _)| i == 0 || lower[..i].ends_with(char::is_whitespace))
    })
}

/// PipLogDrawer classification: 2 = error, 1 = warning, 0 = plain.
fn pip_kind(clean: &str) -> i32 {
    if has_word_start(clean, &["ERROR", "SEVERE", "Exception", "Caused by", "Failed to", "Error:"]) {
        2
    } else if has_word_start(clean, &["WARN", "WARNING"]) {
        1
    } else {
        0
    }
}

/// InstanceHub console classification (case-sensitive substrings).
fn hub_kind(line: &str) -> i32 {
    if ["ERROR", "SEVERE", "Exception", "Caused by", "Failed"].iter().any(|w| line.contains(w)) {
        2
    } else if line.contains("WARN") {
        1
    } else {
        0
    }
}

fn row(num: i32, text: &str, kind: i32) -> HiLogLine {
    HiLogLine { num, text: SharedString::from(text), kind }
}

fn tail70(s: &str) -> String {
    if s.chars().count() > 70 {
        let mut t: String = s.chars().take(67).collect();
        t.push_str("...");
        t
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// view projection
// ---------------------------------------------------------------------------

impl State {
    fn buf(&self, key: &str) -> Option<&Buf> {
        self.bufs.iter().find(|b| b.key == key)
    }
    fn buf_mut(&mut self, key: &str) -> Option<&mut Buf> {
        self.bufs.iter_mut().find(|b| b.key == key)
    }
    /// PiP buffer: the active key, else the newest buffer.
    fn pip_target(&self) -> Option<String> {
        if let Some(a) = &self.active {
            if self.buf(a).is_some() {
                return Some(a.clone());
            }
        }
        self.bufs.last().map(|b| b.key.clone())
    }
    /// Hub console buffer: newest buffer of the hub instance.
    fn hub_target(&self) -> Option<String> {
        if self.hub_instance.is_empty() {
            return None;
        }
        self.bufs.iter().rev().find(|b| b.instance == self.hub_instance).map(|b| b.key.clone())
    }
    fn ensure(&mut self, key: &str) -> bool {
        if self.buf(key).is_some() {
            return false;
        }
        let mut b = Buf::new(key);
        if let Some(l) = store::launches().into_iter().find(|l| l.key == key) {
            b.instance = l.instance;
            b.account = l.account.unwrap_or_default();
            b.running = l.running;
        }
        self.bufs.push(b);
        self.prune();
        true
    }
    fn prune(&mut self) {
        while self.bufs.len() > MAX_BUFFERS {
            let active = self.active.clone();
            let idx = self
                .bufs
                .iter()
                .position(|b| !b.running && Some(&b.key) != active.as_ref())
                .or_else(|| self.bufs.iter().position(|b| Some(&b.key) != active.as_ref()));
            match idx {
                Some(i) => {
                    self.bufs.remove(i);
                }
                None => break,
            }
        }
    }
}

fn fill(model: &VecModel<HiLogLine>, buf: Option<&Buf>, pip: bool) {
    let rows: Vec<HiLogLine> = buf
        .map(|b| b.lines.iter().map(|(n, t)| row(*n, t, if pip { pip_kind(t) } else { hub_kind(t) })).collect())
        .unwrap_or_default();
    model.set_vec(rows);
}

/// Re-project both views + tabs + flags (structure changes: new buffer, active switch, exit, clear).
fn sync(ui: &AppWindow) {
    ST.with(|s| {
        let mut s = s.borrow_mut();
        let pip_t = s.pip_target();
        if pip_t != s.pip_key {
            fill(&s.pip, pip_t.as_deref().and_then(|k| s.buf(k)), true);
            s.pip_key = pip_t.clone();
        }
        let hub_t = s.hub_target();
        if hub_t != s.hub_key {
            fill(&s.hub, hub_t.as_deref().and_then(|k| s.buf(k)), false);
            s.hub_key = hub_t.clone();
        }
        let hb = ui.global::<HomeBridge>();
        let tabs: Vec<HiLogTab> = s
            .bufs
            .iter()
            .map(|b| HiLogTab {
                key: b.key.as_str().into(),
                label: if b.instance.is_empty() {
                    b.key.chars().take(8).collect::<String>().into()
                } else {
                    b.instance.as_str().into()
                },
                account: b.account.as_str().into(),
                running: b.running,
                agent: false,
            })
            .collect();
        hb.set_log_tabs(ModelRc::new(VecModel::from(tabs)));
        let pb = pip_t.as_deref().and_then(|k| s.buf(k));
        hb.set_log_active_key(pip_t.clone().unwrap_or_default().into());
        hb.set_log_has_buffer(pb.is_some());
        hb.set_log_running(pb.is_some_and(|b| b.running));
        hb.set_log_instance(pb.map(|b| b.instance.as_str()).unwrap_or("").into());
        hb.set_hub_log_running(hub_t.as_deref().and_then(|k| s.buf(k)).is_some_and(|b| b.running));
        hb.set_log_serial(hb.get_log_serial() + 1);
        hb.set_hub_log_serial(hb.get_hub_log_serial() + 1);
        let tail = capsule_tail(&s);
        hb.set_capsule_tail(tail.into());
        hb.set_capsule_timing(capsule_timing(&s).into());
    });
}

fn capsule_tail(s: &State) -> String {
    if s.capsule_instance.is_empty() {
        return String::new();
    }
    s.bufs
        .iter()
        .rev()
        .find(|b| b.instance == s.capsule_instance)
        .and_then(|b| b.lines.back())
        .map(|(_, t)| tail70(t))
        .unwrap_or_default()
}

fn capsule_timing(s: &State) -> String {
    let Some((spawn, boot)) = s.timing.get(&s.capsule_instance) else { return String::new() };
    let fmt = |v: &Option<i64>| v.map(|ms| format!("{:.1}", ms as f64 / 1000.0));
    let (p, j) = (fmt(spawn), fmt(boot));
    if p.is_none() && j.is_none() {
        return String::new();
    }
    i18n::tf("home.capsuleTiming", &[("prep", p.as_deref().unwrap_or("—")), ("jvm", j.as_deref().unwrap_or("—"))])
}

// ---------------------------------------------------------------------------
// public API (UI thread)
// ---------------------------------------------------------------------------

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    ST.with(|s| {
        let mut s = s.borrow_mut();
        s.ctx = Some(ctx.clone());
        let hb = ui.global::<HomeBridge>();
        hb.set_log_lines(ModelRc::from(s.pip.clone()));
        hb.set_hub_log_lines(ModelRc::from(s.hub.clone()));
    });
    if prefs::get_bool(PIN_PREF) {
        let sh = ui.global::<Shell>();
        sh.set_pip_log_pinned(true);
        sh.set_pip_log_open(true);
    }

    let hb = ui.global::<HomeBridge>();
    let w = ui.as_weak();
    hb.on_log_select(move |key| {
        ST.with(|s| s.borrow_mut().active = Some(key.to_string()));
        if let Some(ui) = w.upgrade() {
            sync(&ui);
        }
    });
    let w = ui.as_weak();
    hb.on_log_clear(move || {
        clear_active();
        if let Some(ui) = w.upgrade() {
            sync(&ui);
        }
    });
    let w = ui.as_weak();
    hb.on_hub_clear_log(move || {
        // Svelte: `$launchLog.clear()` clears the ACTIVE buffer.
        clear_active();
        if let Some(ui) = w.upgrade() {
            sync(&ui);
        }
    });
    let w = ui.as_weak();
    hb.on_log_copy(move || {
        let text = ST.with(|s| {
            let s = s.borrow();
            s.pip_key
                .as_deref()
                .and_then(|k| s.buf(k))
                .map(|b| b.lines.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n"))
                .unwrap_or_default()
        });
        if text.is_empty() {
            return;
        }
        crate::util::copy_text(&text);
        toast::ok(i18n::t("pip.copied"));
        if let Some(ui) = w.upgrade() {
            ui.global::<HomeBridge>().set_log_copied(true);
            let w2 = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_millis(1600), move || {
                if let Some(ui) = w2.upgrade() {
                    ui.global::<HomeBridge>().set_log_copied(false);
                }
            });
        }
    });
    let c = ctx.clone();
    hb.on_log_stop(move || {
        let inst = ST.with(|s| {
            let s = s.borrow();
            s.pip_key.as_deref().and_then(|k| s.buf(k)).map(|b| b.instance.clone()).unwrap_or_default()
        });
        if inst.is_empty() {
            return;
        }
        if let Some(ui) = c.ui() {
            ui.global::<HomeBridge>().set_log_stopping(true);
        }
        c.call_with(
            move |e| async move { e.stop_instance(&inst).await },
            |ui, _| {
                ui.global::<HomeBridge>().set_log_stopping(false);
                toast::info(i18n::t("pip.stopSuccess"));
                store::refresh_launches();
                store::refresh_instances();
            },
            |ui, e| {
                ui.global::<HomeBridge>().set_log_stopping(false);
                toast::engine_error(&e);
            },
        );
    });
    hb.on_log_collapsed(|| {
        ST.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(k) = s.pip_key.clone() {
                s.dismissed.insert(k);
            }
        });
    });
    hb.on_log_pin(|pinned| prefs::set(PIN_PREF, serde_json::Value::Bool(pinned)));
}

fn clear_active() {
    ST.with(|s| {
        let mut s = s.borrow_mut();
        let Some(k) = s.pip_target() else { return };
        if let Some(b) = s.buf_mut(&k) {
            b.lines.clear();
        }
        // force both views to re-project
        s.pip_key = None;
        s.hub_key = None;
    });
}

/// A launch started from a capsule: make its buffer active and pull the lines
/// emitted before the UI knew the key.
pub fn start(ui: &AppWindow, key: &str, instance: &str, account: &str) {
    ST.with(|s| {
        let mut s = s.borrow_mut();
        s.ensure(key);
        if let Some(b) = s.buf_mut(key) {
            b.instance = instance.into();
            b.account = account.into();
            b.running = true;
        }
        s.active = Some(key.into());
        s.pip_key = None;
        s.hub_key = None;
    });
    sync(ui);
    let Some(ctx) = ST.with(|s| s.borrow().ctx.clone()) else { return };
    let k = key.to_string();
    ctx.call_with(
        move |e| async move { e.launch_log(&k, 0).await },
        {
            let key = key.to_string();
            move |ui, chunk: espectral_engine::types::LogChunk| {
                let replaced = ST.with(|s| {
                    let mut s = s.borrow_mut();
                    let Some(b) = s.buf_mut(&key) else { return false };
                    if chunk.cursor < b.last_seq || chunk.lines.len() <= b.lines.len() {
                        return false;
                    }
                    b.lines.clear();
                    b.next_num = 1;
                    for l in chunk.lines.iter().rev().take(MAX_LINES).rev() {
                        let n = b.next_num;
                        b.lines.push_back((n, clean_ansi(l)));
                        b.next_num += 1;
                    }
                    b.running = chunk.running;
                    b.last_seq = chunk.cursor;
                    s.pip_key = None;
                    s.hub_key = None;
                    true
                });
                if replaced {
                    sync(ui);
                }
            }
        },
        |_ui, _e| {},
    );
}

/// Hub page shows `instance` ("" when the hub is closed).
pub fn set_hub_instance(ui: &AppWindow, instance: &str) {
    let changed = ST.with(|s| {
        let mut s = s.borrow_mut();
        if s.hub_instance == instance {
            return false;
        }
        s.hub_instance = instance.into();
        true
    });
    if changed {
        sync(ui);
    }
}

/// The visible capsule controls `instance`.
pub fn set_capsule_instance(ui: &AppWindow, instance: &str) {
    let changed = ST.with(|s| {
        let mut s = s.borrow_mut();
        if s.capsule_instance == instance {
            return false;
        }
        s.capsule_instance = instance.into();
        true
    });
    if changed {
        sync(ui);
    }
}

/// AppState.launches changed: fill buffer metadata and auto-expand on new running launches.
pub fn launches_changed(ui: &AppWindow) {
    let launches = store::launches();
    let mut expand = false;
    ST.with(|s| {
        let mut s = s.borrow_mut();
        for l in &launches {
            if let Some(b) = s.buf_mut(&l.key) {
                if b.instance.is_empty() {
                    b.instance = l.instance.clone();
                }
                if b.account.is_empty() {
                    b.account = l.account.clone().unwrap_or_default();
                }
                if !l.running {
                    b.running = false;
                }
            }
            if l.running && !s.seen.contains(&l.key) {
                s.seen.insert(l.key.clone());
                s.ensure(&l.key);
                if !s.dismissed.contains(&l.key) {
                    s.active = Some(l.key.clone());
                    expand = true;
                }
            }
        }
    });
    if expand {
        ui.global::<Shell>().set_pip_log_open(true);
    }
    sync(ui);
}

/// Engine events (UI thread).
pub fn on_event(ui: &AppWindow, ev: &EngineEvent) {
    match ev {
        EngineEvent::LaunchLog(l) => append(ui, &l.key, l.seq, &l.line),
        EngineEvent::LaunchExit(x) => {
            ST.with(|s| {
                let mut s = s.borrow_mut();
                s.ensure(&x.key);
                if let Some(b) = s.buf_mut(&x.key) {
                    b.running = false;
                    if !x.instance.is_empty() {
                        b.instance = x.instance.clone();
                    }
                    if let Some(a) = &x.account {
                        b.account = a.clone();
                    }
                }
                if !x.instance.is_empty() && (x.spawn_ms.is_some() || x.boot_ms.is_some()) {
                    s.timing.insert(x.instance.clone(), (x.spawn_ms, x.boot_ms));
                }
                s.hub_key = None;
            });
            sync(ui);
        }
        _ => {}
    }
}

fn append(ui: &AppWindow, key: &str, seq: u64, raw: &str) {
    let text = clean_ansi(raw);
    let mut structural = false;
    let mut unknown = false;
    ST.with(|s| {
        let mut s = s.borrow_mut();
        if s.ensure(key) {
            structural = true;
            unknown = s.buf(key).is_some_and(|b| b.instance.is_empty());
        }
        let cap = s.capsule_instance.clone();
        let Some(b) = s.buf_mut(key) else { return };
        if seq != 0 && seq <= b.last_seq {
            return; // already backfilled by launch_log()
        }
        b.last_seq = b.last_seq.max(seq);
        let num = b.next_num;
        b.next_num += 1;
        b.lines.push_back((num, text.clone()));
        let trimmed = b.lines.len() > MAX_LINES;
        if trimmed {
            b.lines.pop_front();
        }
        let is_capsule = !b.instance.is_empty() && b.instance == cap;
        if structural {
            return;
        }
        let hb = ui.global::<HomeBridge>();
        if s.pip_key.as_deref() == Some(key) {
            if trimmed && s.pip.row_count() > 0 {
                s.pip.remove(0);
            }
            s.pip.push(row(num, &text, pip_kind(&text)));
            hb.set_log_serial(hb.get_log_serial() + 1);
        }
        if s.hub_key.as_deref() == Some(key) {
            if trimmed && s.hub.row_count() > 0 {
                s.hub.remove(0);
            }
            s.hub.push(row(num, &text, hub_kind(&text)));
            hb.set_hub_log_serial(hb.get_hub_log_serial() + 1);
        }
        if is_capsule {
            hb.set_capsule_tail(tail70(&text).into());
        }
    });
    if structural {
        // a buffer we did not start (palette / agent / previous session): learn its instance
        if unknown {
            store::refresh_launches();
        }
        sync(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi() {
        assert_eq!(clean_ansi("\u{1b}[31mERROR\u{1b}[0m boom"), "ERROR boom");
        assert_eq!(clean_ansi("\u{1b}(Bplain"), "plain");
    }

    #[test]
    fn classifies_lines() {
        assert_eq!(pip_kind("[main/ERROR] x"), 0); // not at a word start (same as the JS regex)
        assert_eq!(pip_kind("ERROR: boom"), 2);
        assert_eq!(pip_kind("Caused by: y"), 2);
        assert_eq!(pip_kind("warn something"), 1);
        assert_eq!(hub_kind("[main/ERROR] x"), 2);
        assert_eq!(hub_kind("[main/WARN] x"), 1);
        assert_eq!(hub_kind("hello"), 0);
    }

    #[test]
    fn tail_is_truncated() {
        assert_eq!(tail70(&"x".repeat(100)).chars().count(), 70);
        assert_eq!(tail70("short"), "short");
    }
}
