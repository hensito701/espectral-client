//! Command palette model (port of components/CommandPalette.svelte script).
//! FOUNDATION-OWNED. Builds `PaletteState.items` from the shared store
//! (instances, running launches), the recent-command pref and the query.

use std::cell::RefCell;

use espectral_engine::api::game_launch::{LaunchOutcome, LaunchRequest};
use serde_json::Value;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::bridge::Ctx;
use crate::i18n::{t, tf};
use crate::{prefs, router, store, toast};
use crate::{AppWindow, PaletteItem, PaletteState, Shell};

const MAX_RECENT: usize = 6;
// Layout metrics of shell/command-palette.slint (for scroll-into-view).
const HEADER_H: f32 = 34.0;
const ROW_H: f32 = 50.0;
const SPACING: f32 = 2.0;

#[derive(Clone)]
enum Action {
    Stop(String),
    ViewInstance(String),
    Launch(String),
    Nav(&'static str),
    NewInstance,
    OpenFolder,
    ToggleTheme,
    ToggleLang,
    ToggleLogs,
}

#[derive(Clone)]
struct Cmd {
    id: String,
    category: &'static str,
    title: String,
    description: String,
    icon: &'static str,
    badge: String,
    badge_type: &'static str,
    keywords: Vec<String>,
    action: Action,
}

thread_local! {
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static SHOWN: RefCell<Vec<Cmd>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    let p = ui.global::<PaletteState>();
    let w = ui.as_weak();
    p.on_query_edited(move |q| {
        QUERY.with(|s| *s.borrow_mut() = q.to_string());
        if let Some(ui) = w.upgrade() {
            ui.global::<PaletteState>().set_highlighted(0);
            refresh(&ui);
        }
    });
    let w = ui.as_weak();
    p.on_opened(move || {
        QUERY.with(|s| s.borrow_mut().clear());
        if let Some(ui) = w.upgrade() {
            ui.global::<PaletteState>().set_highlighted(0);
            refresh(&ui);
        }
    });
    let w = ui.as_weak();
    p.on_execute(move |i| {
        let cmd = SHOWN.with(|s| s.borrow().get(i as usize).cloned());
        if let (Some(ui), Some(cmd)) = (w.upgrade(), cmd) {
            execute(&ui, cmd);
        }
    });
    refresh(ui);
}

fn recent_ids() -> Vec<String> {
    match prefs::get("recent_commands") {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).take(MAX_RECENT).collect(),
        _ => vec![],
    }
}

fn record_recent(id: &str) {
    let mut ids = recent_ids();
    ids.retain(|x| x != id);
    ids.insert(0, id.to_string());
    ids.truncate(MAX_RECENT);
    prefs::set("recent_commands", Value::from(ids));
}

fn nav(id: &str, title: &str, desc: &str, route: &'static str, kw: &[&str]) -> Cmd {
    Cmd {
        id: id.into(),
        category: "navigation",
        title: t(title),
        description: t(desc),
        icon: "nav",
        badge: t("command.badge.nav"),
        badge_type: "nav",
        keywords: kw.iter().map(|s| s.to_string()).collect(),
        action: Action::Nav(route),
    }
}

