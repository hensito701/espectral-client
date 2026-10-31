//! Small helpers: the `Util` global (pure formatting callbacks used from
//! bindings), URL opening, clipboard, PNG -> slint::Image. FOUNDATION-OWNED.

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, SharedString};

use crate::i18n::{self, Lang};
use crate::{AppState, AppWindow, Util};

/// Wire the Util global + AppState.open-url / copy-text (UI thread, once).
pub fn init(ui: &AppWindow) {
    let u = ui.global::<Util>();
    u.on_initial(|name| SharedString::from(initial(&name)));
    u.on_name_hue(|name, hue| name_hue(&name, hue));
    u.on_format_bytes(|n| SharedString::from(format_bytes(n as f64)));
    u.on_relative_time(|s, lang| SharedString::from(time_ago(&s, Lang::parse(&lang).unwrap_or(Lang::Es))));
    u.on_format_memory(|mb| SharedString::from(mem_label(mb as f64)));

    let s = ui.global::<AppState>();
    s.on_open_url(|url| open_url(&url));
    s.on_copy_text(|text| copy_text(&text));
}

/// MonogramTile letter.
pub fn initial(name: &str) -> String {
    name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into())
}

/// MonogramTile hue: explicit hue, else the JS `(hash << 5) - hash + code` hash of the name.
pub fn name_hue(name: &str, hue: i32) -> i32 {
    if hue >= 0 {
        return hue.rem_euclid(360);
    }
    let clean = if name.is_empty() { "espectral" } else { name };
    let mut hash: i32 = 0;
    for unit in clean.encode_utf16() {
        hash = hash.wrapping_shl(5).wrapping_sub(hash).wrapping_add(unit as i32);
    }
    (hash % 360).abs()
}

fn trim_zero(s: String) -> String {
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// lib/format.ts formatBytes.
pub fn format_bytes(n: f64) -> String {
    if !n.is_finite() || n < 0.0 {
        return "—".into();
    }
    if n < 1024.0 {
        return format!("{} B", n.round());
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut value = n;
    let mut unit = 0usize;
    value /= 1024.0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let num = if value >= 100.0 { format!("{}", value.round()) } else { trim_zero(format!("{value:.1}")) };
    format!("{num} {}", units[unit])
}

/// lib/format.ts memLabel.
pub fn mem_label(mb: f64) -> String {
    if !mb.is_finite() || mb <= 0.0 {
        return "—".into();
    }
    if mb >= 1024.0 && mb % 1024.0 == 0.0 {
        return format!("{} GB", mb / 1024.0);
    }
    if mb >= 1024.0 {
        return format!("{} GB", trim_zero(format!("{:.1}", mb / 1024.0)));
    }
    format!("{} MB", mb.round())
}

/// Parse an ISO-8601 date or epoch-ms string to epoch ms.
pub fn parse_time_ms(s: &str) -> Option<f64> {
    if let Ok(ms) = s.trim().parse::<f64>() {
        return Some(ms);
    }
    chrono::DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.timestamp_millis() as f64)
}

/// lib/format.ts timeAgo.
pub fn time_ago(input: &str, lang: Lang) -> String {
    let Some(ts) = parse_time_ms(input) else { return "—".into() };
    let now = chrono::Utc::now().timestamp_millis() as f64;
    let diff = now - ts;
    let tr = |k: &str| i18n::lookup(k, lang);
    if diff.abs() < 5000.0 {
        return tr("time.now");
    }
    let key = if diff < 0.0 { "time.in" } else { "time.ago" };
    let fmt = |n: f64, unit: &str| i18n::format_template(&tr(key), &[("n", &format!("{n}")), ("unit", unit)]);
    let s = (diff.abs() / 1000.0).round();
    if s < 60.0 {
        return fmt(s, "s");
    }
    let m = (s / 60.0).round();
    if m < 60.0 {
        return fmt(m, "min");
    }
    let h = (m / 60.0).round();
    if h < 24.0 {
        return fmt(h, "h");
    }
    let d = (h / 24.0).round();
    if d < 30.0 {
        return fmt(d, "d");
    }
    let mo = (d / 30.0).round();
    if mo < 12.0 {
        return fmt(mo, &tr(if mo == 1.0 { "time.unit.month.one" } else { "time.unit.month.other" }));
    }
    let y = (mo / 12.0).round();
    fmt(y, &tr(if y == 1.0 { "time.unit.year.one" } else { "time.unit.year.other" }))
}

/// Open an http(s) URL in the system browser (other schemes are refused).
pub fn open_url(url: &str) {
    let u = url.trim();
    if !(u.starts_with("https://") || u.starts_with("http://")) {
        tracing::warn!(url = u, "refusing to open non-http URL");
        return;
    }
    if let Err(e) = open::that_detached(u) {
        crate::toast::err(format!("{}: {e}", i18n::t("actions.open")));
    }
}

/// Copy text to the clipboard (through Slint's TextInput clipboard support is
/// not reachable from Rust, so this shells out to the platform tool).
pub fn copy_text(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    #[cfg(windows)]
    let candidates: &[(&str, &[&str])] = &[("clip", &[])];
    #[cfg(not(windows))]
    let candidates: &[(&str, &[&str])] =
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])];
    for (cmd, args) in candidates {
        let Ok(mut child) =
            Command::new(cmd).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
        return;
    }
    tracing::warn!("no clipboard tool found");
}

/// Decode PNG bytes to an RGBA pixel buffer (Send: decode off the UI thread,
/// then turn it into an Image on the UI thread with [`to_image`]).
pub fn decode_png(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let data = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    Some(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, w, h))
}

/// Pixel buffer -> Image (UI thread).
pub fn to_image(buf: SharedPixelBuffer<Rgba8Pixel>) -> Image {
    Image::from_rgba8(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_parity() {
        assert_eq!(format_bytes(1536.0), "1.5 KB");
        assert_eq!(format_bytes(512.0), "512 B");
        assert_eq!(format_bytes(250.0 * 1024.0 * 1024.0), "250 MB");
        assert_eq!(mem_label(3072.0), "3 GB");
        assert_eq!(mem_label(1536.0), "1.5 GB");
        assert_eq!(mem_label(512.0), "512 MB");
        assert_eq!(initial("steve"), "S");
        assert_eq!(initial(""), "?");
        assert_eq!(name_hue("x", 400), 40);
        // JS: let h=0; for (c of 'Steve') h=((h<<5)-h+c.charCodeAt(0))|0; Math.abs(h%360)
        assert_eq!(name_hue("Steve", -1), 99);
    }
}
