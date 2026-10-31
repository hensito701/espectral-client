//! SettingsDashboard → "Acceso para agentes" (MCP) section: engine/agent_access,
//! engine/set_agent_access, engine/rotate_agent_token in-process.
//!
//! Ports the `agent…` state, the derived MCP snippets (JSON for Claude Code /
//! Cursor, TOML for Codex), the allowlist draft editor (host / host:port
//! validation, blocked Espectral suffixes, duplicate check) and the token
//! rotation confirmation of `pages/SettingsDashboard.svelte`.

use std::cell::RefCell;
use std::time::Duration;

use espectral_engine::agent::AgentAccessPatch;
use espectral_engine::api::agent::AgentAccessStatus;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::err_text;
use crate::bridge::Ctx;
use crate::router::{self, Route};
use crate::{i18n, toast, util, AppWindow, Page, SettingsBridge};

/// The loaded status + the allowlist draft (edited locally, saved with one PUT).
#[derive(Default)]
struct State {
    status: Option<AgentAccessStatus>,
    draft: Vec<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let b = ui.global::<SettingsBridge>();

    let c = ctx.clone();
    b.on_agent_set_enabled(move |on| set_enabled(&c, on));

    let c = ctx.clone();
    b.on_agent_rotate(move || rotate(&c));

    let c = ctx.clone();
    b.on_agent_allow_add(move || add_server(&c));

    let c = ctx.clone();
    b.on_agent_allow_remove(move |host| remove_server(&c, host.as_str()));

    let c = ctx.clone();
    b.on_agent_allow_save(move || save_allowlist(&c));

    let c = ctx.clone();
    b.on_agent_copy(move |which| copy(&c, which.as_str()));