fn all_commands() -> Vec<Cmd> {
    let mut items = Vec::new();
    for r in store::launches().iter().filter(|l| l.running) {
        items.push(Cmd {
            id: format!("stop-{}", r.instance),
            category: "running",
            title: tf("command.action.stopInstance", &[("name", &r.instance)]),
            description: t("command.action.stopInstanceDesc"),
            icon: "stop",
            badge: t("command.badge.running"),
            badge_type: "running",
            keywords: ["detener", "stop", "cerrar", "kill", "parar", &r.instance]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            action: Action::Stop(r.instance.clone()),
        });
    }
    for i in store::instances() {
        let loader = i.loader.as_str();
        items.push(Cmd {
            id: format!("view-inst-{}", i.name),
            category: "instances",
            title: tf("command.action.viewInstance", &[("name", &i.name)]),
            description: format!("{} · {}", i.version, loader),
            icon: "instance",
            badge: t("command.badge.nav"),
            badge_type: "nav",
            keywords: [i.name.as_str(), i.version.as_str(), loader, "instancia", "ver", "configurar", "mods"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            action: Action::ViewInstance(i.name.clone()),
        });
        items.push(Cmd {
            id: format!("launch-inst-{}", i.name),
            category: "instances",
            title: tf(
                "command.action.launchInstance",
                &[("name", &i.name), ("version", &i.version), ("loader", loader)],
            ),
            description: tf("command.action.launchInstanceDesc", &[("version", &i.version), ("loader", loader)]),
            icon: "play",
            badge: t("command.badge.launch"),
            badge_type: "launch",
            keywords: [
                i.name.as_str(),
                i.version.as_str(),
                loader,
                "lanzar",
                "jugar",
                "iniciar",
                "play",
                "launch",
                "start",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            action: Action::Launch(i.name.clone()),
        });
    }
    items.push(nav(
        "nav-home",
        "command.nav.home",
        "command.nav.homeDesc",
        "#/",
        &["inicio", "home", "carrusel", "instancias", "juegos", "main"],
    ));
    items.push(nav(
        "nav-servers",
        "command.nav.servers",
        "command.nav.serversDesc",
        "#/servers",
        &["servidores", "servers", "radar", "multijugador", "online", "ping", "espectral"],
    ));
    items.push(nav(
        "nav-versions",
        "command.nav.versions",
        "command.nav.versionsDesc",
        "#/library/versions",
        &["versiones", "versions", "manifest", "minecraft", "release", "snapshot", "armory"],
    ));
    items.push(nav(
        "nav-import",
        "command.nav.import",
        "command.nav.importDesc",
        "#/library/import",
        &["importar", "import", "mrpack", "modpack", "lunar", "fastclient", "perfil"],
    ));
    items.push(nav(
        "nav-mods",
        "command.nav.mods",
        "command.nav.modsDesc",
        "#/mods",
        &["mods", "modrinth", "catalog", "presets", "performance", "optifine", "sodium"],
    ));
    items.push(nav(
        "nav-client",
        "command.nav.client",
        "command.nav.clientDesc",
        "#/client",
        &["cliente", "client", "suite", "macros", "fullbright", "aot", "optimizaciones"],
    ));
    items.push(nav(
        "nav-account",
        "command.nav.account",
        "command.nav.accountDesc",
        "#/account",
        &["cuenta", "account", "microsoft", "msa", "offline", "perfil", "avatar", "vault"],
    ));
    items.push(nav(
        "nav-settings",
        "command.nav.settings",
        "command.nav.settingsDesc",
        "#/settings",
        &["ajustes", "settings", "configuracion", "jvm", "memoria", "java", "jdk", "idioma", "tema"],
    ));
    items.push(nav(
        "nav-donations",
        "command.nav.donations",
        "command.nav.donationsDesc",
        "#/donaciones",
        &["donaciones", "donations", "donar", "apoyo", "vip", "donar", "pabellon"],
    ));
    items.push(nav(
        "nav-wardrobe",
        "command.nav.wardrobe",
        "command.nav.wardrobeDesc",
        "#/armario",
        &["armario", "wardrobe", "cosmeticos", "cosmetics", "capa", "cloak", "identidad", "discord", "mvp"],
    ));
    let action = |id: &str, title: &str, desc: &str, icon: &'static str, kw: &[&str], a: Action| Cmd {
        id: id.into(),
        category: "actions",
        title: t(title),
        description: t(desc),
        icon,
        badge: t("command.badge.action"),
        badge_type: "action",
        keywords: kw.iter().map(|s| s.to_string()).collect(),
        action: a,
    };
    items.push(action(
        "action-new-instance",
        "command.action.newInstance",
        "command.action.newInstanceDesc",
        "wizard",
        &["crear", "nueva", "instancia", "new", "instance", "wizard", "agregar"],
        Action::NewInstance,
    ));
    items.push(action(
        "action-open-folder",
        "command.action.openFolder",
        "command.action.openFolderDesc",
        "folder",
        &["carpeta", "directorio", "folder", "data_dir", "abrir", "archivos", "explorer"],
        Action::OpenFolder,
    ));
    items.push(action(
        "action-toggle-theme",
        "command.action.toggleTheme",
        "command.action.toggleThemeDesc",
        "theme",
        &["tema", "theme", "oscuro", "claro", "dark", "light", "modo"],
        Action::ToggleTheme,
    ));
    items.push(action(
        "action-toggle-lang",
        "command.action.toggleLang",
        "command.action.toggleLangDesc",
        "lang",
        &["idioma", "language", "ingles", "english", "espanol", "spanish", "lang"],
        Action::ToggleLang,
    ));
    items.push(action(
        "action-toggle-logs",
        "command.action.toggleLogs",
        "command.action.toggleLogsDesc",
        "logs",
        &["logs", "registros", "terminal", "pip", "consola", "salida", "drawer"],
        Action::ToggleLogs,
    ));
    items
}

/// CommandPalette.svelte fuzzyScore (char-wise over lowercase strings).
fn fuzzy_score(query: &str, target: &str, keywords: &[String]) -> i64 {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 1;
    }
    let t = target.to_lowercase();
    let kw = keywords.iter().map(|k| k.to_lowercase()).collect::<Vec<_>>().join(" ");
    if t == q {
        return 1000;
    }
    if t.starts_with(&q) {
        return 500 + (100 - q.chars().count() as i64);
    }
    if let Some(pos) = t.find(&q) {
        return 300 + (50 - t[..pos].chars().count() as i64);
    }
    if kw.contains(&q) {
        return 200;
    }
    let combined: Vec<char> = format!("{t} {kw}").chars().collect();
    let qc: Vec<char> = q.chars().collect();
    let (mut score, mut qi, mut prev, mut consecutive) = (0i64, 0usize, -2i64, 0i64);
    for (i, &ch) in combined.iter().enumerate() {
        if qi >= qc.len() {
            break;
        }
        if ch == qc[qi] {
            score += 10;
            if prev == i as i64 - 1 {
                consecutive += 1;
                score += consecutive * 5;
            } else {
                consecutive = 0;
            }
            if i == 0 || matches!(combined[i - 1], ' ' | '-' | '/') {
                score += 20;
            }
            prev = i as i64;
            qi += 1;
        }
    }
    if qi == qc.len() {
        score
    } else {
        0
    }
}

