//! Native file / folder pickers (replaces POST /api/pick-file, /api/pick-folder
//! and @tauri-apps/plugin-dialog). FOUNDATION-OWNED.
//!
//! All functions are async and must be awaited OFF the UI thread (inside a
//! `ctx.call(...)` / `ctx.spawn(...)` future):
//!
//! ```ignore
//! ctx.call(
//!     |e| async move {
//!         let Some(path) = dialogs::pick_file("Modpack", &[("Modrinth", &["mrpack"])]).await else { return Ok(None) };
//!         e.import_mrpack(ImportMrpackRequest { path: path.display().to_string(), memory_mb: None }).await.map(Some)
//!     },
//!     |ui, reply| { ... },
//! );
//! ```

// Public API for page modules: not every helper is used by the shell itself.
#![allow(dead_code)]

use std::path::PathBuf;

/// Pick one file. `filters`: (name, extensions without dot).
pub async fn pick_file(title: &str, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    let mut d = rfd::AsyncFileDialog::new().set_title(title);
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    d.pick_file().await.map(|h| h.path().to_path_buf())
}

/// Pick several files.
pub async fn pick_files(title: &str, filters: &[(&str, &[&str])]) -> Vec<PathBuf> {
    let mut d = rfd::AsyncFileDialog::new().set_title(title);
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    d.pick_files().await.map(|v| v.into_iter().map(|h| h.path().to_path_buf()).collect()).unwrap_or_default()
}

/// Pick a folder.
pub async fn pick_folder(title: &str) -> Option<PathBuf> {
    rfd::AsyncFileDialog::new().set_title(title).pick_folder().await.map(|h| h.path().to_path_buf())
}

/// Ask for a save location.
pub async fn save_file(title: &str, file_name: &str, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    let mut d = rfd::AsyncFileDialog::new().set_title(title).set_file_name(file_name);
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    d.save_file().await.map(|h| h.path().to_path_buf())
}

/// Blocking error box (startup failures, before/without the event loop).
pub fn fatal(title: &str, message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