    // Svelte remounts the page on every visit: reload the status, drop the
    // rotate confirmation / allowlist messages.
    let c = ctx.clone();
    router::on_route(move |ui, route: &Route| {
        if route.page == Page::Settings {
            let b = ui.global::<SettingsBridge>();
            // private state of the Svelte component: gone on every remount
            b.set_agent_loaded(false);
            b.set_agent_active(false);
            b.set_agent_enabled(false);
            b.set_agent_saving(false);
            b.set_agent_rotating(false);
            b.set_agent_confirming_rotate(false);
            b.set_agent_token_path("".into());
            b.set_agent_json("".into());
            b.set_agent_toml("".into());
            b.set_agent_json_copied(false);
            b.set_agent_toml_copied(false);
            b.set_agent_allow_error("".into());
            b.set_agent_allow_input("".into());
            b.set_agent_allow_saving(false);
            b.set_agent_allow_dirty(false);
            b.set_agent_allow_list(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
            with(|s| *s = State::default());
            load(&c);
        }
    });
}

// ---------------------------------------------------------------------------
// Status / snippets
// ---------------------------------------------------------------------------

fn load(ctx: &Ctx) {
    ctx.call_with(
        |e| async move { e.agent_access().await },
        |ui, st| {
            with(|s| {
                s.draft = st.allowed_servers.clone();
                s.status = Some(st.clone());
            });
            apply_status(ui, &st);
        },
        |_ui, e| toast::err(format!("{}: {}", i18n::t("dash.agent.loadError"), err_text(&e))),
    );
}

/// Mirror a status into the bridge (`cfg-*`-style) and re-push the draft.
fn apply_status(ui: &AppWindow, st: &AgentAccessStatus) {
    let b = ui.global::<SettingsBridge>();
    b.set_agent_loaded(true);
    b.set_agent_active(st.enabled);
    b.set_agent_enabled(st.enabled);
    b.set_agent_token_path(st.token_path.as_str().into());
    b.set_agent_json(json_snippet(st).into());
    b.set_agent_toml(toml_snippet(st).into());
    push_draft(ui);
}

fn push_draft(ui: &AppWindow) {
    let b = ui.global::<SettingsBridge>();
    let (rows, dirty) = with(|s| {
        let rows: Vec<SharedString> = s.draft.iter().map(|h| SharedString::from(h.as_str())).collect();
        let dirty = s.status.as_ref().is_some_and(|st| s.draft != st.allowed_servers);
        (rows, dirty)
    });
    b.set_agent_allow_list(ModelRc::new(VecModel::from(rows)));
    b.set_agent_allow_dirty(dirty);
}

/// `JSON.stringify({ mcpServers: { espectral: { command, args, env } } }, null, 2)`
fn json_snippet(st: &AgentAccessStatus) -> String {
    // built by hand: `json!` would not guarantee the `command, args, env`
    // order and the pretty printer must match `JSON.stringify(x, null, 2)`.
    let Ok(server) = serde_json::to_value(&st.mcp) else { return String::new() };
    let mut servers = serde_json::Map::new();
    servers.insert("espectral".to_string(), server);
    let mut root = serde_json::Map::new();
    root.insert("mcpServers".to_string(), serde_json::Value::Object(servers));
    serde_json::to_string_pretty(&serde_json::Value::Object(root)).unwrap_or_default()
}

/// TOML basic string: `"` + escaped content + `"` (JS `tomlString`).
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `[mcp_servers.espectral]` block of `~/.codex/config.toml`.
fn toml_snippet(st: &AgentAccessStatus) -> String {
    let m = &st.mcp;
    let args: Vec<String> = m.args.iter().map(|a| toml_string(a)).collect();
    let mut lines = vec![
        "[mcp_servers.espectral]".to_string(),
        format!("command = {}", toml_string(&m.command)),
        format!("args = [{}]", args.join(", ")),
    ];
    if !m.env.is_empty() {
        let env: Vec<String> = m.env.iter().map(|(k, v)| format!("{k} = {}", toml_string(v))).collect();
        lines.push(format!("env = {{ {} }}", env.join(", ")));
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Toggle / rotate / copy
// ---------------------------------------------------------------------------

fn set_enabled(ctx: &Ctx, on: bool) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    if b.get_agent_saving() || !b.get_agent_loaded() {
        return;
    }
    b.set_agent_saving(true);
    ctx.call_with(
        move |e| async move { e.set_agent_access(AgentAccessPatch { enabled: Some(on), allowed_servers: None }).await },
        |ui, st| {
            with(|s| {
                s.draft = st.allowed_servers.clone();
                s.status = Some(st.clone());
            });
            let b = ui.global::<SettingsBridge>();
            b.set_agent_saving(false);
            apply_status(ui, &st);
            toast::ok(i18n::t(if st.enabled { "dash.agent.flashOn" } else { "dash.agent.flashOff" }));
        },
        |ui, e| {
            let b = ui.global::<SettingsBridge>();
            b.set_agent_saving(false);
            // the switch already moved: put it back (Svelte `agentEnabled = !agentEnabled`)
            b.set_agent_enabled(!b.get_agent_enabled());
            toast::err(err_text(&e));
        },
    );
}

fn rotate(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    if b.get_agent_rotating() {
        return;
    }
    b.set_agent_rotating(true);
    ctx.call_with(
        |e| async move { e.rotate_agent_token().await },
        |ui, st| {
            with(|s| {
                s.draft = st.allowed_servers.clone();
                s.status = Some(st.clone());
            });
            let b = ui.global::<SettingsBridge>();
            b.set_agent_rotating(false);
            b.set_agent_confirming_rotate(false);
            apply_status(ui, &st);
            toast::ok(i18n::t("dash.agent.rotateOk"));
        },
        |ui, e| {
            ui.global::<SettingsBridge>().set_agent_rotating(false);
            toast::err(err_text(&e));
        },
    );
}

fn copy(ctx: &Ctx, which: &str) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    let toml = which == "toml";
    let text = if toml { b.get_agent_toml() } else { b.get_agent_json() };
    if text.is_empty() {
        return;
    }
    util::copy_text(&text);
    if toml {
        b.set_agent_toml_copied(true);
    } else {
        b.set_agent_json_copied(true);
    }
    toast::ok(i18n::t("dash.agent.copied"));
    // `useCopy(2000)`: the check mark reverts after 2 s.
    let weak = ctx.weak();
    slint::Timer::single_shot(Duration::from_millis(2000), move || {
        if let Some(ui) = weak.upgrade() {
            let b = ui.global::<SettingsBridge>();
            if toml {
                b.set_agent_toml_copied(false);
            } else {
                b.set_agent_json_copied(false);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Allowlist editor
// ---------------------------------------------------------------------------

/// `host`, `host:port`, IPv4 or bracketed IPv6 — never a URL (`AGENT_HOST_RE`
/// plus the 1–65535 port range). `None` = invalid.
fn valid_server(raw: &str) -> Option<()> {
    if let Some(rest) = raw.strip_prefix('[') {
        let (inner, tail) = rest.split_once(']')?;
        if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
            return None;
        }
        return match tail {
            "" => Some(()),
            _ => valid_port(tail.strip_prefix(':')?),
        };
    }
    let (host, port) = match raw.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (raw, None),
    };
    if !valid_host(host) {
        return None;
    }
    match port {
        Some(p) => valid_port(p),
        None => Some(()),
    }
}

/// `[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?`
fn valid_host(host: &str) -> bool {
    let mut chars = host.chars();
    let Some(first) = chars.next() else { return false };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    let rest = &host[first.len_utf8()..];
    let Some(last) = rest.chars().last() else { return true };
    if !last.is_ascii_alphanumeric() {
        return false;
    }
    rest.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn valid_port(port: &str) -> Option<()> {
    if port.is_empty() || port.len() > 5 || !port.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u32 = port.parse().ok()?;
    (1..=65535).contains(&n).then_some(())
}

/// Public Espectral hosts: never allowlisted (the engine refuses them anyway).
///
/// Compares the host without brackets or port against `agent::BLOCKED_HOSTS`.
/// Deliberate difference: the Svelte check only strips the port of a
/// bracketed IPv6, so `play.uhce.top:25565` slipped past its UI warning (the
/// engine and the mod still refused control there).
fn blocked_host(raw: &str) -> bool {
    let host = match raw.strip_prefix('[') {
        Some(rest) => rest.split_once(']').map_or(rest, |(h, _)| h),
        None => raw.split_once(':').map_or(raw, |(h, _)| h),
    };
    let host = host.to_lowercase();
    espectral_engine::agent::BLOCKED_HOSTS
        .iter()
        .any(|suffix| host.strip_suffix(suffix).is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('.')))
}

fn add_server(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    b.set_agent_allow_error("".into());
    let raw = b.get_agent_allow_input().trim().to_lowercase();
    if raw.is_empty() {
        return;
    }
    if valid_server(&raw).is_none() {
        b.set_agent_allow_error(i18n::t("dash.agent.allowInvalid").into());
        return;
    }
    if blocked_host(&raw) {
        b.set_agent_allow_error(i18n::t("dash.agent.allowBlocked").into());
        return;
    }
    if with(|s| s.draft.contains(&raw)) {
        b.set_agent_allow_error(i18n::t("dash.agent.allowDuplicate").into());
        return;
    }
    with(|s| s.draft.push(raw));
    b.set_agent_allow_input("".into());
    push_draft(&ui);
}

fn remove_server(ctx: &Ctx, host: &str) {
    let Some(ui) = ctx.ui() else { return };
    with(|s| s.draft.retain(|h| h.as_str() != host));
    ui.global::<SettingsBridge>().set_agent_allow_error("".into());
    push_draft(&ui);
}

fn save_allowlist(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let b = ui.global::<SettingsBridge>();
    if b.get_agent_allow_saving() {
        return;
    }
    let list = with(|s| s.draft.clone());
    b.set_agent_allow_saving(true);
    ctx.call_with(
        move |e| async move {
            e.set_agent_access(AgentAccessPatch { enabled: None, allowed_servers: Some(list) }).await
        },
        |ui, st| {
            with(|s| {
                s.draft = st.allowed_servers.clone();
                s.status = Some(st.clone());
            });
            let b = ui.global::<SettingsBridge>();
            b.set_agent_allow_saving(false);
            b.set_agent_allow_error("".into());
            apply_status(ui, &st);
            toast::ok(i18n::t("dash.agent.allowSaved"));
        },
        |ui, e| {
            ui.global::<SettingsBridge>().set_agent_allow_saving(false);
            toast::err(err_text(&e));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_validation_mirrors_the_svelte_regex() {
        for ok in ["localhost", "play.example.net", "127.0.0.1:25566", "[::1]", "[::1]:25565", "a", "1.2.3.4"] {
            assert!(valid_server(ok).is_some(), "{ok} should be valid");
        }
        for bad in [
            "",
            "http://play.example.net",
            "play.example.net:",
            "play.example.net:0",
            "play.example.net:65536",
            "play.example.net:123456",
            "-host",
            "host-",
            "host.",
            "host/path",
            "[::1",
            "[]",
            "host:port",
            "a:b:c",
        ] {
            assert!(valid_server(bad).is_none(), "{bad} should be invalid");
        }
    }

    #[test]
    fn blocked_suffixes_are_never_allowlisted() {
        assert!(blocked_host("uhce.top"));
        assert!(blocked_host("play.uhce.top"));
        assert!(blocked_host("espectral.es"));
        assert!(blocked_host("play.espectral.es"));
        assert!(blocked_host("play.uhce.top:25565"));
        assert!(blocked_host("espectral.es:25566"));
        // brackets and the port of a bracketed IPv6 are stripped
        assert!(!blocked_host("[::1]"));
        assert!(!blocked_host("[::1]:25565"));
        assert!(!blocked_host("notuhce.top"));
        assert!(!blocked_host("uhce.top.example.net"));
    }

    #[test]
    fn toml_escapes_like_the_svelte_helper() {
        assert_eq!(toml_string(r#"C:\a "b""#), r#""C:\\a \"b\"""#);
    }
}
