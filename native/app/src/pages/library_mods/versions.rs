//! VersionArmory (`#/library/versions`): Mojang manifest (engine
//! `list_versions`, cached 6 h by the engine), search + type filter,
//! instance cross references, chunked rendering (100 rows, +100 near the end).

use std::cell::RefCell;
use std::collections::BTreeMap;

use espectral_engine::types::VersionManifest;
use slint::ComponentHandle;

use super::{model, ss};
use crate::bridge::Ctx;
use crate::i18n::{self, Lang};
use crate::{router, store, toast, util, AppWindow, LibraryBridge, Shell, VersionRow};

const CHUNK: usize = 100;

#[derive(Default)]
struct State {
    manifest: Option<VersionManifest>,
    loading: bool,
    error: String,
    limit: usize,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State { limit: CHUNK, ..Default::default() });
}

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    super::set_ctx(ctx);
    let b = ui.global::<LibraryBridge>();
    b.on_v_set_query(|_q| {
        S.with(|s| s.borrow_mut().limit = CHUNK);
        if let Some(ui) = super::ui_handle() {
            recompute(&ui);
        }
    });
    b.on_v_set_filter(|f| {
        S.with(|s| s.borrow_mut().limit = CHUNK);
        if let Some(ui) = super::ui_handle() {
            ui.global::<LibraryBridge>().set_v_filter(f);
            recompute(&ui);
        }
    });
    b.on_v_load_more(|| {
        S.with(|s| s.borrow_mut().limit += CHUNK);
        if let Some(ui) = super::ui_handle() {
            recompute(&ui);
        }
    });
    b.on_v_retry(|| {
        if let Some(ui) = super::ui_handle() {
            fetch(&ui);
        }
    });
    b.on_v_copy(|id| {
        util::copy_text(&id);
        toast::ok(i18n::tf("versions.copiedVersion", &[("version", &id)]));
    });
    b.on_v_create(|id| {
        let Some(ui) = super::ui_handle() else { return };
        router::navigate(&ui, "#/");
        // The Svelte horizon:open-wizard event carried { version }; the wizard
        // reads Shell.wizard-version on open and preselects it.
        ui.global::<Shell>().set_wizard_version(id.clone());
        ui.global::<Shell>().set_wizard_open(true);
        toast::info(format!("{} ({id})", i18n::t("versions.openInWizard")));
    });
}

pub fn on_enter(ui: &AppWindow) {
    if super::debug::has("versions-fake") {
        S.with(|s| s.borrow_mut().manifest = Some(super::debug::fake_manifest()));
    }
    let need = S.with(|s| {
        let s = s.borrow();
        s.manifest.is_none() && !s.loading
    });
    S.with(|s| s.borrow_mut().limit = CHUNK);
    if need {
        fetch(ui);
    } else {
        recompute(ui);
    }
}

fn fetch(ui: &AppWindow) {
    let Some(c) = super::ctx() else { return };
    S.with(|s| {
        let mut s = s.borrow_mut();
        s.loading = true;
        s.error.clear();
    });
    recompute(ui);
    c.call_with(
        |e| async move { e.list_versions().await },
        |ui, manifest| {
            S.with(|s| {
                let mut s = s.borrow_mut();
                s.manifest = Some(manifest);
                s.loading = false;
            });
            recompute(ui);
        },
        |ui, err| {
            S.with(|s| {
                let mut s = s.borrow_mut();
                s.loading = false;
                s.error = err.envelope().message;
            });
            recompute(ui);
        },
    );
}

