//! Hash router (port of App.svelte's `#/…` routing). FOUNDATION-OWNED.
//!
//! `router::navigate(ui, "#/instances/Foo?tab=mods")` (or `Router.navigate`
//! from Slint) parses the route, updates the `Router` global (the shell swaps
//! the page component) and calls every hook registered with [`on_route`] —
//! page modules load their data there:
//!
//! ```ignore
//! router::on_route(move |ui, route| if route.page == Page::Mods { /* load */ });
//! ```

// Public API for page modules: not every helper is used by the shell itself.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, SharedString};

use crate::{AppWindow, Page, Router};

/// A parsed hash route.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub page: Page,
    /// Decoded `:name` of `#/instances/:name` ("" otherwise).
    pub instance: String,
    /// Query string without '?'.
    pub query: String,
    /// The normalized hash ("#/…").
    pub raw: String,
}

impl Route {
    /// Value of a query parameter (`?section=skin` -> `param("section") == Some("skin")`).
    pub fn param(&self, key: &str) -> Option<String> {
        self.query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k == key).then(|| percent_decode(v))
        })
    }
}

/// Parse like App.svelte: strip '#', split off '?', path segments.
pub fn parse(hash: &str) -> Route {
    let raw = if hash.is_empty() {
        "#/".to_string()
    } else if hash.starts_with('#') {
        hash.to_string()
    } else {
        format!("#{hash}")
    };
    let body = raw.trim_start_matches('#');
    let (path, query) = body.split_once('?').unwrap_or((body, ""));
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let p0 = parts.first().copied().unwrap_or("");
    let p1 = parts.get(1).copied().unwrap_or("");
    let mut instance = String::new();
    let page = match (p0, p1) {
        ("", _) => Page::Home,
        ("instances", "") => Page::Home,
        ("instances", name) => {
            instance = percent_decode(name);
            Page::Instance
        }
        ("servers", _) => Page::Servers,
        ("library", "versions") | ("versions", _) => Page::Versions,
        ("library", "import") | ("import", _) => Page::Import,
        ("mods", _) => Page::Mods,
        ("client", _) => Page::Client,
        ("account", _) => Page::Account,
        ("settings", _) => Page::Settings,
        ("donaciones", _) => Page::Donations,
        ("armario", _) => Page::Wardrobe,
        _ => Page::Home,
    };
    Route { page, instance, query: query.to_string(), raw }
}

/// `encodeURIComponent` for building `#/instances/<name>` routes.
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Route of an instance hub.
pub fn instance_route(name: &str) -> String {
    format!("#/instances/{}", encode_component(name))
}

/// `decodeURIComponent` (invalid sequences are kept verbatim, like the JS try/catch).
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

type Hook = Rc<dyn Fn(&AppWindow, &Route)>;

thread_local! {
    static HOOKS: RefCell<Vec<Hook>> = const { RefCell::new(Vec::new()) };
    static HISTORY: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CURRENT: RefCell<Option<Route>> = const { RefCell::new(None) };
}

/// Register a hook called after every navigation (UI thread).
pub fn on_route(f: impl Fn(&AppWindow, &Route) + 'static) {
    HOOKS.with(|h| h.borrow_mut().push(Rc::new(f)));
}

/// The route currently shown.
pub fn current() -> Route {
    CURRENT.with(|c| c.borrow().clone()).unwrap_or_else(|| parse("#/"))
}

/// Navigate (UI thread).
pub fn navigate(ui: &AppWindow, hash: &str) {
    let route = parse(hash);
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        if h.last() != Some(&route.raw) {
            h.push(route.raw.clone());
            if h.len() > 64 {
                h.remove(0);
            }
        }
    });
    apply(ui, route);
}

fn apply(ui: &AppWindow, route: Route) {
    let r = ui.global::<Router>();
    r.set_instance_name(SharedString::from(route.instance.as_str()));
    r.set_query(SharedString::from(route.query.as_str()));
    r.set_route(SharedString::from(route.raw.as_str()));
    r.set_page(route.page);
    r.set_serial(r.get_serial() + 1);
    CURRENT.with(|c| *c.borrow_mut() = Some(route.clone()));
    let hooks: Vec<Hook> = HOOKS.with(|h| h.borrow().clone());
    for h in hooks {
        h(ui, &route);
    }
}

/// History back; falls back to `#/`.
pub fn back(ui: &AppWindow) {
    let prev = HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        h.pop();
        h.last().cloned()
    });
    apply(ui, parse(prev.as_deref().unwrap_or("#/")));
}

/// Wire Router.navigate / Router.back (UI thread, once).
pub fn init(ui: &AppWindow) {
    let w = ui.as_weak();
    ui.global::<Router>().on_navigate(move |hash| {
        if let Some(ui) = w.upgrade() {
            navigate(&ui, &hash);
        }
    });
    let w = ui.as_weak();
    ui.global::<Router>().on_back(move || {
        if let Some(ui) = w.upgrade() {
            back(&ui);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_app_svelte_routes() {
        assert_eq!(parse("").page, Page::Home);
        assert_eq!(parse("#/instances").page, Page::Home);
        let r = parse("#/instances/Espectral%20default?tab=mods");
        assert_eq!(r.page, Page::Instance);
        assert_eq!(r.instance, "Espectral default");
        assert_eq!(r.param("tab").as_deref(), Some("mods"));
        assert_eq!(parse("#/library/versions").page, Page::Versions);
        assert_eq!(parse("#/versions").page, Page::Versions);
        assert_eq!(parse("#/library/import").page, Page::Import);
        assert_eq!(parse("#/account?section=skin").page, Page::Account);
        assert_eq!(parse("#/armario").page, Page::Wardrobe);
        assert_eq!(parse("#/donaciones").page, Page::Donations);
        assert_eq!(parse("#/nope").page, Page::Home);
    }

    #[test]
    fn encode_roundtrip() {
        let n = "Mi pack ñ/1";
        assert_eq!(percent_decode(&encode_component(n)), n);
        assert_eq!(percent_decode("%E0%A4%A"), "%E0%A4%A");
    }
}