fn filtered(query: &str) -> Vec<Cmd> {
    let all = all_commands();
    if query.trim().is_empty() {
        let mut recent = Vec::new();
        for id in recent_ids() {
            if let Some(c) = all.iter().find(|c| c.id == id) {
                if !recent.iter().any(|r: &Cmd| r.id == c.id) {
                    let mut c = c.clone();
                    c.category = "recent";
                    recent.push(c);
                }
            }
        }
        let others: Vec<Cmd> = all.into_iter().filter(|c| !recent.iter().any(|r| r.id == c.id)).collect();
        return recent.into_iter().chain(others).collect();
    }
    let mut scored: Vec<(i64, Cmd)> = all
        .into_iter()
        .filter_map(|c| {
            let mut kws = vec![c.description.clone()];
            kws.extend(c.keywords.iter().cloned());
            let s = fuzzy_score(query, &c.title, &kws);
            (s > 0).then_some((s, c))
        })
        .collect();
    scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    scored.into_iter().map(|(_, c)| c).collect()
}

/// Rebuild PaletteState.items (UI thread). Cheap; called on query / data / language change.
pub fn refresh(ui: &AppWindow) {
    let query = QUERY.with(|q| q.borrow().clone());
    let list = filtered(&query);
    // group by category in the fixed order, like groupedSections
    let order = [
        ("recent", "command.group.recent"),
        ("running", "command.group.running"),
        ("instances", "command.group.instances"),
        ("navigation", "command.group.navigation"),
        ("actions", "command.group.actions"),
    ];
    let mut shown = Vec::new();
    let mut rows = Vec::new();
    let mut top = 0.0f32;
    for (cat, label) in order {
        let mut first = true;
        for c in list.iter().filter(|c| c.category == cat) {
            let section = if first { t(label).to_uppercase() } else { String::new() };
            let height = if first { HEADER_H + ROW_H } else { ROW_H };
            rows.push(PaletteItem {
                id: c.id.as_str().into(),
                section: section.into(),
                title: c.title.as_str().into(),
                description: c.description.as_str().into(),
                icon_kind: c.icon.into(),
                badge: c.badge.as_str().into(),
                badge_type: c.badge_type.into(),
                top,
                height,
            });
            top += height + SPACING;
            first = false;
            shown.push(c.clone());
        }
    }
    SHOWN.with(|s| *s.borrow_mut() = shown);
    let p = ui.global::<PaletteState>();
    p.set_empty_query(SharedString::from(query.as_str()));
    p.set_items(ModelRc::new(VecModel::from(rows)));
}

fn execute(ui: &AppWindow, cmd: Cmd) {
    record_recent(&cmd.id);
    ui.global::<Shell>().set_palette_open(false);
    let Some(ctx) = CTX.with(|c| c.borrow().clone()) else { return };
    match cmd.action {
        Action::Stop(name) => ctx.call(
            move |e| async move { e.stop_instance(&name).await },
            |_ui, _r| {
                toast::info(t("pip.stopSuccess"));
                store::refresh_launches();
            },
        ),
        Action::ViewInstance(name) => router::navigate(ui, &router::instance_route(&name)),
        Action::Launch(name) => {
            let title = cmd.title.clone();
            ctx.call(
                move |e| async move { e.launch_instance(&name, LaunchRequest { dry_run: false, account: None }).await },
                move |_ui, outcome| {
                    if matches!(outcome, LaunchOutcome::Started(_)) {
                        toast::ok(title);
                        store::refresh_launches();
                    }
                },
            )
        }
        Action::Nav(route) => router::navigate(ui, route),
        Action::NewInstance => {
            router::navigate(ui, "#/");
            ui.global::<Shell>().set_wizard_open(true);
        }
        Action::OpenFolder => {
            let dir = ctx.engine().paths().data_dir().to_string_lossy().into_owned();
            ctx.call(
                move |e| async move { e.open_folder(&dir).await },
                |_ui, _| toast::ok(t("command.action.openFolder")),
            );
        }
        Action::ToggleTheme => ui.global::<Shell>().invoke_toggle_theme(),
        Action::ToggleLang => ui.global::<Shell>().invoke_toggle_language(),
        Action::ToggleLogs => {
            let s = ui.global::<Shell>();
            s.set_pip_log_open(!s.get_pip_log_open());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

    #[test]
    fn scoring_matches_js() {
        assert_eq!(fuzzy_score("ir a inicio", "Ir a Inicio", &[]), 1000);
        assert!(fuzzy_score("ajus", "Ir a Ajustes", &[]) > 0);
        assert_eq!(fuzzy_score("zzz", "Ir a Ajustes", &[]), 0);
    }
}