/// Filter the manifest and push the visible slice + counters to the bridge.
pub fn recompute(ui: &AppWindow) {
    let b = ui.global::<LibraryBridge>();
    let query = b.get_v_query().trim().to_lowercase();
    let filter = b.get_v_filter();
    let lang = i18n::lang();

    // version id -> instance names
    let mut installed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for inst in store::instances() {
        if !inst.version.is_empty() {
            installed.entry(inst.version.clone()).or_default().push(inst.name.clone());
        }
    }

    S.with(|s| {
        let s = s.borrow();
        b.set_v_loading(s.loading);
        b.set_v_error(ss(&s.error));
        b.set_v_has_manifest(s.manifest.is_some());
        b.set_v_installed_versions(installed.len() as i32);
        let Some(m) = s.manifest.as_ref() else {
            b.set_v_total(0);
            b.set_v_latest_release(ss(""));
            b.set_v_latest_snapshot(ss(""));
            b.set_v_rows(model(Vec::new()));
            b.set_v_filtered(0);
            b.set_v_has_more(false);
            return;
        };
        b.set_v_total(m.versions.len() as i32);
        b.set_v_latest_release(ss(&m.latest_release));
        b.set_v_latest_snapshot(ss(&m.latest_snapshot));

        let filtered: Vec<_> = m
            .versions
            .iter()
            .filter(|v| match filter {
                5 => installed.contains_key(&v.id),
                1 => v.kind == "release",
                2 => v.kind == "snapshot",
                3 => v.kind == "old_beta",
                4 => v.kind == "old_alpha",
                _ => true,
            })
            .filter(|v| {
                query.is_empty()
                    || v.id.to_lowercase().contains(&query)
                    || v.kind.to_lowercase().contains(&query)
                    || v.release_time.contains(&query)
            })
            .collect();
        let rows: Vec<VersionRow> = filtered
            .iter()
            .take(s.limit)
            .map(|v| VersionRow {
                id: ss(&v.id),
                kind: ss(&v.kind),
                date: ss(format_date(&v.release_time, lang)),
                java: ss(java_requirement(&v.id)),
                installed_in: ss(installed.get(&v.id).map(|n| n.join(", ")).unwrap_or_default()),
                latest_release: m.latest_release == v.id,
                latest_snapshot: m.latest_snapshot == v.id,
            })
            .collect();
        b.set_v_filtered(filtered.len() as i32);
        b.set_v_has_more(s.limit < filtered.len());
        b.set_v_rows(model(rows));
    });
}

/// VersionArmory getJavaRequirement (Mojang JDK policy).
pub fn java_requirement(id: &str) -> &'static str {
    if let Some(rest) = id.strip_prefix("1.") {
        let mut parts = rest.split(|c: char| !c.is_ascii_digit());
        let major: Option<u32> = parts.next().and_then(|p| p.parse().ok());
        if let Some(major) = major {
            // `1.20.5`: the minor follows a '.' directly after the major digits
            let after = &rest[major.to_string().len()..];
            let minor: u32 = after
                .strip_prefix('.')
                .map(|m| m.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
                .and_then(|m| m.parse().ok())
                .unwrap_or(0);
            if major > 20 || (major == 20 && minor >= 5) {
                return "21";
            }
            if major >= 18 {
                return "17";
            }
            if major == 17 {
                return "16";
            }
            return "8";
        }
    }
    "21"
}

/// `toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })`.
pub fn format_date(iso: &str, lang: Lang) -> String {
    if iso.is_empty() {
        return "—".into();
    }
    let date = chrono::DateTime::parse_from_rfc3339(iso)
        .map(|d| d.date_naive())
        .or_else(|_| chrono::NaiveDate::parse_from_str(iso.get(..10).unwrap_or(iso), "%Y-%m-%d"));
    let Ok(d) = date else {
        return iso.split('T').next().unwrap_or(iso).to_string();
    };
    use chrono::Datelike;
    let m = d.month0() as usize;
    match lang {
        Lang::Es => {
            const M: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"];
            format!("{} {} {}", d.day(), M[m], d.year())
        }
        Lang::En => {
            const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{} {}, {}", M[m], d.day(), d.year())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jdk_policy() {
        assert_eq!(java_requirement("1.21.1"), "21");
        assert_eq!(java_requirement("1.20.5"), "21");
        assert_eq!(java_requirement("1.20.4"), "17");
        assert_eq!(java_requirement("1.18"), "17");
        assert_eq!(java_requirement("1.17.1"), "16");
        assert_eq!(java_requirement("1.16.5"), "8");
        assert_eq!(java_requirement("1.8.9"), "8");
        assert_eq!(java_requirement("24w14a"), "21");
        assert_eq!(java_requirement("26.3"), "21");
        assert_eq!(java_requirement("b1.7.3"), "21");
    }

    #[test]
    fn dates() {
        assert_eq!(format_date("2024-06-13T08:24:03+00:00", Lang::Es), "13 jun 2024");
        assert_eq!(format_date("2024-06-13T08:24:03+00:00", Lang::En), "Jun 13, 2024");
        assert_eq!(format_date("", Lang::Es), "—");
    }
}
