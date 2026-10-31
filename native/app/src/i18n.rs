//! Runtime-switchable string tables (port of lib/i18n.svelte.ts). FOUNDATION-OWNED.
//!
//! Tables: every `native/app/i18n/*.json` (`{"es": {...}, "en": {...}}`),
//! embedded by build.rs. Slint reads them through the `T` global
//! (`T.tr("key")`, `T.tr1("key", "name", value)`); Rust code uses
//! [`t`] / [`tf`]. Missing keys fall back to the other language, then the key.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use slint::{ComponentHandle, Model, SharedString};

use crate::{AppWindow, Shell, T};

include!(concat!(env!("OUT_DIR"), "/i18n_files.rs"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::Es => "es",
            Lang::En => "en",
        }
    }
    pub fn parse(s: &str) -> Option<Lang> {
        match s {
            "es" => Some(Lang::Es),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

struct Tables {
    es: HashMap<String, String>,
    en: HashMap<String, String>,
}

static TABLES: OnceLock<Tables> = OnceLock::new();
static LANG: AtomicU8 = AtomicU8::new(0); // 0 = es, 1 = en

fn tables() -> &'static Tables {
    TABLES.get_or_init(|| {
        let mut es = HashMap::new();
        let mut en = HashMap::new();
        for (ns, json) in I18N_FILES {
            let doc: serde_json::Value = match serde_json::from_str(json) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("i18n/{ns}.json: {e}");
                    continue;
                }
            };
            for (lang, map) in [("es", &mut es), ("en", &mut en)] {
                if let Some(obj) = doc.get(lang).and_then(|v| v.as_object()) {
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            map.insert(k.clone(), s.to_string());
                        }
                    }
                }
            }
        }
        Tables { es, en }
    })
}

/// Current language.
pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::Es
    }
}

/// Look up `key` in `lang` (fallback: other language, then the key itself).
pub fn lookup(key: &str, lang: Lang) -> String {
    let t = tables();
    let (first, second) = match lang {
        Lang::Es => (&t.es, &t.en),
        Lang::En => (&t.en, &t.es),
    };
    first.get(key).or_else(|| second.get(key)).cloned().unwrap_or_else(|| key.to_string())
}

/// `t(key)` in the current language.
pub fn t(key: &str) -> String {
    lookup(key, lang())
}

/// `t(key, { name: value, ... })`: replaces `{name}` placeholders.
pub fn tf(key: &str, params: &[(&str, &str)]) -> String {
    format_template(&t(key), params)
}

pub fn format_template(template: &str, params: &[(&str, &str)]) -> String {
    let mut s = template.to_string();
    for (k, v) in params {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// Wire the `T` global and the language actions (UI thread, once).
pub fn init(ui: &AppWindow, initial: Lang) {
    let tg = ui.global::<T>();
    tg.on_lookup(|key, lang| SharedString::from(lookup(&key, Lang::parse(&lang).unwrap_or(Lang::Es))));
    tg.on_format(|template, params| {
        let v: Vec<SharedString> = params.iter().collect();
        let pairs: Vec<(&str, &str)> =
            v.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].as_str(), c[1].as_str())).collect();
        SharedString::from(format_template(&template, &pairs))
    });
    apply(ui, initial);

    let w = ui.as_weak();
    ui.global::<Shell>().on_set_language(move |l| {
        if let (Some(ui), Some(l)) = (w.upgrade(), Lang::parse(&l)) {
            set_language(&ui, l);
        }
    });
    let w = ui.as_weak();
    ui.global::<Shell>().on_toggle_language(move || {
        if let Some(ui) = w.upgrade() {
            set_language(&ui, if lang() == Lang::Es { Lang::En } else { Lang::Es });
        }
    });
}

fn apply(ui: &AppWindow, l: Lang) {
    LANG.store(if l == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
    ui.global::<T>().set_lang(SharedString::from(l.code()));
}

/// Switch language at runtime and persist it (localStorage 'espectral_lang').
pub fn set_language(ui: &AppWindow, l: Lang) {
    apply(ui, l);
    crate::prefs::set("lang", serde_json::Value::from(l.code()));
    crate::palette::refresh(ui);
}

/// Initial language from the persisted pref (default es, like the Svelte store).
pub fn initial_lang() -> Lang {
    crate::prefs::get_str("lang").and_then(|s| Lang::parse(&s)).unwrap_or(Lang::Es)
}
