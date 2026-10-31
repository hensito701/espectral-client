//! Headless engine: `Engine::start` (loopback API + agent API on 127.0.0.1:4199)
//! without the Slint app, so engine work (new versions/loaders) can be tested
//! in-game while only the light engine crate is rebuilt.
//!
//! ```sh
//! ESPECTRAL_DATA_DIR=<data-dir> cargo run -p espectral-engine --example headless
//! ```
//!
//! Drive it with the agent API (`POST /api/agent/launch`, see docs/agent-access.md).
//! Ctrl-C shuts the engine down; running games keep running.

use espectral_engine::{Engine, EngineOptions};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("ESPECTRAL_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    let engine = Engine::start(EngineOptions {
        data_dir: Some(espectral_engine::paths::resolve_data_dir()),
        ..Default::default()
    })
    .await
    .expect("engine failed to start");
    let _ = tokio::signal::ctrl_c().await;
    engine.shutdown().await;
}
