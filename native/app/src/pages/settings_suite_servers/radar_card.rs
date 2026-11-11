//! ServerRadarCard helpers: the MOTD § parser (lib/motd.ts) used by the Home
//! page's server strip. The statuses themselves are the shared `AppState.servers`
//! (src/store.rs polls them every 60 s while Home is shown).

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::bridge::Ctx;
use crate::{AppWindow, MotdLineRow, MotdSegment, ServerRow, SettingsBridge};

pub fn init(_ctx: &Ctx, ui: &AppWindow) {
    ui.global::<SettingsBridge>().on_motd_lines(|row| motd_lines(&row));
}

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
}
