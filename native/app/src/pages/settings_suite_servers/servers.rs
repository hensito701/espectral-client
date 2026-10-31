//! ServerRadar (#/servers) + the pure helpers of ServerRadarCard / PingWaveform.
//!
//! The list itself is the shared `AppState.servers` (src/store.rs polls it
//! every 60 s while Home / Servers is shown); this module adds the manual
//! re-scan, the aggregates, the MOTD § parser and the waveform geometry.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::err_text;
use crate::bridge::Ctx;
use crate::{
    i18n, store, toast, AppWindow, MotdLineRow, MotdSegment, PingWaveGeometry, ServerGridRow, ServerRadarSummary,
    ServerRow, SettingsBridge,
};

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    let b = ui.global::<SettingsBridge>();

    let c = ctx.clone();
    b.on_servers_refresh(move || {
        let Some(ui) = c.ui() else { return };
        let b = ui.global::<SettingsBridge>();
        if b.get_servers_refreshing() {
            return;
        }
        b.set_servers_refreshing(true);
        c.call_with(
            |e| async move { e.server_statuses().await },
            |ui, _reply| {
                ui.global::<SettingsBridge>().set_servers_refreshing(false);
                // the engine just cached the fresh statuses: the shared store re-reads them
                store::refresh_servers();
                toast::ok(i18n::tf("servers.lastUpdated", &[("time", &i18n::t("time.now"))]));
            },
            |ui, e| {
                ui.global::<SettingsBridge>().set_servers_refreshing(false);
                toast::err(format!("{}: {}", i18n::t("servers.errorTitle"), err_text(&e)));
            },
        );
    });

    b.on_servers_summary(|rows| summary(&rows));
    b.on_motd_lines(|row| motd_lines(&row));
    b.on_server_grid(|rows, cols| grid(&rows, cols));
    b.on_ping_wave(ping_wave);
}

fn summary(rows: &ModelRc<ServerRow>) -> ServerRadarSummary {
    let mut total = 0;
    let mut online = 0;
    let mut players = 0;
    let mut last: Option<f64> = None;
    for r in rows.iter() {
        total += 1;
        if r.online {
            online += 1;
            players += r.players_online.max(0);
        }
        if let Ok(ms) = r.fetched_at.parse::<f64>() {
            last = Some(last.map_or(ms, |l: f64| l.max(ms)));
        }
    }
    let health = if total == 0 {
        "offline"
    } else if online == total {
        "optimal"
    } else if online > 0 {
        "degraded"
    } else {
        "offline"
    };
    ServerRadarSummary {
        total,
        online,
        offline: total - online,
        players,
        health: health.into(),
        last_fetched: last.map(|ms| format!("{}", ms as i64)).unwrap_or_default().into(),
    }
}

/// Rows of `cols` cards (CSS grid auto-fill).
fn grid(rows: &ModelRc<ServerRow>, cols: i32) -> ModelRc<ServerGridRow> {
    let cols = cols.max(1) as usize;
    let all: Vec<ServerRow> = rows.iter().collect();
    let out: Vec<ServerGridRow> =
        all.chunks(cols).map(|c| ServerGridRow { items: ModelRc::new(VecModel::from(c.to_vec())) }).collect();
    ModelRc::new(VecModel::from(out))
}

// ---- MOTD (lib/motd.ts) ----------------------------------------------------------

fn mc_color(code: char) -> Option<slint::Color> {
    let hex: u32 = match code {
        '0' => 0x000000,
        '1' => 0x0000aa,
        '2' => 0x00aa00,
        '3' => 0x00aaaa,
        '4' => 0xaa0000,
        '5' => 0xaa00aa,
        '6' => 0xffaa00,
        '7' => 0xaaaaaa,
        '8' => 0x555555,
        '9' => 0x5555ff,
        'a' => 0x55ff55,
        'b' => 0x55ffff,
        'c' => 0xff5555,
        'd' => 0xff55ff,
        'e' => 0xffff55,
        'f' => 0xffffff,
        _ => return None,
    };
    Some(slint::Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8))
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Style {
    color: Option<char>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

/// parseMotd: split one raw line at § codes into styled runs.
fn parse_motd(raw: &str) -> Vec<(String, Style)> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut st = Style::default();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '§' {
            text.push(ch);
            continue;
        }
        let Some(code) = chars.next() else {
            text.push(ch);
            break;
        };
        let c = code.to_ascii_lowercase();
        let flush = |text: &mut String, out: &mut Vec<(String, Style)>, st: &Style| {
            if !text.is_empty() {
                out.push((std::mem::take(text), st.clone()));
            }
        };
        match c {
            '0'..='9' | 'a'..='f' => {
                flush(&mut text, &mut out, &st);
                st.color = Some(c);
            }
            'l' => {
                flush(&mut text, &mut out, &st);
                st.bold = true;
            }
            'o' => {
                flush(&mut text, &mut out, &st);
                st.italic = true;
            }
            'n' => {
                flush(&mut text, &mut out, &st);
                st.underline = true;
            }
            'm' => {
                flush(&mut text, &mut out, &st);
                st.strike = true;
            }
            'r' => {
                flush(&mut text, &mut out, &st);
                st = Style::default();
            }
            // §k (obfuscated) and unknown codes are dropped
            _ => {}
        }
    }
    if !text.is_empty() {
        out.push((text, st));
    }
    out
}

