//! Compiles native/app/ui/app.slint to Rust (included by src/lib.rs).
//!
//! The app crate's build.rs keeps the i18n tables, the missing-key lint and
//! the Windows resources; only the Slint step lives here so that the large
//! generated module is a separate compilation unit.

use std::path::PathBuf;

fn main() {
    // Live preview makes the binary read the .slint sources from this
    // checkout at run time: a shipped build must never do that.
    if std::env::var_os("CARGO_FEATURE_LIVE_PREVIEW").is_some() && std::env::var("PROFILE").as_deref() == Ok("release") {
        panic!("the live-preview feature is for dev builds only; build release without it");
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let app_slint = manifest.join("../app/ui/app.slint");
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config(app_slint, config).expect("slint build failed");
}
