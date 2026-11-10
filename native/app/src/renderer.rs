//! Which renderer draws the window, with an automatic software fallback.
//! FOUNDATION-OWNED.
//!
//! The window draws with femtovg (OpenGL) on a normal PC. Slint's software
//! renderer is picked in two cases:
//!
//! * low memory: at or below [`LOW_MEMORY_LIMIT_BYTES`] it is the default.
//!   Measured on the same 1280x800 window, software uses 20-26 MB instead of
//!   142-150 MB (RAM the game can use) and shows the window in 74-100 ms
//!   instead of 250-260 ms;
//! * OpenGL failure: on some Windows PCs the driver passes Slint's startup
//!   probe but then fails to create the real window's context, loses it, or
//!   crashes inside the driver: the launcher never shows a window (the
//!   WebView-based launcher before 1.7 did not use OpenGL, so those PCs only
//!   broke with the native app). Two checks catch it:
//!
//!   - the event loop ends with an error on OpenGL -> main.rs calls
//!     [`remember_software`] and [`relaunch_software`]: the launcher reopens
//!     with Slint's software renderer;
//!   - an OpenGL start that never drew its first frame (the driver killed or
//!     hung the process) leaves [`PENDING_PREF`] behind; the next start sees it
//!     and switches to software.
//!
//! The failure switch holds for the current launcher version only: after an
//! update the launcher tries OpenGL again. A `SLINT_BACKEND` set outside the
//! launcher always wins (and turns both checks off). The low-memory default
//! applies on every start.

use serde_json::{json, Value};
use slint::{ComponentHandle, RenderingState};

use crate::{prefs, AppWindow};

/// `{ "version", "reason" }`: this launcher version draws with software.
const FALLBACK_PREF: &str = "renderer_fallback";
/// Launcher version whose OpenGL window has not drawn its first frame yet.
const PENDING_PREF: &str = "renderer_gl_pending";
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// 8.5 GiB: the top of the launcher's lowest automatic heap tier —
/// `espectral_engine::config::recommended_memory_mb_for` gives 2048 MB for
/// this much RAM and less, more above it.
const LOW_MEMORY_LIMIT_BYTES: u64 = 9_126_805_504;

/// The renderer this process uses.
#[derive(Debug, Clone, Copy)]
pub struct Renderer {
    /// OpenGL, chosen by the launcher: a failure may switch to software.
    pub can_fall_back: bool,
}
/// True when a PC's RAM is known and at or below [`LOW_MEMORY_LIMIT_BYTES`].
fn is_low_memory_pc(total_bytes: u64) -> bool {
    total_bytes > 0 && total_bytes <= LOW_MEMORY_LIMIT_BYTES
}

/// Slint's software renderer on the winit window backend; Ok when selected.
fn select_software() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new().backend_name("winit".into()).renderer_name("software".into()).select()
}

/// Pick the renderer before the first window exists (prefs must be loaded).
pub fn select() -> Renderer {
    if let Some(backend) = std::env::var_os("SLINT_BACKEND") {
        tracing::info!("[renderer] SLINT_BACKEND={} (set outside the launcher)", backend.to_string_lossy());
        return Renderer { can_fall_back: false };
    }
    if let Some(pending) = prefs::get_str(PENDING_PREF) {
        if pending == VERSION {
            remember_software("the previous start never drew its first OpenGL frame (graphics driver crash or hang)");
        }
        prefs::set(PENDING_PREF, Value::Null);
    }
    let fallback = prefs::get(FALLBACK_PREF).filter(|f| f.get("version").and_then(Value::as_str) == Some(VERSION));
    let Some(fallback) = fallback else {
        // Low-memory PCs start on software: it leaves ~120 MB for the game and
        // the window appears sooner. Selection is best-effort — if Slint
        // refuses, OpenGL below is the fallback.
        let total = espectral_engine::platform::total_memory_bytes();
        if is_low_memory_pc(total) {
            let gb = total as f64 / 1024f64.powi(3);
            match select_software() {
                Ok(()) => {
                    tracing::info!("[renderer] software (low-memory PC: {gb:.1} GB RAM)");
                    return Renderer { can_fall_back: false };
                }
                Err(e) => tracing::warn!("[renderer] software renderer unavailable ({e}); trying OpenGL"),
            }
        }
        tracing::info!("[renderer] OpenGL (femtovg)");
        return Renderer { can_fall_back: true };
    };
    let reason = fallback.get("reason").and_then(Value::as_str).unwrap_or("unknown");
    match select_software() {
        Ok(()) => tracing::info!("[renderer] software (OpenGL failed on this PC: {reason})"),
        Err(e) => tracing::warn!("[renderer] software renderer unavailable ({e}); using the default"),
    }
    Renderer { can_fall_back: false }
}

/// OpenGL start: arm the "never drew a frame" marker; the first rendered
/// frame clears it. Call right before the window is shown.
pub fn watch_first_frame(ui: &AppWindow) {
    prefs::set(PENDING_PREF, Value::from(VERSION));
    let mut armed = true;
    let watched = ui.window().set_rendering_notifier(move |state, _| {
        if armed && matches!(state, RenderingState::AfterRendering) {
            armed = false;
            prefs::set(PENDING_PREF, Value::Null);
            tracing::info!("[renderer] first OpenGL frame drawn");
        }
    });
    if let Err(e) = watched {
        prefs::set(PENDING_PREF, Value::Null);
        tracing::warn!("[renderer] first-frame check unavailable: {e}");
    }
}

/// Draw with the software renderer from now on (this launcher version).
pub fn remember_software(reason: &str) {
    let reason: String = reason.chars().take(300).collect();
    tracing::warn!("[renderer] switching to the software renderer: {reason}");
    prefs::set(FALLBACK_PREF, json!({ "version": VERSION, "reason": reason }));
}

/// Start a new launcher on the software renderer. Its `SLINT_BACKEND` also
/// keeps it from relaunching again if that fails too.
pub fn relaunch_software() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).env("SLINT_BACKEND", "winit-software").spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::is_low_memory_pc;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn low_memory_pc_threshold() {
        assert!(!is_low_memory_pc(0));
        assert!(is_low_memory_pc(4 * GIB));
        assert!(is_low_memory_pc(8 * GIB));
        assert!(is_low_memory_pc(8 * GIB + GIB / 2));
        assert!(!is_low_memory_pc(8 * GIB + GIB / 2 + 1));
        assert!(!is_low_memory_pc(16 * GIB));
    }
}