fn seg(text: &str, st: &Style) -> MotdSegment {
    let color = st.color.and_then(mc_color);
    MotdSegment {
        text: text.into(),
        color: color.unwrap_or_default(),
        has_color: color.is_some(),
        bold: st.bold,
        italic: st.italic,
        underline: st.underline,
        strike: st.strike,
    }
}

fn line(segments: Vec<MotdSegment>) -> MotdLineRow {
    MotdLineRow { segments: ModelRc::new(VecModel::from(segments)) }
}

/// ServerRadarCard motdHtml: raw (§ colors) > clean lines > hostname/host fallback.
fn motd_lines(row: &ServerRow) -> ModelRc<MotdLineRow> {
    let raw: Vec<SharedString> = row.motd_raw.iter().collect();
    let lines: Vec<MotdLineRow> = if !raw.is_empty() {
        raw.iter().map(|l| line(parse_motd(l).iter().map(|(t, st)| seg(t, st)).collect())).collect()
    } else {
        let clean: Vec<SharedString> = row.motd_clean.iter().collect();
        if !clean.is_empty() {
            clean.iter().map(|l| line(vec![seg(l, &Style::default())])).collect()
        } else {
            let host = if row.hostname.is_empty() { &row.host } else { &row.hostname };
            vec![line(vec![seg(host, &Style::default())])]
        }
    };
    ModelRc::new(VecModel::from(lines))
}

// ---- PingWaveform -----------------------------------------------------------------

/// Deterministic pseudo-waveform seeded by the ping (components/PingWaveform.svelte).
fn ping_wave(ping: i32, online: bool, width: f32, height: f32) -> PingWaveGeometry {
    const POINTS: usize = 7;
    let (pad_x, pad_y) = (4.0_f64, 4.0_f64);
    let (w, h) = (width.max(9.0) as f64, height.max(9.0) as f64);
    let inner_w = w - pad_x * 2.0;
    let inner_h = h - pad_y * 2.0;
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(POINTS);
    let line;
    if !online || ping < 0 {
        // offline: flat dashed baseline (Path has no dash array: emit dash segments)
        let y = h - pad_y;
        let x0 = pad_x;
        let x1 = w - pad_x;
        let mut d = String::new();
        let mut x = x0;
        while x < x1 {
            let e = (x + 3.0).min(x1);
            d.push_str(&format!("M {x:.2} {y:.2} L {e:.2} {y:.2} "));
            x += 6.0;
        }
        pts.push((x1, y));
        line = d.trim_end().to_string();
        let area = format!("M {x0:.2} {y:.2} L {x1:.2} {y:.2} L {:.2} {h:.2} L {x0:.2} {h:.2} Z", x1);
        let (lx, ly) = pts[0];
        return PingWaveGeometry { line: line.into(), area: area.into(), dot_x: lx as f32, dot_y: ly as f32 };
    }
    let ping = ping as f64;
    let mut seed = (ping * 137.0 + 49297.0).floor().abs();
    let mut next = || {
        seed = (seed * 9301.0 + 49297.0) % 233280.0;
        seed / 233280.0
    };
    let variance = (ping * 0.12).max(2.0).min(inner_h * 0.4);
    for i in 0..POINTS {
        let x = pad_x + (inner_w / (POINTS - 1) as f64) * i as f64;
        let jitter = (next() - 0.5) * variance;
        let y = (inner_h * 0.5 + jitter).clamp(pad_y, h - pad_y);
        pts.push((x, y));
    }
    let mut d = format!("M {:.2} {:.2}", pts[0].0, pts[0].1);
    for win in pts.windows(2) {
        let ((cx, cy), (nx, ny)) = (win[0], win[1]);
        let cp = (cx + nx) / 2.0;
        d.push_str(&format!(" C {cp:.2} {cy:.2} {cp:.2} {ny:.2} {nx:.2} {ny:.2}"));
    }
    line = d;
    let area = format!("{line} L {:.2} {h:.2} L {pad_x:.2} {h:.2} Z", w - pad_x);
    let (lx, ly) = *pts.last().unwrap_or(&(0.0, 0.0));
    PingWaveGeometry { line: line.into(), area: area.into(), dot_x: lx as f32, dot_y: ly as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motd_parses_colors_and_formats() {
        let segs = parse_motd("§aGreen §lBold§r plain");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].0, "Green ");
        assert_eq!(segs[0].1.color, Some('a'));
        assert!(segs[1].1.bold && segs[1].1.color == Some('a'));
        let keep = parse_motd("§lB§cR");
        assert!(keep[1].1.bold && keep[1].1.color == Some('c'));
        assert_eq!(segs[2].1, Style::default());
    }

    #[test]
    fn wave_is_deterministic_and_in_bounds() {
        let a = ping_wave(42, true, 84.0, 24.0);
        let b = ping_wave(42, true, 84.0, 24.0);
        assert_eq!(a.line, b.line);
        assert!(a.dot_y >= 4.0 && a.dot_y <= 20.0);
        let off = ping_wave(-1, false, 84.0, 24.0);
        assert!(off.line.starts_with("M 4.00 20.00"));
    }
}
